import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from test_profile_game import arguments, engine_output, profile, run_runner


def synthetic_report(acquire_ms=7.3, interval_ms=13.35, uncapped=True, mode="swapchain"):
    return {"status": "complete", "frame_cap": {"uncapped": uncapped},
            "presentation_mode": mode, "limitations": [],
            "presenter": {"per_frame": {"acquire_block_ns": {"mean": acquire_ms * 1_000_000}}},
            "wall_ms": {"frame_interval": {"mean": interval_ms}}}


class PresentationPacingTests(unittest.TestCase):
    def test_evidence_and_threshold(self):
        for acquire, interval, paced in ((7.3, 13.35, True), (10, 13.35, True),
                                         (0.33, 1000 / 195, False), (0.33, 1000 / 199, False),
                                         (5, 10, False), (5.001, 10, True), (0, 10, False)):
            with self.subTest(acquire=acquire, interval=interval):
                report = synthetic_report(acquire, interval)
                profile.annotate_presentation_pacing(report)
                self.assertEqual(report["presentation_paced"], paced)
                self.assertAlmostEqual(report["presentation_pacing"]["acquire_block_fraction"], acquire / interval)
                self.assertEqual(report["presentation_pacing"]["threshold"], 0.5)
                self.assertEqual(report["status"], "complete")
                self.assertEqual(bool(report["limitations"]), paced)

    def test_capped_offscreen_and_missing_counters_are_not_classified(self):
        for changes in ({"frame_cap": {"uncapped": False}}, {"presentation_mode": "offscreen"},
                        {"presenter": None}, {"presenter": {"per_frame": {}}}):
            with self.subTest(changes=changes):
                report = synthetic_report() | changes
                profile.annotate_presentation_pacing(report)
                self.assertNotIn("presentation_paced", report)
                self.assertNotIn("presentation_pacing", report)
                self.assertEqual(report["limitations"], [])

    def test_runner_keeps_complete_status_and_writes_both_annotations(self):
        with tempfile.TemporaryDirectory() as temporary:
            def run(command, **kwargs):
                output = Path(kwargs["env"]["KFX_PERF_OUTPUT"]).parent
                metadata = engine_output(output, arguments(uncapped=True))
                metadata["replay_scope"] = True
                row = [0] * len(profile.PRESENTER_COUNTERS)
                for name in ("acquire_ns", "acquire_block_ns"):
                    row[profile.PRESENTER_COUNTERS.index(name)] = 7_300_000
                metadata["presenter"] = {"per_frame": [row] * 20}
                (output / "raw.csv.json").write_text(json.dumps(metadata))
                rows = ["kind,turn,wall_ns"]
                durations = {"simulation": 100_000, "draw": 1_000_000, "replay": 1_000_000,
                             "presentation": 8_000_000, "present_wait": 100_000, "frame_interval": 13_350_000}
                for turn in range(40, 60):
                    rows += [f"{kind},{turn},{duration}" for kind, duration in durations.items()
                             if kind != "frame_interval" or turn > 40]
                (output / "raw.csv").write_text("\n".join(rows) + "\n")
                return subprocess.CompletedProcess(command, 0, "", "")

            output = run_runner(Path(temporary), run, extra=["--uncapped"])
            report = json.loads((output / "report.json").read_text())
            self.assertEqual(report["status"], "complete")
            self.assertTrue(report["presentation_paced"])
            self.assertAlmostEqual(report["presentation_pacing"]["acquire_block_fraction"], 7.3 / 13.35)
            markdown = (output / "report.md").read_text()
            for text in ("compositor-paced", "54.7%", "not a drawing or engine ceiling",
                         "valid matched comparison of host work", "--offscreen"):
                self.assertIn(text, markdown)


if __name__ == "__main__":
    unittest.main()
