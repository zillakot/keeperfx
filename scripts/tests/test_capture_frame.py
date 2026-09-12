import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("capture_frame", Path(__file__).parents[1] / "capture-frame.py")
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class CaptureTests(unittest.TestCase):
    def test_clone_isolates_saves_settings_and_assets(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, destination = Path(temporary) / "source", Path(temporary) / "work"
            source.mkdir()
            destination.mkdir()
            config = "API_ENABLED =TRUE\nDELTA_TIME=ON\nFRONTEND_RES=640x480w32\n"
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


if __name__ == "__main__":
    unittest.main()
