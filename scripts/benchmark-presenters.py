#!/usr/bin/env python3
import argparse
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("profile_game", Path(__file__).with_name("profile-game.py"))
profile = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(profile)
SCENES = ("quiet", "busy", "possession")


def schedule(pairs):
    entries = []
    for pair in range(1, pairs + 1):
        offset = (pair - 1) % len(SCENES)
        for scene in SCENES[offset:] + SCENES[:offset]:
            for backend in (("original", "rust") if pair % 2 else ("rust", "original")):
                entries.append({"pair": pair, "scene": scene, "backend": backend,
                                "path": f"runs/{scene}-p{pair:02d}-{backend}"})
    return entries


def load_report(directory):
    report = json.loads((directory / "report.json").read_text())
    if report.get("status") != "complete" or report.get("format") != "KFXPERFREPORT01":
        raise RuntimeError(f"incomplete report: {directory}")
    args = argparse.Namespace(**report["request"])
    args.resolution = tuple(args.resolution)
    if args.headless:
        raise RuntimeError("headless results cannot enter a native A/B comparison")
    report.update(profile.summarize(directory, args))
    if report["resources"]["process_cpu"] is None or report["resources"]["rust_allocations"] is None:
        raise RuntimeError(f"matched experiment requires measured CPU and scoped allocation counters: {directory}")
    return report


def identity(report):
    request, engine = report["request"], report["engine"]
    return {"engine_sha256": report["engine_sha256"], "assets_sha256": report["assets"]["sha256"],
            "config_sha256": report["config_sha256"], "platform": report["platform"],
            "settings": report["settings"],
            "request": {key: request[key] for key in ("scene", "campaign", "level", "resolution", "warmup_turns", "turns")},
            "actual": {key: engine[key] for key in ("width", "height", "output_width", "output_height",
                                                    "vsync_actual", "fps_limit", "turns_per_second", "interpolation")}}


def summary(values):
    return {"runs": values, "median": statistics.median(values), "min": min(values), "max": max(values)}


def paired_change(original, rust):
    if original <= 0 or rust <= 0:
        raise RuntimeError("paired duration comparison requires positive durations")
    return {"original": original, "rust": rust, "time_saved_percent": 100 * (original - rust) / original,
            "speedup_factor": original / rust}


def compare(entries, reports):
    result = {"format": "KFXAB01", "aggregation": "Each run has equal weight; median and range of per-run statistics and paired changes.",
              "limitations": profile.LIMITATIONS, "scenes": {}}
    for scene in SCENES:
        pairs = {}
        for entry, report in zip(entries, reports, strict=True):
            if entry["scene"] != scene:
                continue
            if report["request"]["scene"] != scene or report["request"]["backend"] != entry["backend"]:
                raise RuntimeError("report scene/backend does not match scheduled run")
            pair = pairs.setdefault(entry["pair"], {})
            if entry["backend"] in pair:
                raise RuntimeError("duplicate backend in pair")
            pair[entry["backend"]] = report
        if not pairs:
            raise RuntimeError(f"missing scene: {scene}")
        expected = None
        for pair in pairs.values():
            if set(pair) != {"original", "rust"}:
                raise RuntimeError(f"incomplete backend pair for {scene}")
            for report in pair.values():
                current = identity(report)
                if expected is not None and current != expected:
                    raise RuntimeError(f"mismatched build, assets, settings, host or output size for {scene}")
                expected = current
        ordered = [pairs[index] for index in sorted(pairs)]
        scopes = {}
        for scope in profile.KINDS:
            scopes[scope] = {}
            for stat in ("mean", "median", "p95"):
                changes = [paired_change(pair["original"]["wall_ms"][scope][stat], pair["rust"]["wall_ms"][scope][stat])
                           for pair in ordered]
                scopes[scope][stat] = {backend: summary([change[backend] for change in changes]) for backend in ("original", "rust")}
                scopes[scope][stat].update(pairs=changes,
                                          time_saved_percent=summary([change["time_saved_percent"] for change in changes]))
        cpu = None
        if all(pair[backend]["resources"]["process_cpu"] for pair in ordered for backend in ("original", "rust")):
            changes = [paired_change(pair["original"]["resources"]["process_cpu"]["ms_per_turn"],
                                     pair["rust"]["resources"]["process_cpu"]["ms_per_turn"]) for pair in ordered]
            cpu = {backend: summary([change[backend] for change in changes]) for backend in ("original", "rust")}
            cpu.update(pairs=changes, time_saved_percent=summary([change["time_saved_percent"] for change in changes]))
        allocation_runs = {backend: [pair[backend]["resources"]["rust_allocations"] for pair in ordered]
                           for backend in ("original", "rust")}
        result["scenes"][scene] = {"pair_count": len(ordered), "identity": expected, "wall_ms": scopes,
                                   "process_cpu_ms_per_turn": cpu, "rust_allocations": allocation_runs}
    return result


def write_comparison(output, comparison):
    profile.write_json(output / "comparison.json", comparison)
    lines = ["# Matched native presentation comparison", "", comparison["aggregation"], "",
             "Positive time saved means lower duration. These capped runs measure host overhead and frame pacing, not uncapped FPS.", "",
             "| Scene | Scope | Statistic | Original ms | Rust ms | Paired time saved % (median; min to max) |",
             "| --- | --- | --- | ---: | ---: | ---: |"]
    for scene, values in comparison["scenes"].items():
        for scope in ("simulation", "draw", "presentation", "frame_interval"):
            for stat in ("median", "p95"):
                row = values["wall_ms"][scope][stat]
                change = row["time_saved_percent"]
                lines.append(f"| {scene} | {scope} | {'median of run p95s' if stat == 'p95' else 'median of run medians'} | "
                             f"{row['original']['median']:.3f} | {row['rust']['median']:.3f} | "
                             f"{change['median']:.1f}; {change['min']:.1f} to {change['max']:.1f} |")
        cpu = values["process_cpu_ms_per_turn"]
        if cpu:
            change = cpu["time_saved_percent"]
            lines.append(f"| {scene} | Process CPU / turn | median of runs | {cpu['original']['median']:.3f} | "
                         f"{cpu['rust']['median']:.3f} | {change['median']:.1f}; {change['min']:.1f} to {change['max']:.1f} |")
    lines += ["", "Rust allocation counts and bytes are retained per run in comparison.json; their scope excludes the original C/C++/SDL heap.",
              "", *[f"- {item}" for item in comparison["limitations"]], "",
              "Native conditions and run order: manifest.json. Complete reports and raw samples: runs/. "
              "Full lifecycle, gameplay, pixels and total process allocation evidence require separate checks."]
    (output / "comparison.md").write_text("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description="Serial matched native original/Rust presentation measurements.")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--warmup-turns", type=int, default=40)
    parser.add_argument("--turns", type=int, default=200)
    parser.add_argument("--resolution", type=profile.capture.resolution, default=(640, 480))
    parser.add_argument("--conditions", required=True, help="display/scaling, power mode and observed background load during collection")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("the live Rust comparison currently requires native macOS")
    if not 5 <= args.pairs <= 30 or not 1 <= args.warmup_turns <= 600 or not 20 <= args.turns <= 1200:
        parser.error("pairs must be 5..30, warmup turns 1..600 and measured turns 20..1200")
    output = args.out.resolve()
    if not output.is_relative_to((ROOT / "out").resolve()) or output == (ROOT / "out").resolve() or output.exists():
        parser.error("output must be a new directory beneath ignored out")
    engine, game = args.engine.resolve(), args.game_dir.resolve()
    if not engine.is_file() or not (game / "keeperfx.cfg").is_file():
        parser.error("build the engine and prepare assets first")
    output.mkdir(parents=True)
    (output / "runs").mkdir()
    entries = schedule(args.pairs)
    manifest = {"format": "KFXABMANIFEST01", "status": "running", "started_utc": datetime.now(timezone.utc).isoformat(),
                "conditions": args.conditions, "engine_sha256": profile.sha256(engine), "runs": entries}
    profile.write_json(output / "manifest.json", manifest)
    try:
        for entry in entries:
            if profile.sha256(engine) != manifest["engine_sha256"]:
                raise RuntimeError("engine changed during the experiment")
            entry["started_utc"] = datetime.now(timezone.utc).isoformat()
            command = [sys.executable, str(ROOT / "scripts/profile-game.py"), "--engine", str(engine), "--game-dir", str(game),
                       "--out", str(output / entry["path"]), "--scene", entry["scene"], "--backend", entry["backend"],
                       "--warmup-turns", str(args.warmup_turns), "--turns", str(args.turns),
                       "--resolution", f"{args.resolution[0]}x{args.resolution[1]}"]
            entry["command"] = command
            profile.write_json(output / "manifest.json", manifest)
            print(f"Pair {entry['pair']}/{args.pairs}: {entry['scene']} / {entry['backend']}", flush=True)
            subprocess.run(command, check=True)
            entry["finished_utc"] = datetime.now(timezone.utc).isoformat()
            entry["status"] = "complete"
            entry["sha256"] = {name: profile.sha256(output / entry["path"] / name)
                               for name in ("report.json", "raw.csv", "raw.csv.json", "keeperfx.cfg")}
            profile.write_json(output / "manifest.json", manifest)
        reports = [load_report(output / entry["path"]) for entry in entries]
        comparison = compare(entries, reports)
        if any(report["engine_sha256"] != manifest["engine_sha256"] for report in reports):
            raise RuntimeError("report engine identity differs from the experiment")
        write_comparison(output, comparison)
        manifest["status"] = "complete"
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.CalledProcessError) as error:
        manifest.update(status="failed", error=str(error))
        raise
    finally:
        manifest["finished_utc"] = datetime.now(timezone.utc).isoformat()
        profile.write_json(output / "manifest.json", manifest)
    print(output / "comparison.md")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
