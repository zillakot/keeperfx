#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ASSET_DIRS = ("campgns", "creatrs", "data", "fxdata", "ldata", "levels", "mods", "multiplayer", "music", "sound")


def clone_assets(source, destination):
    for name in ASSET_DIRS:
        src, dst = source / name, destination / name
        if not src.is_dir():
            continue
        if sys.platform == "darwin":
            result = subprocess.run(["/bin/cp", "-cR", str(src), str(dst)], capture_output=True)
            if result.returncode == 0:
                continue
            shutil.rmtree(dst, ignore_errors=True)
        shutil.copytree(src, dst)
    for name in ("save", "scrshots"):
        (destination / name).mkdir()
    settings = (source / "keeperfx.cfg").read_text()
    values = {
        "API_ENABLED": "FALSE", "DELTA_TIME": "OFF",
        "FRONTEND_RES": "640x480w32 640x480w32 640x480w32",
        "INGAME_RES": "640x480w32 640x480w32 640x480w32",
    }
    for key, value in values.items():
        settings, count = re.subn(rf"^{key}=.*$", f"{key}={value}", settings, flags=re.M)
        if not count:
            settings += f"\n{key}={value}\n"
    (destination / "keeperfx.cfg").write_text(settings)


def main():
    parser = argparse.ArgumentParser(description="Capture a game frame without opening a window or using personal saves.")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--turn", type=int, default=20)
    args = parser.parse_args()
    if not 1 <= args.turn <= 600:
        parser.error("--turn must be between 1 and 600")
    output, engine = args.out.resolve(), args.engine.resolve()
    if output.exists():
        parser.error("output already exists; choose a new capture directory")
    if not engine.is_file() or not (args.game_dir / "keeperfx.cfg").is_file():
        parser.error("build the engine and prepare the game data first")
    output.parent.mkdir(parents=True, exist_ok=True)
    work_root = ROOT / "out"
    work_root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="frame-capture-", dir=work_root) as temporary:
        work = Path(temporary)
        clone_assets(args.game_dir.resolve(), work)
        environment = dict(os.environ, SDL_VIDEODRIVER="dummy", SDL_RENDER_DRIVER="software",
                           KFX_FRAME_CAPTURE=str(output), KFX_FRAME_CAPTURE_TURN=str(args.turn),
                           KFX_FRAME_CAPTURE_EXIT="1")
        result = subprocess.run([str(engine), "-nointro", "-nosound", "-altinput", "-skipheartzoom",
                                 "-campaign", "keeporig", "-level", "1"], cwd=work, env=environment,
                                capture_output=True, text=True, timeout=60)
        log = (work / "keeperfx.log").read_text(errors="replace") if (work / "keeperfx.log").exists() else ""
        if result.returncode or not all((output / name).is_file() for name in ("frame.kfx", "reference.png", "capture.json")):
            raise RuntimeError(f"capture failed (exit {result.returncode})\n{result.stderr[-2000:]}\n{log[-4000:]}")
        (output / "keeperfx.log").write_text(log)
        (output / "process.log").write_text(result.stdout + result.stderr)
        metadata = json.loads((output / "capture.json").read_text())
        metadata.update(campaign="keeporig", level=1, requested_turn=args.turn,
                        engine_sha256=hashlib.sha256(engine.read_bytes()).hexdigest(),
                        frame_sha256=hashlib.sha256((output / "frame.kfx").read_bytes()).hexdigest(),
                        reference_sha256=hashlib.sha256((output / "reference.png").read_bytes()).hexdigest())
        (output / "capture.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Captured turn {metadata['game_turn']}: {output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        sys.exit(str(error))
