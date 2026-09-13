---
type: guide
description: Reproduce and compare the first opt-in procedural audio pack and private original-source restoration experiments.
---

# Audio audition pack

This is an initial asset experiment for [A2](product/audio-modernization-plan.md).
Seven new procedural families are distributable. An optional local recipe adds
five original-source restoration families, making twelve represented families.
The pack is opt-in and unapproved for artistic quality. It does not change the
engine, activate a Remastered profile or replace any original bank.

## Delivered scope

| Family | Candidate | Variants | Integration |
| --- | --- | ---: | --- |
| Tab click | Muted wooden detent | 1 | Named `TAB_CLICK` |
| Button confirmation | Compact latch | 1 | Named `BUTTON_CLICK2` |
| Refusal | Low double detent | 1 | Named `REFUSAL` |
| Room placement | Stone impact and settling grit | 1 | Named `ROOM_BUILD` |
| Tile sale | Inward granular sweep | 1 | Named `TILE_SELL` |
| Digging | Mineral strike, metal overtone and grit | 3 | Named `DIG_IMPACT` |
| Wall combat impact | Dense stone and dull metal | 3 | Named `STRIKE_WALL` |
| Dungeon-heart hum | Local conservative restoration and loop seam repair | 1 | Private mod `HEART_ENGINE` |
| Imp movement | Local conservative restoration | 4 | Private imp `Foot` |
| Imp hurt | Local conservative restoration | 3 | Private imp `Hit` |
| Imp death | Local conservative restoration | 2 | Private imp `Die` |
| English mentor angry line | Local conservative restoration | 1 | Offline only; language lookup unchanged |

The seven replacement families are design candidates, not recordings of real
materials. Short tails are intentional experiments: digging candidates last
0.85 seconds, while the local original variants last about 2.43–3.25 seconds.
The onset starts immediately, but whether these shorter tails preserve the
right character and event feel remains a listening decision. All new samples
peak at -9 dBFS before engine gain; that is temporary production headroom, not a
final loudness specification or evidence of matched in-game loudness.

The local restoration removes DC, blends 20% of a 4.5 kHz one-pole lowpass with
80% dry signal, and applies a 2 ms boundary fade to one-shots. Heart processing
instead blends each 25 ms edge toward a shared boundary value, retaining the
original frame count. This does not recover lost bandwidth, remove recorded
noise or replace an actor. Source sample rates, mono layout and durations are
retained. Speech is deliberately excluded from the mod until localized lookup
and streamed/banked playback have their own comparison.

## Reproduce

From the repository root, with Python 3.10 or newer:

```sh
python3 scripts/audio-audition.py --output out/audition-new
```

The output must be empty. The command does not install anything or edit existing
settings. It produces `distributable/mods/audio_audition/`, including 11 playback
WAVs, 11 editable 24-bit masters, a manifest and normal `fxdata/sounds.cfg`.
Only these newly synthesized files are in the
[checked-in pack](../config/mods/audio_audition/README.md).
The source recipe is deterministic for a fixed Python/platform math runtime;
its parameters and exact output hashes accompany each build. Export to
16-bit uses round-to-nearest quantization without dither. The game currently
requires 8/16-bit PCM here; 24-bit masters must not be used as playback exports.

To build private comparisons using your own installation:

```sh
python3 scripts/audio-audition.py --output out/audition-private \
  --original-bank out/game/sound/sound.dat \
  --speech-bank out/game/sound/speech_eng.dat
```

This adds `local/originals/`, `local/masters/`, `local/ab/`, source hashes and a
private `local/mods/audio_audition_local/`. The bank reader selects the same
third directory and zero-based sample index as the current engine. It accepts
only mono 8/16-bit PCM for restoration and rejects unsupported formats instead
of substituting a generic ADPCM decoder for the legacy engine conversion.
Original bank filenames, selected sample names, indices and hashes are recorded
in the private manifests. No original audio or derivatives may be copied into
`distributable/` or committed: redistribution permission is unresolved.

## Install in an isolated game copy

Copy `distributable/mods/audio_audition` into the isolated game's `mods/`. Create
or edit that copy's `mods/load_order.cfg`:

```ini
[after_base]
audio_audition

[after_campaign]

[after_map]
```

The existing loader reads config from `mods/<name>/fxdata/sounds.cfg` and audio
from `mods/<name>/sound/`. Only list `audio_audition_local` after
`audio_audition` if the private restoration mod has also been copied into this
isolated installation. Keep the original bank files in place. Omitted named
cues retain their original definitions. There are no numeric redirects: for
example, replacing `DIG_IMPACT` does not replace `DOOR_PLACE`, although both
reference legacy IDs 72–74. Remove the mod entries and restart to compare
Original; do not edit personal game settings or saves for this experiment.

Before using the pack with arbitrary campaigns/maps, resolve the existing
same-name override behavior below and verify fallback for missing variants.
A failed full family must not accidentally borrow another family's custom
buffer. After-map placement is deliberately not used by this pack.

## Comparisons and evidence

Every `local/ab/<cue>.wav` plays Original, 0.5 seconds silence, then the candidate.
The two event windows are padded to equal length and matched by RMS. A common
gain then prevents either side from exceeding 0.8 full scale. Linear
interpolation converts comparison copies to 48 kHz; original WAVs remain intact.
RMS matching is an energy comparison, not perceptual loudness certification;
short transient and long-tail candidates still need a listener's judgement.
Per-pair gain and source metadata are written under `local/comparisons/` or the
local manifest. Final mix balancing must happen in-game, separately.

`quiet_arranged.wav` and `crowded_arranged.wav` compare deterministic offline
arrangements, separated by one second. The crowded arrangement includes a
rapid digging/impact burst and the mentor line when supplied. Their adjacent
JSON files specify cue names, start times, gains and final matching. These are
not gameplay captures and do not exercise spatialization, stacking, the voice
budget or possession. `heart_loop_three_cycles.wav` repeats each heart candidate
three times for seam audition.

The first production run used Python 3.14.4 on macOS and FFmpeg 8.1.1 for
independent WAV decode checks. The [validation artifact](audio/audition-validation.json)
records all eleven distributable exports and masters. Automated checks cover
mono layout, source/master consistency, usable signal, headroom, endpoint
continuity, repeatable variants, matched comparison energy, malformed bank
bounds and preservation of existing output directories:

```sh
python3 -m unittest discover -s scripts/tests -p test_audio_audition.py -v
```

Listening acceptance is open. No headphone or speaker listening was performed
by the producing agent, and passing signal/decoder tests does not establish
artistic quality. Evaluate recognition, fatigue, impact timing, speech masking,
loop repetition and relative levels in quiet/crowded gameplay and possession.
Water/room ambience, restored music, movie transitions and other languages are
not produced here. The inspected local `music/` contained no playable tracks.

## Override reproduction for the engine follow-up

Source inspection of the base revision found a blocker for general campaign
activation. This is a concrete sequence for a runtime regression, not a claim
that this pack has passed it:

1. Load base `TAB_CLICK = 60`.
2. Load this after-base mod's `TAB_CLICK = audition_tab.wav`.
3. Load a later campaign `TAB_CLICK = 61`. Expected: 61.
4. Independently restart and replace step 3 with a different custom filepath.
   Expected: the later file. Repeat at map and after-map tiers and after reload.

`SoundManager::getSoundId()` checks `custom_sounds_` before `sound_registry_`
([source](../src/sound_manager.cpp)), while `registerSound()` only updates the
latter. `loadCustomSound()` and its memory equivalent reuse an existing name
without checking the new file. Single-file named loads use `custom_sounds_`,
while multi-variant loads register the group separately. Consequently config
load ordering alone cannot establish same-name replacement correctness.
Resolve and test those paths before claiming campaign/map compatibility.
