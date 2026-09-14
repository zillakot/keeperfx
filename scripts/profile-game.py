#!/usr/bin/env python3
import argparse
import csv
from datetime import datetime, timezone
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

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("capture_frame", Path(__file__).with_name("capture-frame.py"))
capture = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(capture)
KINDS = ("simulation", "draw", "presentation", "present_wait", "frame_interval")
DRAW_KINDS = ("draw_scene", "draw_raster", "draw_front_raster", "draw_overlays")
DRAWING_COUNTERS = ("submits", "dispatches", "waits", "wait_ns", "checkpoints",
                    "checkpoint_copy_bytes", "validation_waits",
                    "flagged_invalid_frames", "status_stalls", "upload_bytes", "readback_bytes",
                    "full_readbacks", "full_readback_bytes", "buffers", "buffer_bytes",
                    "batches", "commands", "ordered_sprites",
                    "arena_evictions", "arena_overflows", "arena_bytes_uploaded",
                    "tile_allocations", "tile_entries",
                    "bridge_target_flushes", "bridge_target_runs",
                    "terrain_tile_entries", "prepared_row_words", "prepared_row_allocations",
                    "gpu_raster_ns", "gpu_terrain_prepare_ns",
                    "gpu_shadow_mask_ns", "gpu_target_trig_ns", "gpu_ordered_sprite_ns",
                    "gpu_minimap_ns", "gpu_lens_ns", "gpu_present_ns",
                    "gpu_timed_passes", "gpu_untimed_passes",
                    "host_staged_asset_bytes", "arena_bytes_resident")
DRAWING_GAUGES = ("host_staged_asset_bytes", "arena_bytes_resident")
SETTINGS = {
    "DELTA_TIME": "ON", "TURNS_PER_SECOND": "20", "FRAMES_PER_SECOND": "60", "VSYNC": "OFF",
    "FREEZE_GAME_ON_FOCUS_LOST": "OFF", "CAPTURE_CURSOR": "OFF",
    "CURSOR_EDGE_CAMERA_PANNING": "OFF", "LOCK_CURSOR_IN_POSSESSION": "OFF",
}
LIMITATIONS = [
    "Per-scope timings are monotonic wall-clock durations, including scheduling and blocking; they are not CPU-time counters.",
    "GPU execution time is collected only when KFX_WGPU_GPU_TIMING=1 and the adapter supports timestamp queries: the gpu_*_ns drawing counters are per-pass GPU durations. Presentation and present_wait remain host-side wall clock.",
    "Presentation includes present_wait; these overlapping scopes must not be added together.",
    "Frame intervals measure observed presentation pacing; simulation samples count actual game updates.",
    "Seeds and population snapshots are observations, not a guarantee of deterministic replay.",
    "The 60 FPS cap limits observed frame rate; lower presentation duration is not an uncapped gameplay FPS speedup.",
    "Rust allocation counts cover successful Rust global-allocator alloc/realloc calls and requested bytes only; C/C++, SDL and driver/GPU allocations are excluded. SDL zeros do not establish a total-heap advantage.",
]
DRAWING_LIMITATIONS = [
    "Drawing counters are deltas between consecutive presented frames inside the measured window; the first presentation only establishes the baseline, so there is one fewer counter frame than presentation sample.",
    "wait_ns is host time blocked inside device polls, not GPU execution time; it is already included in the enclosing draw and presentation wall-clock scopes.",
    "Only the gpu_*_ns counters are GPU execution time, and only when KFX_WGPU_GPU_TIMING=1; they are per-pass durations resolved from timestamp queries and are not comparable with the host wall-clock scopes. gpu_untimed_passes counts passes that went unstamped, so a window with a nonzero value under-reports.",
    "Counters cover the drawing context the bridge owns. Presenter surface acquisition and any drawing done outside that context are not counted.",
    "host_staged_asset_bytes is a host-side gauge sampled at frame end: the CPU copies the drawing context stages, not GPU memory, and not a per-frame delta, so its window total is meaningless.",
    "arena_bytes_resident is a gauge sampled at frame end: GPU bytes suballocated in the persistent asset arena, free-listed slots and power-of-two class padding included, and not a per-frame delta.",
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


def configure(work):
    path = work / "keeperfx.cfg"
    settings = path.read_text()
    for key, value in SETTINGS.items():
        settings, count = re.subn(rf"^{key}\s*=.*$", f"{key}={value}", settings, flags=re.M)
        if not count:
            settings += f"\n{key}={value}\n"
    path.write_text(settings)


def environment_for(args, output):
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("KFX_PERF_", "KFX_FRAME_CAPTURE"))
                   and not key.startswith("KFX_WGPU_")
                   and key not in ("KFX_PRESENT_BACKEND", "SDL_VIDEODRIVER", "SDL_VIDEO_DRIVER", "SDL_RENDER_DRIVER", "SDL_RENDER_VSYNC")}
    if args.headless:
        environment.update(SDL_VIDEODRIVER="dummy", SDL_VIDEO_DRIVER="dummy", SDL_RENDER_DRIVER="software")
    elif sys.platform == "darwin":
        environment.update(SDL_VIDEODRIVER="cocoa", SDL_VIDEO_DRIVER="cocoa", SDL_RENDER_DRIVER="metal")
    environment.update(KFX_PRESENT_BACKEND="wgpu" if args.backend == "rust" else "sdl",
                       SDL_RENDER_VSYNC="0", KFX_PERF_OUTPUT=str(output / "raw.csv"),
                       KFX_PERF_DRAW_BREAKDOWN="1" if getattr(args, "draw_breakdown", False) else "0",
                       KFX_WGPU_GPU_TIMING="1" if getattr(args, "gpu_timing", False) else "0",
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
    if (metadata["vsync_actual"] != 0 or metadata["turns_per_second"] != 20
            or metadata["fps_limit"] != 60 or metadata["interpolation"] not in (True, 1)):
        raise RuntimeError("engine did not apply the requested VSync, turn rate, frame cap or interpolation")
    if args.backend == "rust" and (args.headless or sys.platform != "darwin"):
        raise RuntimeError("Rust measurements require a native macOS window")
    expected_backend = (("cocoa", "wgpu-metal") if args.backend == "rust" else
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
        if (not isinstance(details, dict) or details.get("backend") != "Metal" or not details.get("adapter")
                or details.get("present_mode") not in ("Immediate", "Mailbox") or not details.get("format")):
            raise RuntimeError("Rust backend did not confirm its adapter, format and actual non-VSync present mode")
    breakdown = metadata.get("draw_breakdown", False)
    if type(breakdown) is not bool or breakdown != getattr(args, "draw_breakdown", False):
        raise RuntimeError("engine draw breakdown does not match the request")
    kinds = KINDS + DRAW_KINDS if breakdown else KINDS
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
    limitations = LIMITATIONS + ([] if resource_report["process_cpu"] is not None else ["Process CPU time is not available in this run."])
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
    return {"engine": metadata, "wall_ms": {kind: distribution(values) for kind, values in samples.items()},
            "resources": resource_report, "drawing": drawing_report,
            "percentile_method": "linear interpolation at (sample_count - 1) * percentile / 100",
            "limitations": limitations + (["HEADLESS SOFTWARE SMOKE TEST: not a native presentation baseline."] if args.headless else [])}


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
    if tuple(drawing.get("counters", ())) != DRAWING_COUNTERS:
        raise RuntimeError("drawing counter names do not match this profiler")
    rows = drawing.get("per_frame")
    if not isinstance(rows, list) or drawing.get("frames") != len(rows):
        raise RuntimeError("drawing counter frame count does not match the recorded rows")
    for row in rows:
        if not isinstance(row, list) or len(row) != len(DRAWING_COUNTERS) or any(
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
    if tuple(drawing.get("gauges", ())) != DRAWING_GAUGES:
        raise RuntimeError("drawing gauge names do not match this profiler")
    result["per_frame"] = {name: drawing_distribution([row[index] for row in rows],
                                                      name in DRAWING_GAUGES)
                           for index, name in enumerate(DRAWING_COUNTERS)}
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


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def write_report(output, report):
    request, actual = report["request"], report["engine"]
    label = "HEADLESS SOFTWARE SMOKE TEST" if request["headless"] else "Native performance baseline"
    lines = [f"# {label}", "", f"Scene: {request['scene']}; campaign: {request['campaign']}; level: {request['level']}.",
             f"Actual turns: {actual['start']['turn']}–{actual['end']['turn'] - 1} ({request['turns']} simulation updates).",
             f"Backend: {actual['video_driver']} / {actual['renderer']}; logical resolution: {actual['width']}×{actual['height']}; "
             f"output: {actual['output_width']}×{actual['output_height']}.",
             f"Frame cap: {actual['fps_limit']}; VSync: {actual['vsync_actual']}; interpolation: {actual['interpolation']}.",
             f"Population at start/end: creatures {actual['start']['creatures']}/{actual['end']['creatures']}; "
             f"things {actual['start']['things']}/{actual['end']['things']}.", "",
             "Wall-clock milliseconds; percentile estimates use linear interpolation.", "",
             "| Scope | Samples | Mean | Median | p90 | p95 | p99 | Max |",
             "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for kind, stats in report["wall_ms"].items():
        lines.append(f"| {kind} | {stats['count']} | " + " | ".join(f"{stats[key]:.3f}" for key in ("mean", "median", "p90", "p95", "p99", "max")) + " |")
    interval = report["wall_ms"]["frame_interval"]["mean"]
    if interval:
        lines += ["", f"Observed presentation rate from mean frame interval: {1000 / interval:.2f} frames/s."]
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
            waits = drawing["per_frame"]["wait_ns"]
            lines += ["", f"Blocking host wait: {waits['mean'] / 1_000_000:.3f} ms mean, "
                      f"{waits['p95'] / 1_000_000:.3f} ms p95 per frame. No GPU execution time is "
                      "collected; every column above is host-side."]
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
    parser.add_argument("--headless", action="store_true", help="dummy/software smoke test, not a native performance baseline")
    args = parser.parse_args()
    if args.backend == "rust" and (args.headless or sys.platform != "darwin"):
        parser.error("--backend rust requires native macOS; --headless is only for the original backend")
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
              "engine_sha256": sha256(engine), "settings": SETTINGS}
    write_json(output / "report.json", report)
    try:
        with tempfile.TemporaryDirectory(prefix="profile-game-", dir=work_root) as temporary:
            work = Path(temporary)
            try:
                capture.clone_assets(game, work, args.resolution)
                configure(work)
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
                if "Performance capture failed:" in result.stderr:
                    raise RuntimeError("engine reported a performance capture failure")
                report.update(summarize(output, args), status="complete")
            except subprocess.TimeoutExpired as error:
                for name, value in (("stdout.log", error.stdout), ("stderr.log", error.stderr)):
                    (output / name).write_text(value.decode(errors="replace") if isinstance(value, bytes) else value or "")
                raise RuntimeError(f"engine exceeded the {report['timeout_seconds']} second deadline") from error
            finally:
                if (work / "keeperfx.log").is_file():
                    (output / "keeperfx.log").write_bytes((work / "keeperfx.log").read_bytes())
        write_report(output, report)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
        report.update(status="failed", error=str(error))
        write_json(output / "report.json", report)
        raise RuntimeError(f"profiling failed: {error}; diagnostics preserved in {output}") from error
    write_json(output / "report.json", report)
    print(f"Profiled {args.turns} {args.scene} simulation turns: {output / 'report.md'}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError) as error:
        sys.exit(str(error))
