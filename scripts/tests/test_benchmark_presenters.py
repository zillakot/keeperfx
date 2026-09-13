import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("benchmark_presenters", Path(__file__).parents[1] / "benchmark-presenters.py")
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


def reports_for(entries):
    reports = []
    for entry in entries:
        duration = (10 if entry["backend"] == "original" else 5) * entry["pair"]
        reports.append({"engine_sha256": "engine", "assets": {"sha256": "assets"}, "config_sha256": "config",
                        "platform": {"system": "Darwin"}, "settings": {"FRAMES_PER_SECOND": "60"},
                        "request": {"scene": entry["scene"], "backend": entry["backend"], "campaign": "keeporig",
                                    "level": 20 if entry["scene"] == "busy" else 1, "resolution": [640, 480],
                                    "warmup_turns": 40, "turns": 200},
                        "engine": {"width": 640, "height": 480, "output_width": 640, "output_height": 480,
                                   "vsync_actual": 0, "fps_limit": 60, "turns_per_second": 20, "interpolation": True},
                        "wall_ms": {kind: {"mean": duration, "median": duration, "p95": duration * 2,
                                           "count": 600 * entry["pair"]} for kind in benchmark.profile.KINDS},
                        "resources": {"process_cpu": {"ms_per_turn": duration}, "rust_allocations": None}})
    return reports


class BenchmarkTests(unittest.TestCase):
    def test_serial_order_counterbalances_backends_and_rotates_scenes(self):
        entries = benchmark.schedule(5)
        self.assertEqual(len(entries), 30)
        for index in range(0, len(entries), 2):
            first, second = entries[index:index + 2]
            self.assertEqual(first["scene"], second["scene"])
            self.assertEqual(first["pair"], second["pair"])
            self.assertEqual(first["backend"], "original" if first["pair"] % 2 else "rust")
            self.assertNotEqual(first["backend"], second["backend"])
        self.assertEqual([entries[i]["scene"] for i in (0, 6, 12)], ["quiet", "busy", "possession"])
        self.assertEqual(len({entry["path"] for entry in entries}), 30)

    def test_pair_ratios_and_run_weighted_percentiles(self):
        entries = benchmark.schedule(5)
        comparison = benchmark.compare(entries, reports_for(entries))
        for scene in benchmark.SCENES:
            values = comparison["scenes"][scene]
            self.assertEqual(values["pair_count"], 5)
            row = values["wall_ms"]["presentation"]
            self.assertEqual(row["median"]["original"]["median"], 30)
            self.assertEqual(row["median"]["rust"]["median"], 15)
            self.assertEqual(row["p95"]["original"]["median"], 60)
            self.assertEqual(row["median"]["time_saved_percent"]["runs"], [50] * 5)
            self.assertEqual(row["median"]["pairs"][0]["speedup_factor"], 2)

    def test_regression_is_negative_time_saved(self):
        result = benchmark.paired_change(2, 3)
        self.assertEqual(result["time_saved_percent"], -50)
        self.assertAlmostEqual(result["speedup_factor"], 2 / 3)
        with self.assertRaises(RuntimeError):
            benchmark.paired_change(0, 3)

    def test_rejects_mismatched_or_incomplete_pairs(self):
        entries = benchmark.schedule(5)
        reports = reports_for(entries)
        for key, value in (("engine_sha256", "different"), ("config_sha256", "different"),
                           ("assets", {"sha256": "different"}), ("platform", {"system": "Linux"})):
            changed = copy.deepcopy(reports)
            changed[1][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, "mismatched"):
                benchmark.compare(entries, changed)
        changed = copy.deepcopy(reports)
        changed[1]["engine"]["output_width"] = 1280
        with self.assertRaisesRegex(RuntimeError, "mismatched"):
            benchmark.compare(entries, changed)
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            benchmark.compare(entries[:-1], reports[:-1])
        changed = copy.deepcopy(reports)
        changed[0]["request"]["backend"] = "rust"
        with self.assertRaisesRegex(RuntimeError, "scene/backend"):
            benchmark.compare(entries, changed)

    def test_unavailable_cpu_does_not_become_zero(self):
        entries = benchmark.schedule(5)
        reports = reports_for(entries)
        reports[0]["resources"]["process_cpu"] = None
        comparison = benchmark.compare(entries, reports)
        self.assertIsNone(comparison["scenes"]["quiet"]["process_cpu_ms_per_turn"])


if __name__ == "__main__":
    unittest.main()
