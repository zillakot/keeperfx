---
type: guide
description: Build and run the native Apple Silicon development version with matching game assets and Homebrew libraries.
---

# macOS development build

For offline graphics development, see [frame capture and Rust replay](frame-feedback.md).

This fork targets Apple Silicon with a native CMake build. It requires Homebrew
libraries at runtime; the CI artifact is an engine binary, not a standalone app.

## Build

Install Xcode Command Line Tools and Homebrew. Run these commands from the
repository root:

```sh
brew install cmake ninja pkgconf sdl3 sdl3_image sdl3_mixer ffmpeg \
    openal-soft luajit libspng minizip miniupnpc libnatpmp
scripts/build-macos.sh
```

CMake builds astronomy, centijson and enet6 from pinned source revisions. The
executable is `out/macos/keeperfx`. The build script also creates
`out/game/KeeperFX.app`, which uses the same installed Homebrew libraries.

## Game data

Extract a complete KeeperFX release into `out/game`, then overlay the alpha
patch matching the upstream source revision. Downloads are available at
<https://keeperfx.net/downloads>. Alpha patches alone do not contain all assets.
Use `_keeperfx.cfg` from the patch as the initial `keeperfx.cfg` for a new install.

Copy the 14 files in [the original-file list](files_required_from_original_dk.txt)
into `out/game`, preserving their `data/` and `sound/` paths. Files copied from a
supported digital installation can be used; the native engine does not run the
original Windows executable.

The `out` directory is ignored by Git. Keep original assets and personal saves
there, outside tracked source files.

## Run

Double-click `out/game/KeeperFX.app` in Finder. Alternatively, run
`scripts/launch-macos.command` in Terminal to skip the intro. Both use `out/game`,
where settings, saves, screenshots and `keeperfx.log` are stored. Keep the app
beside its game data. Quit through the game menu.

For an initial windowed setup, set these values in `out/game/keeperfx.cfg`:

```ini
FRONTEND_RES=1280x800w32 1280x800w32 1280x800w32
INGAME_RES=1280x800w32 1440x900w32 1920x1080w32
```

Native Steam integration and audio-CD playback are unavailable. Music files can
be placed in `out/game/music`.

## Validation scope

[PR #1](https://github.com/zillakot/keeperfx/pull/1) records the initial native
launch, gameplay and save/reload checks on macOS 26.6.2, plus the user's successful
initial playtest. That build used complete KeeperFX 1.4.0 assets with alpha 5388,
matching upstream commit `d1c961b2b`.

Those checks establish a working development baseline. They do not establish
full-campaign completion, Intel support or cross-platform save/multiplayer
compatibility. Keep later runtime findings in the relevant issue or PR.
Keep prototype saves separate from an existing Windows installation.
