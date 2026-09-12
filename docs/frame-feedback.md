# Frame capture and Rust replay

The standalone Rust/wgpu tool replays the existing indexed framebuffer. It does
not replace the game's renderer or change its artwork. Reference and replay must
match every RGBA byte, including alpha.

## Quick feedback on macOS

With the native build dependencies, Rust stable, and game data in `out/game`:

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

Black difference pixels mean an exact match. A mismatch produces the report and
returns exit code 1. The capture's palette expansion is also checked against SDL
before reporting success. GPU setup and render/readback timings include CPU work
and are not gameplay FPS or GPU-only benchmarks.

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
`KFX_FRAME_CAPTURE_EXIT=1` exits after the single capture attempt. The Python
runner sets these variables only for its headless child process and checks for
complete output even if the engine returns success after a startup failure.

## Checks without original assets

```sh
export CARGO_TARGET_DIR="$PWD/out/rust-target"
cargo test --locked --manifest-path tools/frame-replay/Cargo.toml
cargo run --locked --manifest-path tools/frame-replay/Cargo.toml -- --fixture out/fixture
scripts/preview-frame.sh out/fixture out/fixture-check 3
```

The synthetic fixture uses all palette indices, asymmetric rows and an odd width
to exercise row alignment, orientation and scaling. CI runs parser/comparison
tests, renders scales 1/2/3 through software Vulkan, and verifies that changing a
single index causes a failed comparison. Captured game artwork remains under
the ignored `out` directory and is not uploaded by CI.
