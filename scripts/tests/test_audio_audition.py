import importlib.util
import math
from pathlib import Path
import struct
import tempfile
import unittest
import wave

SPEC = importlib.util.spec_from_file_location('audio_audition', Path(__file__).parents[1] / 'audio-audition.py')
AUDIO = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIO)


class AuditionTests(unittest.TestCase):
    def test_exports_match_masters_and_have_headroom(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            AUDIO.build(root)
            mod = root / 'distributable/mods/audio_audition'
            for path in (mod / 'sound').glob('*.wav'):
                with wave.open(str(path)) as wav:
                    self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 48000))
                    pcm = [v[0] for v in struct.iter_unpack('<h', wav.readframes(wav.getnframes()))]
                self.assertGreater(max(map(abs, pcm)), 1000)
                self.assertLess(max(map(abs, pcm)), 12000)
                self.assertEqual(pcm[0], 0)
                self.assertEqual(pcm[-1], 0)
                with wave.open(str(mod / 'masters' / path.name)) as wav:
                    self.assertEqual(wav.getsampwidth(), 3)
                    self.assertEqual(wav.getnframes(), len(pcm))
                    raw = wav.readframes(wav.getnframes())
                    masters = [int.from_bytes(raw[i:i + 3], 'little', signed=True) for i in range(0, len(raw), 3)]
                self.assertLess(max(abs(x / 32767 - y / 8388607) for x, y in zip(pcm, masters)), 1 / 32767)

    def test_generated_mod_preserves_named_variants_and_has_no_redirects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mod = AUDIO.build(root)
            lines = (mod / 'fxdata/sounds.cfg').read_text().splitlines()[1:]
            self.assertEqual(len(lines), 7)
            for line in lines:
                cue, _, filename, count = line.split()
                self.assertTrue(cue[0].isalpha())
                for variant in range(1, int(count) + 1):
                    actual = filename if int(count) == 1 else filename.replace('1.wav', f'{variant}.wav')
                    self.assertTrue((mod / 'sound' / actual).is_file())
            self.assertFalse((root / 'local').exists())
            self.assertNotIn('after_map', '\n'.join(lines))

    def test_seeds_produce_distinct_reproducible_variants(self):
        a = AUDIO.synthesize('dig', .3, 330, 1)
        self.assertEqual(a, AUDIO.synthesize('dig', .3, 330, 1))
        self.assertNotEqual(a, AUDIO.synthesize('dig', .3, 330, 2))

    def test_loop_repair_preserves_duration_and_closes_boundary(self):
        source = [.1 + .2 * math.sin(i * .077) for i in range(22050)]
        fixed = AUDIO.restore(source, 22050, True)
        self.assertEqual(len(fixed), len(source))
        self.assertAlmostEqual(fixed[0], fixed[-1], places=12)
        self.assertGreater(AUDIO.rms(fixed), .05)

    def test_level_matching_pads_duration_and_uses_common_headroom(self):
        a, b, _, _ = AUDIO.match_pair([.1, -.1], [.8, -.8, .8, -.8])
        self.assertEqual(len(a), len(b))
        self.assertAlmostEqual(AUDIO.rms(a), AUDIO.rms(b), places=12)
        self.assertLessEqual(max(map(abs, a + b)), .8)
        with self.assertRaisesRegex(ValueError, 'silence'):
            AUDIO.match_pair([0.], [.1])

    def test_bank_reader_rejects_unbounded_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bank.dat'
            for raw in (b'', b'\xff' * 200, bytes(200)):
                path.write_bytes(raw)
                with self.assertRaises(ValueError):
                    AUDIO.bank_sample(path, 1)

    def test_private_source_recipe_cannot_leak_into_distributable_pack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'sample.wav'
            AUDIO.write_wav(source, [.2 * math.sin(i * .1) for i in range(1024)], 22050)
            raw = source.read_bytes()
            header, table, count = 12, 192, 1068
            base = table + count * 32
            data = bytearray(base)
            struct.pack_into('<IIII', data, header + 18 + 32, table, base, count * 32, count)
            for i in range(count):
                struct.pack_into('<18sIIIBB', data, table + i * 32, b'synthetic.wav', 0, 22050, len(raw), 0, 0)
            data.extend(raw)
            data.extend(struct.pack('<I', header))
            bank = root / 'synthetic.dat'
            bank.write_bytes(data)
            samples, rate, original, metadata = AUDIO.bank_sample(bank, 493)
            self.assertEqual((len(samples), rate, original), (1024, 22050, raw))
            self.assertEqual(metadata['source_sha256'], AUDIO.digest(raw))
            private = AUDIO.build(root / 'private', bank, bank)
            public = AUDIO.build(root / 'public')
            self.assertEqual(sorted(p.relative_to(public) for p in public.rglob('*')),
                             sorted(p.relative_to(private) for p in private.rglob('*')))
            for path in public.rglob('*'):
                if path.is_file():
                    self.assertEqual(path.read_bytes(), (private / path.relative_to(public)).read_bytes())
            self.assertTrue((root / 'private/local/ab/mentor_angry.wav').is_file())
            self.assertTrue((root / 'private/local/ab/heart_loop_three_cycles.wav').is_file())

    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'keep').write_text('existing audition')
            with self.assertRaisesRegex(ValueError, 'empty'):
                AUDIO.build(root)
            self.assertEqual((root / 'keep').read_text(), 'existing audition')


if __name__ == '__main__':
    unittest.main()
