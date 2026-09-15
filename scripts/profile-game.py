#!/usr/bin/env python3
import argparse
import contextlib
import csv
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("capture_frame", Path(__file__).with_name("capture-frame.py"))
capture = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(capture)
KINDS = ("simulation", "draw", "presentation", "present_wait", "frame_interval")
PRESENTER_COUNTERS = ("acquire_ns", "acquire_block_ns", "reconfigure_count", "present_record_ns",
                      "submit_ns", "replay_ns", "allocations", "allocated_bytes")
REPLAY_PHASES = ("replay_pack_ns", "replay_upload_ns", "replay_bind_ns", "replay_encode_ns",
                 "replay_tile_index_ns", "replay_other_ns", "replay_submit_wait_ns")
REPLAY_COUNTS = ("replay_bind_groups", "replay_buffers", "replay_passes", "replay_staged_bytes")
UPLOAD_FIELDS = ('upload_queue_writes', 'upload_queue_bytes', 'upload_queued_bytes', 'upload_ring_overflows', 'upload_overflow_bytes', 'upload_oversized_frames', 'upload_padding_bytes', 'upload_records_capacity', 'upload_records_used', 'upload_records_high_water', 'upload_indices_capacity', 'upload_indices_used', 'upload_indices_high_water', 'upload_uniforms_capacity', 'upload_uniforms_used', 'upload_uniforms_high_water', 'upload_arena_dirty_bytes')
UPLOAD_LABELS = ('record_ring', 'index_ring', 'uniform_ring', 'immutable_ordered_commands', 'ordered_tile_lists', 'drawing_dimensions', 'ordered_sprite_commands', 'sprite_target_dimensions', 'ordered_sprite_layer', 'minimap_target_view', 'shadow_arena_region', 'snapshot_triangle_commands', 'snapshot_triangle_tiles', 'snapshot_image_commands', 'snapshot_image_tile_lists', 'immutable_gpoly_vertices', 'gpoly_viewport', 'gpoly_row_layout', 'arena_assets', 'snapshot_tables', 'arena_flush', 'immutable_asset_versions', 'triangle_immutable_assets', 'snapshot_triangle_fallback_assets', 'immutable_lens_sources_and_maps', 'gpu_snapshot_sampling_arena', 'ordered_sprite_identity_layer', 'sprite_artwork_and_run_boundaries', 'minimap_semantic_cells_and_styles', 'immutable_shadow_artwork', 'persistent_asset_arena', 'effect_target_view', 'compatibility')
UPLOAD_METRICS = ('creates', 'create_bytes', 'reservations', 'payload_bytes', 'writes', 'write_bytes')
UPLOAD_COUNTERS = UPLOAD_FIELDS + tuple(f"upload_{label}_{metric}" for label in UPLOAD_LABELS for metric in UPLOAD_METRICS)
REPLAY_COUNTERS = REPLAY_PHASES + REPLAY_COUNTS + UPLOAD_COUNTERS
DRAW_KINDS = ("draw_scene", "draw_raster", "draw_front_raster", "draw_overlays")
ARENA_RESOURCE_KINDS = ('sprite', 'ordered_sprite', 'cursor', 'trig', 'terrain_tile', 'terrain_fade', 'native_table', 'minimap', 'shadow', 'target_trig_geometry', 'target_trig_table', 'image', 'raw_image', 'tiled_image', 'movie', 'map_view', 'bitmap', 'lens', 'other')
ARENA_KIND_METRICS = ('bytes', 'misses', 'hits', 'source_bytes', 'distinct_lengths', 'length_overflows')
ARENA_KIND_COUNTERS = tuple(f"arena_{kind}_{metric}" for kind in ARENA_RESOURCE_KINDS for metric in ARENA_KIND_METRICS)
DRAWING_COUNTERS = ("submits", "dispatches", "waits", "wait_ns", "checkpoints",
                    "checkpoint_copy_bytes", "validation_waits",
                    "flagged_invalid_frames", "status_stalls", "asset_upload_bytes", "command_upload_bytes", "upload_bytes", "readback_bytes",
                    "full_readbacks", "full_readback_bytes", "buffers", "buffer_bytes",
                    "batches", "commands", "ordered_sprites",
                    "ordered_sprite_layers", "ordered_sprite_passes",
                    "arena_evictions", "arena_overflows", "arena_bytes_uploaded",
                    "tile_allocations", "tile_entries",
                    "bridge_target_flushes", "bridge_target_runs",
                    "terrain_tile_entries", "prepared_row_words", "prepared_row_allocations",
                    "tile_entries_clear", "tile_entries_rect", "tile_entries_image", "tile_entries_gpoly_span",
                    "tile_entries_circle_filled", "tile_entries_circle_outline", "tile_entries_sprite", "tile_entries_raw_image",
                    "tile_entries_tiled_image", "tile_entries_trig", "tile_entries_lens", "tile_entries_shadow",
                    "tile_entries_reserved12", "tile_entries_movie", "tile_entries_map_view", "tile_entries_bitmap",
                    "tile_entries_transition", "tile_entries_terrain_tri",
                    "gpu_raster_ns", "gpu_terrain_prepare_ns",
                    "gpu_shadow_mask_ns", "gpu_target_trig_ns", "gpu_ordered_sprite_ns",
                    "gpu_minimap_ns", "gpu_lens_ns", "gpu_present_ns",
                    "gpu_timed_passes", "gpu_untimed_passes", "gpu_pass_union_ns",
                    "target_trig_geometry_bytes",
                    "target_trig_table_bytes",
                    "other_asset_upload_bytes",
                    "target_trig_table_hits",
                    "target_trig_table_misses",
                    "target_trig_asset_buffers",
                    "shadow_pairs",
                    "preparer_buffers",
                    "preparer_buffer_bytes",
                    "arena_misses_new_id",
                    "arena_misses_forget",
                    "arena_misses_size_class",
                    "arena_misses_generation",
                    "arena_misses_eviction",
                    "arena_miss_new_id_bytes",
                    "arena_miss_forget_bytes",
                    "arena_miss_size_class_bytes",
                    "arena_miss_generation_bytes",
                    "arena_miss_eviction_bytes",
                    "arena_explicit_forgets",
                    *ARENA_KIND_COUNTERS, "arena_trig_texture_source_bytes", *REPLAY_COUNTERS,
                    "host_staged_asset_bytes", "arena_bytes_resident", "arena_scratch_bytes_peak",
                    "arena_capacity_bytes",
                    "arena_live_bytes",
                    "arena_retired_bytes",
                    "arena_growth_peak_bytes")
UPLOAD_GAUGES = ('upload_records_capacity', 'upload_records_used', 'upload_records_high_water', 'upload_indices_capacity', 'upload_indices_used', 'upload_indices_high_water', 'upload_uniforms_capacity', 'upload_uniforms_used', 'upload_uniforms_high_water')
DRAWING_GAUGES = (*UPLOAD_GAUGES, "host_staged_asset_bytes", "arena_bytes_resident", "arena_scratch_bytes_peak",
                  "arena_capacity_bytes",
                  "arena_live_bytes",
                  "arena_retired_bytes",
                  "arena_growth_peak_bytes")
SETTINGS = {
    "DELTA_TIME": "ON", "TURNS_PER_SECOND": "20", "FRAMES_PER_SECOND": "60", "VSYNC": "OFF",
    "FREEZE_GAME_ON_FOCUS_LOST": "OFF", "CAPTURE_CURSOR": "OFF",
    "CURSOR_EDGE_CAMERA_PANNING": "OFF", "LOCK_CURSOR_IN_POSSESSION": "OFF",
}
TIMING_LOCK_PATH = "/private/tmp/keeperfx-timing.lock"
DEFAULT_MAX_LOAD = 0.5
CAPPED_FPS_LIMIT = 60
UNCAPPED_FPS_LIMIT = 0
CAPPED_LIMITATION = "The 60 FPS cap limits observed frame rate; lower presentation duration is not an uncapped gameplay FPS speedup."
UNCAPPED_LIMITATIONS = [
    "UNCAPPED RUN: FRAMES_PER_SECOND=0 removes the engine frame limiter, so drawing runs as fast as the host allows. Its FPS figures are not comparable with capped runs.",
    "Uncapped figures are host wall-clock pacing of this process on this host under these conditions, not a portable frame-rate claim.",
    "The simulation still targets 20 turns per second. A measured turns-per-second below 20 means the host could not sustain the simulation, so the run does not measure a drawing ceiling.",
    "VSync stays off and the presenter must report a non-VSync present mode; an uncapped run behind VSync would measure the display, not the engine.",
]
OFFSCREEN_LIMITATION = ("OFFSCREEN RUN: there is no swapchain, so presentation, present_wait, frame_interval and "
                        "observed FPS are not comparable with a windowed run. Drawing counters and per-pass GPU "
                        "timestamps are.")
LIMITATIONS = [
    "Per-scope timings are monotonic wall-clock durations, including scheduling and blocking; they are not CPU-time counters.",
    "GPU execution time is collected only when KFX_WGPU_GPU_TIMING is 1 or 2 and the adapter supports timestamp queries: the gpu_*_ns drawing counters are per-pass GPU durations. Presentation and present_wait remain host-side wall clock.",
    "Presentation includes present_wait; these overlapping scopes must not be added together.",
    "Frame intervals measure observed presentation pacing; simulation samples count actual game updates.",
    "Seeds and population snapshots are observations, not a guarantee of deterministic replay.",
    CAPPED_LIMITATION,
    "Rust allocation counts cover successful Rust global-allocator alloc/realloc calls and requested bytes only; C/C++, SDL and driver/GPU allocations are excluded. SDL zeros do not establish a total-heap advantage.",
]
DRAWING_LIMITATIONS = [
    "Drawing counters are deltas between consecutive presented frames inside the measured window; the first presentation only establishes the baseline, so there is one fewer counter frame than presentation sample.",
    "wait_ns is host time blocked inside device polls, not GPU execution time; it is already included in the enclosing draw and presentation wall-clock scopes.",
    "Only the gpu_*_ns counters are GPU execution time, and only when KFX_WGPU_GPU_TIMING is 1 or 2; they are per-pass durations resolved from timestamp queries and are not comparable with the host wall-clock scopes. gpu_untimed_passes counts passes that went unstamped, so a window with a nonzero value under-reports.",
    "A gpu_*_ns pass window runs from that pass's begin stamp to its end stamp, so it includes time the pass spent stalled on its dependencies: it attributes cost rather than measuring it, and the sum of the windows decomposes nothing. gpu_pass_union_ns is the union of the frame's pass intervals, so overlapping windows count once; it is an upper bound on GPU occupancy and equals the window sum whenever the windows do not overlap, which is what this Metal adapter shows. Only --serial-gpu-timing measures pass cost exclusively, and it serialises the frame to do so, so that run is a diagnostic and not a throughput baseline.",
    "Counters cover the drawing context the bridge owns. Presenter surface acquisition and any drawing done outside that context are not counted.",
    "host_staged_asset_bytes is a host-side gauge sampled at frame end: the CPU copies the drawing context stages, not GPU memory, and not a per-frame delta, so its window total is meaningless.",
    "arena_bytes_resident is a gauge sampled at frame end: GPU bytes suballocated in the persistent asset arena, free-listed slots and power-of-two class padding included, and not a per-frame delta.",
    "arena_scratch_bytes_peak is a high-water gauge over the process: the widest extent the arena's transient regions reached inside one pinning scope, which one submit per frame makes a whole frame rather than a batch.",
]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def asset_identity(directory):
    digest = hashlib.sha256()
    count = total = 0
    for name in sorted(capture.ASSET_DIRS):
        for path in sorted((directory / name).rglob("*")):
            if path.is_file():
                relative = path.relative_to(directory).as_posix()
                digest.update(relative.encode() + b"\0" + sha256(path).encode() + b"\n")
                count += 1
                total += path.stat().st_size
    return {"sha256": digest.hexdigest(), "files": count, "bytes": total,
            "scope": list(capture.ASSET_DIRS),
            "algorithm": "SHA256 of sorted relative UTF-8 path, NUL, file SHA256 hex, newline"}


def uncapped(args):
    return bool(getattr(args, "uncapped", False))


def offscreen(args):
    return bool(getattr(args, "offscreen", False))


def _ioreg_text():
    return subprocess.run(["ioreg", "-n", "Root", "-d1"], capture_output=True, text=True,
                          timeout=30, check=True).stdout


def console_locked(probe=_ioreg_text):
    """True or False when the session lock state is readable, None when the probe fails."""
    try:
        text = probe()
    except (OSError, ValueError, subprocess.SubprocessError):
        return None
    if not isinstance(text, str):
        return None
    match = re.search(r'"CGSSessionScreenIsLocked"\s*=\s*(\w+)', text)
    return False if match is None else match.group(1) == "Yes"


def load_per_core(probe=os.getloadavg, cpus=os.cpu_count):
    """One-minute load average per core; it lags a job that just started."""
    try:
        return probe()[0] / (cpus() or 1)
    except (OSError, ValueError, TypeError, IndexError):
        return None


def evaluate_guards(args, locked, load):
    findings = []
    if locked and not offscreen(args):
        findings.append({"reason": "console_locked",
                         "detail": "the console session is locked, so the swapchain path cannot acquire a "
                                   "drawable; --offscreen measures without one"})
    threshold = getattr(args, "max_load", DEFAULT_MAX_LOAD)
    if load is not None and load > threshold:
        findings.append({"reason": "background_load",
                         "detail": f"one-minute load average per core {load:.3f} exceeds --max-load {threshold}"})
    return findings


def occlusion_reason(stderr, presentations):
    """Post-run, measured window only. A skip inside the window makes the engine call
    performance_failed, which writes the marker; a startup skip before the window is
    not a finding, and renderer_details only ever carries the startup snapshot."""
    if "Rust surface acquisition skipped" in (stderr or ""):
        return {"reason": "occluded", "detail": "the engine reported a skipped surface acquisition"}
    if not presentations:
        return {"reason": "occluded", "detail": "no frame was presented inside the measured window"}
    return None


def post_run_occlusion(output, stderr):
    presentations = 0
    raw = output / "raw.csv"
    if raw.is_file():
        try:
            presentations = sum(1 for line in raw.read_text().splitlines() if line.startswith("presentation,"))
        except OSError:
            presentations = 0
    return occlusion_reason(stderr, presentations)


class Refusal(RuntimeError):
    """Conditions made the run unmeasurable. Distinct from a failed run."""

    def __init__(self, reason, detail):
        super().__init__(f"{reason}: {detail}")
        self.reason, self.detail = reason, detail


@contextlib.contextmanager
def timing_lock(path=None):
    """Serializes timing runs and builds. flock is advisory and per open file description,
    so a nested acquire would self-deadlock; KFX_TIMING_LOCK_HELD=1 means a parent holds it."""
    path = path or TIMING_LOCK_PATH
    if os.environ.get("KFX_TIMING_LOCK_HELD") == "1":
        yield {"path": path, "waited_seconds": 0.0, "held_by_parent": True, "previous_holder": None}
        return
    handle = open(path, "a+")
    try:
        started = time.monotonic()
        previous = None
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            handle.seek(0)
            previous = handle.read(4096).strip() or None
            print(f"waiting for the timing lock held by {previous or 'an unnamed holder'}", flush=True)
            fcntl.flock(handle, fcntl.LOCK_EX)
        waited = time.monotonic() - started
        handle.seek(0)
        handle.truncate()
        handle.write(json.dumps({"holder": str(ROOT), "pid": os.getpid(),
                                 "started_utc": datetime.now(timezone.utc).isoformat()}))
        handle.flush()
        yield {"path": path, "waited_seconds": waited, "held_by_parent": False, "previous_holder": previous}
    finally:
        # Never unlink: that would race a waiter which already opened this path.
        handle.close()


def settings_for(args):
    """Isolated engine settings; FRAMES_PER_SECOND=0 disables the engine frame limiter."""
    values = dict(SETTINGS)
    if uncapped(args):
        values["FRAMES_PER_SECOND"] = str(UNCAPPED_FPS_LIMIT)
    return values


def frame_cap_for(args):
    limit = UNCAPPED_FPS_LIMIT if uncapped(args) else CAPPED_FPS_LIMIT
    return {"uncapped": uncapped(args), "requested_fps_limit": limit,
            "label": "uncapped" if uncapped(args) else f"capped at {limit} FPS"}


def configure(work, settings=SETTINGS):
    path = work / "keeperfx.cfg"
    values = path.read_text()
    for key, value in settings.items():
        values, count = re.subn(rf"^{key}\s*=.*$", f"{key}={value}", values, flags=re.M)
        if not count:
            values += f"\n{key}={value}\n"
    path.write_text(values)


def gpu_timing_level(args):
    if getattr(args, "serial_gpu_timing", False):
        return "2"
    return "1" if getattr(args, "gpu_timing", False) else "0"


def environment_for(args, output):
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("KFX_PERF_", "KFX_FRAME_CAPTURE"))
                   and not key.startswith("KFX_WGPU_")
                   and key not in ("KFX_PRESENT_BACKEND", "SDL_VIDEODRIVER", "SDL_VIDEO_DRIVER", "SDL_RENDER_DRIVER", "SDL_RENDER_VSYNC")}
    if args.headless:
        environment.update(SDL_VIDEODRIVER="dummy", SDL_VIDEO_DRIVER="dummy", SDL_RENDER_DRIVER="software")
    elif sys.platform == "darwin":
        environment.update(SDL_VIDEODRIVER="cocoa", SDL_VIDEO_DRIVER="cocoa", SDL_RENDER_DRIVER="metal")
    backend = ("wgpu-offscreen" if offscreen(args) else "wgpu") if args.backend == "rust" else "sdl"
    environment.update(KFX_PRESENT_BACKEND=backend,
                       SDL_RENDER_VSYNC="0", KFX_PERF_OUTPUT=str(output / "raw.csv"),
                       KFX_PERF_DRAW_BREAKDOWN="1" if getattr(args, "draw_breakdown", False) else "0",
                       KFX_WGPU_GPU_TIMING=gpu_timing_level(args),
                       KFX_PERF_TURN=str(args.warmup_turns), KFX_PERF_TURNS=str(args.turns),
                       KFX_PERF_SCENE="possession" if args.scene == "possession" else "dungeon")
    return environment


def distribution(values):
    ordered = sorted(value / 1_000_000 for value in values)
    def percentile(percent):
        index = (len(ordered) - 1) * percent / 100
        low = math.floor(index)
        high = math.ceil(index)
        return ordered[low] + (ordered[high] - ordered[low]) * (index - low)
    return {"count": len(ordered), "mean": statistics.fmean(ordered), "median": statistics.median(ordered),
            "p90": percentile(90), "p95": percentile(95), "p99": percentile(99), "max": ordered[-1]}


def summarize(output, args):
    metadata = json.loads((output / "raw.csv.json").read_text())
    if metadata.get("format") != "KFXPERF01" or metadata.get("complete") is not True:
        raise RuntimeError("engine did not produce a complete KFXPERF01 run; rebuild the engine")
    start, end = metadata["start"]["turn"], metadata["end"]["turn"]
    if start < args.warmup_turns or end - start != args.turns:
        raise RuntimeError("engine turn range does not match requested warmup and measured turns")
    scene = "possession" if args.scene == "possession" else "dungeon"
    view = "creature" if args.scene == "possession" else "dungeon_top"
    if metadata.get("scene") != scene or metadata.get("view") != view:
        raise RuntimeError("engine scene or view does not match the request")
    if (metadata["width"], metadata["height"]) != args.resolution:
        raise RuntimeError("engine logical resolution does not match the request")
    cap = frame_cap_for(args)
    if (metadata["vsync_actual"] != 0 or metadata["turns_per_second"] != 20
            or metadata["fps_limit"] != cap["requested_fps_limit"] or metadata["interpolation"] not in (True, 1)):
        raise RuntimeError("engine did not apply the requested VSync, turn rate, frame cap or interpolation")
    cap["engine_fps_limit"] = metadata["fps_limit"]
    if args.backend == "rust" and (args.headless or sys.platform != "darwin"):
        raise RuntimeError("Rust measurements require a native macOS window")
    expected_backend = ((("cocoa", "wgpu-metal-offscreen") if offscreen(args) else ("cocoa", "wgpu-metal"))
                        if args.backend == "rust" else
                        (("dummy", "software") if args.headless else (("cocoa", "metal") if sys.platform == "darwin" else None)))
    if expected_backend and (metadata["video_driver"], metadata["renderer"]) != expected_backend:
        raise RuntimeError(f"engine backend does not match requested {expected_backend}")
    if metadata["output_width"] <= 0 or metadata["output_height"] <= 0:
        raise RuntimeError("engine did not report a valid presentation output size")
    if not args.headless and metadata["video_driver"] in ("dummy", "offscreen", "unknown", ""):
        raise RuntimeError("native baseline requires a window-system video driver")
    if args.backend == "original" and str(metadata["renderer"]).startswith("wgpu"):
        raise RuntimeError("actual renderer does not match requested original backend")
    if args.backend == "rust":
        details = json.loads(metadata.get("renderer_details", "{}"))
        modes = ("Offscreen",) if offscreen(args) else ("Immediate", "Mailbox")
        if (not isinstance(details, dict) or details.get("backend") != "Metal" or not details.get("adapter")
                or details.get("present_mode") not in modes or not details.get("format")):
            raise RuntimeError("Rust backend did not confirm its adapter, format and actual presentation mode")
    breakdown = metadata.get("draw_breakdown", False)
    if type(breakdown) is not bool or breakdown != getattr(args, "draw_breakdown", False):
        raise RuntimeError("engine draw breakdown does not match the request")
    kinds = KINDS + DRAW_KINDS if breakdown else KINDS
    if metadata.get("replay_scope") is True:
        kinds += ("replay",)
    samples = {kind: [] for kind in kinds}
    sample_turns = {kind: [] for kind in kinds}
    pending_draw = []
    with (output / "raw.csv").open(newline="") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != ["kind", "turn", "wall_ns"]:
            raise RuntimeError("unexpected performance CSV schema")
        for row in reader:
            kind, turn, duration = row["kind"], int(row["turn"]), int(row["wall_ns"])
            if kind not in samples or duration < 0 or not start <= turn <= end:
                raise RuntimeError("invalid sample kind, duration or turn")
            if breakdown:
                if kind in DRAW_KINDS:
                    pending_draw.append((kind, turn))
                    if len(pending_draw) > len(DRAW_KINDS):
                        raise RuntimeError("duplicate draw breakdown samples")
                elif kind == "draw":
                    if pending_draw != [(child, turn) for child in DRAW_KINDS]:
                        raise RuntimeError("draw breakdown must be a complete ordered block before its parent")
                    pending_draw.clear()
                elif pending_draw:
                    raise RuntimeError("draw breakdown block interrupted before its parent")
            samples[kind].append(duration)
            sample_turns[kind].append(turn)
    if pending_draw:
        raise RuntimeError("draw breakdown has no enclosing draw sample")
    if sample_turns["simulation"] != list(range(start, end)):
        raise RuntimeError("simulation samples must cover every measured turn exactly once in order")
    if not all(samples.values()):
        raise RuntimeError("performance output is missing a timing category")
    presentations = sample_turns["presentation"]
    if sample_turns["draw"] != presentations or sample_turns["present_wait"] != presentations:
        raise RuntimeError("each drawn frame must have exactly one matching presentation and present_wait")
    if "replay" in samples and sample_turns["replay"] != presentations:
        raise RuntimeError("replay must cover every presentation in order")
    if sample_turns["frame_interval"] != presentations[1:]:
        raise RuntimeError("frame intervals must cover consecutive presentations, excluding the first")
    if any(duration <= 0 for duration in samples["frame_interval"]):
        raise RuntimeError("frame intervals must be positive")
    if any(wait > present for wait, present in zip(samples["present_wait"], samples["presentation"])):
        raise RuntimeError("present_wait cannot exceed its enclosing presentation duration")
    if breakdown:
        for kind in DRAW_KINDS:
            if sample_turns[kind] != sample_turns["draw"]:
                raise RuntimeError("each draw must have exactly one sample per breakdown category")
        unaccounted = [draw - sum(children) for draw, *children in zip(
            samples["draw"], *(samples[kind] for kind in DRAW_KINDS))]
        if any(value < 0 for value in unaccounted):
            raise RuntimeError("draw breakdown exceeds its enclosing draw duration")
        samples["draw_unaccounted"] = unaccounted
    resource_report = summarize_resources(metadata.get("resources"), args.turns, len(presentations))
    drawing_report = summarize_drawing(metadata.get("drawing"), len(presentations))
    limitations = ([item for item in LIMITATIONS if item != CAPPED_LIMITATION] + UNCAPPED_LIMITATIONS
                   if cap["uncapped"] else list(LIMITATIONS))
    limitations += [OFFSCREEN_LIMITATION] if offscreen(args) else []
    limitations += [] if resource_report["process_cpu"] is not None else ["Process CPU time is not available in this run."]
    if drawing_report is None:
        limitations += ["Drawing-backend counters are absent from this engine build."]
    else:
        if "+" in drawing_report["backend"]:
            raise RuntimeError(f"drawing backend changed during the measured window: {drawing_report['backend']}")
        limitations += DRAWING_LIMITATIONS if drawing_report["available"] else [
            f"Drawing-backend counters are unavailable: the active drawing backend was {drawing_report['backend']}."]
    if breakdown:
        limitations += [
            "Draw includes scene preparation, raster dispatch, front-view raster dispatch and overlays; these nested series must not be added to draw.",
            "Raster dispatch combines terrain, sprites, shadows and bucket overlays; no per-command or per-pixel timers are collected.",
            "draw_unaccounted is draw minus its non-overlapping coarse child scopes, computed per frame; it includes other draw work and instrumentation overhead.",
            "Breakdown zero samples mean the scope was not visited or took less than clock resolution; they do not prove a drawing family was absent.",
            "Compare matched runs with and without --draw-breakdown to measure instrumentation overhead; overhead is not assumed negligible.",
        ]
    presenter_report = summarize_presenter(metadata.get("presenter"), samples,
                                           required=metadata.get("replay_scope") is True and args.backend == "rust")
    replay_report = (summarize_replay(metadata["presenter"].get("replay"), samples)
                     if presenter_report else None)
    if replay_report:
        limitations.append("Replay host phases are exclusive wall-clock intervals inside frame_flush, including scheduling; they are not GPU time. Submit/wait covers queue submission and blocking GPU drains; other covers status and cleanup. Each attribution row is a counter delta around that presentation's ResidentTarget call, including the first presentation. Cumulative drawing deltas include prior replay and checkpoint work and are not used for this comparison. Staged bytes count API payload bytes, including uniform and parameter buffers, not GPU allocation capacity.")
        if not replay_report["within_5_percent"]:
            limitations.append("Replay host attribution differs from the outer replay scope by more than 5% on one or more frames; inspect the signed residual before choosing an optimization.")
    wall_ms = {kind: distribution(values) for kind, values in samples.items()}
    window_ms = resource_report.get("wall_ms")
    observed = {"frames_per_second": 1000 / wall_ms["frame_interval"]["mean"],
                "turns_per_second": None if not window_ms else args.turns / (window_ms / 1000),
                "frame_cap": cap["label"]}
    return {"engine": metadata, "frame_cap": cap, "observed": observed, "wall_ms": wall_ms,
            "presentation_mode": "offscreen" if offscreen(args) else "swapchain",
            "resources": resource_report, "drawing": drawing_report, "presenter": presenter_report, "replay_host": replay_report,
            "percentile_method": "linear interpolation at (sample_count - 1) * percentile / 100",
            "limitations": limitations + (["HEADLESS SOFTWARE SMOKE TEST: not a native presentation baseline."] if args.headless else [])}


def summarize_presenter(presenter, samples, required=False):
    if required and (not isinstance(presenter, dict) or not presenter.get("per_frame")):
        raise RuntimeError("presenter counters must cover every presentation")
    if presenter is None:
        return None
    rows = presenter.get("per_frame")
    if not isinstance(rows, list):
        raise RuntimeError("invalid presenter counters")
    if not rows:
        return None
    if len(rows) != len(samples["presentation"]) or "replay" not in samples:
        raise RuntimeError("presenter counters must cover every presentation")
    if any(not isinstance(row, list) or len(row) != len(PRESENTER_COUNTERS)
           or any(type(value) is not int or value < 0 for value in row) for row in rows):
        raise RuntimeError("invalid presenter counter row")
    if any(row[1] > row[0] for row in rows):
        raise RuntimeError("acquire block exceeds acquire")
    cpu = [present - row[1] - wait for present, row, wait in
           zip(samples["presentation"], rows, samples["present_wait"])]
    if any(value < 0 for value in cpu):
        raise RuntimeError("presentation waits exceed presentation")
    samples["presentation_cpu"] = cpu
    counters = {name: drawing_distribution([row[index] for row in rows])
                for index, name in enumerate(PRESENTER_COUNTERS)}
    residual = [present - row[0] - row[3] - row[4]
                for present, row in zip(samples["presentation"], rows)]
    return {"frames": len(rows), "per_frame": counters,
            "residual_ms": distribution(residual),
            "residual_fraction": sum(residual) / sum(samples["presentation"])}



def summarize_replay(replay_data, samples):
    if replay_data is None:
        return None
    if not isinstance(replay_data, dict) or tuple(replay_data.get("counters", ())) not in (REPLAY_COUNTERS, REPLAY_PHASES + REPLAY_COUNTS):
        raise RuntimeError("replay attribution counter names do not match this profiler")
    names = replay_data["counters"]
    rows = replay_data.get("per_frame")
    replay = samples.get("replay")
    if not isinstance(rows, list) or not rows or replay is None or len(rows) != len(replay):
        raise RuntimeError("replay attribution must cover every presentation")
    if any(not isinstance(row, list) or len(row) != len(names)
           or any(type(value) is not int or value < 0 for value in row) for row in rows):
        raise RuntimeError("invalid replay attribution row")
    phases = {name: [row[names.index(name)] for row in rows] for name in REPLAY_PHASES}
    totals = [sum(values) for values in zip(*phases.values())]
    residual = [outer - total for outer, total in zip(replay, totals)]
    outside = sum(abs(value) > outer * 0.05 for value, outer in zip(residual, replay))
    return {"source": "presenter.replay", "frames": len(rows), "phases_ms": {name: distribution(values) for name, values in phases.items()},
            "counts": {name: drawing_distribution([row[names.index(name)] for row in rows], name in UPLOAD_GAUGES)
                       for name in (*REPLAY_COUNTS, *UPLOAD_COUNTERS) if name in names},
            "total_ms": distribution(totals), "replay_ms": distribution(replay),
            "residual_ms": distribution(residual),
            "residual_fraction": sum(residual) / sum(replay) if sum(replay) else None,
            "max_absolute_residual_fraction": max((abs(value) / outer for value, outer in
                                                   zip(residual, replay) if outer), default=None),
            "frames_outside_5_percent": outside, "within_5_percent": outside == 0}


def drawing_distribution(values, gauge=False):
    ordered = sorted(values)
    if not ordered:
        raise RuntimeError("drawing counter series is empty")
    def percentile(percent):
        index = (len(ordered) - 1) * percent / 100
        low, high = math.floor(index), math.ceil(index)
        return ordered[low] + (ordered[high] - ordered[low]) * (index - low)
    return {"min": ordered[0], "mean": statistics.fmean(ordered), "p95": percentile(95),
            "max": ordered[-1], "total": None if gauge else sum(ordered)}


def summarize_drawing(drawing, presentations):
    if drawing is None:
        return None
    if type(drawing.get("available")) is not bool or not isinstance(drawing.get("backend"), str) \
            or not drawing["backend"]:
        raise RuntimeError("invalid drawing counter availability or backend")
    names = tuple(drawing.get("counters", ()))
    additions = {name for name in DRAWING_COUNTERS if name.startswith(("target_trig_", "preparer_", "arena_miss"))}
    additions.update(("other_asset_upload_bytes", "shadow_pairs", "arena_explicit_forgets",
                      "arena_capacity_bytes", "arena_live_bytes", "arena_retired_bytes", "arena_growth_peak_bytes"))
    schemas = [tuple(name for name in DRAWING_COUNTERS if name not in omitted)
               for omitted in (set(), additions, {"asset_upload_bytes", "command_upload_bytes"},
                               additions | {"asset_upload_bytes", "command_upload_bytes"})]
    arena_additions = set(ARENA_KIND_COUNTERS) | {"arena_trig_texture_source_bytes"}
    schemas += [tuple(name for name in schema if name not in arena_additions) for schema in schemas]
    schemas += [tuple(name for name in schema if name != "replay_submit_wait_ns") for schema in schemas]
    schemas += [tuple(name for name in schema if name not in UPLOAD_COUNTERS) for schema in schemas]
    schemas += [tuple(name for name in schema if name not in REPLAY_COUNTERS) for schema in schemas]
    if names not in schemas:
        raise RuntimeError("drawing counter names do not match this profiler")
    rows = drawing.get("per_frame")
    if not isinstance(rows, list) or drawing.get("frames") != len(rows):
        raise RuntimeError("drawing counter frame count does not match the recorded rows")
    for row in rows:
        if not isinstance(row, list) or len(row) != len(names) or any(
                type(value) is not int or value < 0 for value in row):
            raise RuntimeError("invalid drawing counter row")
    result = {"backend": drawing["backend"], "available": drawing["available"],
              "frames": len(rows), "scope": "per presented frame inside the measured window"}
    if not drawing["available"]:
        if rows:
            raise RuntimeError("drawing counters are unavailable but rows were recorded")
        result["per_frame"] = None
        return result
    if len(rows) != presentations - 1:
        raise RuntimeError("drawing counter frames must cover every measured presentation but the first")
    if tuple(drawing.get("gauges", ())) != tuple(name for name in DRAWING_GAUGES if name in names):
        raise RuntimeError("drawing gauge names do not match this profiler")
    if arena_additions.issubset(names):
        byte_indices = [names.index(f"arena_{kind}_bytes") for kind in ARENA_RESOURCE_KINDS]
        total_index = names.index("arena_bytes_uploaded")
        if any(sum(row[index] for index in byte_indices) != row[total_index] for row in rows):
            raise RuntimeError("arena resource bytes do not sum to arena_bytes_uploaded")
        result["arena_upload_partition"] = {"conserved": True, "kinds": list(ARENA_RESOURCE_KINDS)}
    result["per_frame"] = {name: drawing_distribution([row[index] for row in rows],
                                                      name in DRAWING_GAUGES)
                           for index, name in enumerate(names)}
    return result


def summarize_resources(resources, turns, presentations):
    result = {"scope": "active measurement window; excludes startup, warmup, sample output and shutdown",
              "process_cpu": None, "rust_allocations": None}
    if resources is None:
        return result
    wall = resources["wall_ns"]
    if type(wall) is not int or wall <= 0:
        raise RuntimeError("invalid resource measurement wall duration")
    result["wall_ms"] = wall / 1_000_000
    cpu = resources["process_cpu"]
    if cpu["available"]:
        if cpu["source"] not in ("GetProcessTimes", "getrusage(RUSAGE_SELF)") or any(
                type(cpu[key]) is not int or cpu[key] < 0 for key in ("user_ns", "system_ns")):
            raise RuntimeError("invalid process CPU counters")
        total = cpu["user_ns"] + cpu["system_ns"]
        result["process_cpu"] = {"source": cpu["source"], "user_ms": cpu["user_ns"] / 1_000_000,
                                 "system_ms": cpu["system_ns"] / 1_000_000, "total_ms": total / 1_000_000,
                                 "ms_per_turn": total / 1_000_000 / turns,
                                 "ms_per_presentation": total / 1_000_000 / presentations,
                                 "core_equivalents": total / wall}
    allocations = resources["rust_allocations"]
    if allocations["available"]:
        if any(type(allocations[key]) is not int or allocations[key] < 0 for key in ("calls", "requested_bytes")):
            raise RuntimeError("invalid Rust allocation counters")
        result["rust_allocations"] = {"calls": allocations["calls"], "requested_bytes": allocations["requested_bytes"],
                                      "calls_per_presentation": allocations["calls"] / presentations,
                                      "requested_bytes_per_presentation": allocations["requested_bytes"] / presentations,
                                      "scope": "Rust global allocator; excludes C/C++, SDL, driver/GPU; not total process allocations or retained memory"}
    return result


def annotate_presentation_pacing(report):
    if not report["frame_cap"]["uncapped"] or report["presentation_mode"] != "swapchain":
        return
    counters = (report.get("presenter") or {}).get("per_frame", {})
    acquisition = counters.get("acquire_block_ns")
    if acquisition is None:
        return
    ratio = acquisition["mean"] / (report["wall_ms"]["frame_interval"]["mean"] * 1_000_000)
    report["presentation_paced"] = ratio > 0.5
    report["presentation_pacing"] = {"acquire_block_fraction": ratio, "threshold": 0.5}
    if report["presentation_paced"]:
        report["limitations"].append(
            f"This uncapped cell is compositor-paced: surface acquisition blocks for {ratio:.1%} "
            "of the mean frame interval (threshold: >50%). It is not a drawing or engine ceiling; "
            "it remains a valid matched comparison of host work. Use --offscreen for ceiling comparisons.")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def write_report(output, report):
    request, actual = report["request"], report["engine"]
    cap = report.get("frame_cap") or frame_cap_for(argparse.Namespace(uncapped=not actual["fps_limit"]))
    label = "HEADLESS SOFTWARE SMOKE TEST" if request["headless"] else "Native performance baseline"
    label += " (uncapped)" if cap["uncapped"] else " (60 FPS cap)"
    lines = [f"# {label}", "", f"Scene: {request['scene']}; campaign: {request['campaign']}; level: {request['level']}.",
             f"Actual turns: {actual['start']['turn']}–{actual['end']['turn'] - 1} ({request['turns']} simulation updates).",
             f"Backend: {actual['video_driver']} / {actual['renderer']}; logical resolution: {actual['width']}×{actual['height']}; "
             f"output: {actual['output_width']}×{actual['output_height']}; "
             f"presentation mode: {report.get('presentation_mode', 'swapchain')}.",
             f"Frame cap: {cap['label']} (engine frame limit {actual['fps_limit']}); "
             f"VSync: {actual['vsync_actual']}; interpolation: {actual['interpolation']}.",
             f"Population at start/end: creatures {actual['start']['creatures']}/{actual['end']['creatures']}; "
             f"things {actual['start']['things']}/{actual['end']['things']}.", "",
             "Wall-clock milliseconds; percentile estimates use linear interpolation.", "",
             "| Scope | Samples | Mean | Median | p90 | p95 | p99 | Max |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for kind, stats in report["wall_ms"].items():
        lines.append(f"| {kind} | {stats['count']} | " + " | ".join(f"{stats[key]:.3f}" for key in ("mean", "median", "p90", "p95", "p99", "max")) + " |")
    interval = report["wall_ms"]["frame_interval"]["mean"]
    if interval:
        lines += ["", f"Observed presentation rate from mean frame interval: {1000 / interval:.2f} frames/s "
                  f"({cap['label']})."]
    turns = (report.get("observed") or {}).get("turns_per_second")
    if turns:
        lines += ["", f"Observed simulation rate over the measured window: {turns:.2f} turns/s "
                  "(requested 20; an uncapped run that falls below it did not sustain the simulation)."]
    details = actual.get("renderer_details")
    if details:
        lines += ["", f"Actual renderer details: `{details}`."]
    resources = report["resources"]
    cpu, allocations = resources["process_cpu"], resources["rust_allocations"]
    if cpu:
        lines += ["", f"Measured-window process CPU ({cpu['source']}): {cpu['total_ms']:.3f} ms "
                  f"({cpu['user_ms']:.3f} user + {cpu['system_ms']:.3f} system), "
                  f"{cpu['ms_per_turn']:.3f} ms/turn, {cpu['ms_per_presentation']:.3f} ms/presentation, "
                  f"{cpu['core_equivalents']:.3f} CPU cores on average."]
    if allocations:
        lines += ["", f"Rust global-allocator calls: {allocations['calls']}; requested bytes: {allocations['requested_bytes']}. "
                  "This excludes C/C++, SDL, driver/GPU allocations and does not measure retained memory."]
    presenter = report.get("presenter")
    if presenter:
        lines += ["", "Presenter host counters per frame (nanoseconds, calls or bytes):", "",
                  "| Counter | Mean | p95 | Max |", "| --- | ---: | ---: | ---: |"]
        for name, stats in presenter["per_frame"].items():
            lines.append(f"| {name} | {stats['mean']:.2f} | {stats['p95']:.2f} | {stats['max']} |")
        lines += ["", f"Unattributed presentation residual: {presenter['residual_ms']['mean']:.4f} ms "
                  f"({presenter['residual_fraction']:.2%}); acquire + present record + submit, excluding replay."]
    replay = report.get("replay_host")
    if replay:
        lines += ["", "## Replay host attribution", "",
                  f"Exclusive host intervals over {replay['frames']} frames, sampled around each presentation's replay, including the first.", "",
                  "| Phase (ms/frame) | Mean | p95 | Max |", "| --- | ---: | ---: | ---: |"]
        for name, stats in {**replay["phases_ms"], "Phase sum": replay["total_ms"],
                            "Replay scope": replay["replay_ms"], "Residual against replay": replay["residual_ms"]}.items():
            lines.append(f"| {name} | {stats['mean']:.6f} | {stats['p95']:.6f} | {stats['max']:.6f} |")
        fraction = replay["residual_fraction"]
        label = "unavailable (zero replay)" if fraction is None else f"{fraction:.2%}"
        lines += ["", f"Signed residual / replay: {label}; frames outside ±5%: {replay['frames_outside_5_percent']}.", "",
                  "| Count per frame | Mean | p95 | Max |", "| --- | ---: | ---: | ---: |"]
        for name, stats in replay["counts"].items():
            lines.append(f"| {name} | {stats['mean']:.2f} | {stats['p95']:.2f} | {stats['max']} |")
    drawing = report.get("drawing")
    if drawing:
        lines += ["", f"Active drawing backend: {drawing['backend']}."]
        if drawing["per_frame"]:
            lines += ["", f"Drawing-backend counters over {drawing['frames']} measured frames.", "",
                      "| Counter | Min | Mean | p95 | Max | Window total |",
                      "| --- | ---: | ---: | ---: | ---: | ---: |"]
            for name, stats in drawing["per_frame"].items():
                total = "gauge" if stats["total"] is None else stats["total"]
                lines.append(f"| {name} | {stats['min']} | {stats['mean']:.2f} | {stats['p95']:.2f} | "
                             f"{stats['max']} | {total} |")
            if drawing.get("arena_upload_partition"):
                values = drawing["per_frame"]
                total = values["arena_bytes_uploaded"]["total"]
                lines += ["", "Arena uploads by resource kind (bytes conserved in every frame).", "",
                          "| Kind | GPU bytes/frame | Share | Misses/frame | Hits/frame | Source bytes/frame | Distinct lengths/frame | Length overflows |",
                          "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
                for kind in ARENA_RESOURCE_KINDS:
                    c = {metric: values[f"arena_{kind}_{metric}"] for metric in ARENA_KIND_METRICS}
                    share = c["bytes"]["total"] / total if total else 0
                    lines.append(f"| {kind} | {c['bytes']['mean']:.2f} | {share:.2%} | {c['misses']['mean']:.2f} | "
                                 f"{c['hits']['mean']:.2f} | {c['source_bytes']['mean']:.2f} | "
                                 f"{c['distinct_lengths']['mean']:.2f} | {c['length_overflows']['total']} |")
                lines += ["", f"General TRIG packed texture source bytes/frame: {values['arena_trig_texture_source_bytes']['mean']:.2f}."]
            waits = drawing["per_frame"]["wait_ns"]
            lines += ["", f"Blocking host wait: {waits['mean'] / 1_000_000:.3f} ms mean, "
                      f"{waits['p95'] / 1_000_000:.3f} ms p95 per frame. This wait is host wall time; "
                      "gpu_*_ns counters are separate GPU timestamp measurements when enabled."]
    lines += ["", *[f"- {item}" for item in report["limitations"]], "",
              f"Engine SHA256: `{report['engine_sha256']}`", f"Asset content SHA256: `{report['assets']['sha256']}`", "",
              "Exact request, platform, content identities, seeds and actual settings: report.json. Samples: raw.csv."]
    (output / "report.md").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description="Opt-in bounded wall-clock profiling with isolated game assets and settings.")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--out", type=Path, required=True, help="new directory beneath the repository's ignored out directory")
    parser.add_argument("--scene", choices=("quiet", "busy", "possession"), default="quiet")
    parser.add_argument("--backend", choices=("original", "rust"), default="original")
    parser.add_argument("--campaign", default="keeporig")
    parser.add_argument("--level", type=int, help="override the preset map (quiet/possession: 1; busy: 20)")
    parser.add_argument("--resolution", type=capture.resolution, default=(640, 480))
    parser.add_argument("--warmup-turns", type=int, default=40, help="earliest game turn to begin measuring (1..600)")
    parser.add_argument("--turns", type=int, default=200, help="actual simulation updates to measure (20..1200)")
    parser.add_argument("--draw-breakdown", action="store_true", help="coarse nested CPU drawing timings; compare against a matched run without this flag")
    parser.add_argument("--gpu-timing", action="store_true", help="resolve per-pass GPU execution time into the gpu_*_ns drawing counters")
    parser.add_argument("--serial-gpu-timing", action="store_true", help="as --gpu-timing, but drain the queue after every timed submission so the per-pass windows are exclusive; costs throughput and is not a performance baseline")
    parser.add_argument("--uncapped", action="store_true",
                        help="remove the engine frame limiter (FRAMES_PER_SECOND=0); simulation stays at 20 turns/s and VSync stays off")
    parser.add_argument("--offscreen", action="store_true",
                        help="present into an offscreen texture ring instead of a swapchain; measurement mode, no window output")
    parser.add_argument("--max-load", type=float, default=DEFAULT_MAX_LOAD,
                        help="refuse the run when the one-minute load average per core exceeds this")
    parser.add_argument("--ignore-guards", action="store_true",
                        help="record environment guard findings without refusing; never applies to the timing lock")
    parser.add_argument("--headless", action="store_true", help="dummy/software smoke test, not a native performance baseline")
    args = parser.parse_args()
    if args.backend == "rust" and (args.headless or sys.platform != "darwin"):
        parser.error("--backend rust requires native macOS; --headless is only for the original backend")
    if args.offscreen and (args.backend != "rust" or args.headless or sys.platform != "darwin"):
        parser.error("--offscreen requires --backend rust on native macOS and is incompatible with --headless")
    if args.level is None:
        args.level = 20 if args.scene == "busy" else 1
    if not 1 <= args.warmup_turns <= 600 or not 20 <= args.turns <= 1200:
        parser.error("--warmup-turns must be 1..600 and --turns must be 20..1200")
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.campaign) or not 1 <= args.level <= 99999:
        parser.error("--campaign must be an identifier and --level must be 1..99999")
    output, engine, game = args.out.resolve(), args.engine.resolve(), args.game_dir.resolve()
    work_root = (ROOT / "out").resolve()
    if not output.is_relative_to(work_root) or output == work_root:
        parser.error("performance output must be beneath the repository's ignored out directory")
    if output.exists():
        parser.error("output already exists; choose a new directory")
    if not engine.is_file() or not (game / "keeperfx.cfg").is_file():
        parser.error("build the engine and prepare the game data first")
    output.mkdir(parents=True)
    request = {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()}
    report = {"format": "KFXPERFREPORT01", "status": "preparing", "request": request,
              "started_utc": datetime.now(timezone.utc).isoformat(),
              "platform": {"system": platform.system(), "release": platform.release(),
                           "version": platform.version(), "machine": platform.machine(), "processor": platform.processor()},
              "engine_sha256": sha256(engine), "settings": settings_for(args), "frame_cap": frame_cap_for(args)}
    write_json(output / "report.json", report)
    try:
        with timing_lock() as lock:
            report["timing_lock"] = lock
            guards = {"max_load": args.max_load, "ignored": bool(args.ignore_guards),
                      "offscreen": offscreen(args), "console_locked": console_locked(),
                      "load_per_core": load_per_core()}
            guards["findings"] = evaluate_guards(args, guards["console_locked"], guards["load_per_core"])
            report["environment_guards"] = guards
            write_json(output / "report.json", report)
            if guards["findings"] and not args.ignore_guards:
                raise Refusal(guards["findings"][0]["reason"], guards["findings"][0]["detail"])
            run_engine(output, engine, game, work_root, args, report, guards)
            annotate_presentation_pacing(report)
            write_report(output, report)
            write_json(output / "report.json", report)
    except Refusal as error:
        report.update(status="refused", refusal={"reason": error.reason, "detail": error.detail})
        write_json(output / "report.json", report)
        raise RuntimeError(f"profiling refused ({error.reason}): {error.detail}; diagnostics preserved in {output}") from error
    except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
        report.update(status="failed", error=str(error))
        write_json(output / "report.json", report)
        raise RuntimeError(f"profiling failed: {error}; diagnostics preserved in {output}") from error
    print(f"Profiled {args.turns} {args.scene} simulation turns ({report['frame_cap']['label']}): {output / 'report.md'}")


def run_engine(output, engine, game, work_root, args, report, guards):
    with tempfile.TemporaryDirectory(prefix="profile-game-", dir=work_root) as temporary:
        work = Path(temporary)
        try:
            capture.clone_assets(game, work, args.resolution)
            configure(work, report["settings"])
            (output / "keeperfx.cfg").write_bytes((work / "keeperfx.cfg").read_bytes())
            report["assets"] = asset_identity(work)
            report["config_sha256"] = sha256(work / "keeperfx.cfg")
            command = [str(engine), "-nointro", "-nosound", "-altinput", "-skipheartzoom",
                       "-campaign", args.campaign, "-level", str(args.level)]
            environment = environment_for(args, output)
            report.update(command=command, environment={key: value for key, value in environment.items()
                          if key.startswith(("KFX_PERF_", "SDL_")) or key == "KFX_PRESENT_BACKEND"},
                          timeout_seconds=120 + math.ceil((args.warmup_turns + args.turns) / 20), status="running")
            write_json(output / "report.json", report)
            result = subprocess.run(command, cwd=work, env=environment, capture_output=True,
                                    text=True, timeout=report["timeout_seconds"])
            (output / "stdout.log").write_text(result.stdout)
            (output / "stderr.log").write_text(result.stderr)
            report["returncode"] = result.returncode
            if result.returncode:
                raise RuntimeError(f"engine exited with {result.returncode}")
            occlusion = post_run_occlusion(output, result.stderr)
            guards["occlusion"] = occlusion
            if occlusion and not args.ignore_guards:
                raise Refusal(occlusion["reason"], occlusion["detail"])
            if "Performance capture failed:" in result.stderr:
                raise RuntimeError("engine reported a performance capture failure")
            report.update(summarize(output, args), status="complete")
            guards["load_per_core_end"] = load_per_core()
            if guards["load_per_core_end"] is not None and guards["load_per_core_end"] > args.max_load:
                report["limitations"] += [
                    f"Background load per core reached {guards['load_per_core_end']:.3f} by the end of the run, "
                    f"above the --max-load {args.max_load} threshold; the run is annotated, not discarded."]
            if args.ignore_guards:
                report["limitations"] += [
                    "--ignore-guards was set: environment guard findings were recorded in "
                    "environment_guards but not enforced."]
        except subprocess.TimeoutExpired as error:
            for name, value in (("stdout.log", error.stdout), ("stderr.log", error.stderr)):
                (output / name).write_text(value.decode(errors="replace") if isinstance(value, bytes) else value or "")
            raise RuntimeError(f"engine exceeded the {report['timeout_seconds']} second deadline") from error
        finally:
            if (work / "keeperfx.log").is_file():
                (output / "keeperfx.log").write_bytes((work / "keeperfx.log").read_bytes())


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError) as error:
        sys.exit(str(error))
