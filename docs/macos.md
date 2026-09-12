# macOS development build

This fork targets Apple Silicon with a native CMake build. It requires Homebrew
libraries at runtime; the CI artifact is an engine binary, not a standalone app.

## Build

Install Xcode Command Line Tools and Homebrew, then run:

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

Copy these original Dungeon Keeper files into the indicated directories:

| Directory | Files |
| --- | --- |
| `out/game/data` | `bluepal.dat`, `bluepall.dat`, `dogpal.pal`, `hitpall.dat`, `lightng.pal`, `redpal.col`, `redpall.dat`, `slab0-0.dat`, `slab0-1.dat`, `vampal.pal`, `whitepal.col` |
| `out/game/sound` | `atmos1.sbk`, `atmos2.sbk`, `bullfrog.sbk` |

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
be placed in `out/game/music`. Intel Macs and cross-platform save/multiplayer
compatibility have not been validated. Keep prototype saves separate from an
existing Windows installation.
