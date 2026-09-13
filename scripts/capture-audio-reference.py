#!/usr/bin/env python3
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def summarize_trace(path):
    records = [json.loads(line) for line in path.read_text().splitlines()]
    if len(records) < 2 or records[0].get("format") != "KFXAUDIO1" or records[-1].get("event") != "trace_end":
        raise ValueError("trace missing header or clean-exit footer")
    body, footer = records[1:-1], records[-1]
    if footer.get("records") != len(body) or footer.get("overflow") != 0:
        raise ValueError("trace incomplete or capacity exceeded; capture a shorter reference")
    voices, peak = set(), 0
    previous_tick = 0
    for index, record in enumerate(body):
        if record.get("sequence") != index or record["tick"] < previous_tick:
            raise ValueError("trace event order is invalid")
        previous_tick = record["tick"]
        if record["event"] in ("start", "restart"):
            voices.add(record["voice"])
        elif record["event"] in ("complete", "stop", "stop_all"):
            voices.discard(record["voice"])
        peak = max(peak, len(voices))
    counts = Counter(record["event"] for record in body)
    if not counts["openal_ready"]:
        raise ValueError("OpenAL did not initialize; this is not an audio-enabled reference")
    return dict(events=dict(sorted(counts.items())), records=len(body),
                peak_observed_openal_voices=peak,
                drops={key: value for key, value in sorted(counts.items()) if key.startswith("drop_")},
                tick_range=[min(r["tick"] for r in body), max(r["tick"] for r in body)],
                turn_range=[min(r["turn"] for r in body), max(r["turn"] for r in body)])


def dependency_evidence(engine):
    command = ["otool", "-L", str(engine)] if sys.platform == "darwin" else ["ldd", str(engine)]
    if not shutil.which(command[0]):
        return dict(command=command, available=False)
    result = subprocess.run(command, capture_output=True, text=True)
    return dict(command=command, returncode=result.returncode, output=result.stdout + result.stderr)


def main():
    parser = argparse.ArgumentParser(description="Capture an isolated audio-enabled command reference; no audio recording is made.")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--scene", choices=("menu", "dungeon", "possession"), default="dungeon")
    parser.add_argument("--campaign", default="keeporig")
    parser.add_argument("--level", type=int, default=1)
    parser.add_argument("--turn", type=int, default=20)
    parser.add_argument("--frames", type=int, default=8)
    parser.add_argument("--interval", type=int, default=20)
    args = parser.parse_args()
    if not 1 <= args.turn <= 600 or not 1 <= args.frames <= 32 or not 1 <= args.interval <= 60:
        parser.error("turn must be 1–600; frames 1–32; interval 1–60")
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.campaign) or not 1 <= args.level <= 99999:
        parser.error("invalid campaign or level")
    output, engine, source = args.out.resolve(), args.engine.resolve(), args.game_dir.resolve()
    work_root = (ROOT / "out").resolve()
    if not output.is_relative_to(work_root) or output == work_root or output.exists():
        parser.error("output must be a new directory beneath repository out/")
    if not engine.is_file() or not (source / "keeperfx.cfg").is_file():
        parser.error("engine or game data missing")
    spec = importlib.util.spec_from_file_location("capture_frame", ROOT / "scripts/capture-frame.py")
    capture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    output.mkdir(parents=True)
    retained_engine = output / engine.name
    shutil.copy2(engine, retained_engine)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(("KFX_", "SDL_", "ALSOFT_"))}
    environment.update(SDL_VIDEODRIVER="dummy", SDL_RENDER_DRIVER="software",
                       KFX_AUDIO_TRACE=str(output / "audio.jsonl"),
                       KFX_FRAME_CAPTURE=str(output / "frames"), KFX_FRAME_CAPTURE_TURN=str(args.turn),
                       KFX_FRAME_CAPTURE_EXIT="1", KFX_FRAME_CAPTURE_SCENE=args.scene,
                       KFX_FRAME_CAPTURE_COUNT=str(args.frames), KFX_FRAME_CAPTURE_INTERVAL=str(args.interval))
    command = [str(retained_engine), "-nointro", "-altinput", "-skipheartzoom"]
    if args.scene != "menu":
        command += ["-campaign", args.campaign, "-level", str(args.level)]
    with tempfile.TemporaryDirectory(prefix="audio-reference-", dir=work_root) as temporary:
        work = Path(temporary)
        capture.clone_assets(source, work)
        shutil.copy2(work / "keeperfx.cfg", output / "keeperfx.cfg")
        timeout = max(120, 60 + (args.turn + args.frames * args.interval) // 20)
        try:
            result = subprocess.run(command, cwd=work, env=environment, capture_output=True, text=True, timeout=timeout)
            (output / "process.log").write_text(result.stdout + result.stderr)
        finally:
            if (work / "keeperfx.log").exists():
                shutil.copy2(work / "keeperfx.log", output / "keeperfx.log")
        if result.returncode:
            raise RuntimeError(f"engine exited with {result.returncode}; inspect {output}")
    directories = capture.capture_directories(output / "frames", args.frames)
    expected_view = {"menu": "main_menu", "dungeon": "dungeon_top", "possession": "creature"}[args.scene]
    for directory in directories:
        state = json.loads((directory / "capture.json").read_text())
        if state.get("view") != expected_view or state.get("scene") != args.scene:
            raise RuntimeError("reference scene was not reached")
        if args.scene != "menu" and state["game_turn"] < args.turn:
            raise RuntimeError("reference precedes requested turn")
    summary = summarize_trace(output / "audio.jsonl")
    manifest = dict(format="KFXAUDIOREFERENCE1", scene=args.scene, campaign=args.campaign, level=args.level,
                    requested_turn=args.turn, frames=args.frames, interval=args.interval,
                    command=command, environment={key: value for key, value in environment.items()
                                               if key.startswith(("KFX_", "SDL_"))}, summary=summary,
                    engine_sha256=hashlib.sha256(retained_engine.read_bytes()).hexdigest(),
                    dependencies=dependency_evidence(retained_engine),
                    evidence="Native OpenAL command submission and observed completion; no signal recording or listening assessment")
    (output / "reference.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(dict(output=str(output), summary=summary), indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        sys.exit(str(error))
