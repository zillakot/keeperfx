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


def resolution(value):
    match = re.fullmatch(r"([0-9]+)x([0-9]+)", value)
    if not match:
        raise argparse.ArgumentTypeError("resolution must be WIDTHxHEIGHT")
    width, height = map(int, match.groups())
    if not (320 <= width <= 8192 and 200 <= height <= 8192 and width * height <= 16 * 1024 * 1024):
        raise argparse.ArgumentTypeError("resolution must be at least 320x200, at most 8192 per axis and 16 megapixels")
    return width, height


def clone_assets(source, destination, size=(640, 480)):
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
    mode = f"{size[0]}x{size[1]}w32"
    values = {
        "API_ENABLED": "FALSE", "DELTA_TIME": "OFF", "TURNS_PER_SECOND": "20",
        "FRONTEND_RES": " ".join([mode] * 3),
        "INGAME_RES": " ".join([mode] * 3),
    }
    for key, value in values.items():
        settings, count = re.subn(rf"^{key}\s*=.*$", f"{key}={value}", settings, flags=re.M)
        if not count:
            settings += f"\n{key}={value}\n"
    (destination / "keeperfx.cfg").write_text(settings)


def capture_directories(output, count):
    directories = [output] if count == 1 else [output / f"frame-{index:04}" for index in range(count)]
    if count > 1:
        sequence = json.loads((output / "sequence.json").read_text())
        expected = [{"frame": f"frame-{index:04}/frame.kfx", "reference": f"frame-{index:04}/reference.png"}
                    for index in range(count)]
        if sequence != {"format": "KFXSEQ01", "frames": expected}:
            raise RuntimeError("capture sequence manifest does not match requested frames")
    if not all((directory / name).is_file() for directory in directories
               for name in ("frame.kfx", "reference.png", "capture.json")):
        raise RuntimeError("capture output is incomplete")
    return directories


def main():
    parser = argparse.ArgumentParser(description="Capture game references headlessly with isolated assets, settings and saves.")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--out", type=Path, required=True, help="new directory beneath the repository's ignored out directory")
    parser.add_argument("--scene", choices=("dungeon", "menu", "possession"), default="dungeon")
    parser.add_argument("--campaign", default="keeporig")
    parser.add_argument("--level", type=int, default=1)
    parser.add_argument("--resolution", type=resolution, default=(640, 480))
    parser.add_argument("--turn", type=int, default=20, help="earliest gameplay turn; ignored for menu")
    parser.add_argument("--frames", type=int, default=1)
    parser.add_argument("--interval", type=int, default=1, help="eligible presentation calls between captures")
    args = parser.parse_args()
    if not 1 <= args.turn <= 600:
        parser.error("--turn must be between 1 and 600")
    if not 1 <= args.frames <= 32 or not 1 <= args.interval <= 60:
        parser.error("--frames must be between 1 and 32; --interval between 1 and 60")
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.campaign) or not 1 <= args.level <= 99999:
        parser.error("--campaign must be an identifier; --level must be between 1 and 99999")
    output, engine = args.out.resolve(), args.engine.resolve()
    work_root = ROOT / "out"
    if not output.is_relative_to(work_root.resolve()) or output == work_root.resolve():
        parser.error("capture output must be beneath the repository's ignored out directory")
    if output.exists():
        parser.error("output already exists; choose a new capture directory")
    if not engine.is_file() or not (args.game_dir / "keeperfx.cfg").is_file():
        parser.error("build the engine and prepare the game data first")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="frame-capture-", dir=work_root) as temporary:
        work = Path(temporary)
        clone_assets(args.game_dir.resolve(), work, args.resolution)
        environment = {key: value for key, value in os.environ.items() if not key.startswith("KFX_FRAME_CAPTURE")}
        environment.update(SDL_VIDEODRIVER="dummy", SDL_RENDER_DRIVER="software",
                           KFX_FRAME_CAPTURE=str(output), KFX_FRAME_CAPTURE_TURN=str(args.turn),
                           KFX_FRAME_CAPTURE_EXIT="1", KFX_FRAME_CAPTURE_SCENE=args.scene,
                           KFX_FRAME_CAPTURE_COUNT=str(args.frames), KFX_FRAME_CAPTURE_INTERVAL=str(args.interval))
        command = [str(engine), "-nointro", "-nosound", "-altinput", "-skipheartzoom"]
        if args.scene != "menu":
            command += ["-campaign", args.campaign, "-level", str(args.level)]
        scheduled_turns = (0 if args.scene == "menu" else args.turn) + (args.frames - 1) * args.interval
        timeout = max(120, 60 + (scheduled_turns + 19) // 20)
        result = subprocess.run(command, cwd=work, env=environment,
                                capture_output=True, text=True, timeout=timeout)
        log = (work / "keeperfx.log").read_text(errors="replace") if (work / "keeperfx.log").exists() else ""
        try:
            if result.returncode:
                raise RuntimeError(f"engine exited with {result.returncode}")
            directories = capture_directories(output, args.frames)
        except (OSError, ValueError, RuntimeError) as error:
            raise RuntimeError(f"capture failed: {error}\n{result.stderr[-2000:]}\n{log[-4000:]}") from error
        (output / "keeperfx.log").write_text(log)
        (output / "process.log").write_text(result.stdout + result.stderr)
        engine_hash = hashlib.sha256(engine.read_bytes()).hexdigest()
        previous_ordinal = None
        for index, directory in enumerate(directories):
            metadata = json.loads((directory / "capture.json").read_text())
            if (metadata["width"], metadata["height"]) != args.resolution:
                raise RuntimeError(f"engine captured {metadata['width']}x{metadata['height']}, not requested {args.resolution}")
            expected_view = {"menu": "main_menu", "dungeon": "dungeon_top", "possession": "creature"}[args.scene]
            if metadata.get("scene") != args.scene or metadata.get("view") != expected_view or metadata.get("frame_index") != index:
                raise RuntimeError("capture scene or frame index does not match the request; rebuild the engine")
            ordinal = metadata.get("present_ordinal", 0)
            if ordinal < 1 or (previous_ordinal is not None and ordinal - previous_ordinal < args.interval):
                raise RuntimeError("capture presentation order does not match requested interval")
            previous_ordinal = ordinal
            if args.scene != "menu" and metadata["game_turn"] < args.turn:
                raise RuntimeError("capture precedes requested gameplay turn")
            metadata.update(campaign=args.campaign if args.scene != "menu" else None,
                            level=args.level if args.scene != "menu" else None, requested_scene=args.scene,
                            requested_turn=args.turn if args.scene != "menu" else None,
                            requested_resolution=list(args.resolution), requested_interval=args.interval,
                            engine_sha256=engine_hash,
                            frame_sha256=hashlib.sha256((directory / "frame.kfx").read_bytes()).hexdigest(),
                            reference_sha256=hashlib.sha256((directory / "reference.png").read_bytes()).hexdigest())
            (directory / "capture.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Captured {args.frames} {args.scene} frame(s): {output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        sys.exit(str(error))
