import argparse
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("drawing_coverage", Path(__file__).parents[1] / "drawing-coverage.py")
COVERAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COVERAGE)
CONTROL = COVERAGE.CONTROL


def counters(**values):
    base = {name: 0 for name in COVERAGE.GATE_COUNTERS + COVERAGE.REPORTED_COUNTERS}
    base.update(verified_batches=10, frames=5)
    base.update(values)
    return base


def scene(name, families):
    return dict(name=name, launch={}, steps=(), families=families)


def result(name, values, status="complete"):
    return dict(scene=name, status=status, counters=values, gate=COVERAGE.gate_of(values))


class MatrixTests(unittest.TestCase):
    def test_every_reachable_family_is_targeted_by_a_scene(self):
        targeted = {name for entry in COVERAGE.SCENES for name in entry["families"]}
        known = {family["name"] for family in COVERAGE.FAMILIES}
        self.assertEqual(targeted - known, set(), "scene targets a family that is not in the inventory")
        for family in COVERAGE.FAMILIES:
            if family.get("expect", "nonzero") == "nonzero":
                self.assertIn(family["name"], targeted, f"{family['name']} has no scene")

    def test_counter_names_match_the_engine_dump(self):
        source = Path(__file__).parents[2] / "src/kfx/renderer"
        dump = (source / "RendererSoftware.cpp").read_text()
        names = set(re.findall(r'([a-z0-9_]+)\\?":%', dump))
        kinds = re.findall(r"X\((\w+)\)", (source / "ArenaKindCounters.h").read_text())
        metrics = ("bytes", "misses", "hits", "source_bytes", "distinct_lengths", "length_overflows")
        names |= {f"arena_{kind}_{metric}" for kind in kinds for metric in metrics}
        for family in COVERAGE.FAMILIES:
            for counter in family["counters"]:
                self.assertIn(counter, names, f"{family['name']} names a counter the engine does not emit")
        for counter in COVERAGE.GATE_COUNTERS + COVERAGE.REPORTED_COUNTERS:
            self.assertIn(counter, names)

    def test_scene_value_prefers_the_first_non_zero_counter(self):
        family = dict(name="x", counters=("a", "b"), exclusive=True)
        self.assertEqual(COVERAGE.scene_value({"a": 0, "b": 7}, family), ("b", 7))
        self.assertEqual(COVERAGE.scene_value({"a": 3, "b": 7}, family), ("a", 3))
        self.assertEqual(COVERAGE.scene_value({}, family), ("a", 0))
        self.assertEqual(COVERAGE.scene_value({}, dict(name="y", counters=(), exclusive=False)), (None, None))

    def test_gate_reports_failures_and_blocks_a_pass(self):
        gate = COVERAGE.gate_of(counters(failures=2, rejected_commands=1))
        self.assertFalse(gate["passed"])
        self.assertEqual(gate["failed"], {"failures": 2, "rejected_commands": 1})
        self.assertTrue(COVERAGE.gate_of(counters())["passed"])
        self.assertFalse(COVERAGE.gate_of(counters(verified_batches=0))["passed"])

    def test_summary_classifies_each_family(self):
        scenes = [scene("busy", ("Minimap",)), scene("lens", ("Built-in lenses",))]
        results = {"busy": result("busy", counters(arena_minimap_misses=12, gpu_triangles=99)),
                   "lens": result("lens", counters(arena_lens_bytes=4096))}
        summary = COVERAGE.summarize(scenes, results)
        rows = {row["family"]: row for row in summary["families"]}
        self.assertEqual(rows["Minimap"]["status"], "measured")
        self.assertEqual(rows["Minimap"]["measured_in"], ["busy"])
        self.assertEqual(rows["Built-in lenses"]["measured_in"], ["lens"])
        self.assertEqual(rows["Movies"]["status"], "not reached")
        self.assertEqual(rows["Legacy gpoly span sink"]["status"], "zero as expected")
        self.assertEqual(rows["General lines"]["status"], "no counter")
        self.assertEqual(rows["Dungeon terrain (gpoly)"]["scenes"]["busy"]["value"], 99)

    def test_shared_counters_are_attributed_by_scene(self):
        scenes = [scene("dbc-text", ("DBC (Asian) glyph bitmaps",)), scene("other", ())]
        results = {"dbc-text": result("dbc-text", counters(arena_bitmap_misses=5)),
                   "other": result("other", counters(arena_bitmap_misses=7))}
        rows = {row["family"]: row for row in COVERAGE.summarize(scenes, results)["families"]}
        self.assertEqual(rows["DBC (Asian) glyph bitmaps"]["status"], "measured (scene-attributed)")
        self.assertEqual(rows["Huge bitmaps"]["status"], "not reached (shared counter moved elsewhere)")
        self.assertEqual(rows["Huge bitmaps"]["nonzero_in"], ["dbc-text", "other"])
        self.assertEqual(rows["Huge bitmaps"]["measured_in"], [])

    def test_failed_scene_counters_do_not_count_as_coverage(self):
        scenes = [scene("busy", ("Minimap",))]
        results = {"busy": result("busy", counters(arena_minimap_misses=12, failures=1))}
        rows = {row["family"]: row for row in COVERAGE.summarize(scenes, results)["families"]}
        self.assertEqual(rows["Minimap"]["status"], "not reached")
        self.assertEqual(rows["Minimap"]["scenes"]["busy"]["value"], 12)

    def test_markdown_lists_every_scene_and_family(self):
        scenes = [scene("busy", ("Minimap",))]
        results = {"busy": result("busy", counters(arena_minimap_misses=1234))}
        text = COVERAGE.markdown(COVERAGE.summarize(scenes, results))
        self.assertIn("| busy | complete | 5 | 10 | 0 | pass |", text)
        self.assertIn("**1,234**", text)
        self.assertIn("`arena_minimap_misses`", text)

    def test_scene_selection(self):
        self.assertEqual([entry["name"] for entry in COVERAGE.selected("dungeon-busy,lua-*")],
                         ["dungeon-busy", "lua-lens"])
        with self.assertRaisesRegex(ValueError, "no scene matches"):
            COVERAGE.selected("nothing")


class HostGuardTests(unittest.TestCase):
    def test_console_wait_retries_until_unlocked(self):
        states = [True, True, False]
        slept = []
        COVERAGE.await_console(probe=lambda: states.pop(0), sleep=slept.append)
        self.assertEqual(slept, [5, 5])

    def test_console_wait_gives_up_with_a_clear_message(self):
        with self.assertRaisesRegex(RuntimeError, "locked"):
            COVERAGE.await_console(timeout=0, probe=lambda: True, sleep=lambda _: None)

    def test_scene_waits_for_another_game_and_then_refuses(self):
        with mock.patch.object(COVERAGE, "await_no_game", side_effect=RuntimeError("already running")), \
                mock.patch.object(COVERAGE.CONTROL, "launch") as launch:
            with self.assertRaisesRegex(RuntimeError, "already running"):
                COVERAGE.run_scene(COVERAGE.SCENES[0], argparse.Namespace(lifetime=600), Path("/nonexistent"))
            launch.assert_not_called()

    def test_game_wait_retries_then_reports_the_holder(self):
        states = [["9 keeperfx"], []]
        slept = []
        COVERAGE.await_no_game(probe=lambda: states.pop(0), sleep=slept.append)
        self.assertEqual(slept, [5])
        with self.assertRaisesRegex(RuntimeError, "9 keeperfx"):
            COVERAGE.await_no_game(timeout=0, probe=lambda: ["9 keeperfx"], sleep=lambda _: None)

    def test_counters_are_read_from_the_session_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / "drawing.json").write_text(json.dumps({"frames": 3}))
            self.assertEqual(COVERAGE.read_counters(work), {"frames": 3})
            (work / "drawing.json").write_text("{partial")
            with self.assertRaisesRegex(RuntimeError, "not readable"):
                COVERAGE.read_counters(work, attempts=2, sleep=lambda _: None)


class LaunchOptionTests(unittest.TestCase):
    def options(self, **changes):
        values = dict(backend="wgpu", verify=False, draw_backend="wgpu", draw_verify=True, lifetime=600,
                      campaign="keeporig", level=None, cheats=False, play_movies=False, smoothing=False,
                      ingame_res=None, language=None, rotate_mode=None, startup_timeout=None)
        values.update(changes)
        return argparse.Namespace(**values)

    def prepare(self, **changes):
        directory = Path(tempfile.mkdtemp())
        (directory / "save").mkdir()
        (directory / "keeperfx.cfg").write_text("API_ENABLED=FALSE\nAPI_PORT=0\nINGAME_RES=640x480w32\nLANGUAGE=ENG\n")
        engine = directory / "keeperfx"
        engine.write_bytes(b"engine")
        manifest = CONTROL.prepare_session(self.options(**changes), directory, engine, 5000)
        return directory, manifest

    def test_default_session_keeps_the_existing_arguments(self):
        directory, manifest = self.prepare()
        self.assertEqual(manifest["args"], ["-altinput", "-skipheartzoom", "-nointro"])
        config = (directory / "keeperfx.cfg").read_text()
        self.assertIn("API_ENABLED=TRUE", config)
        self.assertIn("API_PORT=5000", config)
        self.assertIn("INGAME_RES=640x480w32 DESKTOP 800x600w32", config)
        self.assertIn("LANGUAGE=ENG", config)
        self.assertFalse((directory / "save/settings.toml").exists())

    def test_family_options_reach_the_engine(self):
        _, manifest = self.prepare(cheats=True, smoothing=True, play_movies=True, level=20)
        self.assertEqual(manifest["args"], ["-altinput", "-skipheartzoom", "-bullfrog", "-alex", "-vidsmooth",
                                            "-campaign", "keeporig", "-level", "20"])

    def test_language_and_video_mode_reach_the_configuration(self):
        directory, _ = self.prepare(language="CHI", ingame_res="320x200w32")
        config = (directory / "keeperfx.cfg").read_text()
        self.assertIn("LANGUAGE=CHI", config)
        self.assertIn("INGAME_RES=320x200w32", config)

    def test_rotate_mode_is_written_to_the_isolated_settings(self):
        directory, _ = self.prepare(rotate_mode=2)
        self.assertEqual((directory / "save/settings.toml").read_text(), "[video]\nrotate_mode = 2\n")

    def test_invalid_options_are_refused(self):
        for changes, message in ((dict(language="CHINESE"), "three-letter"),
                                 (dict(ingame_res="320x200"), "video mode"),
                                 (dict(rotate_mode=5), "rotate mode"),
                                 (dict(startup_timeout=5), "startup timeout"),
                                 (dict(lifetime=10), "lifetime")):
            with self.assertRaisesRegex(ValueError, message):
                CONTROL.validate_launch(self.options(**changes))


if __name__ == "__main__":
    unittest.main()
