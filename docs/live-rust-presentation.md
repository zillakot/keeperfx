---
type: guide
description: Build, select and validate optional live Rust/wgpu presentation on Apple Silicon while preserving SDL input and CPU gameplay drawing.
---

# Live Rust presentation

The optional presenter runs the existing indexed-frame palette shader directly
on a Metal window surface. Terrain, creatures, effects, menus, HUD and the software
cursor are still drawn by the existing C/C++ CPU renderer. SDL owns the window,
events, mouse and keyboard. The default presenter remains SDL.

## Build and select

Install Rust stable and the [Mac build dependencies](macos.md), then configure the
optional library and build the native executable:

```sh
cmake -S . -B out/macos -DKFX_RUST_PRESENTER=ON
cmake --build out/macos --target keeperfx --parallel 8
cd out/game
KFX_PRESENT_BACKEND=wgpu ../macos/keeperfx -nointro
```

`KFX_PRESENT_BACKEND=sdl`, or leaving it unset, selects the original presenter in
the same binary. `KFX_RUST_PRESENTER` defaults to `OFF`; enabling it requires Cargo
and macOS. Windows/Linux builds retain the existing implementation and do not
link Rust. Live support on those systems, Intel Macs and universal packaging has
not been validated. The local app bundle needs its executable refreshed separately
when building with the CMake commands above.

Use a separate asset installation with empty `save/` and `scrshots/` directories
for gameplay validation. The [profiling runner](performance-baselines.md) creates
such an installation automatically and accepts `--backend original|rust`.

## Ownership and lifecycle

[RendererSoftware](../src/kfx/renderer/RendererSoftware.cpp) creates an SDL Metal
view only when Rust is selected. It never creates an SDL renderer while that view
is in use. Rust borrows the view's `CAMetalLayer` and owns its instance, surface,
device, queue, palette/index textures, uniform, bind group and pipeline. Destruction
drops the Rust surface before destroying the SDL Metal view. Shutdown leaves SDL
window and event ownership with the existing platform layer.

The [C ABI](../src/kfx/renderer/RustPresenter.h) uses opaque handles, fixed-width
scalars, byte pointers and explicit lengths. All calls occur serially on the main
thread. The caller must keep handles and borrowed buffers valid for each call;
indices may have padded rows, and the palette is exactly 256 RGBA8 entries. Rust
validates dimensions, lengths, pitch and device limits before constructing input
slices. Upload calls copy bytes into wgpu-owned staging storage before returning;
no input pointer is retained. Rust catches panics at the boundary, reports errors
through a bounded UTF-8 buffer and marks a failed presenter terminal. Foreign
pointers must still satisfy the C contract; Rust cannot validate arbitrary addresses.

SDL continues to process focus, mouse capture, fullscreen and resize events.
When the cursor is uncaptured, absolute SDL event coordinates map from logical
window size to framebuffer size, including button events after a pointer warp.
Captured mouse movement retains its existing relative-input path.
Every frame queries physical window dimensions; the surface is reconfigured when
size or VSync changes. Minimized or zero-size windows skip presentation. Acquisition
Timeout/Occluded also skips a frame; outdated surfaces reconfigure and lost
surfaces are recreated, with at most one immediate retry. Late device errors are
observed through nonblocking polling. A terminal error releases the Rust presenter
and view, then creates the original SDL renderer. It does not repeatedly retry
Rust during the same renderer lifetime. Profiling treats any skipped/failed frame
or fallback as an invalid sample run.

## Pixels and timing

The same shader is used for replay and live output. The live target must expose
`Bgra8Unorm` or `Rgba8Unorm`; display-encoded palette bytes must not be encoded a
second time by an sRGB attachment. There is no blending, preserving RGBA values
including transparent RGB in renderer tests. Game palettes are opaque, and the
surface compositor uses opaque alpha. The image fills the window with nearest
sampling, matching the existing SDL full-window presentation. Integer scales
replicate pixels exactly; arbitrary window sizes necessarily produce uneven pixel
widths. Monitor color management remains outside byte comparison.

Indices/palette are uploaded and one pass renders directly into the acquired
surface. Indexed texture and binding change only when input dimensions change;
there is no retained offscreen output texture in the live path. There is no routine
readback or GPU completion wait. wgpu/driver submission and staging allocations
still occur; retained resources do not imply allocation-free presentation.

`presentation` covers cursor composition, acquisition, polling/reconfiguration,
input upload, drawing submission, present submission and cursor cleanup. The nested
`present_wait` measures only the backend-specific present API; compare total
presentation rather than treating the two APIs' nested durations as equivalent
GPU work. [Performance baselines](performance-baselines.md) describe paired runs,
process CPU counters, scoped Rust host allocations and reproducibility limits.

## Focused validation

```sh
cargo test --locked --manifest-path tools/frame-replay/Cargo.toml --features live-surface
cargo test --locked --manifest-path tools/frame-replay/Cargo.toml --features live-surface -- --ignored
```

GPU tests need native Metal access. Existing replay tests cover resource reuse,
size changes, rejected inputs and device errors. Direct BGRA attachment tests add
padded rows, scales 1/2/8, palette and alpha transitions, transparent RGB and a
negative comparison. Ordinary CI tests do not require original game assets.

For an isolated live test, set `KFX_WGPU_VERIFY=1`. This explicitly enables surface
`COPY_SRC`, reads back each acquired rendered surface before presenting and compares
every channel against the uploaded indices and palette at the actual output size.
Any mismatch is terminal and logs fallback. Shutdown logs total presented and
verified frames. This debug path is deliberately synchronous and must not be used
for performance measurements. Ordinary screenshots and frame captures still use
the indexed CPU image and are not proof of surface output by themselves.

`KFX_WGPU_FAIL_INIT=1` injects initialization failure; `KFX_WGPU_FAIL_AFTER=N`
injects a terminal failure after N presented frames. Both must log SDL fallback.
The profile runner removes inherited verification/fault flags and rejects a Rust
run that actually presents through SDL. Keep lifecycle and interactive gameplay
findings alongside paired measurements in the delivery PR; unit tests cannot
establish successful input, audio, save/reload or window behavior.
