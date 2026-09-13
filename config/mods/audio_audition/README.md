# Experimental audio audition

Seven procedural sound families, opt-in for isolated listening sessions. This is
an artistic prototype, not an approved Remastered profile. No original game
samples are included. The samples and recipe use the repository's
[GPL-2.0-or-later license](../../../LICENSE).

Copy this directory into an isolated game installation's `mods/`, then add
`audio_audition` under `[after_base]` in `mods/load_order.cfg`. Do not activate it
in personal saves or campaign/mod collections until override compatibility has
been verified. Remove the load-order entry and restart to return to Original.

`fxdata/sounds.cfg` replaces named cues only; numeric IDs shared by other cues
are untouched. `sound/` contains mono 48 kHz/16-bit playback files. `masters/`
contains lossless 24-bit masters; the current game WAV path does not accept
24-bit files. `manifest.json` records the source, recipe parameters and hashes.
The seed, filtering, envelopes and export quantization are implemented in
[scripts/audio-audition.py](../../../scripts/audio-audition.py).

See the [audition guide](../../../docs/audio-audition.md) for regeneration,
private original/candidate comparisons, scope and acceptance gaps.
