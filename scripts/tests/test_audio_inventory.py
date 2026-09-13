import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audio_inventory", Path(__file__).parents[1] / "audio_inventory.py")
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class AudioInventoryTests(unittest.TestCase):
    def test_comments_and_quoted_paths_preserve_source_lines(self):
        text = '; ignored\n[sounds]\nHIT = "custom/a;b.wav" 3 ; note\n; NONE = 4\n'
        self.assertEqual(list(inventory.config_rows(text)),
                         [("sounds", "HIT", '"custom/a;b.wav" 3', 3)])

    def test_disabled_and_consecutive_variants(self):
        self.assertEqual(inventory.variant_ids("72 3 STACK=duck:4"), ([72, 73, 74], 3))
        self.assertEqual(inventory.variant_ids("0 4"), ([], 4))
        self.assertEqual(inventory.variant_ids("72 0"), ([], 0))
        self.assertEqual(inventory.variant_ids("HIT_WALL 3"), ([], None))
        self.assertEqual(inventory.literal_ids("112 + SOUND_RANDOM(3)"), ([112, 113, 114], 3))
        self.assertEqual(inventory.literal_ids("base + SOUND_RANDOM(count)"), ([], None))
        with self.assertRaises(ValueError):
            inventory.variant_ids("1 100000000")

    def test_multiline_calls_ignore_comments_definitions_and_string_contents(self):
        text = '''// play_sample(0, 2)
const char *label = "play_sample(0, 3)";
void play_sample(int emitter, int sample) {}
thing_play_sample(thing,
    112 + SOUND_RANDOM(3), 100, 0, 3, 0, 2, 256);
output_message_from_path("speech/a,b.ogg", 10);
'''
        calls = list(inventory.calls(text, inventory.CALLS))
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][1][1], "112 + SOUND_RANDOM(3)")
        self.assertEqual(calls[0][2], 4)
        self.assertEqual(calls[1][1][0], '"speech/a,b.ogg"')

    def make_repo(self, root):
        files = {
            "config/fxdata/sounds.cfg": "[sounds]\nDIG = 72 3\n36 = custom/splash.wav 2 SPLASH STACK=limit:2\n[speech]\nGameSaved = none\n",
            "config/creatrs/imp.cfg": "[sounds]\nFoot = 9 4\nHit = HIT\nDie = 0 1\n",
            "config/mods/test/sounds.cfg": "[sounds]\nDIG = custom/dig01.wav 2\n",
            "config/fxdata/effects.toml": "[effect1]\nSound = 47\n",
            "src/gui_soundmsgs.h": "enum TbSpeechMessages {\n SMsg_None = 0,\n SMsg_GameSaved,\n /* SMsg_Obsolete = 90, */\n SMsg_MAX = 3,\n};\n",
            "src/example.c": "void f() { play_non_3d_sample(72); thing_play_sample(t, dynamic_id, 100); }\n",
            "campgns/test/map00001.txt": "REM PLAY_MESSAGE(PLAYER0,SOUND,1)\nPLAY_MESSAGE(PLAYER0,SOUND,72)\n",
            "lang/speech_eng.pot": "msgid \"speech text is not an audio asset\"\n",
        }
        for path, text in files.items():
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        return files

    def test_manifest_keeps_layers_unknowns_and_speech_gaps(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_repo(root)
            result = inventory.build(root)
            cues = {cue["cue_key"]: cue for cue in result["cues"]}
            self.assertEqual(cues["named:DIG"]["legacy_ids"], [72, 73, 74])
            self.assertEqual(cues["named:36"]["redirect_from"], 36)
            self.assertEqual(cues["named:36"]["variant_count"], 2)
            self.assertEqual(cues["named:36"]["stack_policy"], "STACK=limit:2")
            self.assertEqual(cues["named:36"]["source_format_hint"], "wav")
            custom = cues["config:config/mods/test/sounds.cfg:sounds:DIG"]
            self.assertEqual(custom["legacy_ids"], [])
            self.assertIsNone(custom["duration_seconds"])
            self.assertEqual(custom["provenance"]["redistribution"], "unknown")
            self.assertEqual(cues["speech:GameSaved"]["legacy_ids"], [1])
            self.assertFalse(cues["speech:bank_2"]["named_message"])
            self.assertEqual(result["counts"]["references_by_kind"]["campaign_script"], 1)
            self.assertEqual(result["counts"]["tracked_media_files"], 0)
            self.assertEqual(len(result["speech_translation_sources"]), 1)
            self.assertEqual(inventory.render(result), inventory.render(inventory.build(root)))
            self.assertEqual(json.loads(inventory.render(result)), result)

    def test_untracked_media_is_not_published_and_source_changes_invalidate_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_repo(root)
            original = inventory.build(root)
            (root / "config/private.wav").write_bytes(b"private recording")
            self.assertEqual(inventory.build(root), original)
            with (root / "config/fxdata/sounds.cfg").open("a") as stream:
                stream.write("; source changed\n")
            self.assertNotEqual(inventory.build(root)["input_sha256"], original["input_sha256"])

    def test_cli_check_reports_drift_without_rewriting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_repo(root)
            command = [sys.executable, str(Path(inventory.__file__)), "--root", str(root)]
            subprocess.run(command, check=True, capture_output=True)
            subprocess.run(command + ["--check"], check=True, capture_output=True)
            output = root / "docs/audio/cue-inventory.json"
            original = output.read_bytes()
            with (root / "config/fxdata/sounds.cfg").open("a") as stream:
                stream.write("; drift\n")
            checked = subprocess.run(command + ["--check"], capture_output=True)
            self.assertEqual(checked.returncode, 1)
            self.assertIn(b"Inventory is stale", checked.stderr)
            self.assertEqual(output.read_bytes(), original)

    def test_duplicate_definition_fails_instead_of_silently_overwriting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_repo(root)
            (root / "config/fxdata/sounds.cfg").write_text("[sounds]\nDIG=72\nDIG=73\n")
            with self.assertRaisesRegex(ValueError, "duplicate cue keys"):
                inventory.build(root)


if __name__ == "__main__":
    unittest.main()
