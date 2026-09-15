#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
API_ACTIONS = {"script": "map_command", "console": "console_command"}


def read_session(path):
    path = path.resolve()
    data = json.loads(path.read_text())
    if data.get("format") != "KFXCONTROL1" or not re.fullmatch(r"[0-9a-f]{64}", data.get("token", "")) or not re.fullmatch(r"[0-9a-f]{32}", data.get("session_id", "")):
        raise ValueError("not a native control session descriptor")
    if not isinstance(data.get("port"), int) or not 1024 <= data["port"] <= 65535:
        raise ValueError("invalid session port")
    if Path(data["work"]).resolve() != path.parent:
        raise ValueError("session directory does not match descriptor")
    return data


class Client:
    def __init__(self, session, timeout=20):
        self.session = session
        self.deadline = time.monotonic() + timeout
        self.ack = 0
        connect_deadline = time.monotonic() + 2
        while True:
            self.socket = socket.create_connection(("127.0.0.1", session["port"]), timeout=2)
            self.stream = self.socket.makefile("rb")
            try:
                self.request("state")
                break
            except (OSError, ConnectionError):
                self.close()
                if time.monotonic() >= connect_deadline:
                    raise
                time.sleep(0.05)
            except Exception:
                self.close()
                raise

    def close(self):
        self.stream.close()
        self.socket.close()

    def exchange(self, message):
        self.ack += 1
        self.socket.settimeout(max(0.01, min(2, self.deadline - time.monotonic())))
        message = dict(message, token=self.session["token"], ack=self.ack)
        self.socket.sendall(json.dumps(message, separators=(",", ":")).encode() + b"\n")
        while True:
            raw = self.stream.readline(4097)
            if not raw or len(raw) > 4096:
                raise ConnectionError("control connection closed or oversized response")
            reply = json.loads(raw)
            if reply.get("ack") != self.ack:
                continue
            if not reply.get("success"):
                raise RuntimeError(reply.get("error", reply))
            return reply

    def request(self, op, **fields):
        data = self.exchange(dict(action="control", op=op, **fields))["data"]
        if data.get("session") != self.session["session_id"]:
            raise RuntimeError("wrong game session")
        return data

    def api(self, action, **fields):
        """Non-control API action; the game answers without a control state block."""
        self.exchange(dict(action=action, **fields))

    def run(self, op, **fields):
        conditions = fields.pop("until", [])
        expected = {}
        allowed = {"frontend", "view", "width", "height", "fullscreen", "minimized", "focused", "grabbed", "paused", "presenter"}
        for condition in conditions:
            key, separator, value = condition.partition("=")
            if not separator or key not in allowed:
                raise ValueError("invalid state predicate")
            expected[key] = value if key == "presenter" else json.loads(value)
        before = self.request("state")
        if op in API_ACTIONS:
            self.api(API_ACTIONS[op], command=fields["command"])
            result = self.request("state")
        else:
            result = self.request(op, **fields)
        if op == "quit":
            exit_path = Path(self.session["work"]) / "exit.json"
            while not exit_path.exists():
                if time.monotonic() >= self.deadline:
                    raise TimeoutError("game did not exit after quit")
                time.sleep(0.05)
            result["exit"] = json.loads(exit_path.read_text())
            if result["exit"]["returncode"] != 0 or result["exit"]["timed_out"]:
                raise RuntimeError(f"game exited abnormally: {result['exit']}")
        if op not in ("state", "quit") and op not in API_ACTIONS:
            command = result["command"]
            while result["busy"] or result["completed"] < command:
                if time.monotonic() >= self.deadline:
                    raise TimeoutError("control action did not finish; disconnect releases held inputs")
                time.sleep(0.03)
                result = self.request("state")
        if result.get("error") and op not in ("state", "cancel"):
            raise RuntimeError(result["error"])
        while any(result.get(key) != value for key, value in expected.items()):
            if time.monotonic() >= self.deadline:
                raise TimeoutError(f"state predicate not reached: {expected}")
            time.sleep(0.05)
            result = self.request("state")
        for state in (before, result):
            state.pop("session", None)
        output = dict(input_source="native game event injection", op=op, before=before, after=result)
        if op in API_ACTIONS:
            output.update(input_source="game API action", action=API_ACTIONS[op], command=fields["command"])
        if op == "snapshot":
            screenshot = Path(self.session["work"]) / "scrshots" / f"control-{result['command']}.png"
            if not screenshot.is_file():
                raise RuntimeError("game did not produce the scheduled screenshot")
            output["screenshot"] = str(screenshot)
        return output


def validate_launch(args):
    if not 60 <= args.lifetime <= 3600 or (args.level is not None and not 1 <= args.level <= 99999):
        raise ValueError("lifetime must be 60..3600 seconds and level 1..99999")
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.campaign):
        raise ValueError("invalid campaign identifier")
    language = getattr(args, "language", None)
    if language and not re.fullmatch(r"[A-Za-z]{3}", language):
        raise ValueError("language must be a three-letter code")
    modes = getattr(args, "ingame_res", None)
    if modes and not all(re.fullmatch(r"\d{3,4}x\d{3,4}[wf]\d{1,2}|DESKTOP", mode) for mode in modes.split()):
        raise ValueError("invalid in-game video mode list")
    if getattr(args, "rotate_mode", None) not in (None, 0, 1, 2):
        raise ValueError("rotate mode must be 0, 1 or 2")
    turns = getattr(args, "turns_per_second", None)
    if turns is not None and not 1 <= turns <= 50:
        raise ValueError("turns per second must be 1..50")
    startup = getattr(args, "startup_timeout", None)
    if startup is not None and not 30 <= startup <= 600:
        raise ValueError("startup timeout must be 30..600 seconds")


def setting(text, key, value):
    """Replace the configuration line, or add it when the file does not carry the key."""
    text, count = re.subn(rf"^{key}\s*=.*$", f"{key}={value}", text, flags=re.M)
    return text if count else text + f"\n{key}={value}\n"


def prepare_session(args, work, engine, port):
    """Rewrite the cloned configuration for this session and return its descriptor."""
    config = work / "keeperfx.cfg"
    text = config.read_text().replace("API_ENABLED=FALSE", "API_ENABLED=TRUE")
    text = setting(text, "API_PORT", port)
    text = setting(text, "INGAME_RES", getattr(args, "ingame_res", None) or "640x480w32 DESKTOP 800x600w32")
    if getattr(args, "language", None):
        text = setting(text, "LANGUAGE", args.language)
    if getattr(args, "turns_per_second", None):
        # The legacy fixed pacing ignores the rate, so the delta-time loop has to be on.
        text = setting(text, "DELTA_TIME", "ON")
        text = setting(text, "TURNS_PER_SECOND", args.turns_per_second)
    if getattr(args, "movie_scaling", None) is not None:
        text = setting(text, "RESIZE_MOVIES", "ON" if args.movie_scaling else "OFF")
    config.write_text(text)
    if getattr(args, "rotate_mode", None) is not None:
        (work / "save/settings.toml").write_text(f"[video]\nrotate_mode = {args.rotate_mode}\n")
    manifest = dict(format="KFXCONTROL1", token=secrets.token_hex(32), session_id=secrets.token_hex(16),
                    port=port, work=str(work), engine=str(engine),
                    engine_sha256=hashlib.sha256(engine.read_bytes()).hexdigest(),
                    backend=args.backend, verify=args.verify, lifetime=args.lifetime,
                    draw_backend=args.draw_backend, draw_verify=args.draw_verify,
                    args=["-altinput", "-skipheartzoom"])
    manifest["args"] += ["-bullfrog"] if getattr(args, "play_movies", False) else ["-nointro"]
    if getattr(args, "cheats", False):
        manifest["args"] += ["-alex"]
    if getattr(args, "smoothing", False):
        manifest["args"] += ["-vidsmooth"]
    if args.level:
        manifest["args"] += ["-campaign", args.campaign, "-level", str(args.level)]
    return manifest


def launch(args):
    validate_launch(args)
    work = args.out.resolve()
    if not work.is_relative_to((ROOT / "out").resolve()) or work == (ROOT / "out").resolve():
        raise ValueError("session output must be a new directory under repository out/")
    if work.exists():
        raise ValueError("session output already exists")
    source, engine = args.game_dir.resolve(), args.engine.resolve()
    if not engine.is_file() or not (source / "keeperfx.cfg").is_file():
        raise ValueError("engine or game data missing")
    work.mkdir(parents=True, mode=0o700)
    spec = importlib.util.spec_from_file_location("capture_frame", ROOT / "scripts/capture-frame.py")
    capture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(capture)
    capture.clone_assets(source, work)
    if args.copy_saves:
        shutil.copytree(source / "save", work / "save", dirs_exist_ok=True)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    manifest = prepare_session(args, work, engine, port)
    descriptor = work / "session.json"
    descriptor.write_text(json.dumps(manifest, indent=2) + "\n")
    descriptor.chmod(0o600)
    with (work / "supervisor.log").open("w") as log:
        subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "_supervise", "--session", str(descriptor)],
                         stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    deadline = time.monotonic() + (getattr(args, "startup_timeout", None) or 60)
    while time.monotonic() < deadline:
        if (work / "exit.json").exists():
            raise RuntimeError(f"game exited during startup; inspect {work / 'keeperfx.log'}")
        try:
            client = Client(manifest)
            try:
                state = client.run("state")
            finally:
                client.close()
            return dict(session=str(descriptor), state=state)
        except (OSError, RuntimeError):
            time.sleep(0.2)
    raise TimeoutError(f"game control startup timed out; session supervisor will stop it after {args.lifetime}s")


def supervise(path):
    data = read_session(path)
    work = path.resolve().parent
    env = {k: v for k, v in os.environ.items() if not k.startswith(("KFX_", "SDL_", "ALSOFT_"))}
    env.update(KFX_DEV_CONTROL_TOKEN=data["token"], KFX_DEV_CONTROL_SESSION=data["session_id"],
               KFX_PRESENT_BACKEND=data["backend"])
    if sys.platform == "darwin":
        env["SDL_VIDEO_DRIVER"] = "cocoa"
    if data["verify"]:
        env["KFX_WGPU_VERIFY"] = "1"
    if data.get("draw_backend", "software") != "software":
        env["KFX_DRAW_BACKEND"] = data["draw_backend"]
    if data.get("draw_verify"):
        env["KFX_WGPU_DRAW_VERIFY"] = "1"
        env["KFX_WGPU_DRAW_STATS"] = str(work / "drawing.json")
    with (work / "stdout.log").open("w") as stdout, (work / "stderr.log").open("w") as stderr:
        process = subprocess.Popen([data["engine"], *data["args"]], cwd=work, env=env,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        (work / "process.json").write_text(json.dumps(dict(pid=process.pid, started=time.time())))
        timed_out = False
        try:
            code = process.wait(timeout=data["lifetime"])
        except subprocess.TimeoutExpired:
            timed_out = True
            process.terminate()
            try:
                code = process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                code = process.wait()
        (work / "exit.json").write_text(json.dumps(dict(returncode=code, timed_out=timed_out, ended=time.time())))


def main():
    parser = argparse.ArgumentParser(description="Control an isolated opt-in KeeperFX session through native game events.")
    sub = parser.add_subparsers(dest="command", required=True)
    launch_parser = sub.add_parser("launch")
    launch_parser.add_argument("--out", type=Path, required=True)
    launch_parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    launch_parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    launch_parser.add_argument("--backend", choices=("sdl", "wgpu"), default="sdl")
    launch_parser.add_argument("--verify", action="store_true")
    launch_parser.add_argument("--draw-backend", choices=("software", "wgpu"), default="software")
    launch_parser.add_argument("--draw-verify", action="store_true", help="compare GPU drawing against the CPU oracle; not a performance run")
    launch_parser.add_argument("--copy-saves", action="store_true")
    launch_parser.add_argument("--lifetime", type=int, default=1200)
    launch_parser.add_argument("--campaign", default="keeporig")
    launch_parser.add_argument("--level", type=int)
    launch_parser.add_argument("--cheats", action="store_true", help="pass -alex, which the lua/dbc console commands require")
    launch_parser.add_argument("--play-movies", action="store_true", help="keep the startup movies and force the Bullfrog logo movie")
    launch_parser.add_argument("--smoothing", action="store_true", help="pass -vidsmooth, which enables the screen smoothing pass")
    launch_parser.add_argument("--ingame-res", help="replace the in-game video mode list, e.g. 320x200w32")
    launch_parser.add_argument("--language", help="three-letter language code written to the isolated configuration")
    launch_parser.add_argument("--rotate-mode", type=int, choices=(0, 1, 2), help="0 iso wibble, 1 iso straight, 2 front view")
    launch_parser.add_argument("--startup-timeout", type=int, help="seconds to wait for the control API; startup movies delay it")
    launch_parser.add_argument("--turns-per-second", type=int, help="slow the simulation so short animations span drawn frames")
    launch_parser.add_argument("--movie-scaling", type=int, choices=(0, 1), help="0 plays movies unscaled, which uses the movie draw kind")
    supervisor = sub.add_parser("_supervise", help=argparse.SUPPRESS)
    supervisor.add_argument("--session", type=Path, required=True)
    operations = ("state", "move", "click", "drag", "key", "chord", "cycle-mode", "wait", "resize",
                  "minimize", "restore", "focus", "snapshot", "script", "console", "cancel", "quit")
    for op in operations:
        command = sub.add_parser(op)
        command.add_argument("--session", type=Path, required=True)
        command.add_argument("--until", action="append", default=[], help="wait for a state predicate, e.g. fullscreen=true")
        if op in ("move", "click", "drag"):
            command.add_argument("x", type=int)
            command.add_argument("y", type=int)
        if op in ("click", "drag"):
            command.add_argument("--button", type=int, choices=(1, 2, 3), default=1)
        if op == "drag":
            command.add_argument("to_x", type=int)
            command.add_argument("to_y", type=int)
        if op == "key":
            command.add_argument("key")
        if op == "chord":
            command.add_argument("keys", nargs="+", help='SDL key names, e.g. "Left Alt" R')
        if op == "resize":
            command.add_argument("width", type=int)
            command.add_argument("height", type=int)
        if op in API_ACTIONS:
            command.add_argument("command", help="level script line (script) or console command (console)")
        if op in ("click", "drag", "key", "chord", "wait", "cycle-mode"):
            command.add_argument("--frames", type=int, default=3)
    args = parser.parse_args()
    if args.command == "_supervise":
        supervise(args.session)
        return
    if args.command == "launch":
        try:
            validate_launch(args)
        except ValueError as error:
            parser.error(str(error))
        output = launch(args)
    else:
        data = read_session(args.session)
        if (args.session.parent / "exit.json").exists():
            raise RuntimeError("session has ended")
        fields = {k: v for k, v in vars(args).items() if k not in ("command", "session")}
        client = Client(data)
        try:
            output = client.run(args.command, **fields)
        finally:
            client.close()
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        print(json.dumps(dict(error=str(error))), file=sys.stderr)
        sys.exit(1)
