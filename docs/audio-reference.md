---
type: guide
description: Capture bounded audio command references with isolated assets and native playback, before changing audio policy or backends.
---

# Audio command references

This is the first engine slice of [A1](product/audio-modernization-plan.md#delivery-sequence).
It records existing effect/banked-speech playback decisions and scene frames with
sound enabled. It does not record an audio waveform, implement Rust audio, or
establish listening quality, backend parity or end-to-end latency.

## Capture

Build the native engine using [the macOS guide](macos.md), then run:

```sh
python3 scripts/capture-audio-reference.py --scene dungeon \
    --out out/audio-reference/dungeon
python3 scripts/capture-audio-reference.py --scene menu \
    --out out/audio-reference/menu
python3 scripts/capture-audio-reference.py --scene possession \
    --out out/audio-reference/possession
```

Each run copies the game assets to a temporary directory, creates fresh saves and
settings, disables the control API, and retains a copy of the executable, its
SHA-256, dynamic dependency information, effective configuration, process/game
logs, scene frames, `audio.jsonl` and `reference.json`. `--game-dir` and `--engine`
select alternate asset/build directories. Output must be a new directory beneath
ignored `out/`. Do not commit the original assets, screenshots or recordings.
Record the source commit and any build diff beside each retained executable;
the executable hash identifies the binary, not which source built it.

The default captures eight frames at 20-presentation intervals, starting at turn
20 for gameplay. `--turn`, `--frames` and `--interval` control that window. Menu
references retain a stationary game turn while the audio tick advances.
Possession uses the existing frame fixture's controlled creature selection.
These are scene references, not automated combat, save/reload or listening tests.

Video uses SDL's dummy driver, while audio uses the default native device.
Inherited `KFX_`, `SDL_` and `ALSOFT_` overrides are removed. The run intentionally
fails validation if OpenAL cannot initialize; a screenshot alone is insufficient.
The existing `capture-frame.py` keeps its `-nosound` behavior. Audio runs are
separate from graphics benchmarks and must not run concurrently with them.

## Trace contract

`KFX_AUDIO_TRACE=/absolute/path/audio.jsonl` enables tracing for other controlled
local runs. The file is overwritten at startup. Keep this path in a new ignored
output directory. An unset variable adds no allocation or file access. When
active, initialization allocates a fixed 65,536-record buffer; game-thread events
copy scalar values and static event names into memory. No extra sound randomness,
OpenAL queries, policy calls or output writes occur per event. The buffer is
written on normal process exit; a killed process is not a complete reference.
Overflow preserves gameplay and reports discarded records in the footer; the
runner rejects such traces. Shorten the capture instead of interpreting truncation
as fewer drops. This is diagnostic instrumentation with memory/CPU overhead,
not a performance measurement mode.

`KFXAUDIO1` JSON Lines records have a sequence, audio tick, game turn, sound RNG
seed snapshot, emitter, requested/resolved cue, voice, volume, pan, requested
pitch, repeat flag and priority. Numbers preserve legacy units. Fields not
applicable to an event are zero. The relevant events are:

| Event | Meaning |
| --- | --- |
| `managed_request` | Managed scheduler selected a slot (`voice` is its slot index); carries requested priority. |
| `drop_managed_full` | The managed scheduler returned no slot. |
| `request` | Entry to `play_sample`, after caller-side variation selection. |
| `start`, `restart` | OpenAL submission succeeded; resolved cue reflects numeric redirection and `voice` is the backend sample ID. |
| `drop_tick_gate`, `drop_stack_cap` | Existing default tick gate or explicit concurrency cap rejected the request. |
| `drop_invalid_emitter`, `drop_invalid_cue`, `drop_sources_full`, `drop_backend_error` | Other playback failures, kept distinct. |
| `complete` | The existing monitor observed a stopped OpenAL source. |
| `stop`, `stop_all` | Explicit stop submission succeeded. |
| `openal_ready`, `openal_failed` | Initialization result; ready `voice` is the allocated backend source count. |
| `music_request`, `music_start`, `music_failed` | File-music request/submission outcome; filenames and asset bytes are omitted. |
| `tick`, `sdl_shutdown` | Audio monitor tick and SDL audio teardown. |

The summary counts observed starts/stops/completions to report peak observed
OpenAL voices and drops by reason. It is not a hardware voice measurement.
Managed slot IDs and backend voice IDs are different namespaces. Successful
`play_sample` records have no priority because priority belongs to the managed
scheduler. Requested pitch may differ from an easter-egg pitch; gain is the
submitted legacy volume before backend master gain and duck scaling. No waveform
or decoded sample bytes are logged.

## Verification and remaining A1 work

```sh
python3 -m unittest discover -s scripts/tests -p 'test_audio_reference.py' -v
```

The synthetic C++ harness compiles the current production `play_sample` body with
stubbed playback objects. It checks same-emitter restart, default tick gating,
explicit stack caps, source exhaustion, invalid inputs and numeric redirection.
The same request sequence must preserve return values and sound RNG state with
tracing disabled, enabled, full, or unable to open its output. This proves bounded
policy noninterference for those cases; stubbed audio is not signal validation.
The summary tests reject missing exits, overflow, failed initialization and
invalid ordering, and prevent a restart from counting as another voice.

Still required for A1: controlled crowded combat, mentor queue/suppression and
streamed-speech tracing, selected-variation provenance, parameter updates and
actual duck-scaled gain, managed eviction/capacity occupancy, bank-reset markers,
music identity and completion, save/reload and movie references, signal captures,
device recovery, retained dependency packages, and audio-enabled cost/latency
measurements. Source loading and random events make separate native runs
nonidentical; compare policy against a recorded completion schedule before any
migration. Windows/Linux runtime and listening remain separate checks.
