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

`KFX_PRESENT_BACKEND=wgpu-offscreen` selects the same presenter with no swapchain: it
renders into a two-slot texture ring, takes its output size from the logical
framebuffer rather than the window, and hides the window it still needs for events.
Nothing reaches the screen, so this is a measurement mode and not a player feature; the
[profiling runner](performance-baselines.md#offscreen-measurement-mode) selects it with
`--offscreen`. One dependence on visibility survives: `SDL_HideWindow` drops focus, and
`FREEZE_GAME_ON_FOCUS_LOST` would then park the loop
([`game_loop.c`](../src/game_loop.c)), so that setting must be off. The runner forces it
off in its isolated configuration; a hand-run offscreen session must do the same.

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
the swapchain path retains no offscreen output texture, while the offscreen
measurement mode retains exactly two. There is no routine
readback or GPU completion wait in this presenter path. The partial drawing bridge
below does require synchronous readback. wgpu/driver submission and staging allocations
still occur; retained resources do not imply allocation-free presentation.

`presentation` covers cursor composition, acquisition, polling/reconfiguration,
input upload, drawing submission, present submission and cursor cleanup. On the GPU
drawing path the frame's single submission happens inside the nested `present_wait`,
so that scope now carries the whole frame's queue submission as well as the present
API. Compare total presentation rather than treating the two APIs' nested durations
as equivalent GPU work. [Performance baselines](performance-baselines.md) describe paired runs,
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
and SDL presentation stay default. After the single-stream restructure it holds
the 60 FPS cap at 1920×1080 with `draw` at 0.78–0.85 ms, and the remaining host
cost sits in presentation. The
[status section](product/rust-port-plan.md#status-2026-09-14-closing) records the
final measurement, the parity results and the work that follows.

The [indexed backend](../tools/frame-replay/src/draw.rs) stores one `u32` palette
index per pixel. A frame is one immutable command stream in the root's coordinates:
every record carries the origin of the view it was issued against, so views are
offset aliases of the root and a target change is not a boundary. One counting sort
over renderer-owned scratch bins the whole stream into 16×16 tiles, preserving
command order, and a retired frame leaves its record buffers behind for the next one; each GPU invocation owns one destination pixel and evaluates its
ordered commands. The stream is cut into raster passes only at genuinely serial work
— ordered sprites, the alias lens, minimap modes that read the target, snapshot
copies, creature shadows and terrain triangles — and each pass dispatches over the
box its own records reach, reading only that pass's slice of the shared tile index. Exact
integer operations preserve texture/shade lookup and destination-index palette
composition. The [drawing C ABI](../src/kfx/renderer/WgpuDraw.h) owns copied,
immutable resource versions and validates complete batches before submission.
`KFX_WGPU_DRAW_ABI_VERSION` tags `struct KfxWgpuDrawCommand`, not the library as a whole, so
it stays at 1 while that record is unchanged: the widened `KfxWgpuFrameCounters` and the added
`kfx_wgpu_draw_frame_status` are appended, and the crate is a `staticlib` linked into the same
binary, so no mismatched build can observe either.
Its direct GPU-target palette presentation API is tested offscreen, but the game
still presents the synchronized native framebuffer. Host validation rejects a batch
before any target write; a lookup only the kernel can find out of range skips its own
write and raises a frame flag instead, and the frame presents as drawn.

[WgpuTerrainBridge](../src/kfx/renderer/WgpuTerrainBridge.cpp) batches selected
terrain and generic commands between audited world-dispatch boundaries. Accepted textured gpoly calls
copy original unsorted vertices and return before CPU sorting, setup, clipping or
scan conversion. [GPU preparation](../tools/frame-replay/src/gpoly.rs) runs once per frame into a
renderer-owned row arena compressed to each triangle's covered rows, and every
triangle enters the raster stream as its own record. No prepared rows are read back
to construct draw commands. A destination pixel consumes only the triangles whose
conservative screen-space box reaches its 16x16 tile, in submission order: the box is
the clamped vertex range, which the setup kernel provably never writes outside, so
the raster visits exactly the covered span and the per-pixel shade check replaces the
separate span-validation pass. Terrain therefore no longer cuts the raster into
segments, and it composes with the other families in one pass. Pending terrain spans, terrain
triangles and generic commands share one ordered record list, so the span and
triangle paths no longer flush each other. A record carries the view it was issued
against, so a target or view change opens a run inside that list instead of closing
it: `bridge_target_flushes` is structurally zero and `bridge_target_runs` counts the
changes that used to flush. Inside a resident frame a batch closes
only at a kind the GPU packer accepts alone (transition, minimap, lens effect),
an ordered sprite, a snapshot, readback or
barrier, a rasterizer change, texture or fade cache eviction, 32,768 pending
spans, the 4,096 command and triangle cap, or verification mode. A
world-bucket boundary and an emitter head only stop terrain from continuing;
outside a resident lease every command still flushes, so a command accepted
inside a frame reaches the native target at the next barrier, readback or frame
end rather than before it returns. When a pending run cannot be submitted the
bridge rasterizes it on the CPU if it holds only terrain, and otherwise drops it
and marks the frame invalid so presentation waits for a full CPU redraw; it never
drops a run and leaves the frame valid. Pixel, box, HV-line and circle hooks in
[bflib_vidraw.c](../src/kfx/renderer/software/bflib_vidraw.c) use the same bridge.
Circles execute their integer coverage recurrence on GPU. The
[sprite adapter](../src/kfx/renderer/software/WgpuSprite.c) decodes RLE into immutable
index/coverage assets and copies native scale ranges and tables; GPU source selection
performs supported scaling, flips, remap and blending. Scaled solid horizontal flips
with duplicated rows use ordered GPU run copies, preserving native extra-left pixels,
four-byte copy grouping and target alignment. The [cursor adapter](../src/kfx/renderer/WgpuCursor.cpp)
uses GPU sprite scaling and immutable GPU snapshots for backup, keyed composition
and opaque restoration. The [shadow adapter](../src/kfx/renderer/software/WgpuShadow.h)
sends original RLE artwork and vertices; the GPU generates the silhouette into a
resident scratch buffer, stamps it into a mask slot and samples that slot in both
mode10 triangles. Mutable sprite artwork/remap/blend tables that
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

The shadow slice preserves the native partial clear and the retained scratch bytes,
but the chain now lives on the GPU: the mask reads and writes a resident 256x256
scratch buffer, stamps a mask slot and feeds both triangles from it. No prior
scratch is uploaded, no mask is read back and the CPU scratch is not mirrored. The cursor slice at `feat/wgpu-drawing`
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
enabled comparison. A queued frame writes the canonical target as it is recorded;
there is no per-flush scratch copy and no rollback. An out-of-range GPU lookup skips
its own write, the frame presents as drawn, and the flag reaches the bridge through
`kfx_wgpu_draw_frame_status` at the head of a later frame — one or two, because the word is
copied and cleared once per frame in the encoder `frame_submit` closes, so the eight-slot
ring now covers eight frames rather than eight flushes. A ring slot whose map has not
completed still defers the publish to the next frame. A deferral counts
`frame_status_stalls`; it never drops a flag, because the status word is only cleared in the
encoder that copies it out. The bridge then counts the frame and takes the existing full
redraw. **Retained GPU snapshots are the exception to that recovery.** Neither
`kfx_wgpu_draw_frame_abort` nor `WgpuTerrainBridge::FullRedraw` invalidates
`target_snapshots` — precisely what the deleted transactional rollback used to restore — so a
snapshot taken from a flagged frame keeps those pixels until its owner releases it.
`capture_map_fade_buffer` in [engine_redraw.c](../src/engine_redraw.c) is the one reachable
case: it snapshots the live screen and holds it across a whole map fade, so a flagged frame
can outlive the two-frame window there. Transitions release their snapshots in the same call,
and the minimap background dictionary has no C emitter for the mode that retains one.
Occurrences are measured at zero. Failure reconstructs
accepted original triangles with the native gpoly rasterizer using immutable vertices
and resources; the retained span path uses its CPU interpreter. Reconstruction writes
scratch storage and commits only after the complete batch succeeds. Invalid replay
shades or allocation failure leave the native target unchanged and report a rejected
batch. Failure
disables GPU consumption for the bridge lifetime. A declined 2D command runs its
legacy pixel loop once. Neither path reruns gameplay or picking wrappers. Resource/target changes and cache limits
also flush pending work. Complete GPU target ownership will require equivalent
same-frame recovery for every new command and persistent effect target.

### Drawing validation and counters

Production frames never block: the aggregate validation wait and the per-batch
validation readbacks are gone, and the status flag reaches the CPU through a mapped
ring one or two frames later. `KFX_WGPU_DRAW_VERIFY=1` compares bridge output with a
separate CPU oracle before committing it, and keeps its blocking reads. Its CPU oracle still
rejects an out-of-range shade outright where the kernels flag and skip it, so the two
disagree by construction; that batch is counted as `verification_flagged_shades` and left
uncompared, with the GPU result authoritative, instead of failing the bridge. Only
`verification_flagged_shades` and the frame flag report such a batch — it is deliberately not
a verification failure. For creature shadows the oracle runs on the game's own scratch and its
mask is compared against a blocking read of the resident chain; when the two priors
already differ the shadow is counted as `shadow_prior_divergence` instead of compared, and
the CPU scratch is re-seeded from the resident chain so the next shadows are verified again.
This verifies indexed drawing; `KFX_WGPU_VERIFY=1` separately checks acquired wgpu
presentation surfaces. A screenshot of the synchronized native
image does not prove a window surface was acquired or displayed. Visible surface
validation for this drawing candidate is pending an unlocked display; the current
native evidence and its source/binary limits are in the coverage ledger.

`KFX_WGPU_DRAW_FAIL_INIT=1` injects drawing initialization failure;
`KFX_WGPU_DRAW_FAIL_AFTER=N` injects failure after N successful drawing batches.
`KFX_WGPU_DRAW_STATS` accepts an output JSON path for cumulative counts:

- `gpu_triangles`: committed original-vertex terrain; `cpu_triangles`: declined triangle calls; `replayed_triangles` / `rejected_triangles`: recovery outcomes.
- `gpu_spans` / `gpu_pixels`: retained span-path work only; `native_commands`: committed generic drawing commands, including primitives, sprites, raw images and clears; `gpu_sprite_commands`: committed sprite subset.
- `cpu_gpoly_spans`: declined span sink calls only; `cpu_replayed_spans`: span recovery.
- `verified_triangles` / `verified_batches`: successfully compared triangles/batches; `verification_cpu_spans` and `verification_cpu_commands`: explicitly enabled CPU oracle work; `verification_flagged_shades`: batches the CPU oracle could not reproduce because a kernel flagged and skipped an out-of-range shade, counted rather than compared.
- `bridge_initial_index_bytes`: native index bytes supplied for composition; `gpu_asset_upload_bytes`, `gpu_command_upload_bytes` and `gpu_api_readback_bytes`: actual widened GPU transfers.

- `gpu_submits`, `gpu_dispatches`, `gpu_waits`, `gpu_wait_ns`, `gpu_buffers`, `gpu_buffer_bytes`: queue submissions, compute dispatches, blocking device polls with their measured host stall, and buffer allocations. **A presented frame is one command buffer.** Every pass of the frame — the terrain prepare, each raster segment, each shadow mask and its triangles, the ordered-sprite layers, the lens and minimap passes, the cursor backup, composition and restore and the palette render pass — records into a single encoder opened at the frame's first record, and `kfx_wgpu_present` finishes and submits it. A flush is no longer a submission boundary: `frame_flush` replays what the frame has queued into that open encoder, which is why `frame_checkpoints` is structurally zero. Serial dependencies stay pass boundaries inside the encoder, where wgpu inserts the usage-transition barriers, so the per-pixel value sequence is the one the per-batch path produced.
- `gpu_ordered_sprites`: the serial row-copy sprite subset of `gpu_sprite_commands`; `gpu_host_staged_asset_bytes`: host-side staged asset bytes the drawing context holds, a gauge rather than a total, and not GPU memory.
- `ordered_sprite_layers` and `ordered_sprite_passes`: layers of mutually disjoint ordered sprites and the compute passes serving them, one pass of *M* workgroups per layer. They are equal by construction, so a divergence is a bug. `ordered_sprite_layers` below `gpu_ordered_sprites` is the only way the layering pays: measured on a busy 640x480 pair it is 2.68 against 2.69 ordered sprites, because consecutive ordered sprites are rare in the stream — a raster record between two of them orders them and ends the run. Each sprite's bound is its own scaling ranges, not its clip rectangle, which is always the whole drawing window.
- `arena_evictions`, `arena_overflows`, `arena_bytes_uploaded`: persistent asset arena LRU reclaims, exhausted allocations that reject a batch, and bytes actually written into the arena. `arena_bytes_resident` is a gauge: the arena extent suballocated so far in expanded bytes — one `u32` per source byte, free-listed slots and power-of-two class padding included — so it bounds the live working set rather than tracking it exactly. Assets the arena does not own yet — the per-shadow `submit_target_triangles` tables — stay in `gpu_asset_upload_bytes` without appearing in `arena_bytes_uploaded`.
- `bridge_solo_batches`: the `gpu_batches` subset a single command occupied alone because its kind cannot share a submission. Shadows left this set: the shadow route still takes one command, but it keeps its place in the ordered record list instead of flushing around itself.
- `gpu_shadow_commands`: committed creature shadows. `shadow_scratch_upload_bytes` and `shadow_scratch_copy_bytes` are zero because the mask chain is GPU resident; `shadow_scratch_readback_bytes` is zero unless `KFX_WGPU_DRAW_VERIFY` is set, which adds one blocking 256 KiB scratch read per shadow.
- `shadow_prior_divergence`: verification only. Counts *events*, not every shadow after the first: a shadow whose resident prior scratch no longer matched the `big_scratch` bytes the software path would have used is skipped for the mask and pixel comparison and kept out of `verified_batches`, and the CPU scratch is then re-seeded from the resident prior so verification resumes on the next shadow. `shadow_scratch_copy_bytes` counts those re-seeds and is zero without them. It measures how often the resident chain and the legacy shared scratch disagree; it is not a failure count.
- `rejected_commands` / `rejected_spans`: pending generic commands and terrain spans the target never received because the run was dropped without a CPU replay; each such drop invalidates the frame.
- `frame_checkpoints`, `frame_gpu_checkpoint_copy_bytes`, `frame_validation_waits` and `frame_validation_bytes`: flushes that had to cut the frame's submission, and what a flush used to cost. All four are structurally zero: a flush records the batches straight into the root and replays into the open encoder, and the status word is published once per frame from `frame_submit` instead of once per flush.
- `frame_flagged_invalid`, `frame_status_reads` and `frame_status_stalls`: frames a kernel flagged as having an out-of-range lookup, completed status ring reads, and publishes skipped because every ring slot was still mapped. A stall only defers the flag to the next publish; it never loses it.
- `tile_allocations` counts growths of the persistent binning scratch and is zero after warm-up; `tile_entries` is the per-frame size of the tile index the raster passes share. The ordered-sprite preflight builds an index it never uploads and is excluded from `tile_entries`.
- `tile_entries_<kind>` splits `tile_entries` by record kind and sums to it. A record is binned by the destination box its validator proves it cannot write outside, not by its clip: the union of a sprite's per-call scaling ranges, a triangle's vertex bounding box, a huge bitmap's row and run extents, a glyph's scaled rectangle plus its shadow offset, and for an ordered sprite the same `write_rect` the layering uses. `RAW_IMAGE` keeps the whole target, because it writes index 0 outside its destination rectangle. The clip stays the pixel-level guard, so the sampler arithmetic is unchanged.
- `terrain_tile_entries` is the terrain share of `tile_entries`. Terrain inner-loop iterations are exactly 256 times it, because a 16x16 tile's 256 pixels each iterate that tile's list once. `prepared_row_words` is the compressed prepared-row arena the frame used, eight words per covered row, and `prepared_row_allocations` counts its growths, which reach zero after warm-up.
- `bridge_target_flushes`: pending work flushed because the target was not a view of the frame root at all. Target and view changes inside the frame root no longer flush; `bridge_target_runs` counts those.
- `gpu_batches` counts bridge submission routes, not GPU submissions. Inside a queued frame a route is an `enqueue_commands` call that may still merge with its neighbour, so a lower count means fewer FFI crossings and fewer command copies, not fewer dispatches; `gpu_submits` and `gpu_dispatches` measure those.
- `gpu_raster_ns`, `gpu_terrain_prepare_ns`, `gpu_shadow_mask_ns`, `gpu_target_trig_ns`, `gpu_ordered_sprite_ns`, `gpu_minimap_ns`, `gpu_lens_ns` and `gpu_present_ns`: GPU execution time per pass kind, collected only when `KFX_WGPU_GPU_TIMING` is `1` or `2` and the adapter supports `TIMESTAMP_QUERY`. The pass descriptors carry `timestamp_writes`, which needs that feature alone; `TIMESTAMP_QUERY_INSIDE_ENCODERS`, which this Metal adapter lacks, would only be needed to stamp outside a pass. Results resolve into an eight-slot ring and are read through the same non-blocking map the status ring uses, so timing never adds a wait. Copy-only submissions carry no pass and are untimed by construction. `gpu_timed_passes` counts the stamped passes; `gpu_untimed_passes` counts submissions or passes that found no free ring slot, so a nonzero value means the per-pass totals under-report. These are GPU durations and are not comparable with the host wall-clock `draw` and `presentation` scopes, which include queue submission, surface acquisition and CPU work.
- **A pass window is not exclusive.** It runs from that pass's own begin stamp to its own end stamp, so it includes whatever the pass waited through: it attributes cost rather than measuring it, and the sum of the windows decomposes nothing. `gpu_pass_union_ns` is the union of the frame's timed pass intervals, closed once per frame, so windows that overlap in time count once; it is an upper bound on the frame's GPU occupancy. Measured, it equals the window sum on this Metal adapter — the windows are disjoint and the stall sits *inside* each one — so it bounds the total without separating work from waiting. Only `KFX_WGPU_GPU_TIMING=2` (`scripts/profile-game.py --serial-gpu-timing`) does that: it drains the queue after every timed submission, which leaves each window holding its own pass alone, and it serialises the frame to do so, so that run is a diagnostic and not a throughput baseline. On a busy 1080p pair it reports 3.549 ms of GPU work against an 8.006 ms window sum.

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
