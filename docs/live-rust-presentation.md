---
type: guide
description: Build, select and validate optional Rust/wgpu presentation and partial GPU drawing on Apple Silicon, including ownership, synchronization and fallback.
---

# Live Rust presentation

The optional presenter runs the existing indexed-frame palette shader directly
on a Metal window surface. Drawing defaults to the C/C++ CPU renderer; the
separate opt-in [partial GPU drawing path](#partial-gpu-drawing) replaces selected
terrain and 2D pixel loops. SDL owns the window, events, mouse and keyboard.
The default presenter remains SDL.

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

For framebuffer presentation, indices/palette are uploaded and one pass renders directly into the acquired
surface. Indexed texture and binding change only when input dimensions change;
there is no retained offscreen output texture in the live path. There is no routine
readback or GPU completion wait in this presenter path. The partial drawing bridge
below does require synchronous readback. wgpu/driver submission and staging allocations
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

## Partial GPU drawing

Build with `KFX_RUST_PRESENTER=ON` as above. Set `KFX_DRAW_BACKEND=wgpu` to
route the implemented terrain and 2D families through compute drawing;
`KFX_DRAW_BACKEND=software` or leaving it unset keeps CPU drawing. This selector
is independent of `KFX_PRESENT_BACKEND=sdl|wgpu`: GPU drawing can feed either
presenter through the synchronized native image. Unsupported platforms retain
software drawing. The [canonical coverage ledger](product/rust-port-plan.md#execution-and-coverage-ledger)
records exactly which paths are implemented and validated. Full GPU drawing and
a speedup are not established.

This path merges to `master` as an opt-in partial foundation; software drawing
and SDL presentation stay default. At 1920×1080 it currently costs about 68–70 ms
per drawn frame against 3.3–3.5 ms software, dominated by waiting rather than
computation. The [status section](product/rust-port-plan.md#status-2026-09-14)
records the measurement, the diagnosis and the single-stream restructure that
follows.

The [indexed backend](../tools/frame-replay/src/draw.rs) stores one `u32` palette
index per pixel. CPU binning preserves command order within 16×16 tiles; each GPU
invocation owns one destination pixel and evaluates its ordered commands. Exact
integer operations preserve texture/shade lookup and destination-index palette
composition. The [drawing C ABI](../src/kfx/renderer/WgpuDraw.h) owns copied,
immutable resource versions and validates complete batches before submission.
Its direct GPU-target palette presentation API is tested offscreen, but the game
still presents the synchronized native framebuffer.

[WgpuTerrainBridge](../src/kfx/renderer/WgpuTerrainBridge.cpp) batches selected
terrain between audited world-dispatch boundaries. Accepted textured gpoly calls
copy original unsorted vertices and return before CPU sorting, setup, clipping or
scan conversion. [GPU preparation](../tools/frame-replay/src/gpoly.rs) supplies its
row buffer directly to the [ordered pixel pass](../tools/frame-replay/src/draw_triangles.rs);
no prepared rows are read back to construct draw commands. Each destination pixel
consumes the bounded triangle batch in submission order. A four-byte GPU shade
validation flag is read back before target writes. Other bucket entries flush
terrain first; switching between original triangles and the retained span path
also flushes pending work. Pixel, box, HV-line and circle hooks in
[bflib_vidraw.c](../src/kfx/renderer/software/bflib_vidraw.c) use the same bridge.
Circles execute their integer coverage recurrence on GPU. The
[sprite adapter](../src/kfx/renderer/software/WgpuSprite.c) decodes RLE into immutable
index/coverage assets and copies native scale ranges and tables; GPU source selection
performs supported scaling, flips, remap and blending. Scaled solid horizontal flips
with duplicated rows use ordered GPU run copies, preserving native extra-left pixels,
four-byte copy grouping and target alignment. The [cursor adapter](../src/kfx/renderer/WgpuCursor.cpp)
uses GPU sprite scaling and immutable GPU snapshots for backup, keyed composition
and opaque restoration. The [shadow adapter](../src/kfx/renderer/software/WgpuShadow.h)
sends original RLE artwork and vertices; the GPU generates the silhouette and samples
its snapshot in both mode10 triangles. Mutable sprite artwork/remap/blend tables that
overlap the target decline before submission. Ordinary sprite glyphs reach these
wrappers; direct DBC glyph writes remain CPU. The [raw adapter](../src/kfx/renderer/software/WgpuRawImage.c)
submits source images for exact native scaling/letterbox and clipped slab tiling.
Full SDL clip clears run on GPU and preserve row padding; nonfull clips remain native.
The [lens adapter](../src/kfx/lense/WgpuLens.cpp) sends remap maps, mist texture/fade
rows and overlay artwork to indexed GPU kernels. Source/target overlaps execute in
native row-major order, including earlier-write visibility across different pitches.
Map preparation and once-only mist animation/palette lifecycle stay native. Mist
lightness 32–63, out-of-viewport map entries, asset/destination aliases and resource
limits retain fallback; full lens lifecycle validation remains open.
General lines, circle radii above 8,191, remaining image/effect transforms and the other
ledger gaps remain unfinished. Enabled adapters flush terrain before unsupported fallback.

At each CPU composition boundary, the bridge supplies the current CPU target as
an initial indexed image, executes owned GPU commands, reads the complete result
back, and commits it to the native target. This preserves interleaved CPU drawing,
cursor composition and existing screenshot/recording behavior. It also incurs
full-target transfers and waits. Resource versions are repacked/uploaded per
batch; the path has no measured performance benefit.

The shadow slice at `feat/wgpu-drawing` commit `3add2d680` preserves the native
partial clear and retained scratch bytes. Its generated mask feeds both triangles
before a counted 64 KiB compatibility mirror commit; subsequent shadows still
upload the prior scratch checkpoint. The cursor slice at `feat/wgpu-drawing`
commit `95c4ec603` keeps native scale/hotspot and begin/end-swap timing. Its SDL
wrappers synchronize the screen for backup/draw/restore and retain native
recovery checkpoints. The borrowed-context target methods queue GPU copies without those
transfers, but their owner must outlive the cursor and supply recovery history. These
seams do not establish complete frame residency, visible presentation or speedup.

The native cursor oracle links the actual SDL3 surface runtime, pointer handler and
C drawing ABI. `live-surface` exposes the offscreen drawing ABI on Linux for the
frame-replay workflow's software Vulkan tests; live window presentation remains
macOS-only. Missing GPU adapters fail the fixture rather than skip its comparisons.

The native destination changes only after successful execution/readback and any
enabled comparison. Failure reconstructs accepted original triangles with the
native gpoly rasterizer using immutable vertices and resources; the retained span
path uses its CPU interpreter. Reconstruction writes scratch storage and commits
only after the complete batch succeeds. Invalid replay shades or allocation
failure leave the native target unchanged and report a rejected batch. Failure
disables GPU consumption for the bridge lifetime. A declined 2D command runs its
legacy pixel loop once. Neither path reruns gameplay or picking wrappers. Resource/target changes and cache limits
also flush pending work. Complete GPU target ownership will require equivalent
same-frame recovery for every new command and persistent effect target.

### Drawing validation and counters

`KFX_WGPU_DRAW_VERIFY=1` compares bridge output with a separate CPU oracle before
committing it. This verifies indexed drawing; `KFX_WGPU_VERIFY=1` separately checks
acquired wgpu presentation surfaces. A screenshot of the synchronized native
image does not prove a window surface was acquired or displayed. Visible surface
validation for this drawing candidate is pending an unlocked display; the current
native evidence and its source/binary limits are in the coverage ledger.

`KFX_WGPU_DRAW_FAIL_INIT=1` injects drawing initialization failure;
`KFX_WGPU_DRAW_FAIL_AFTER=N` injects failure after N successful drawing batches.
`KFX_WGPU_DRAW_STATS` accepts an output JSON path for cumulative counts:

- `gpu_triangles`: committed original-vertex terrain; `cpu_triangles`: declined triangle calls; `replayed_triangles` / `rejected_triangles`: recovery outcomes.
- `gpu_spans` / `gpu_pixels`: retained span-path work only; `native_commands`: committed generic drawing commands, including primitives, sprites, raw images and clears; `gpu_sprite_commands`: committed sprite subset.
- `cpu_gpoly_spans`: declined span sink calls only; `cpu_replayed_spans`: span recovery.
- `verified_triangles` / `verified_batches`: successfully compared triangles/batches; `verification_cpu_spans` and `verification_cpu_commands`: explicitly enabled CPU oracle work.
- `bridge_initial_index_bytes`: native index bytes supplied for composition; `gpu_asset_upload_bytes`, `gpu_command_upload_bytes` and `gpu_api_readback_bytes`: actual widened GPU transfers, including four-byte triangle validation flags.

- `gpu_submits`, `gpu_dispatches`, `gpu_waits`, `gpu_wait_ns`, `gpu_buffers`, `gpu_buffer_bytes`: queue submissions, compute dispatches, blocking device polls with their measured host stall, and buffer allocations.
- `gpu_ordered_sprites`: the serial row-copy sprite subset of `gpu_sprite_commands`; `gpu_host_staged_asset_bytes`: host-side staged asset bytes the drawing context holds, a gauge rather than a total, and not GPU memory.
- No GPU execution time is collected. It was not attempted because the Metal adapter reports `TIMESTAMP_QUERY` but not `TIMESTAMP_QUERY_INSIDE_ENCODERS`, so a timestamp per submission is unavailable and the copy-only submissions carry no pass for `timestamp_writes`.

Zero declined spans is not a whole-renderer CPU-drawing count. These counters
measure work and transfers, not elapsed GPU time or whole-process memory. Keep
verification, fault injection, control and capture separate from performance runs.

The same snapshot feeds the profiler's measured-window counters
([performance baselines](performance-baselines.md#drawing-backend-counters)), so
the two never diverge; `KFX_WGPU_DRAW_STATS` remains lifetime-cumulative and
still writes its file every presented frame, which invalidates a timing run.
The current bridge's routine readbacks cannot simply be disabled for a benchmark;
removing them requires the remaining composition migration.

Asset-free native fixtures and explicit GPU tests reproduce bounded correctness:

```sh
cmake -S tests/gpoly -B out/gpoly-tests -DKFX_GPOLY_ASAN=ON
cmake --build out/gpoly-tests
ctest --test-dir out/gpoly-tests --output-on-failure
KFX_GPOLY_TRIANGLE_FIXTURE="$PWD/out/gpoly-tests/triangles.bin" \
  cargo test --locked --manifest-path tools/frame-replay/Cargo.toml \
  --test gpoly_gpu -- --ignored --test-threads=1
KFX_GPOLY_TRIANGLE_FIXTURE="$PWD/out/gpoly-tests/triangles.bin" \
  cargo test --locked --manifest-path tools/frame-replay/Cargo.toml \
  --test draw_triangles_gpu -- --ignored --test-threads=1
cmake -S tests/primitives -B out/primitive-tests -DKFX_PRIMITIVE_ASAN=ON
cmake --build out/primitive-tests
ctest --test-dir out/primitive-tests --output-on-failure
KFX_PRIMITIVE_FIXTURE="$PWD/out/primitive-tests/primitives.bin" \
  cargo test --locked --manifest-path tools/frame-replay/Cargo.toml \
  --lib gpu_actual_legacy_primitives -- --ignored --test-threads=1
```

These tests need SDL3 headers and a working GPU adapter; they use synthetic assets.
The [workflow](../.github/workflows/frame-replay.yml) also generates native fixtures
for required software-Vulkan checks. A local Metal pass does not establish remote
CI success, native gameplay coverage or exact-head delivery validation.
