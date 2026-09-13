---
type: reference
description: Reproducible source cue inventory, override rules and unresolved asset evidence for audio modernization.
---

# Audio cue inventory

The [machine-readable manifest](cue-inventory.json) starts A1 of the
[audio modernization plan](../product/audio-modernization-plan.md). It inventories
tracked definitions and selected playback references. It does not measure sound,
load original banks, resolve an active campaign, or prove runtime compatibility.

The integrated snapshot contains 115 named effect definitions (108 base and
seven audition-mod definitions), 417 creature definitions and 125 speech bank
slots, plus two music resolution entry points. Counts describe definitions, not unique recordings: aliases share
bank IDs and campaign definitions can override base creatures. The manifest's
`counts` is authoritative after regeneration.

The scanned `src`, `config`, `campgns` and `lang` directories contain 22 tracked
audio files: eleven new procedural audition exports and eleven editable masters.
The [audition guide](../audio-audition.md) records their provenance and validation.
Original bank contents, external mods, localized recordings and their rights
still require the local asset audit and listening work.

## Generate and inspect

From the repository root, using Python 3.11 or newer and Git:

```sh
python3 scripts/audio_inventory.py
python3 scripts/audio_inventory.py --check
python3 -m unittest discover -s scripts/tests -p test_audio_inventory.py -v
```

Generation writes only `docs/audio/cue-inventory.json`; `--check` reports drift
without writing. Inputs come from `git ls-files`, and their current worktree
contents are read. Untracked recordings are never scanned or embedded. Stage new
source/config files before generating to include them. The output has no clock,
absolute machine paths or changing commit ID; relevant input hashes and source
line references make it reproducible and reviewable. Regenerate when a source
change moves references or changes a hashed input, including engine tracing work.

The dedicated Audio inventory CI job runs the parser tests and drift check.
These checks establish extraction behavior and freshness, not sound quality.

For example, list the original imp's families:

```sh
python3 - <<'PY'
import json
from pathlib import Path
inventory = json.loads(Path('docs/audio/cue-inventory.json').read_text())
for cue in inventory['cues']:
    if cue['source']['path'] == 'config/creatrs/imp.cfg':
        print(cue['cue_key'], cue['legacy_ids'], cue['variant_count'])
PY
```

## Schema and coverage

`schema_version` is 1. `cues` carries the production audit fields:

| Field | Meaning |
| --- | --- |
| `cue_key` | Stable definition identity: `named:NAME`, `config:path:section:field`, `speech:NAME` or a music resolver key. Not a new runtime registry. |
| `source` and `definition` | Repository path, one-based line and raw configured value or bank-relative speech index. |
| `category`, `trigger` | Definition class and config field/name; trigger is an identifier, not a reconstructed gameplay condition. |
| `legacy_bank`, `legacy_ids` | Statically known `sound` or `speech` bank indices, with consecutive numeric variants expanded. Empty IDs mean disabled or unresolved; inspect the raw definition. |
| `variant_count` | Declared count when statically available. This does not establish that every file exists or loads. |
| `stack_policy` | Declared named-registry `STACK` token, otherwise the default tick gate. Creature rows do not establish a stack policy. |
| `source_format_hint` | Custom path extension only; not decoder evidence. |
| `duration_seconds`, `loop_points_frames`, `source_format`, `channels`, `language` | Unknown until actual source assets and decoded output are inspected. `null` never means zero duration, mono, no loop or English. |
| `override_rule` | Resolution family described below; not a claim that precedence passes at runtime. |
| `production_decision` | `unassessed` until retain/restore/replace is decided by audition. |
| `provenance` | Audio source, creator, license, permission, redistribution status, editable master and export recipe. Repository code licensing does not establish original audio rights. |

Store production findings in the audition pack's authored manifest, keyed back
to these cue keys. Do not hand-edit generated rows: regeneration would erase
changes. A later measured-asset importer can join those findings by cue key and
source hash without conflating definition coverage with audio-file coverage.

The generator reads every tracked CFG/TOML under the scan roots. `[sounds]`
entries form named or creature definitions; `[speech]` in `sounds.cfg` forms
speech override definitions. Other fields whose names contain `sound`, or start
with `messages`, remain raw `config_field` references. These include sound
priorities and variant parameters as well as IDs; they are not all cue IDs.

The speech enum supplies slots 1 through `SMsg_MAX - 1`. Unnamed slots remain
explicit `speech:bank_N` rows with `named_message: false`; this does not prove a
recording exists in a language's bank. Slot zero is the disabled sentinel.
`lang/speech_*.po` and `.pot` are listed as text sources, not recordings or proof
of localization coverage.

`references` retains selected C/C++ playback calls and campaign `PLAY_MESSAGE` /
`PLAY_MUSIC` calls with file/line evidence. The explicit C/C++ function list is
`CALLS` in [the generator](../../scripts/audio_inventory.py). Literal indices and
`N + SOUND_RANDOM(M)` expand; arbitrary expressions remain
`unresolved_expression`. The scanner handles nested arguments and multiline
calls but is not a C preprocessor or compiler. Calls in inactive preprocessor
branches can appear. It does not evaluate Lua, macro expansions, creature named
aliases, runtime registration, external mods, ZIP contents or arbitrary script
commands. It preserves config layering rather than pretending to choose the
winning active definition. Duplicate definition keys fail for explicit review.

## Resolution rules

These are source observations at the snapshot revision, with separate rules for
config layering and file lookup. They require dedicated runtime coverage before
an audition pack can promise campaign compatibility.

- **Config layering:** base fxdata, after-base mods, campaign config,
  after-campaign mods, per-map config, then after-map mods. See
  [config.c](../../src/config.c), `load_config` (line 2275). The default
  `sounds.cfg` header omits the final after-map tier.
- **`named_custom`:** extension-first search (`.wav`, `.mp3`, `.ogg`, `.flac`
  when omitted), then each tier in priority order: after-map mods' `sound/`,
  campaign levels, after-campaign mods' `sound/`, campaign config, after-base
  mods' `sound/`, fxdata, campaign media and main directory. Thus a lower tier's
  WAV can precede a higher tier's OGG when the extension is omitted. Explicit
  filenames avoid this ambiguity. ZIP lookup follows filesystem lookup. See
  [sound_manager.cpp](../../src/sound_manager.cpp),
  `resolve_sounds_cfg_sound_path` (line 662) and `load_named_sound_fs_or_zip`
  (line 732).
- **`creature_custom`:** the corresponding creature search also includes campaign
  creature and base creature directories; see `resolve_creature_sound_path`
  (line 608) in the same file.
- **Custom variants:** named `hit.wav 3` expands to `hit1.wav`, `hit2.wav`,
  `hit3.wav`; trailing digits preserve their width and increment. Creature paths
  without trailing digits instead repeat the same path. Both expansion paths cap
  at 32. See `sound_manager_load_named_sound` (line 772) and
  [config_crtrmodel.c](../../src/config_crtrmodel.c),
  `expand_numbered_sound_paths` (line 2286). Do not infer filenames from the
  `sounds.cfg` example's prose or assume named and creature expansion is identical.
- **`speech_bank_or_override`:** banked speech uses `speech_<language>.dat`, then
  `speech.dat`, then English bank fallback. An explicit `[speech]` override
  switches to path lookup; `none`, `null` and `0` silence it. See
  [bflib_sndlib.cpp](../../src/bflib_sndlib.cpp), `load_sound_banks` (around line
  502), and [gui_soundmsgs.cpp](../../src/gui_soundmsgs.cpp), `output_message`
  (line 342).
- **`speech_path`:** language variant, English variant, base path, then first
  available language directory. Each filesystem phase searches campaign config,
  campaign levels, campaign media and main. Finally the current map ZIP tries
  `speech/<language>/`, `speech/eng/`, then `speech/`. The arbitrary-language
  filesystem fallback is enumeration-dependent. See `resolve_speech_path`
  (line 216) in `gui_soundmsgs.cpp`; English precedes the base path, unlike the
  default config's explanatory text.
- **`music_resolution`:** numbered tracks begin at 2 and select from the first
  available format class in FLAC, WAV, OGG, MP3 priority, using the resolver's
  file enumeration order within that class. Track zero stops music. A numbered
  track is not an immutable filename. Filename playback via `play_music_fgroup`
  searches enabled music mods after-map, after-campaign, then after-base in
  reverse list order before the requested game directory. See
  `play_music_fgroup` (line 736) and `resolve_track_music_path` (line 782) in
  `bflib_sndlib.cpp`.

A source-level precedence defect remains at this snapshot:
`SoundManager::loadCustomSound` (line 157 in `sound_manager.cpp`) returns an
already loaded same-name custom asset without comparing paths, and `getSoundId`
(line 331) prefers custom assets over numeric registry entries. Later custom or
numeric definitions can therefore fail to replace an earlier custom definition.
This inventory records the intended ordering and the conflicting implementation;
it does not mark the defect fixed or claim an in-game reproduction. Resolve and
validate it before relying on remastered pack fallbacks or override guarantees.

## Remaining A1 evidence

The custom decoder accepts RIFF/WAV, MP3, BMU-wrapped MP3 and SDL-decoded formats
including OGG/FLAC (`decode_audio_buffer_and_store`, `bflib_sndlib.cpp`, around line
1366). Original banks use legacy WAV parsing, including a special conversion from the
legacy MSADPCM-tagged data to mono 8-bit output for `heart6a.wav` (around line 353). Decoder support does not
establish the format or channel layout of every cue. Movie sound remains in the
separate FFmpeg/SDL path in [bflib_fmvids.cpp](../../src/bflib_fmvids.cpp) and is
outside this cue extractor.

The next evidence is the local original-bank and custom-file audit: exact file
hashes, per-language presence, decoded duration/rate/channels, looping, clipping,
source rights and remaster decisions. Join selected runtime trace IDs back to
this inventory for the menu, dungeon, combat, possession, speech, music,
save/reload and movie references. Voice/drop counts, latency, CPU cost, capture
quality and listening outcomes remain outside this manifest. A1 is incomplete
until those measured references exist.
