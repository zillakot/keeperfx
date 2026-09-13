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
    values = dict(headless=True, backend="original", scene="quiet", warmup_turns=40, turns=20, resolution=(640, 480))
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
                "scene": "dungeon", "view": "dungeon_top",
                "resources": {"wall_ns": 1_000_000_000,
                              "process_cpu": {"available": True, "source": "getrusage(RUSAGE_SELF)",
                                              "user_ns": 200_000_000, "system_ns": 100_000_000},
                              "rust_allocations": {"available": True, "calls": 10, "requested_bytes": 2000}}}
    metadata["draw_breakdown"] = getattr(args, "draw_breakdown", False)
    rows = ["kind,turn,wall_ns"]
    for turn in range(start, end):
        rows += [f"{kind},{turn},{(turn - start + 1) * 1000000}" for kind in profile.KINDS
                 if kind != "frame_interval" or turn != start]
    if metadata["draw_breakdown"]:
        expanded = []
        for row in rows:
            if row.startswith("draw,"):
                _, turn, duration = row.split(",")
                expanded += [f"{kind},{turn},{int(duration) // 10 if kind != 'draw_front_raster' else 0}"
                             for kind in profile.DRAW_KINDS]
            expanded.append(row)
        rows = expanded
    (output / "raw.csv").write_text("\n".join(rows) + "\n")
    (output / "raw.csv.json").write_text(json.dumps(metadata))
    return metadata


def drawing_metadata(output, frames, backend="wgpu", available=True):
    metadata = json.loads((output / "raw.csv.json").read_text())
    per_frame = [[index + position for position in range(len(profile.DRAWING_COUNTERS))]
                 for index in range(frames)]
    metadata["drawing"] = {"available": available, "backend": backend, "frames": len(per_frame),
                           "counters": list(profile.DRAWING_COUNTERS),
                           "gauges": list(profile.DRAWING_GAUGES), "per_frame": per_frame}
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
            self.assertTrue(any("not CPU-time counters" in item for item in report["limitations"]))
            self.assertEqual(report["resources"]["process_cpu"]["total_ms"], 300)
            self.assertEqual(report["resources"]["process_cpu"]["ms_per_turn"], 15)
            self.assertEqual(report["resources"]["process_cpu"]["core_equivalents"], 0.3)
            self.assertEqual(report["resources"]["rust_allocations"]["calls_per_presentation"], 0.5)
            self.assertTrue(any("HEADLESS" in item for item in report["limitations"]))

    def test_draw_breakdown_is_nested_and_reports_per_frame_remainder(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            args = arguments(draw_breakdown=True)
            engine_output(output, args)
            report = profile.summarize(output, args)
            for kind in (*profile.DRAW_KINDS, "draw_unaccounted"):
                self.assertEqual(report["wall_ms"][kind]["count"], 20)
            self.assertEqual(report["wall_ms"]["draw_scene"]["mean"], 1.05)
            self.assertEqual(report["wall_ms"]["draw_front_raster"]["max"], 0)
            self.assertAlmostEqual(report["wall_ms"]["draw_unaccounted"]["mean"], 7.35)
            self.assertTrue(any("combines terrain, sprites" in item for item in report["limitations"]))
            with self.assertRaisesRegex(RuntimeError, "does not match"):
                profile.summarize(output, arguments())

    def test_draw_breakdown_rejects_missing_duplicate_reordered_or_overlapping_children(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            args = arguments(draw_breakdown=True)
            engine_output(output, args)
            raw = (output / "raw.csv").read_text()
            first = "draw_scene,40,100000\n"
            second = "draw_raster,40,100000\n"
            invalids = [raw.replace(first, ""), raw.replace(first, first + first),
                        raw.replace(first + second, second + first),
                        raw.replace(first, "draw_scene,41,100000\n"),
                        raw.replace(first, "draw_scene,40,1000000\n"),
                        raw.replace(first, first + "simulation,40,1000000\n"),
                        raw + "draw_scene,60,0\n"]
            for index, invalid in enumerate(invalids):
                with self.subTest(index=index):
                    (output / "raw.csv").write_text(invalid)
                    with self.assertRaises(RuntimeError):
                        profile.summarize(output, args)

    def test_draw_breakdown_pairs_multiple_draws_in_one_simulation_turn(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            args = arguments(draw_breakdown=True)
            engine_output(output, args)
            path = output / "raw.csv"
            raw = path.read_text()
            extra = "".join(f"{kind},40,0\n" for kind in profile.DRAW_KINDS)
            extra += "draw,40,5000000\npresentation,40,1000000\npresent_wait,40,1000000\nframe_interval,40,1000000\n"
            path.write_text(raw.replace("simulation,41,2000000\n", extra + "simulation,41,2000000\n"))
            report = profile.summarize(output, args)
            self.assertEqual(report["wall_ms"]["draw_unaccounted"]["count"], 21)
            self.assertAlmostEqual(report["wall_ms"]["draw_unaccounted"]["mean"], (147 + 5) / 21)

    def test_draw_breakdown_request_requires_engine_support(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            with self.assertRaisesRegex(RuntimeError, "does not match"):
                profile.summarize(output, arguments(draw_breakdown=True))

    def test_drawing_counters_cover_the_measured_window_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            drawing_metadata(output, 19)
            report = profile.summarize(output, arguments())
            drawing = report["drawing"]
            self.assertEqual(drawing["backend"], "wgpu")
            self.assertEqual(drawing["frames"], 19)
            self.assertEqual(set(drawing["per_frame"]), set(profile.DRAWING_COUNTERS))
            submits = drawing["per_frame"]["submits"]
            self.assertEqual((submits["min"], submits["max"], submits["total"]), (0, 18, 171))
            self.assertEqual(submits["mean"], 9)
            self.assertAlmostEqual(submits["p95"], 17.1)
            self.assertEqual(drawing["per_frame"]["gpu_spans"]["min"], 17)
            self.assertIsNone(drawing["per_frame"]["arena_bytes_resident"]["total"])
            self.assertEqual(drawing["per_frame"]["ordered_sprites"]["min"], 15)
            self.assertTrue(any("not GPU execution time" in item for item in report["limitations"]))
            self.assertTrue(any("distinct from every host wall-clock column" in item
                                for item in report["limitations"]))

    def test_drawing_counters_reject_window_and_metadata_mismatches(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            path = output / "raw.csv.json"
            cases = {
                "must cover every measured presentation": lambda d: d.update(
                    frames=18, per_frame=d["per_frame"][:18]),
                "do not match this profiler": lambda d: d.update(
                    counters=list(profile.DRAWING_COUNTERS)[:-1]),
                "does not match the recorded rows": lambda d: d.update(frames=20),
                "invalid drawing counter row": lambda d: d["per_frame"].__setitem__(
                    0, [-1] * len(profile.DRAWING_COUNTERS)),
                "gauge names do not match": lambda d: d.update(gauges=[]),
                "invalid drawing counter availability": lambda d: d.update(backend=""),
                "unavailable but rows were recorded": lambda d: d.update(available=False),
                "backend changed during the measured window": lambda d: d.update(
                    backend="wgpu+wgpu-fallback"),
            }
            for message, mutate in cases.items():
                with self.subTest(message=message):
                    metadata = drawing_metadata(output, 19)
                    mutate(metadata["drawing"])
                    path.write_text(json.dumps(metadata))
                    with self.assertRaisesRegex(RuntimeError, message):
                        profile.summarize(output, arguments())

    def test_software_drawing_backend_is_reported_without_counters(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            engine_output(output)
            metadata = drawing_metadata(output, 0, backend="software", available=False)
            (output / "raw.csv.json").write_text(json.dumps(metadata))
            report = profile.summarize(output, arguments())
            self.assertEqual(report["drawing"]["backend"], "software")
            self.assertIsNone(report["drawing"]["per_frame"])
            self.assertTrue(any("active drawing backend was software" in item
                                for item in report["limitations"]))
            request = dict(vars(arguments()), campaign="keeporig", level=1)
            profile.write_report(output, dict(report, request=request,
                                              engine_sha256="0", assets={"sha256": "0"}))
            self.assertIn("Active drawing backend: software.", (output / "report.md").read_text())

    def test_legacy_wall_report_does_not_invent_cpu_or_allocation_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            metadata = engine_output(output)
            del metadata["resources"]
            (output / "raw.csv.json").write_text(json.dumps(metadata))
            report = profile.summarize(output, arguments())
            self.assertIsNone(report["resources"]["process_cpu"])
            self.assertIsNone(report["resources"]["rust_allocations"])
            self.assertIn("Process CPU time is not available in this run.", report["limitations"])

    def test_resource_counters_reject_invalid_values_and_keep_unavailable_distinct(self):
        resources = {"wall_ns": 1000, "process_cpu": {"available": False}, "rust_allocations": {"available": False}}
        report = profile.summarize_resources(resources, 20, 60)
        self.assertIsNone(report["process_cpu"])
        self.assertIsNone(report["rust_allocations"])
        for value in (0, -1, "1000", True):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                profile.summarize_resources(dict(resources, wall_ns=value), 20, 60)
        resources["process_cpu"] = {"available": True, "source": "steady_clock", "user_ns": 2, "system_ns": 1}
        with self.assertRaisesRegex(RuntimeError, "CPU counters"):
            profile.summarize_resources(resources, 20, 60)
        resources["process_cpu"] = {"available": False}
        resources["rust_allocations"] = {"available": True, "calls": -1, "requested_bytes": 20}
        with self.assertRaisesRegex(RuntimeError, "allocation counters"):
            profile.summarize_resources(resources, 20, 60)

    def test_rust_requires_actual_native_adapter_and_non_vsync_mode(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(profile.sys, "platform", "darwin"):
            output = Path(temporary)
            args = arguments(headless=False, backend="rust")
            metadata = engine_output(output, args)
            metadata.update(renderer="wgpu-metal", video_driver="cocoa",
                            renderer_details=json.dumps({"adapter": 'Apple "Test" GPU', "backend": "Metal",
                                                         "present_mode": "Immediate", "format": "Bgra8Unorm"}))
            (output / "raw.csv.json").write_text(json.dumps(metadata))
            self.assertIn("presentation", profile.summarize(output, args)["wall_ms"])
            for field, value in (("renderer", "metal"), ("video_driver", "dummy"),
                                 ("renderer_details", "{}"), ("renderer_details", "[]"),
                                 ("renderer_details", metadata["renderer_details"].replace("Immediate", "Fifo")),
                                 ("renderer_details", metadata["renderer_details"].replace("Metal", "Vulkan"))):
                with self.subTest(field=field, value=value):
                    (output / "raw.csv.json").write_text(json.dumps(dict(metadata, **{field: value})))
                    with self.assertRaises(RuntimeError):
                        profile.summarize(output, args)
            (output / "raw.csv.json").write_text(json.dumps(metadata))
            with self.assertRaisesRegex(RuntimeError, "native macOS"):
                profile.summarize(output, arguments(backend="rust"))

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
                     "SDL_RENDER_DRIVER": "software", "SDL_RENDER_VSYNC": "1", "PATH": "/bin",
                     "KFX_PRESENT_BACKEND": "wgpu", "KFX_WGPU_FAIL_INIT": "1"}
        with mock.patch.dict(os.environ, inherited, clear=True), mock.patch.object(profile.sys, "platform", "darwin"):
            environment = profile.environment_for(arguments(headless=False), Path("/output"))
            self.assertEqual(environment["SDL_VIDEODRIVER"], "cocoa")
            self.assertEqual(environment["SDL_RENDER_DRIVER"], "metal")
            self.assertEqual(environment["SDL_RENDER_VSYNC"], "0")
            self.assertFalse(any(key.startswith("KFX_FRAME_CAPTURE") for key in environment))
            self.assertNotIn("KFX_PERF_UNKNOWN", environment)
            self.assertNotIn("KFX_WGPU_FAIL_INIT", environment)
            self.assertEqual(environment["KFX_PRESENT_BACKEND"], "sdl")
            self.assertEqual(environment["KFX_PERF_TURNS"], "20")
            self.assertEqual(environment["KFX_PERF_DRAW_BREAKDOWN"], "0")
            self.assertEqual(profile.environment_for(arguments(draw_breakdown=True), Path("/output"))["KFX_PERF_DRAW_BREAKDOWN"], "1")
            rust_environment = profile.environment_for(arguments(headless=False, backend="rust"), Path("/output"))
            self.assertEqual(rust_environment["KFX_PRESENT_BACKEND"], "wgpu")
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
            self.assertEqual(report["request"]["backend"], "original")
            self.assertEqual(report["environment"]["KFX_PRESENT_BACKEND"], "sdl")
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
