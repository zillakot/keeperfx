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
SETTINGS = {
    "DELTA_TIME": "ON", "TURNS_PER_SECOND": "20", "FRAMES_PER_SECOND": "60", "VSYNC": "OFF",
    "FREEZE_GAME_ON_FOCUS_LOST": "OFF", "CAPTURE_CURSOR": "OFF",
    "CURSOR_EDGE_CAMERA_PANNING": "OFF", "LOCK_CURSOR_IN_POSSESSION": "OFF",
}
LIMITATIONS = [
    "All timings are monotonic wall-clock durations, including scheduling and blocking; CPU time is not collected.",
    "GPU execution time is not collected. Presentation and present_wait are host-side durations, not GPU timings.",
    "Presentation includes present_wait; these overlapping scopes must not be added together.",
    "Frame intervals measure observed presentation pacing; simulation samples count actual game updates.",
    "Seeds and population snapshots are observations, not a guarantee of deterministic replay.",
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
                   and key not in ("SDL_VIDEODRIVER", "SDL_VIDEO_DRIVER", "SDL_RENDER_DRIVER", "SDL_RENDER_VSYNC")}
    if args.headless:
        environment.update(SDL_VIDEODRIVER="dummy", SDL_VIDEO_DRIVER="dummy", SDL_RENDER_DRIVER="software")
    elif sys.platform == "darwin":
        environment.update(SDL_VIDEODRIVER="cocoa", SDL_VIDEO_DRIVER="cocoa", SDL_RENDER_DRIVER="metal")
    environment.update(SDL_RENDER_VSYNC="0", KFX_PERF_OUTPUT=str(output / "raw.csv"),
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
    expected_backend = ("dummy", "software") if args.headless else (("cocoa", "metal") if sys.platform == "darwin" else None)
    if expected_backend and (metadata["video_driver"], metadata["renderer"]) != expected_backend:
        raise RuntimeError(f"engine backend does not match requested {expected_backend}")
    if metadata["output_width"] <= 0 or metadata["output_height"] <= 0:
        raise RuntimeError("engine did not report a valid presentation output size")
    if not args.headless and metadata["video_driver"] in ("dummy", "offscreen", "unknown", ""):
        raise RuntimeError("native baseline requires a window-system video driver")
    samples = {kind: [] for kind in KINDS}
    sample_turns = {kind: [] for kind in KINDS}
    with (output / "raw.csv").open(newline="") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != ["kind", "turn", "wall_ns"]:
            raise RuntimeError("unexpected performance CSV schema")
        for row in reader:
            kind, turn, duration = row["kind"], int(row["turn"]), int(row["wall_ns"])
            if kind not in samples or duration < 0 or not start <= turn <= end:
                raise RuntimeError("invalid sample kind, duration or turn")
            samples[kind].append(duration)
            sample_turns[kind].append(turn)
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
    return {"engine": metadata, "wall_ms": {kind: distribution(values) for kind, values in samples.items()},
            "percentile_method": "linear interpolation at (sample_count - 1) * percentile / 100",
            "limitations": LIMITATIONS + (["HEADLESS SOFTWARE SMOKE TEST: not a native presentation baseline."] if args.headless else [])}


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
    parser.add_argument("--campaign", default="keeporig")
    parser.add_argument("--level", type=int, help="override the preset map (quiet/possession: 1; busy: 20)")
    parser.add_argument("--resolution", type=capture.resolution, default=(640, 480))
    parser.add_argument("--warmup-turns", type=int, default=40, help="earliest game turn to begin measuring (1..600)")
    parser.add_argument("--turns", type=int, default=200, help="actual simulation updates to measure (20..1200)")
    parser.add_argument("--headless", action="store_true", help="dummy/software smoke test, not a native performance baseline")
    args = parser.parse_args()
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
                              if key.startswith(("KFX_PERF_", "SDL_"))},
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
