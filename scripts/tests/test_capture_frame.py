import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("capture_frame", Path(__file__).parents[1] / "capture-frame.py")
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class CaptureTests(unittest.TestCase):
    def test_runner_deadline_covers_accepted_schedules(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            engine = root / "keeperfx"
            engine.touch()
            (root / "keeperfx.cfg").touch()
            for scene, turn, frames, interval, expected in (
                ("dungeon", 20, 1, 1, 120),
                ("dungeon", 600, 32, 60, 183),
                ("possession", 600, 32, 60, 183),
                ("menu", 600, 32, 60, 153),
            ):
                with self.subTest(scene=scene, frames=frames):
                    arguments = ["capture-frame.py", "--engine", str(engine), "--game-dir", str(root),
                                 "--out", str(root / "out/capture"), "--scene", scene, "--turn", str(turn),
                                 "--frames", str(frames), "--interval", str(interval)]
                    with mock.patch.object(capture, "ROOT", root), mock.patch("sys.argv", arguments), \
                            mock.patch.object(capture, "clone_assets"), \
                            mock.patch.object(capture.subprocess, "run",
                                              side_effect=subprocess.TimeoutExpired("keeperfx", expected)) as run:
                        with self.assertRaises(subprocess.TimeoutExpired):
                            capture.main()
                    self.assertEqual(run.call_args.kwargs["timeout"], expected)
                    self.assertEqual(list((root / "out").iterdir()), [])

    def test_clone_isolates_saves_settings_and_assets(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, destination = Path(temporary) / "source", Path(temporary) / "work"
            source.mkdir()
            destination.mkdir()
            config = "API_ENABLED =TRUE\nDELTA_TIME=ON\nTURNS_PER_SECOND=5\nFRONTEND_RES=640x480w32\n"
            (source / "keeperfx.cfg").write_text(config)
            (source / "save").mkdir()
            (source / "save/personal.sav").write_bytes(b"personal")
            (source / "settings.dat").write_bytes(b"personal settings")
            (source / "data").mkdir()
            (source / "data/asset").write_bytes(b"original")
            capture.clone_assets(source, destination, (800, 600))
            (destination / "data/asset").write_bytes(b"changed")
            self.assertEqual((source / "data/asset").read_bytes(), b"original")
            self.assertEqual((source / "keeperfx.cfg").read_text(), config)
            self.assertEqual(list((destination / "save").iterdir()), [])
            self.assertFalse((destination / "settings.dat").exists())
            settings = (destination / "keeperfx.cfg").read_text()
            self.assertIn("API_ENABLED=FALSE", settings)
            self.assertIn("DELTA_TIME=OFF", settings)
            self.assertIn("TURNS_PER_SECOND=20", settings)
            self.assertIn("INGAME_RES=800x600w32 800x600w32 800x600w32", settings)

    def test_sequence_requires_every_file_and_exact_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            frames = []
            for index in range(2):
                directory = output / f"frame-{index:04}"
                directory.mkdir()
                for name in ("frame.kfx", "reference.png", "capture.json"):
                    (directory / name).touch()
                frames.append({"frame": f"{directory.name}/frame.kfx", "reference": f"{directory.name}/reference.png"})
            manifest = {"format": "KFXSEQ01", "frames": frames}
            (output / "sequence.json").write_text(json.dumps(manifest))
            self.assertEqual(len(capture.capture_directories(output, 2)), 2)
            frames.reverse()
            (output / "sequence.json").write_text(json.dumps(manifest))
            with self.assertRaises(RuntimeError):
                capture.capture_directories(output, 2)
            frames.reverse()
            (output / "sequence.json").write_text(json.dumps(manifest))
            (output / "frame-0001/reference.png").unlink()
            with self.assertRaises(RuntimeError):
                capture.capture_directories(output, 2)


@unittest.skipUnless(os.environ.get("KFX_TEST_ENGINE"), "set KFX_TEST_ENGINE and KFX_TEST_GAME_DIR for native capture checks")
class NativeCaptureTests(unittest.TestCase):
    def test_failed_capture_respects_exit_preference(self):
        engine = Path(os.environ["KFX_TEST_ENGINE"]).resolve()
        game = Path(os.environ["KFX_TEST_GAME_DIR"]).resolve()
        with tempfile.TemporaryDirectory(prefix="capture-failure-", dir=capture.ROOT / "out") as temporary:
            work = Path(temporary)
            capture.clone_assets(game, work)
            existing = work / "existing"
            existing.mkdir()
            for output in (existing, work / "missing-parent/capture"):
                for exit_flag in (None, "0", "1"):
                    with self.subTest(output=output.name, exit_flag=exit_flag):
                        environment = {key: value for key, value in os.environ.items()
                                       if not key.startswith("KFX_FRAME_CAPTURE")}
                        environment.update(SDL_VIDEODRIVER="dummy", SDL_RENDER_DRIVER="software",
                                           KFX_FRAME_CAPTURE=str(output), KFX_FRAME_CAPTURE_TURN="20")
                        if exit_flag is not None:
                            environment["KFX_FRAME_CAPTURE_EXIT"] = exit_flag
                        command = [str(engine), "-nointro", "-nosound", "-altinput", "-skipheartzoom",
                                   "-campaign", "keeporig", "-level", "1"]
                        with subprocess.Popen(command, cwd=work, env=environment,
                                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
                            timed_out = False
                            try:
                                _, stderr = process.communicate(timeout=8)
                            except subprocess.TimeoutExpired:
                                timed_out = True
                                process.kill()
                                _, stderr = process.communicate(timeout=5)
                        self.assertEqual(stderr.count("Frame capture failed in"), 1, stderr[-2000:])
                        self.assertEqual(timed_out, exit_flag != "1")
                        if exit_flag == "1":
                            self.assertEqual(process.returncode, 0)
                        self.assertEqual(list(existing.iterdir()), [])
                        self.assertFalse((work / "missing-parent").exists())


if __name__ == "__main__":
    unittest.main()
