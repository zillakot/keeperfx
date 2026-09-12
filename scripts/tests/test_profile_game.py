import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("profile_game", Path(__file__).parents[1] / "profile-game.py")
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


def arguments(**changes):
    values = dict(headless=True, scene="quiet", warmup_turns=40, turns=20, resolution=(640, 480))
    values.update(changes)
    return argparse.Namespace(**values)


def engine_output(output, args=None):
    args = args or arguments()
    start, end = args.warmup_turns, args.warmup_turns + args.turns
    snapshot = {"action_seed": 1, "ai_seed": 2, "player_seed": 3, "unsync_seed": 4,
                "sound_seed": 5, "creatures": 3, "things": 18}
    metadata = {"format": "KFXPERF01", "complete": True, "start": dict(snapshot, turn=start),
                "end": dict(snapshot, turn=end), "renderer": "software", "video_driver": "dummy",
                "width": 640, "height": 480, "output_width": 640, "output_height": 480,
                "vsync_actual": 0, "turns_per_second": 20, "fps_limit": 60, "interpolation": True,
                "scene": "dungeon", "view": "dungeon_top"}
    rows = ["kind,turn,wall_ns"]
    for turn in range(start, end):
        rows += [f"{kind},{turn},{(turn - start + 1) * 1000000}" for kind in profile.KINDS
                 if kind != "frame_interval" or turn != start]
    (output / "raw.csv").write_text("\n".join(rows) + "\n")
    (output / "raw.csv.json").write_text(json.dumps(metadata))
    return metadata


class ProfileTests(unittest.TestCase):
    def test_statistics_and_raw_preservation(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            raw = (output / "raw.csv").read_bytes()
            report = profile.summarize(output, arguments())
            stats = report["wall_ms"]["simulation"]
            self.assertEqual(stats["count"], 20)
            self.assertEqual(stats["mean"], 10.5)
            self.assertEqual(stats["median"], 10.5)
            self.assertAlmostEqual(stats["p90"], 18.1)
            self.assertAlmostEqual(stats["p95"], 19.05)
            self.assertAlmostEqual(stats["p99"], 19.81)
            self.assertEqual(stats["max"], 20)
            self.assertEqual((output / "raw.csv").read_bytes(), raw)
            self.assertTrue(any("CPU time is not collected" in item for item in report["limitations"]))
            self.assertTrue(any("HEADLESS" in item for item in report["limitations"]))

    def test_non_mac_native_rejects_headless_backend(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(profile.sys, "platform", "linux"):
            output = Path(temporary)
            engine_output(output)
            with self.assertRaisesRegex(RuntimeError, "window-system"):
                profile.summarize(output, arguments(headless=False))

    def test_report_rejects_incomplete_or_wrong_actual_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for field, value in (("complete", False), ("renderer", "metal"), ("width", 800),
                                 ("output_width", 0), ("vsync_actual", 1), ("interpolation", False), ("fps_limit", 0),
                                 ("scene", "possession"), ("view", "creature")):
                with self.subTest(field=field):
                    metadata = engine_output(output)
                    metadata[field] = value
                    (output / "raw.csv.json").write_text(json.dumps(metadata))
                    with self.assertRaises(RuntimeError):
                        profile.summarize(output, arguments())
            engine_output(output)
            raw = (output / "raw.csv").read_text()
            for invalid in (raw.replace("simulation,41,", "simulation,40,"),
                            raw.replace("simulation,41,2000000\n", ""),
                            raw.replace("draw,41,2000000", "draw,41,-1"),
                            raw.replace("draw,41,2000000", "draw,61,2000000")):
                (output / "raw.csv").write_text(invalid)
                with self.assertRaises(RuntimeError):
                    profile.summarize(output, arguments())

    def test_rejects_unpaired_or_fabricated_presentation_samples(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            raw = (output / "raw.csv").read_text()
            for invalid in (raw.replace("draw,41,2000000\n", ""),
                            raw.replace("presentation,41,2000000\n", ""),
                            raw.replace("present_wait,41,2000000\n", ""),
                            raw + "frame_interval,40,1000000\n",
                            raw.replace("frame_interval,41,2000000", "frame_interval,41,0"),
                            raw.replace("frame_interval,41,2000000\n", ""),
                            raw.replace("present_wait,41,2000000", "present_wait,41,2000001")):
                with self.subTest(invalid=invalid[-80:]):
                    (output / "raw.csv").write_text(invalid)
                    with self.assertRaises(RuntimeError):
                        profile.summarize(output, arguments())

    def test_reported_engine_failure_overrides_complete_files_and_zero_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def run(command, **kwargs):
                engine_output(Path(kwargs["env"]["KFX_PERF_OUTPUT"]).parent)
                return subprocess.CompletedProcess(command, 0, "", "Performance capture failed: writing metadata")
            with self.assertRaisesRegex(RuntimeError, "engine reported a performance capture failure"):
                self.run_runner(root, run)
            self.assertEqual(json.loads((root / "out/profile/report.json").read_text())["status"], "failed")
            self.assertFalse((root / "out/profile/report.md").exists())

    def test_environment_is_opt_in_and_native_by_default(self):
        inherited = {"KFX_FRAME_CAPTURE": "bad", "KFX_FRAME_CAPTURE_COUNT": "32",
                     "KFX_PERF_UNKNOWN": "bad", "SDL_VIDEODRIVER": "dummy",
                     "SDL_RENDER_DRIVER": "software", "SDL_RENDER_VSYNC": "1", "PATH": "/bin"}
        with mock.patch.dict(os.environ, inherited, clear=True), mock.patch.object(profile.sys, "platform", "darwin"):
            environment = profile.environment_for(arguments(headless=False), Path("/output"))
            self.assertEqual(environment["SDL_VIDEODRIVER"], "cocoa")
            self.assertEqual(environment["SDL_RENDER_DRIVER"], "metal")
            self.assertEqual(environment["SDL_RENDER_VSYNC"], "0")
            self.assertFalse(any(key.startswith("KFX_FRAME_CAPTURE") for key in environment))
            self.assertNotIn("KFX_PERF_UNKNOWN", environment)
            self.assertEqual(environment["KFX_PERF_TURNS"], "20")
            environment = profile.environment_for(arguments(scene="possession"), Path("/output"))
            self.assertEqual(environment["KFX_PERF_SCENE"], "possession")
            self.assertEqual(environment["SDL_VIDEODRIVER"], "dummy")

    def test_asset_identity_tracks_contents_and_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "data").mkdir()
            asset = root / "data/asset"
            asset.write_bytes(b"first")
            first = profile.asset_identity(root)
            self.assertEqual(first["files"], 1)
            self.assertEqual(first["bytes"], 5)
            asset.write_bytes(b"other")
            second = profile.asset_identity(root)
            self.assertNotEqual(first["sha256"], second["sha256"])
            asset.rename(root / "data/renamed")
            self.assertNotEqual(second["sha256"], profile.asset_identity(root)["sha256"])

    def run_runner(self, root, behavior, headless=True):
        engine = root / "engine"
        engine.write_bytes(b"engine")
        (root / "keeperfx.cfg").write_text("DELTA_TIME=OFF\nVSYNC=ON\n")
        (root / "settings.dat").write_bytes(b"personal")
        (root / "save").mkdir()
        (root / "save/personal.sav").write_bytes(b"personal")
        output = root / "out/profile"
        argv = ["profile-game.py", "--engine", str(engine), "--game-dir", str(root),
                "--out", str(output), "--scene", "busy", "--turns", "20"]
        if headless:
            argv += ["--headless"]
        with mock.patch.object(profile, "ROOT", root), mock.patch("sys.argv", argv), \
                mock.patch.object(profile.platform, "processor", return_value="test CPU"), \
                mock.patch.object(profile.subprocess, "run", side_effect=behavior):
            profile.main()
        return output

    def test_runner_isolated_success_and_busy_map(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def run(command, **kwargs):
                work = kwargs["cwd"]
                self.assertEqual(command[-4:], ["-campaign", "keeporig", "-level", "20"])
                self.assertEqual(kwargs["timeout"], 123)
                self.assertFalse((work / "settings.dat").exists())
                self.assertEqual(list((work / "save").iterdir()), [])
                config = (work / "keeperfx.cfg").read_text()
                for key, value in profile.SETTINGS.items():
                    self.assertIn(f"{key}={value}", config)
                engine_output(Path(kwargs["env"]["KFX_PERF_OUTPUT"]).parent)
                (work / "keeperfx.log").write_text("engine log")
                return subprocess.CompletedProcess(command, 0, "stdout", "stderr")
            output = self.run_runner(root, run)
            report = json.loads((output / "report.json").read_text())
            self.assertEqual(report["status"], "complete")
            self.assertEqual(report["request"]["level"], 20)
            self.assertEqual((output / "keeperfx.log").read_text(), "engine log")
            self.assertIn("HEADLESS SOFTWARE SMOKE TEST", (output / "report.md").read_text())
            self.assertEqual((root / "keeperfx.cfg").read_text(), "DELTA_TIME=OFF\nVSYNC=ON\n")
            self.assertEqual((root / "save/personal.sav").read_bytes(), b"personal")
            self.assertEqual(list((root / "out").iterdir()), [output])

    def test_timeout_and_exit_failure_preserve_diagnostics(self):
        for timeout, headless in ((True, True), (False, True), (True, False), (False, False)):
            with self.subTest(timeout=timeout, headless=headless), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                def run(command, **kwargs):
                    (kwargs["cwd"] / "keeperfx.log").write_text("failure details")
                    output = Path(kwargs["env"]["KFX_PERF_OUTPUT"]).parent
                    (output / "raw.csv").write_text("partial output")
                    if timeout:
                        raise subprocess.TimeoutExpired(command, kwargs["timeout"], output=b"partial stdout", stderr=b"partial stderr")
                    return subprocess.CompletedProcess(command, 7, "partial stdout", "partial stderr")
                with self.assertRaisesRegex(RuntimeError, "diagnostics preserved"):
                    self.run_runner(root, run, headless=headless)
                output = root / "out/profile"
                self.assertEqual(json.loads((output / "report.json").read_text())["status"], "failed")
                self.assertEqual((output / "keeperfx.log").read_text(), "failure details")
                self.assertEqual((output / "stdout.log").read_text(), "partial stdout")
                self.assertEqual((output / "stderr.log").read_text(), "partial stderr")
                self.assertEqual((output / "raw.csv").read_text(), "partial output")
                self.assertEqual(list((root / "out").iterdir()), [output])


if __name__ == "__main__":
    unittest.main()
