---
type: guide
description: Capture game frames headlessly and compare standalone Rust/wgpu output against SDL without affecting personal saves.
---

# Frame capture and Rust replay

For live original-engine timing, use the separate [performance baseline runner](performance-baselines.md).

The standalone Rust/wgpu tool replays the existing indexed framebuffer. It does
not replace the game's renderer or change its artwork. Reference and replay must
match every RGBA byte, including alpha.

## Quick feedback on macOS

Install the dependencies from [macOS development](macos.md) and Rust stable,
and prepare game data in `out/game`. Run from the repository root:

```sh
scripts/graphics-feedback.sh
```

This builds the engine, captures original campaign level 1 at turn 20 without
opening a window, and renders that capture through wgpu. Open the printed
`comparison/report.html` path in a browser. An optional new output directory and
integer scale can be supplied:

```sh
scripts/graphics-feedback.sh out/graphics-experiment 2
```

The capture process uses a temporary copy of the assets, its own settings and
empty save directory. On macOS, APFS clones avoid copying the asset bytes when
supported. It disables audio, the game API and interpolation. The installed app
and personal saves are not modified.

Game clocks and random seeds can make newly captured scenes differ. For
repeatable renderer comparisons, reuse an existing capture rather than starting
a new simulation:

```sh
scripts/preview-frame.sh out/graphics-experiment/capture out/replay-v2 3
```

Scales 1–8 use integer nearest-neighbor replication. Inputs and scaled outputs
are limited to 8192 pixels per axis and 16 megapixels. Existing capture/report
directories are rejected so an older result cannot be mistaken for a new one.

## Reference cases

The runner accepts `--scene dungeon|menu|possession`, `--campaign`, `--level`,
`--resolution WIDTHxHEIGHT`, `--turn`, `--frames` and `--interval`. Defaults retain
the original single dungeon frame at 640×480 on campaign `keeporig`, level 1,
turn 20. Menu capture waits for the main menu; gameplay capture waits for the
requested turn. Possession enters an owned creature through the engine's control
path and waits for the creature view before capturing.

For example, after building the engine:

```sh
python3 scripts/capture-frame.py --scene menu --resolution 800x600 --out out/reference-menu
python3 scripts/capture-frame.py --scene dungeon --campaign keeporig --level 2 --resolution 1280x800 --frames 3 --interval 2 --out out/reference-dungeon
python3 scripts/capture-frame.py --scene possession --frames 3 --out out/reference-possession
scripts/preview-frame.sh out/reference-possession out/possession-comparison 2
```

`--frames` accepts 1–32; `--interval` accepts 1–60 eligible presentation calls.
Presentation calls are distinct from simulation turns, and successive captures
may contain identical pixels. Each sequence preserves its original order and
actual capture metadata. The runner rejects a silently substituted resolution,
incomplete sequences, and output outside the repository's ignored `out` tree.
Scenes are selected from fresh temporary games; personal saves are never loaded.
A campaign without a suitable owned creature cannot produce a possession case.
The runner fixes its isolated settings to 20 turns per second and allows the
scheduled duration plus 60 seconds for startup and capture, with a minimum timeout
of 120 seconds and a maximum of 183 seconds for the accepted arguments.

Palette animation may occur in captured game sequences, but those frames do not
guarantee a palette-only transition. The synthetic sequence below provides that
controlled case, including alpha transitions that the game's opaque palette
cannot produce.

## Outputs

| File | Purpose |
| --- | --- |
| `capture/frame.kfx` | Frozen indices and display palette |
| `capture/reference.png` | SDL-converted image from the same presentation call |
| `capture/capture.json` | Dimensions, turn and SHA-256 hashes of the engine and captured files |
| `capture/keeperfx.log`, `process.log` | Capture-process diagnostics |
| `comparison/reference.png`, `gpu.png` | Reference and GPU readback at the requested scale |
| `comparison/comparison.png` | Reference on the left, GPU output on the right |
| `comparison/difference.png` | Maximum RGBA error per pixel, amplified eight times in red |
| `comparison/report.json`, `report.html` | Metrics and an offline viewer with view/zoom controls |

With multiple frames, each `frame-NNNN/` directory contains the three capture
files. Root `sequence.json` records their order; logs stay at the root. The preview
script detects the sequence and writes one comparison report per frame plus a
sequence summary. Each frame's metadata includes dimensions, view, turn,
presentation ordinal, requested settings and content/engine hashes.

Black difference pixels mean an exact match. A mismatch produces the report and
returns exit code 1. The capture's palette expansion is also checked against SDL
before reporting success. `setup_ms` records shared renderer initialization once (the same value is repeated
in each frame report). `render_readback_ms` includes that frame's resource resizing,
uploads, drawing, readback and CPU work. Neither is gameplay FPS or a GPU-only
benchmark, and these boundaries differ from the former per-frame setup timing.

The output target is `Rgba8Unorm`: the palette already contains display-encoded
bytes, so an sRGB render target would incorrectly encode them a second time.
This tests offscreen output; window scaling, monitor color management, input and
live performance still need separate checks.

## Capture format

`KFXFRM01` is followed by little-endian `uint32` width and height, then 256 RGBA8
palette entries and `width × height` one-byte indices in top-to-bottom row order.
Rows have no padding. Trailing bytes and invalid dimensions are rejected.

The game capture hook is inactive unless `KFX_FRAME_CAPTURE` names a new output
directory whose parent exists. `KFX_FRAME_CAPTURE_TURN` defaults to 20;
`KFX_FRAME_CAPTURE_SCENE` defaults to `dungeon`, `KFX_FRAME_CAPTURE_COUNT` to 1,
and `KFX_FRAME_CAPTURE_INTERVAL` to 1. `KFX_FRAME_CAPTURE_EXIT=1` exits after the
capture attempt or sequence. The Python runner sets these variables only for its
headless child process and checks for complete output even if the engine returns
success after a startup failure.

A sequence manifest has format `KFXSEQ01` and a `frames` array containing `frame`
and `reference` paths relative to the manifest. Every entry pairs a frozen indexed
frame with its exact RGBA reference; it is not an instruction to rerun gameplay.

## Checks without original assets

```sh
export CARGO_TARGET_DIR="$PWD/out/rust-target"
cargo test --locked --manifest-path tools/frame-replay/Cargo.toml
cargo run --locked --manifest-path tools/frame-replay/Cargo.toml -- --fixture out/fixture
scripts/preview-frame.sh out/fixture out/fixture-check 3
cargo run --locked --manifest-path tools/frame-replay/Cargo.toml -- --sequence-fixture out/sequence-fixture
scripts/preview-frame.sh out/sequence-fixture out/sequence-check 2
python3 -m unittest discover -s scripts/tests -v
```

With a built engine and original assets, also verify failed live captures keep
gameplay running unless `KFX_FRAME_CAPTURE_EXIT=1`:

```sh
KFX_TEST_ENGINE=out/macos/keeperfx KFX_TEST_GAME_DIR=out/game \
  python3 -m unittest discover -s scripts/tests -v
```

The synthetic fixtures use all palette indices, asymmetric rows and odd widths
to exercise row alignment, orientation and scaling. Ordered cases include
palette-only changes, transparent entries with nonzero RGB, index changes and
dimension changes. CI runs parser/comparison and capture-isolation tests, renders
through software Vulkan, and verifies deliberate index, palette and alpha
mismatches. Captured game artwork remains under the ignored `out` directory and
is not uploaded by CI.

## Reusable renderer

The Rust package exposes `frame::Frame` and `gpu::Renderer` as a library.
The offline CLI creates one renderer per invocation, including all entries in a
sequence. A future adapter can select a surface-compatible device and queue and
pass them to `Renderer::new`; surface and window integration remain separate work.

`Renderer::render(&Frame, scale)` copies the input bytes, submits GPU work and
returns a borrowed `Rgba8Unorm` texture. It does not wait for completion or map a
readback buffer. Submit any copy or sampling of that texture before the next
render, which may overwrite it. The caller must serialize rendering and consumers
on the supplied queue. The shader preserves transparent RGB and alpha without
blending or a second sRGB conversion.

The renderer owns the device, queue, pipeline, palette texture and uniform buffer.
It retains the indexed texture and bind group until input dimensions change, and
the output texture and view until scaled output dimensions change. Scale-only
changes update the uniform and output; palette-only updates retain every resource.
There is one current input and output allocation, with no cache of previous sizes.
Dropped resources remain alive inside wgpu until submitted work using them
completes; callers control the amount of work in flight. Offline replay waits for
each frame, so it cannot accumulate an unbounded sequence of pending readbacks.

The library validates dimensions, scales, palette/index lengths and device texture
limits before changing retained resources. Rejected inputs leave the renderer
usable. It owns the supplied device's error/loss callbacks and records the first
GPU error as terminal. `check_status()` reports errors delivered so far; a future
adapter must drive device polling and handle polling failures as well as late
callbacks, then replace the device and renderer after failure. This extraction
does not implement automatic device recovery or a live fallback.

Only the binary's `offline` module allocates/maps readback buffers. It checks the
device buffer limit, bounds completion waits and unmaps even on readback failure.
GPU-dependent tests run explicitly in CI and can be run locally with:

```sh
cargo test --locked --manifest-path tools/frame-replay/Cargo.toml -- --ignored
```

Those tests check actual texture/bind-group identity, size and scale transitions
through one renderer, pending-work resource lifetimes, exact RGBA restoration after
rejected inputs, deliberate mismatches and terminal validation/device-loss errors.
They require a GPU adapter; the regular unit tests remain GPU-independent.

## Validation scope

[PR #2](https://github.com/zillakot/keeperfx/pull/2) records zero-difference Metal
comparisons for a real game frame at 1× and 2×, synthetic alignment/alpha cases,
and a deliberate one-pixel failure. CI separately checks synthetic frames through
software Vulkan and builds the game on macOS, Linux and Windows.

The recorded cached build/capture/replay run took about 7.5 seconds locally.
That measures the developer feedback loop, not gameplay performance. See the
[project overview](architecture/project-overview.md#purpose-of-this-fork) for the
boundary between the live engine and this prototype.
