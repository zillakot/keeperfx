---
type: architecture
description: Target architecture for the optional wgpu drawing path — one command stream, one asset arena, one encoder and one submit per frame — with acceptance counters, ABI changes, fixtures and a PR-sized migration sequence.
---

# Single-stream wgpu renderer

The wgpu drawing backend currently issues one full-target compute dispatch and one `queue.submit` per
batch, re-uploads every asset per batch, and blocks on GPU completion several times per frame. This
design replaces that with **one immutable per-frame command stream, one persistent asset arena, one
tile index, one command encoder and one `queue.submit`**, keeping the arithmetic of every existing
pixel kernel byte-identical.

Context: [project overview](project-overview.md) for the engine and the role of Rust,
[live Rust presentation](../live-rust-presentation.md) for the Metal surface path and its fallback, and
[rust port plan](../product/rust-port-plan.md#graphics-and-performance-track) for the delivery track.

## Goals and non-goals

1. Exact indexed-pixel parity with the software reference for every drawing family, preserved at every
   migration step by the fixtures that already prove it.
2. One `queue.submit` per frame in the production path; zero blocking `device.poll(PollType::Wait)`.
3. Per-frame CPU work independent of resolution and linear in command count; GPU work linear in pixels,
   with per-pixel command count bounded by tile occupancy, not by frame command count.
4. Immutable game data (sprite artwork, terrain tiles, fade and ghost tables, remaps) uploaded once.
5. A structure that survives the removal of all C/C++ drawing: the C side becomes a command emitter and
   every CPU pixel path is deletable without redesigning the renderer.

Non-goals: changing what any pixel computes (only *addressing* changes); multithreaded command
production, since [`WgpuDraw.h`](../../src/kfx/renderer/WgpuDraw.h) already requires serialization;
replacing SDL presentation, which stays the default and the fallback; retiring `KFX_WGPU_VERIFY`, the
offline fixtures or the CPU oracles, which stay off the production path; non-macOS live presentation,
where the `live-surface` offscreen ABI remains the Linux CI path.

## Acceptance criteria, per frame

Busy scene, wgpu drawing, VSync off, 60 FPS cap, 20 turns/s. "Now" columns are process-lifetime counters
from the 2026-09-14 diagnostics divided by their frame counts (345 frames at 640x480, 257 at 1920x1080).
Rows marked *derived* are arithmetic on those or counts read off the code; no counter exists for them
yet, and blocking wait *nanoseconds* are likewise unmeasured and must also reach zero, which is why
instrumentation is step 1. Pass count alone does not settle the dispatch budget: total per-pixel
command-list iterations must stay within 5% of the single-pass count.

| Criterion | Now 640x480 | Now 1920x1080 | Target |
| --- | ---: | ---: | --- |
| `queue.submit` calls | ~125 *derived*; 83.9 → 45.2 → 37.2 → **1.00 measured** | ~125 *derived*; 76.1 → 39.1 → 38.3 → **1.00 measured** | **1**, hard cap 2 — **met** |
| Blocking `device.poll(Wait)` | 25.7 → **0 measured** | 18.9 → **0 measured** | **0** — met |
| Frame checkpoints | 11.8 → 1.0 → **0 measured** | 9.6 → 1.0 → **0 measured** | **0** — met |
| GPU→GPU checkpoint copy bytes | 29.01 MB | 159.05 MB | **0** |
| Asset upload bytes | 29.44 MB | 25.60 MB | **≤ 256 KB** steady state |
| Command + tile-index upload bytes | 0.90 MB | 5.11 MB | ≤ 0.5 MB / ≤ 1.5 MB |
| C-side resource copy bytes | 1.95 MB | 1.66 MB | **≤ 64 KB** |
| Rust requested bytes per presentation | 98.2 MB | 120.1 MB | **≤ 512 KB** |
| `create_buffer`, `create_buffer_init`, `create_bind_group` calls | ~500 each *derived*; **87.3 measured**, unmoved by PR 13 | ~500 each *derived*; **90.4 measured**, unmoved by PR 13 | **≤ 8** each; needs the byte-packed arena and persistent uniforms |
| Full-target compute dispatches | ~86 *derived*; total dispatches 128.7 → **59.1 measured** | ~86 *derived*; total dispatches 113.1 → **48.6 measured** | **≤ 4** typical, hard cap 16 |
| Terrain inner-loop iterations | 308 M *derived* → **1.85 M measured** | 2,065 M *derived* → **7.85 M measured** | **≤ 10 M / ≤ 25 M** — met |
| Terrain GPU time per frame | 5.24 ms → **0.04 ms** plus its share of the raster | 26.40 ms → **0.04 ms** plus its share of the raster | no separate pass |
| Prepared terrain row arena | 12.6 MB → **0.68 MB** | 28.3 MB → **1.74 MB** | ≤ 4 MB |
| Shadow mask readbacks / bytes | 14.0, 3.67 MB | 9.5, 2.49 MB | **0** |
| Exact indexed parity | 76,859 verified batches, 0 failures | — | unchanged, plus the frame-order fixture |

Software reference, busy scene, 2026-09-14: 640x480 draw 1.218 ms mean and presentation 0.576 ms;
1920x1080 draw 3.318 ms mean, 3.638 ms p95 and presentation 0.560 ms; 60.00 FPS and 20.00 turns/s at
both. GPU drawing today: 25.863 / 9.350 ms at 640x480 (28.23 FPS) and 70.442 / 54.214 ms at 1920x1080
(8.00 FPS, simulation degraded to 8.00 turns/s).

| Configuration | Criterion |
| --- | --- |
| 640x480 logical, 60 FPS cap | Observed FPS ≥ 59.9 and 20.00 turns/s. `PerfDraw` mean ≤ 1.5 ms, p95 ≤ 3.0 ms. `PerfPresentation` mean ≤ 0.6 ms. Process CPU per presentation ≤ 5.0 ms. |
| 1920x1080 logical, 60 FPS cap | Observed FPS ≥ 59.9 and 20.00 turns/s sustained. `PerfDraw` mean ≤ 9 ms, p95 ≤ 12 ms. `PerfPresentation` mean ≤ 1.0 ms. GPU pass total by `TIMESTAMP_QUERY` ≤ 8 ms. Process CPU per presentation ≤ 8 ms. |
| 1920x1080 logical, uncapped | Frame interval mean ≤ 16.7 ms, p95 ≤ 20.0 ms at 20.00 turns/s. **Measured `e00fcd322`, busy, wgpu drawing: 85.258 / 100.505 ms — not met**, and at 11.73 turns/s that run does not measure a ceiling at all. Software drawing on the same presenter does meet the row: 13.353 / 18.281 ms at 19.99 turns/s. |

This row is now measurable: [`profile-game.py --uncapped`](../performance-baselines.md#uncapped-measurement-mode)
clears `fps_limit_current`, leaving the simulation at 20 turns/s and VSync off. The
full uncapped matrix, including the per-pass GPU totals behind the 1080p figure and
the 640x480 rows (76.96 FPS busy, 93.91 quiet), is recorded there. The uncapped
ceiling is GPU execution: `PerfDraw` falls to ~1.5 ms because the CPU only encodes,
and the host blocks in `PerfPresentation` for the frame's passes, of which raster is
57.644 ms of a 91.649 ms 1080p total. That raster figure predates step 9, which cuts the
capped 1080p raster window 46.124 → 2.859 ms, so the row is stale rather than wrong and
needs its own uncapped re-run; nothing here claims what it will read.

Estimated 1080p budget: CPU 0.6–1.2 ms to build, bin and encode; ≤ 1 MB of uploads; one prepare pass, ~10
shadow mask passes over 256x256, 1–3 raster dispatches at 2–5 ms, one palette pass at ≤ 0.4 ms.

## Frame data model

### Per-frame command stream

One append-only `Vec<StreamCommand>` per frame, uploaded once and never mutated after upload, replacing
`QueuedFrame::batches` in [`draw_frame.rs`](../../tools/frame-replay/src/draw_frame.rs) — today a
`Vec<Batch>` of alternating `Batch::Commands` and `Batch::Triangles` chunks whose boundaries come from
`enqueue_commands`' `compatible` closure and from whoever calls `checkpoint_target`. The record is 128
bytes (32 words), packed for a WGSL `array<Command>` with `vec4` alignment; the current `Command` in
[`draw.rs`](../../tools/frame-replay/src/draw.rs) is 112 bytes, and the four extra words carry what the
per-batch uniform did.

| Words | Field | Notes |
| ---: | --- | --- |
| 0–3 | `kind`, `blend`, `flags`, `colour` | `kind` gains `TERRAIN_TRI`, `LENS`, `MINIMAP`, `SHADOW_TRI`; `flags` bit 0 aux binding, bit 1 serial-pass marker, bit 2 full-view bounds |
| 4–11 | `bounds`, `clip` | **root-space**, half-open; `bounds` is the destination box the record's own validated data proves it cannot write outside, not the emitter's target rectangle; `clip` already intersected with the view rectangle |
| 12–15 | `source_offset`, `table_offset`, `source_pitch`, `aux_offset` | word offsets into the asset arena; `aux_offset` is the prepared-row base, shadow mask slot or snapshot base |
| 16–24 | `source_x/y/width/height`, `start_low/high`, `step_low/high`, `transparent` | unchanged |
| 25–27 | `origin_x`, `origin_y`, `view_width` | root-space origin of the view the command was issued against; 28–31 are reserved zero |

Words 25–27 are what lets one dispatch serve every view. Every view from `create_target_view` inherits
`pitch: parent.pitch` and only adds `offset: parent.offset + y*parent.pitch + x`, so all views of a root
share its pitch and a view-local pixel is exactly `root_pixel − origin`. Samplers in
[`draw.wgsl`](../../tools/frame-replay/src/draw.wgsl) take `pixel − origin` instead of `id.xy`,
`pixel_address` collapses to `root_offset + y*root_pitch + x`, and `DrawParameters.pitch/offset` go. A
busy frame is ~1,100 commands plus ~1,000 triangles, so ~270 KB; budget a persistent 65,536-record ring
(8 MB) and refuse larger frames through the existing `MAX_COMMANDS` error path.

### Family mapping

Sprites, glyphs and text ([`software/WgpuSprite.c`](../../src/kfx/renderer/software/WgpuSprite.c)),
primitives ([`software/bflib_vidraw.c`](../../src/kfx/renderer/software/bflib_vidraw.c)), raw and tiled
images, backgrounds, zoom, movies, map views, general triangles in all 27 modes
([`software/WgpuTrig.h`](../../src/kfx/renderer/software/WgpuTrig.h)) and terrain spans keep their
existing kinds and run in the main raster. The families whose representation or pass changes:

| Family | Producer | Stream representation | Pass |
| --- | --- | --- | --- |
| Terrain triangles | `WgpuTerrainBridge::DrawTriangle` via `kfx_gpoly_triangle_sink` | one `TERRAIN_TRI` per triangle: `source_offset`/`table_offset` its texture and fade arena offsets, `aux_offset` (word 15) its prepared-row base, bounds the clamped vertex box; words 16-24 unused | prepare, into the first raster segment's encoder, + raster |
| Creature shadows | [`software/WgpuShadow.h`](../../src/kfx/renderer/software/WgpuShadow.h) | one mask record plus two `TRIG` records whose `aux_offset` is the shadow's mask slot | mask chain (head) + raster |
| Ordered sprites (scaled solid horizontal flips) | `WgpuSprite.c`, `source_x` bit 3 | `SPRITE` with the serial flag | serial layer |
| Minimap | [`frontmenu_ingame_map.c`](../../src/frontmenu_ingame_map.c) | modes 1–3 fold into `MINIMAP`; mode 0 reads the stored background from the arena; mode 4 records a copy | mode 4 = copy boundary, rest raster |
| Built-in lenses | [`kfx/lense/WgpuLens.cpp`](../../src/kfx/lense/WgpuLens.cpp) | non-alias folds into `LENS`; alias keeps its single-workgroup pass | raster / serial pass |
| Transitions and smoothing | [`software/WgpuTransition.c`](../../src/kfx/renderer/software/WgpuTransition.c) | `TRANSITION` against a snapshot arena offset | copy boundary + raster |
| Cursor backup / compose / restore | [`WgpuCursor.cpp`](../../src/kfx/renderer/WgpuCursor.cpp) via `bflib_mspointer.cpp` | backup = recorded copy; compose and restore = `IMAGE` records against snapshot offsets | tail passes |

### Persistent asset arena

Today every asset is expanded byte→`u32` into a fresh `Vec<u32>` per batch, re-serialised and uploaded
through `create_buffer_init`; nothing is cached across batches or frames. That is the 25–29 MB/frame of
asset upload, and it does not shrink with resolution. Replacement: one persistent `wgpu::Buffer` per
drawing context, with:

- **Handle table** `HashMap<u64, Residency { offset_words, len_words, generation, last_used_frame }>`;
  `create_resource` writes into the arena with `queue.write_buffer` and keeps CPU bytes only for
  `KFX_WGPU_VERIFY` or a CPU oracle. Power-of-two size classes with free lists, assets aligned to 4 words
  so byte packing later does not re-lay out the arena.
- **Generations.** A handle is `(id, generation)`; the C side interns by
  `(pointer, length, width, height, pitch, generation)` instead of the full `memcmp` in
  `WgpuTerrainBridge::ResourceFor`, which today rescans up to 64 × 8 KiB texture entries and 4 × 16 KiB
  fade entries per triangle. Generations bump at the existing invalidation points: `FullRedraw`, a
  `ReadBarrier` hit on the asset range, palette and level changes.
- **LRU residency** over `last_used_frame`, never evicting anything the frame under construction
  references. Capacity is `min(max_storage_buffer_binding_size, max_buffer_size)` — 128 MiB at wgpu
  defaults, since every `request_device` passes `&Default::default()` — so 32 MiB of real asset bytes
  under phase-1 `u32` expansion.
- **A ring for per-frame mutables** (movie frames, the minimap world descriptor, lens overlay maps), the
  only recurring upload, and **snapshot regions**, so `submit_target_images`' per-batch sampling
  allocation and `submit_target_triangles`' two 256 KB copies per shadow become copies into stable regions.

### Tile binning

The model is unchanged: 16x16 tiles, a per-tile ascending list of stream indices, a kernel that iterates
the list in order. What changes:

- **Once per frame, not once per batch.** `bin_commands` allocates
  `vec![Vec::new(); ceil(W/16)*ceil(H/16)]` on every call — 1,200 `Vec`s per batch at 640x480, 8,160 at
  1080p, times ~64 batches — and is a visible allocation hot spot in the 1080p stack sample. Replacement:
  a counting sort over two renderer-owned `Vec<u32>` cleared with `fill(0)`, zero allocation per frame
  after warm-up, estimated ~0.1 ms at 1080p.
- **Full-view commands dominate the index.** `CLEAR`, `RAW_IMAGE`, `LENS` and full-screen `BITMAP` cover
  all 8,160 tiles at 1080p, which is why command upload bytes scale 5.71× per frame with pixel count for
  0.94× the commands. Held in reserve: a *universal* list the kernel merges with the tile list by
  ascending stream index — two cursors, exactly order-preserving.
- **Terrain triangles are binned too.** They are not today: `draw_triangles.wgsl` `render` loops every
  triangle for every pixel, so its cost is `pixels × triangles` — 308 M inner iterations per frame at
  640x480, 2,065 M at 1080p — however the triangles are grouped into batches. Merging terrain batches is
  neutral on that term; only binning removes it. Bin by the CPU-side vertex bounding box: the prepared
  row's own `bounds` still rejects non-covered pixels, so a conservative box is exact.

### Resident target and views

Unchanged: `Target { width, height, indices, root, pitch, offset }`, one `u32` index per pixel, views as
offset/pitch aliases sharing the root buffer.
[`WgpuTerrainBridge.cpp`](../../src/kfx/renderer/WgpuTerrainBridge.cpp) `FrameView` derives a view
rectangle from pointer arithmetic against `m_frame_target.pixels` and `SubmissionTarget` caches the
handle; keep that while C drawing exists, and pass view rectangles explicitly once it is retired. Other
persistent buffers: command stream ring (8 MB), tile index (~1 MB at 1080p), prepared terrain rows
(~2 MB at 1080p once compressed to covered rows, at the measured triangle population), shadow scratch (256 KB), 16 mask slots of 256 KB,
and a 4 × 256 B status and staging ring.

## Ordering semantics and pass boundaries

For every pixel *p*, the sequence of values written to *p* over a frame must be identical to the current
per-batch sequence. In the single-stream model that follows from a stronger statement:

> **Merge theorem.** If every command in a raster pass computes its contribution to *p* from *p*'s own
> current value and immutable inputs only, then one pass over the concatenated, globally-ordered tile
> lists produces exactly the same per-pixel sequence as any partition of that stream into
> consecutively-submitted passes. The proof is the per-pixel loop: the pixel's value is a left fold over
> its tile list, and folding a concatenation equals folding the parts in order.

Splitting the raster is therefore a performance question only.

| Case violating the premise | Why | Becomes |
| --- | --- | --- |
| Ordered sprite row copies | one invocation reads and writes other pixels of the same target, serially | serial layer pass |
| Alias lens | `source()` reads the target it writes, in native row-major order | its own single-workgroup pass |
| Snapshot creation, cursor backup, minimap mode 4 | read a finished target into an immutable copy | recorded `copy_buffer_to_buffer` |
| Shadow scratch accumulation | shadow *i*'s mask is a function of shadow *i−1*'s scratch | hoisted mask chain |
| CPU readback (`readback`, `Materialize`, screenshots, verify) | maps GPU memory on the host | its own submit, off the production path |

wgpu inserts the necessary barriers between compute passes recorded in the same `CommandEncoder` and
between a copy and a following pass, so every case except CPU readback is an *intra-encoder* boundary:

```
P0     gpoly prepare                    1 dispatch, O(triangles)
P1..PN shadow mask chain                N ≈ 10, 32x32 workgroups over 256x256
R0     raster, stream range [0, i0)     full target, tile-binned
S0     serial op at i0                  ordered-sprite layer / alias lens / copy
R1..Rk raster, remaining ranges
copy   cursor backup region → snapshot arena slot, then C cursor compose
PR     palette present render pass into the acquired surface view, then RS cursor restore
finish() → queue.submit([cb]) → queue.present(frame)
```

Each raster pass iterates a *sub-range* of each tile list, so total per-pixel iterations across R0..Rk
equal the single-pass count exactly. Each split costs one extra load and store of the destination per
pixel — 16.6 MB at 1080p, ~0.08 ms at 200 GB/s — so ten splits cost ~0.8 ms, against the 1,423 MB of the
same traffic that today's ~86 full-target passes move.

**Ordered sprites (implemented).** `sprite_ordered` is `@workgroup_size(1)`. The record index is a
workgroup-indexed parameter, so one dispatch of *M* workgroups handles *M* sprites, and consecutive
sprites are assigned to **layers** by greedy rectangle disjointness computed on the CPU. A naive "one
dispatch of N workgroups" would race, since `sprite_ordered` writes arbitrary pixels; a sprite may join a
layer only if no record between it and the layer's other members overlaps it, which a contiguous run in
frame order gives for free. Worst case degenerates to one pass per sprite.

The rectangle is **not** the clip rectangle: the emitter sets clip from the drawing window, so every
ordered sprite in a real frame carries the whole play area. It is the sprite's own scaling ranges, which
`validate` proves contiguous and ascending and `validate_target` proves every run lies inside, grown one
column to the left because a row copy spans `[leftmost - 1, rightmost]` exactly as the native
right-to-left kernel does. Where that column crosses a row start it lands on the tail of the previous
row, and the rectangle widens to the whole row band one row higher.

**Shadow residency (implemented, without hoisting).** The chain is `mask_i = f(scratch_{i-1}, artwork_i)`,
then `scratch_i = mask_i`, then two `TRIG` commands sample `mask_i`. Because the mask never depends on the
frame target, the chain lives in a persistent 256x256 scratch buffer and each mask is stamped into one of
**two** resident slots. Mask *i* is recorded immediately ahead of `TRIG` *i* rather than hoisted to the
head of the frame. The invariant that makes slot reuse safe is **order**, not the slot count: mask *i*
and the triangles reading slot *i* are adjacent *passes of one encoder*, which wgpu separates with its
own usage-transition barriers exactly as the queue separates two submissions. `SLOTS = 2` therefore
survives one submit per frame, and the 192-case chain is pinned against a per-submit replay of the same
chain. Hoisting would remove only pass-setup overhead and stays out of scope.

**Cursor.** Backup, compose, palette pass and restore are recorded into the frame's encoder in the order
`bflib_mspointer.cpp` already imposes through `LbMouseOnBeginSwap`/`LbMouseOnEndSwap`, so the semantics
survive. The checkpoint inside `PerfPresentation` was `ResidentTarget`'s own flush; a flush is no longer
a submission boundary, so it costs a replay into the open encoder and `frame_checkpoints` is zero.

## Submission and validation

**One submit per frame (implemented).** One `CommandEncoder` is opened at the frame's first record and
`kfx_wgpu_present` finishes and submits it; `queue.present(frame)` is not a submit. The encoder is built
first and the surface acquired last, because holding the swapchain image across the whole CPU frame
serialises against the compositor and a `CommandEncoder` accepts passes until `finish()`. If acquisition
returns `Timeout`/`Occluded` the palette pass is omitted, the encoder is still finished and submitted so
the cursor background and the target stay consistent, and the frame is reported as skipped as today —
`prepare_present` returns 0 without submitting and the caller still reaches `kfx_wgpu_present`. **The
submit cannot live in `prepare_present`**: `LbMouseOnEndSwap` issues the cursor restore after it returns
and that restore must be inside the encoder, so `kfx_wgpu_present` owns the submit.

`queue.write_buffer` is staged and applied at the **start of the next submit, before every command in
it**. Under one submit per frame every staged write of the frame therefore lands before every pass of
the frame, which is safe only if no byte range is written twice in a frame with different contents.
Four sites had to change for that to hold:

- The stream command and tile buffers are a **ring bump-allocated inside the open encoder**: a frame
  that flushes more than once appends a second region rather than rewriting the first, and each raster
  pass binds its own sub-range. The ring is sized between frames from the previous frame's demand; a
  region it cannot hold becomes its own `create_buffer_init` buffer.
- **Arena pinning is scoped to the encoder**, not to the batch. `begin_batch` does not recycle transient
  regions while an encoder is open, and a release retires its region instead of returning it to the free
  list, so no region a recorded pass reads is rewritten by a later staged upload of the same frame.
- The per-call **snapshot-triangle and snapshot-image arenas are built at creation** rather than written
  after it, which removes the hazard instead of managing it.
- **Arena growth is forbidden while the encoder is open**, because it changes the buffer identity under
  bind groups the encoder already holds and its forward copy would be overtaken by the staged writes of
  the same submission. `frame_begin` pre-grows to the demand the previous frames showed; a growth a
  frame still has to refuse becomes an `OVERFLOW` host rejection into `FullRedraw`.

Two paths stay outside the one submit by construction, both off the production path: `readback` records
its copies into the frame's encoder and submits it before it blocks, and `KFX_WGPU_DRAW_VERIFY`'s
per-shadow `shadow_scratch_read` does the same, so a verify run submits once per shadow as before.

Palette expansion is unchanged in substance: `present_into` is one fullscreen render pass with a 1 KiB
palette buffer, an 8-word parameter buffer and one bind group. Those buffers become persistent, the bind
group is rebuilt only when the target or surface format changes, and the pass joins the frame encoder.

**Validation without blocking.** Landed in PR 6. `validate_frame_status` used to allocate a staging
buffer, copy every per-batch status word into it, submit, `map_async`, and block in `device.poll(Wait)`
once per flush, on top of the per-batch flag readbacks outside a frame. What replaced it:

1. A persistent `status` buffer of 8 `u32` words: word 0 is the frame flag, the rest name the kernel
   check that raised it. The tail copy clears it in the same encoder, so no clearing pass is needed.
2. The general-triangle check moved *into* `draw.wgsl`: an out-of-range `trig_sample` skips the write
   for that command and raises the flag, and the dedicated `validate_trig` pass with its ~30 full-target
   dispatches per frame is deleted. The terrain check folds the same way. PR 8 deleted the
   `validate` entry point and `draw_triangles.wgsl` with it: a per-span walk is only stricter than a
   per-pixel fold if the raster visits fewer pixels than the span covers, and the conservative-box
   proof below shows the two pixel sets are identical.
3. `encoder.copy_buffer_to_buffer(status → staging_ring[cursor % 8])` at the tail, `map_async`, **no**
   blocking poll. A slot still mapped defers the publish and counts `status_stalls`; the flag stays in
   the status buffer until the next publish.
4. The result is read one or two frames later inside `frame_begin` and surfaced through
   `kfx_wgpu_draw_frame_status`. The drain runs the non-blocking `device.poll(Poll)` itself so the
   mechanism does not depend on which presenter is running.

**Invalid frames.** With validation in the kernel there is no pre-write rejection: an invalid command
writes nothing, the frame presents as drawn, the flag reaches the CPU one or two frames later, and the
bridge counts the frame and calls `FullRedraw()`, the recovery protocol that already exists. Host
validation still rejects a batch before any target write, but batches accepted earlier in the same frame
stay in the root: an aborted frame is redrawn, not rolled back. `frame_flush`'s rollback of
`minimap.background` and `target_snapshots` was dropped explicitly with the transactional scratch, which
bounds the recovery: a snapshot already taken from a flagged frame keeps those pixels until its owner
releases it, and `capture_map_fade_buffer` holds one across a whole map fade. The two-frame report is
likewise typical, not guaranteed, while `checkpoint_target` publishes the status word several times per
frame and a still-mapped ring slot defers a publish; PR 12's single encoder removes both.
Device loss is unchanged: `check_status()` reads `Renderer::failure` without blocking, the bridge's `Fail`
marks the frame invalid and attempts `ReplayPending` CPU reconstruction for the families that have a CPU
rasterizer (terrain spans and triangles only), and `RendererSoftware` falls back to SDL.

## C ABI

**Unchanged.** `struct KfxWgpuDrawCommand`, every kind constant and therefore
`KFX_WGPU_DRAW_ABI_VERSION`, which tags that record rather than the library: PR 6 appended
fields to `KfxWgpuFrameCounters` and added `kfx_wgpu_draw_frame_status`, and the crate is a
`staticlib` linked into the same binary, so no mismatched build can observe either. There are
**no C command producer changes**: the 128-byte stream record is an internal Rust/WGSL layout, not the C wire format. Also
unchanged: `kfx_wgpu_draw_target_create/release`, `_target_view`, `_submit`, `_submit_triangles`,
`_readback`, `_prepare_present`, `kfx_wgpu_present`, `kfx_wgpu_draw_counters`, the
`frame_begin/flush/end/abort` names and signatures, and the `kfx_wgpu_native_read_barrier` /
`kfx_wgpu_native_cpu_barrier` lease model.

**Changed semantics on unchanged signatures:**

| Entry point | Was | Becomes |
| --- | --- | --- |
| `kfx_wgpu_draw_frame_flush` | replay all queued batches into a fresh scratch, aggregate-validate with a blocking wait, copy back | landed: replay straight into the root and publish the status word; still on the production path until `ResidentTarget` stops calling it |
| `kfx_wgpu_draw_frame_end` | flush then clear the frame | build the encoder, record every pass, submit once |
| `kfx_wgpu_draw_target_snapshot` | implicit checkpoint of the target | record a copy at the current stream position into an arena slot |
| `kfx_wgpu_draw_submit_target_images` | own dispatch, own submit, checkpoints the root | append `IMAGE`/`TRANSITION` records |
| `kfx_wgpu_draw_submit_target_triangles` | own arena, own validate pass, own dispatch, own submit | append `TRIG` records with `aux_offset` pointing at a mask slot |

The atomic-commit and pre-write-rejection contracts in
[`KfxWgpuFrame.h`](../../src/kfx/renderer/KfxWgpuFrame.h),
[`WgpuDraw.h`](../../src/kfx/renderer/WgpuDraw.h) and
[`WgpuTargetResource.h`](../../src/kfx/renderer/WgpuTargetResource.h) are rewritten to flag-and-recover.

**New and modified signatures:**

```c
/* Interns immutable bytes by caller identity instead of content. */
uint64_t kfx_wgpu_draw_resource_intern(void *drawing, const void *key, uint64_t generation,
    const uint8_t *bytes, size_t length, uint32_t width, uint32_t height, uint32_t pitch,
    char *error, size_t capacity);

/* Non-blocking. Status of the most recent frame whose staging read has completed. */
int32_t kfx_wgpu_draw_frame_status(void *drawing, uint32_t *flags, char *error, size_t capacity);

/* Shadow scratch is GPU-resident. The read is blocking and exists only for KFX_WGPU_VERIFY. */
int32_t kfx_wgpu_draw_shadow_scratch_reset(void *drawing, char *error, size_t capacity);
int32_t kfx_wgpu_draw_shadow_scratch_read(void *drawing, uint8_t *mirror, size_t length,
    char *error, size_t capacity);

/* mirror/length removed. */
int32_t kfx_wgpu_draw_submit_shadow(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *command, char *error, size_t capacity);
```

`struct KfxWgpuFrameCounters` gains, appended: `submits`, `passes`, `raster_dispatches`, `wait_count`,
`wait_ns`, `buffers_created`, `buffer_bytes_created`, `bind_groups_created`, `arena_bytes_resident`,
`arena_bytes_uploaded`, `tile_entries`, `ordered_sprites`, `invalid_frames`; bump
`KFX_WGPU_DRAW_ABI_VERSION` or size-prefix the counter call. `kfx_wgpu_cursor_counters()` already exists
in [`WgpuCursor.cpp`](../../src/kfx/renderer/WgpuCursor.cpp); only its wiring into the stats sidecar is
missing. The shadow source layout in [`WgpuShadow.h`](../../src/kfx/renderer/WgpuShadow.h) loses its
65,536-byte prior-scratch section, becoming `HEADER(32) + GEOMETRY(120) + RLE`, which also removes two
`malloc`s and the scratch, fade and ghost `memcpy`s in the software adapter.

**Removed by this design** in `WgpuTerrainBridge.cpp`: the per-command `Flush()`/`ExecutePending()` pair
in `SubmitNative`; `ResourceFor`'s content `memcmp` cache; the 8 KiB zero-initialised `std::array` and
32 × 32-byte de-pad memcpy per triangle; `m_shadow_scratch`, `shadow_mirror` and the shadow branch of
`ExecutePending`; the per-command `kfx_wgpu_draw_resource_create`/`_release` pair.

**Deleted when C/C++ drawing is retired:** `Materialize`, `m_readback` and the readback counters;
`CpuBarrier`, `ReadBarrier`, `ValidateCpuLease`, `m_expected`, `m_cpu_checkpoint`; `RasterizePending`,
`ReplayPending` and the `KFX_GPOLY_DECLINED` fallback protocol; `PrepareNativeTarget`'s initial
full-frame CPU upload; `GpolyCapture.c` and `KfxGpolyRasterizer`; `kfx_wgpu_submit`; the `m_verify`
runtime comparison block and every per-family `Oracle` callback (offline fixtures stay);
`RendererSoftware::PresentFrame`'s SDL blit path; `FrameView` and `SubmissionTarget` pointer arithmetic
against `lbDrawSurface->pixels`; and the `software/Wgpu*` adapters' accept/decline protocol and read
barriers, which become unconditional emitters.

## Kernels

Reused unchanged: [`gpoly_prepare.wgsl`](../../tools/frame-replay/src/gpoly_prepare.wgsl) (96-bit
fixed-point triangle setup); `trig_sample` and its 27 modes; all sampler arithmetic;
`sprite_copy_forward` and the body of `sprite_ordered`; the shadow RLE decode and stamping loop.

| Adapted kernel | Change | Risk |
| --- | --- | --- |
| `draw.wgsl` `draw` | root-space coordinates, per-command origin, `[begin,end)` sub-range, merged tile lists, new kinds, in-kernel status writes | medium |
| every sampler signature | `id.xy` → `pixel − origin` | mechanical, but touches all of them at once |
| `draw_triangles.wgsl` `render` | folded into `draw.wgsl` as `terrain_sample(c, pixel)` reading the prepared-row arena at `aux_offset` | medium-high |
| `draw_triangles.wgsl` `validate`, `draw_trig.wgsl` validate pipeline | deleted; the flag moves into the raster kernel | contract change |
| `draw_shadow.wgsl` | landed: prior scratch from a binding instead of `source[152u+address]`, RLE cursor at `152u`, result written to both the persistent scratch and the frame's mask slot | medium |
| `draw_sprites.wgsl` `sprite_ordered` | reads its record index from a parameter so *M* workgroups serve *M* sprites | medium |
| `draw_effects.wgsl`, `draw_minimap.wgsl` | non-alias lens and minimap modes 1–3 folded into `draw.wgsl`; the alias path and minimap modes 0 and 4 keep their own pipeline and snapshot dependency | medium |
| all asset reads (phase 2) | `assets[i]` → `asset_byte(i)` from a byte-packed arena | **high**; family by family behind the fixtures |

## Fixtures

New fixtures required for parity:

1. **Frame order** (the central one). Landed as
   [`draw_frame_order_gpu.rs`](../../tools/frame-replay/tests/draw_frame_order_gpu.rs): 48 seeded
   interleaved steps across three views covering every family, per-batch versus single-stream,
   byte-equal `readback` plus one raster pass per non-empty raster run.
2. **View rebasing.** Landed as
   [`draw_views_gpu.rs`](../../tools/frame-replay/tests/draw_views_gpu.rs). A root pitch wider than
   the root width is unconstructible — `create_target` always sets `pitch = width` — so the fixture
   uses nested views at nonzero offsets in a wide root, one command of each family per view, with
   root-space compared against view-space rendering.
3. **Ordered-sprite layering.** Asserts layering never reorders an overlapping pair and that a
   fully-overlapping set degenerates to one sprite per layer, extending
   `tests/sprites/copy_fixture.c`'s 576 alignment cases into a multi-sprite frame.
4. **Shadow hoisting.** Landed as the interleaved queued-frame chain, which interleaves shadows with
   queued `RECT` commands and a scratch reset; `IMAGE`, general `TRIG`, ordered `SPRITE` and periodic
   full-view `CLEAR` interleaves remain open. Extends the `tests/shadows` chains so shadows interleave with other families,
   asserting hoisted mask passes equal the alternating mask/raster chain.
5. **Arena residency and eviction.** Forces eviction mid-frame; asserts identical pixels, a generation
   bump producing a new upload, and that a stale generation never aliases.
6. **Byte-packed assets** (phase 2), re-running every family fixture with unaligned asset offsets.
7. **Non-blocking status.** Landed as [`draw_status_gpu.rs`](../../tools/frame-replay/tests/draw_status_gpu.rs)
   and the flagged-frame block of [`bridge_test.cpp`](../../tests/terrain-vertices/bridge_test.cpp):
   an injected invalid lookup surfaces within two frames, the frame presents as drawn, and the bridge
   recovers through `FullRedraw` without failing.
8. **Structural counters.** `submits == 1`, `wait_count == 0`, `checkpoints == 0`,
   `buffers_created <= 8` on a synthetic full frame; this is what stops the structure regressing.
   Partly landed with PR 6: `draw_status_gpu.rs` asserts zero blocking waits, zero validation waits and
   zero checkpoint copy bytes over a steady frame sequence. `submits` and `checkpoints` land with PR 12.
   PR 10 added the present-tail half: `draw_target_resources_gpu.rs` asserts one submit and no
   checkpoint for a whole backup/compose/palette/restore tail, including the acquisition-skip cycle.

Existing fixtures must stay green at every step: all thirteen `tools/frame-replay/tests/draw_*_gpu.rs`
and `gpoly_gpu.rs`, the CMake oracle generators under `tests/`, the `tests/cursor` native lifecycle test
and [`replay.py`](../../tools/frame-replay/tests/replay.py). They exercise single-family batches through
the public API, so they keep working while the C wire format is unchanged; every step must leave
[`frame-replay.yml`](../../.github/workflows/frame-replay.yml) green.

## Migration sequence

Each step is one PR and keeps every existing fixture green.

| PR | Change | Acceptance counter it must move |
| ---: | --- | --- |
| 1 | **Instrumentation** (`TIMESTAMP_QUERY` delivered in PR 8). Route every `queue.submit` through one helper and every `create_buffer`/`create_buffer_init`/`create_bind_group` through helpers; add the new counters including `wait_ns` and `ordered_sprites`; time the three `device.poll(Wait)` sites; add a per-frame ring; expose the counters through [`performance_capture.cpp`](../../src/performance_capture.cpp) with window semantics and **no per-frame file I/O**; make `report_drawing` interval- or shutdown-driven; wire `kfx_wgpu_cursor_counters()` into the sidecar; optional `TIMESTAMP_QUERY`. | none directly; every *derived* row above becomes measured |
| 2 | **Free CPU wins.** `ResourceFor` → pointer+generation intern; `check_queued_target` → running byte total; `released_resources` → `HashSet`; **and `create_resource`, which runs the identical O(resources) byte sum on every resource creation**; `DrawTriangle`'s 8 KiB array → reused member scratch. | `resource_snapshot_bytes` 1.95 MB → ~0 |
| 3 | **Bridge batching.** `SubmitNative` accumulates into `m_pending`; flush only at target change, shadow, transition, ordered sprite, snapshot or readback. | `gpu_batches` 139 → 10–20 |
| 4 | **Persistent asset arena**, `u32` expansion kept, kernels unchanged. Delivered, GPU drawing behind the SDL presenter: asset plus command upload 28.78 MB → 15.14 MB per frame and Rust requested bytes 93.1 MB → 28.4 MB per presentation; the wgpu-presenter pair is outstanding. The ≤ 0.3 MB target needs PR 14 and emitters that stop baking position into the asset. |
| 5 | **Shadow residency.** Delivered, GPU drawing behind the SDL presenter: persistent GPU scratch and two mask slots; CPU mirror, readback and snapshot dropped; each mask submitted immediately ahead of its `TRIG` pair, not hoisted. Checkpoints 9.4 → 1.0, blocking waits 29.8 → 2.9 and `shadow_scratch_readback_bytes` → 0; the wgpu-presenter pair is outstanding. |
| 6 | **Non-blocking validation, no double copy.** Delivered, GPU drawing behind the SDL presenter: the flag lives in the raster kernels, a mapped ring reads it one or two frames later, batches write straight into the root and the transactional scratch and its snapshot rollback are gone. Blocking waits outside the CPU presenter's own readbacks and `frame_gpu_checkpoint_copy_bytes` are structurally 0; the wgpu-presenter pair is outstanding. |
| 7 | **Single command stream, root space, one tile index.** Delivered, GPU drawing behind the SDL presenter: per-command view origins, counting-sort binning into renderer-owned scratch, one raster pass per serial segment sized to the tiles its records reach, and the bridge's target-change flushes removed. Tile-list allocations → 0, bridge target-change flushes 39.3 → 0, buffer allocations 198.5 → 161.3 and Rust allocator calls 30.0 M → 5.7 M per measured window; **Rust batches stayed ~73**, because ~38 creature shadows per frame each close a raster segment. CPU drawing and presentation improve; GPU blocking wait rises about 2 ms per frame and observed FPS falls 1–3. **That cost is unattributed.** Two candidates were measured and rejected: the record layout (above), and root-space tile misalignment — binning the same busy frame against each record's own view yields 117,693 entries against 121,849 in root space, 3.5%, which cannot account for a 20% wait. What did move with it is one dispatch and one submit per raster pass where the per-batch path merged them. The ~38 figure is shadow *submits*: consecutive shadows share one boundary, so the frame cuts fewer raster passes than that. Fixtures 1 and 2 landed. |
| 8 | **Terrain triangles in the stream.** Delivered, on the wgpu presenter with GPU drawing: one `TERRAIN_TRI` record per triangle binned by its conservative box, a prepared-row arena compressed to covered rows in a renderer-owned buffer, the separate validate pass deleted under the superset proof, the bridge's 128-triangle cap removed, and opt-in per-pass GPU timestamps so the terrain share is attributed rather than inferred. Measured terrain iterations **1.85 M at 640x480** and **7.85 M at 1920x1080** (`terrain_tile_entries` 7,230 and 30,660, times 256), against derived 308 M and 2,065 M. Per-frame GPU time 14.00 → 9.29 ms at 640x480 and 88.07 → 87.81 ms at 1080p; dispatches 128.7 → 59.1 and 113.1 → 48.6; submits 83.9 → 45.2 and 76.1 → 39.1. Presentation 16.26 → 5.71 ms and observed FPS 54.95 → 60.00 at 640x480, at 20.03 turns/s; 103.16 → 76.83 ms and 9.52 → 12.65 FPS at 1080p, where turns/s rose 9.57 → 13.98 but is still dragged below 20. Prepared rows 0.68 MB and 1.74 MB against 12.6 MB and 28.3 MB uncompressed, with zero arena growths after warm-up. **The 1080p GPU total did not fall**: terrain's 26.40 ms became 12.0 ms of extra raster time and the rest was taken back by the minimap and ordered-sprite passes, whose own times rose. Full-target dispatches do not reach ~2; that needs PRs 10, 12 and 13. |
| 9 | **Tight bin boxes.** Delivered, on the wgpu presenter with GPU drawing: every sampled kind is binned by the destination box its validator proves it cannot write outside — the union of a sprite's scaling ranges, a triangle's vertex bounding box, a bitmap's row and run extents or its scaled glyph rectangle — instead of the whole-target bounds five emitters set. The clip stays the pixel-level guard, so the sampler arithmetic is unchanged, and `RAW_IMAGE` keeps the whole target because it writes index 0 outside its destination rectangle. Measured on matched busy pairs: `tile_entries` 126,750 → 9,917 at 640x480 and 897,385 → 46,149 at 1080p, so pixel-command evaluations fall 32.4 M → 2.5 M and 229.7 M → 11.8 M. The raster pass falls 5.223 → 1.184 ms and 46.124 → 2.859 ms, the shadow triangles 1.314 → 0.663 ms and 7.876 → 1.001 ms because their dispatch is now the vertex box, and the *unchanged* minimap pass 3.175 → 0.805 ms and 16.901 → 2.414 ms, which confirms that window as attribution rather than cost. Presentation 76.369 → 4.765 ms and observed FPS 12.80 → 60.00 at 1080p, at 20.04 turns/s; a quiet 1080p pair moves 60.340 → 4.476 ms and 16.16 → 60.00. Both 1080p scenes now sit on the frame cap. Also publishes `tile_entries_<kind>` and `gpu_pass_union_ns`, the union of the frame's pass intervals, which measures equal to the window sum — the windows are disjoint and each holds its own stall, so only a serialised run (`KFX_WGPU_GPU_TIMING=2`) is exclusive, and it reports **3.549 ms of GPU work against an 8.006 ms window sum**. `RAW_IMAGE` is now the residual: at 8,160 tiles per full-view record it is most of the 46,149 that remain after terrain's 30,660, and splitting it into its destination rectangle plus up to four index-0 fill rectangles would be exact. | `tile_entries` <= 15,000 / <= 80,000; `gpu_raster_ns` <= 1.0 / <= 8 ms; presentation <= 4 / <= 25 ms — all met; serialised GPU sum 3.549 ms under a 17.284 ms frame interval |
| 10 | **Ordered sprites into layers.** Delivered: consecutive ordered sprites whose write rectangles are disjoint share one dispatch of *M* workgroups, an overlapping sprite opens the next layer, and layers run in frame order. A sprite is bounded by its own scaling ranges, not by its clip rectangle, which the emitter always sets to the whole drawing window. **The acceptance counter did not move.** Measured on five matched busy 640x480 pairs: `ordered_sprite_layers` 2.68 against `ordered_sprites` 2.69, so layering merges about one sprite in 250. Consecutive ordered sprites are rare — a 20-turn trace found 4 runs of two against 559 runs of one — because a raster record between two of them ends the run. `submits` 76.9 → 74.9 and `buffers` 165.5 → 155.9, both inside a run-to-run spread wider than the effect. No FPS change is attributable. | per-sprite submits → 0; sized by PR 1's `ordered_sprites` |
| 11 | **Cursor at the tail of one encoder.** Delivered: target snapshots, target images and the palette render pass record into a present tail that `kfx_wgpu_present` submits, the acquisition-skip path finishes, and every other submit is ordered behind. `LbMouseOnEndSwap` runs before the present call so the restore joins it. **The checkpoint inside `PerfPresentation` does not go here.** It is not the cursor: `lbPointerAdvancedDraw` is never set, so `OnBeginSwap` draws the direct scaled sprite into the frame stream and `OnEndSwap` does nothing, and the measured 1.0 is `ResidentTarget`'s own terminal `frame_flush`. Hoisting `ResidentTarget` above `LbMouseOnBeginSwap` would leave the direct cursor to reopen the queued frame and make `present_into` flush it a second time — 2.0 checkpoints per frame, not 0. Measured on a matched busy 640x480 triple: checkpoints 1.00 → 1.00, submits 77.4 → 77.2, buffers 168.5 → 166.1, all inside run-to-run spread. | the advanced-draw swap: six submissions → one |
| 12 | **Fold lens and minimap.** Non-alias lens and minimap modes 1–3 become stream kinds. | two fewer pipelines and their per-call buffers |
| 13 | **One encoder, one submit**, palette render pass included. Delivered: every pass of a frame records into one encoder that `kfx_wgpu_present` finishes and submits, a flush replays into it instead of ending anything, and the submit lives in the present call because the cursor restore is issued after `prepare_present` returns. The staged-write law forced four changes — a stream ring bump-allocated inside the encoder, encoder-scoped arena pinning with retired releases, per-call arenas built at creation, and arena growth taken before the encoder opens. Measured on serial matched busy pairs: **`submits` 37.2 → 1.00 at 640x480 and 38.3 → 1.00 at 1080p**, `checkpoints` 1.0 → 0, `waits` 0 → 0, `dispatches` unchanged (44.85 → 44.85, 46.18 → 46.19). Presentation 5.154 → 3.917 ms and 6.167 → 4.729 ms, both scenes staying at 60.00 FPS and 20 turns/s. Uncapped over three matched triples: 640x480 **141.1 → 189.6 FPS** with the frame interval 7.105 → 5.290 ms; 1080p 164.0 → 168.4 FPS, inside a run-to-run spread of ±30 FPS, so no 1080p gain is claimed. **`buffers` did not move** (87.3 → 87.3, 90.4 → 90.4): the per-call command, tile and parameter buffers are unchanged by this step and need PR 14. The unserialised pass-window union rose (3.825 → 4.390 ms and 7.809 → 9.569 ms) because a window now absorbs the barrier wait that used to sit between submissions; the serialised, exclusive figure did not (4.881 → 4.801 ms and 8.212 → 7.853 ms), and the two builds serialise at different granularity so that pair is indicative only. | `submits` → 1 — **met** |
| 14 | **Byte-packed arena.** Delivered: byte offsets/readers, aligned byte uploads and GPU snapshot packing; all native/GPU fixtures pass in packed and expanded builds. Queue transport remains. | Asset footprint ÷4 for matched allocation traces; host timing and surface gates pending. |
| 15 | **1080p acceptance.** Re-run clean matched pairs at 640x480 and 1920x1080 with PR 1's counters and GPU timestamps. | every row in the acceptance tables becomes measured |

## Decisions

- **Binning terrain by the conservative box is exact, and that is what lets the span-validation pass
  go.** `gpoly_prepare.wgsl` sorts by y, starts at the lowest vertex, breaks at the highest or at the
  view edge, and writes no row below zero, so every written row's `y` lies in the clamped vertex y
  range. Its x accumulators are anchored at a vertex and advanced by a slope truncated toward zero
  (`slope` at `gpoly_prepare.wgsl:70`), and the `clipped` branch only narrows the interval, so every
  covered pixel's `x` lies in the clamped vertex x range. The box therefore contains every written row
  and every covered pixel, the raster's own bounds test is the row guard, and the raster visits exactly
  the set the per-span pass walked. The proof, not the assertion, is what the deletion rests on, and
  `gpoly_gpu.rs` fails if any native row falls outside a triangle's extent.
- **Resolution target.** The 60 FPS target applies to a **1920x1080 logical framebuffer**, not only a
  1920x1080 output of a 640x480 framebuffer. The 2026-09-14 measurement confirmed the engine honours a
  1920x1080 logical framebuffer with logical size equal to physical output, and that the software path
  holds 60.00 FPS and 20.00 turns/s there. A 1080p logical framebuffer multiplies every full-target term
  by 6.75, which is what makes terrain binning (PR 8) mandatory.
- **Invalid frames flag and recover.** No per-frame previous-root copy; recovery is `FullRedraw` within
  one or two frames, as above. Presenting a retained previous root would cost one extra full-target copy
  per frame (8.3 MB at 1080p) for a case measured at zero occurrences. The atomic-commit contracts in
  `KfxWgpuFrame.h`, `WgpuDraw.h` and `WgpuTargetResource.h` are rewritten accordingly.
- **Resource identity.** `ResourceFor` is keyed by pointer plus generation, bumped on every path that
  mutates a texture or fade table in place. In-place mutation without a bump is a bug to be caught by the
  verify oracle, not a supported case.
- **Verification.** `KFX_WGPU_DRAW_VERIFY` keeps the shadow-scratch comparison through the blocking
  `kfx_wgpu_draw_shadow_scratch_read`, used only in verify runs; `KFX_WGPU_VERIFY` is the separate
  presentation-surface check.
- **Parity policy.** Exact indexed-byte parity with the software reference remains the acceptance for
  the port, with three carve-outs. Undefined legacy behavior is not a compatibility requirement:
  uninitialized triangle steps and shadow scratch contents written by unrelated `big_scratch` users may
  differ. Deliberately new features — smoothing, higher-resolution art, remastered looks — live behind
  profile flags and are judged against their own contract, not against the software path. Once the
  C/C++ drawing is retired, the Rust path becomes the reference and parity becomes a regression test
  against recorded frames. Anything else that differs is a defect. The tooling that enforces this is in
  the [development tooling plan](../product/development-tooling-plan.md).

## Open measurement

**Measured, 2026-09-14.** Busy scene, 640x480, `KFX_DRAW_BACKEND=wgpu` with the SDL presenter (the
Rust presenter could not acquire a drawable on the measuring host), Apple M5 Metal, 304 measured frames:
`arena_bytes_resident` 12.09 MB mean and 13.34 MB maximum, with `arena_evictions` and `arena_overflows`
at zero. **Units differ between the gauge and the budget:** the gauge counts *expanded* arena bytes, one
`u32` per source byte and therefore the GPU footprint, while the 32 MiB phase-1 budget counts *real
asset bytes*. The 13.34 MB peak is thus 3.34 MB of real asset bytes — 10% of the 32 MiB budget and 10%
of the 128 MiB storage binding — matching the ≈ 3.5 MiB raw / ≈ 14 MiB arena estimate. The gauge is the
suballocated extent, so it is an upper bound on the live working set. The working set therefore fits
phase-1 `u32` expansion at the default 128 MiB storage binding, and PR 14 does not need to move earlier.
1920x1080 is unmeasured; the per-pixel term that grows there is the initial root image, so the bound
should rise by roughly the framebuffer difference rather than change class.

## Risks

| Risk | Mitigation |
| --- | --- |
| Origin rebasing touches every sampler at once | Landed with the view-rebasing and frame-order fixtures; every family fixture already covers the samplers |
| Reading a record's view through the tile buffer costs a dependent load in the hottest loop | Only kinds at or above `SPRITE` read it, once per command. Measured against the spec's 128-byte record with the origin inline over five interleaved triples: blocking wait 14.32 versus 13.99 ms and 43.6 versus 44.1 FPS, inside a per-triple spread of 1 ms and 2 FPS, so the layout is not the cost and the 112-byte record stays |
| Creature shadows are the dominant serial boundary, not ordered sprites | ~38 shadow submits per frame at busy 640x480 against ~3 ordered sprites; back-to-back shadows share one boundary, so the raster-pass count is lower than the submit count. "≈ 4 raster passes" still needs the mask chain hoisted (PR 12) as well as PRs 8-11 |
| Shadow mask hoisting is the subtlest correctness claim here | Dropped: masks record in stream position, and the 192-case chain now runs interleaved across queued frames and a scratch reset |
| Ordered-sprite layering degenerates to one pass per sprite, and the population is unknown | Bounded worst case equals today's behaviour minus the submits. `gpu_sprite_commands` counts all sprites, ordered or not, so PR 1's `ordered_sprites` must size this before PR 9. They need `mode < 4 && scale_up && !blend && (flip & 1)` and a y range with `count > 1`, so upscaled sprites at 1080p could make them the dominant serial boundary |
| Compressed prepared-row extents disagree with `gpoly_prepare.wgsl` clipping | The extents must be a proven superset; `gpoly_gpu.rs` compares 597,800 setup words against native and is the guard |
| `TIMESTAMP_QUERY` unavailable on a target device | The feature request is optional; timings fall back to host wall time |
| Counter regressions creep back after the restructure | `draw_one_submit_gpu.rs` asserts `submits == 1`, `wait_count == 0` and `checkpoints == 0` over a synthetic full frame in CI |
| A staged `write_buffer` lands before a pass that should have seen the old bytes | The stream ring bumps inside the encoder, arena pinning is encoder-scoped and a release retires its region, the per-call arenas are built at creation, and growth is forbidden while the encoder is open. `draw_one_submit_gpu.rs` pins an asset uploaded mid-frame and a frame replayed twice against their per-submit replays |
| One command buffer grows past a driver limit | A busy frame records about 114 passes; a debug assertion fails at 4,096 |
