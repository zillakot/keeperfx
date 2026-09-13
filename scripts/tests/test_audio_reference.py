import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("audio_reference", ROOT / "scripts/capture-audio-reference.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


class TraceSummaryTests(unittest.TestCase):
    def summarize(self, records):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "trace.jsonl"
            path.write_text("".join(json.dumps(record) + "\n" for record in records))
            return REFERENCE.summarize_trace(path)

    def records(self, events):
        return ([dict(event="trace_begin", format="KFXAUDIO1")]
                + [dict(event=event, sequence=i, tick=i, turn=0, voice=voice)
                   for i, (event, voice) in enumerate(events)]
                + [dict(event="trace_end", records=len(events), overflow=0)])

    def test_restart_keeps_one_voice_and_stops_release_it(self):
        result = self.summarize(self.records([("openal_ready", 2), ("start", 1), ("restart", 1),
                                             ("start", 2), ("stop", 1), ("complete", 2), ("drop_tick_gate", 0)]))
        self.assertEqual(result["peak_observed_openal_voices"], 2)
        self.assertEqual(result["drops"], {"drop_tick_gate": 1})

    def test_rejects_incomplete_disabled_overflow_and_reordered_traces(self):
        records = self.records([("openal_ready", 2), ("start", 1)])
        variants = [records[:-1], self.records([("openal_failed", 0)])]
        overflow = json.loads(json.dumps(records))
        overflow[-1]["overflow"] = 1
        variants.append(overflow)
        reordered = json.loads(json.dumps(records))
        reordered[2]["sequence"] = 0
        variants.append(reordered)
        for variant in variants:
            with self.subTest(variant=variant), self.assertRaises(ValueError):
                self.summarize(variant)


@unittest.skipUnless(shutil.which("c++"), "C++ compiler required for production policy harness")
class ProductionPolicyTests(unittest.TestCase):
    def test_trace_preserves_production_policy_results_and_random_seed(self):
        source = (ROOT / "src/bflib_sndlib.cpp").read_text()
        policy = source[source.index('extern "C" SoundMilesID play_sample('):source.index('extern "C" void stop_sample(')]
        harness = r'''
#include <vector>
#include <unordered_map>
#include <cstdio>
#include <exception>
#include "audio_trace_buffer.h"
using SoundEmitterID = long;
using SoundSmplTblID = int;
using SoundMilesID = int;
using SoundVolume = int;
using SoundPan = int;
using SoundPitch = int;
#define ERRORLOG(...) ((void)0)
#define NORMAL_PITCH 100
struct { int frame_skip = 0; unsigned sound_random_seed = 42; } game;
unsigned random_sound(unsigned range) { game.sound_random_seed = game.sound_random_seed * 1664525 + 1013904223; return game.sound_random_seed % range; }
#define SOUND_RANDOM(range) random_sound(range)
struct openal_buffer {};
struct sample { openal_buffer buffer; };
struct source {
    int emit_id = 0, smptbl_id = 0, mss_id = 0, flags = 0, base_gain = 0;
    void stop() {} void gain(int) {} void pan(int) {} void repeat(bool) {} void pitch(int) {} void play(const openal_buffer &) {}
};
std::vector<source> g_sources = {{0, 0, 1}, {0, 0, 2}};
std::vector<sample> g_banks[2] = {std::vector<sample>(10), std::vector<sample>(10)};
std::vector<sample> g_custom_bank(2);
int g_speech_offset = 1000, g_custom_offset = 2000;
std::unordered_map<int, int> g_id_redirects;
struct SoundStackPolicy { int mode = 0, max_instances = 1; };
std::unordered_map<int, SoundStackPolicy> g_stack_policies;
std::unordered_map<int, unsigned long> g_tick_samples_last_tick;
unsigned long g_audio_tick_counter = 0;
bool g_bb_king_mode = true;
int bb_king_mode = 1, SStack_Duck = 1;
void apply_duck_gain(int) {}
void sound_trace_event(const char *event, long emitter, long requested, long resolved, long voice,
                       long volume, long pan, long pitch, long repeats, long priority) {
    audio_trace_append({event, g_audio_tick_counter, 0, game.sound_random_seed,
                       emitter, requested, resolved, voice, volume, pan, pitch, repeats, priority});
}
'''
        main = r'''
int main() {
    audio_trace_init();
    auto play = [](int emitter, int cue) { printf("%d ", play_sample(emitter, cue, 128, 64, 100, 0, 3)); };
    play(1, 1); play(1, 1); play(2, 1);
    ++g_audio_tick_counter; play(2, 1);
    play(3, 2);
    play(0, 1); play(1, 999);
    g_sources = {{0, 0, 1}, {0, 0, 2}};
    g_stack_policies[3] = {0, 1};
    play(1, 3); play(2, 3);
    g_id_redirects[4] = 2000;
    play(2, 4);
    printf("seed=%u\n", game.sound_random_seed);
}
'''
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            (work / "harness.cpp").write_text(harness + policy + main)
            subprocess.run(["c++", "-std=c++17", "-I", str(ROOT / "src"), str(work / "harness.cpp"),
                            "-o", str(work / "harness")], check=True, capture_output=True)
            env = {key: value for key, value in os.environ.items() if key != "KFX_AUDIO_TRACE"}
            disabled = subprocess.check_output([str(work / "harness")], env=env)
            trace = work / "trace.jsonl"
            enabled = subprocess.check_output([str(work / "harness")], env=env | {"KFX_AUDIO_TRACE": str(trace)})
            self.assertEqual(disabled, enabled)
            self.assertTrue(enabled.startswith(b"1 1 0 2 0 0 0 1 0 2 seed="), enabled)
            records = [json.loads(line) for line in trace.read_text().splitlines()]
            events = [record["event"] for record in records]
            for event in ("restart", "drop_tick_gate", "drop_stack_cap", "drop_sources_full", "drop_invalid_emitter", "drop_invalid_cue"):
                self.assertIn(event, events)
            self.assertTrue(any(r.get("requested") == 4 and r.get("resolved") == 2000 and r["event"] == "start" for r in records))
            subprocess.run(["c++", "-std=c++17", "-DKFX_AUDIO_TRACE_CAPACITY=2", "-I", str(ROOT / "src"),
                            str(work / "harness.cpp"), "-o", str(work / "bounded")], check=True, capture_output=True)
            bounded = subprocess.check_output([str(work / "bounded")], env=env | {"KFX_AUDIO_TRACE": str(trace)})
            self.assertEqual(disabled, bounded)
            records = [json.loads(line) for line in trace.read_text().splitlines()]
            self.assertEqual(records[-1]["records"], 2)
            self.assertGreater(records[-1]["overflow"], 0)
            failed = subprocess.check_output([str(work / "harness")], env=env | {"KFX_AUDIO_TRACE": str(work / "missing/trace")})
            self.assertEqual(disabled, failed)


if __name__ == "__main__":
    unittest.main()
