---
type: guide
description: Collect isolated native presentation comparisons, wall-time distributions, process CPU time and scoped Rust allocation counts.
---

# Game performance measurements

Build the engine and prepare assets as described in [Mac development](macos.md).
Then run each scenario separately from the repository root:

```sh
python3 scripts/profile-game.py --scene quiet --out out/perf-quiet
python3 scripts/profile-game.py --scene busy --out out/perf-busy
python3 scripts/profile-game.py --scene possession --out out/perf-possession
```

The original presenter is selected explicitly by default. With the optional Rust
presenter built on macOS, add `--backend rust` for live Rust/wgpu measurements.
The runner requires Cocoa/Metal and rejects a fallback to SDL in a requested Rust
run. Actual Rust adapter, surface format and present mode are recorded separately
from requested settings. Rust measurements require a native window; headless
smoke tests support only the original presenter.

These commands open a native game window and exit after the bounded sample.
Keep the window visible during measurement, or use the
[offscreen measurement mode](#offscreen-measurement-mode), which has no swapchain and
therefore no such requirement. In an opted-in local game the hook
suppresses gameplay command input (including mouse-look) from startup through
completion, while SDL event polling continues. Other applications,
window occlusion, power mode, temperature and display configuration can affect
results; record those conditions when comparing runs. The runner clones only
assets into a temporary installation, creates empty saves, and records its own
configuration. It disables audio, the API, cursor capture and edge panning.
Personal saves and settings are not loaded or modified.

Defaults are a 640×480 window, 20 simulation turns per second, interpolation on,
60 drawing frames per second, VSync off, warmup to turn 40 and 200 measured turns
(about ten seconds). Possession waits for the controlled creature view before
starting. Options support other resolutions and bounded warmup/durations; use `--help`
for accepted values. VSync, interpolation and turn rate are fixed by the runner;
the frame cap is 60 FPS unless [`--uncapped`](#uncapped-measurement-mode) removes it.
Keep settings, engine build, assets and host conditions identical for comparisons.

## Scenarios and reproducibility

| Scenario | Fresh starting state and input |
| --- | --- |
| `quiet` | Original campaign (`keeporig`) level 1; initial dungeon camera, no commands |
| `busy` | Original campaign level 20; initial dungeon camera, no commands; existing AI dungeon and populated hero stronghold |
| `possession` | Original campaign level 1; enter the first eligible owned creature by thing index through the normal control function, then no commands |

The busy case exercises a larger live world. It does not promise a crowded camera,
combat, or a late-game stress test. Population snapshots in the engine metadata
show the actual loaded and evolved workload. Campaign and level overrides must
be reported as different scenarios, with their resulting population and view.

Each run records engine and asset identities, isolated configuration, command,
requested settings, actual presentation backend, video driver, VSync, framebuffer/output
sizes and engine frame limit. Start/end snapshots record simulation turn, creature
and total thing counts, active camera position/angles/zoom, controlled thing index
and the five game RNG states. These seeds are observations after startup; this
runner does not fix every startup clock/random source or replay commands. Matching
settings and asset hashes reproduce the procedure, not an identical simulation
state. Do not interpret similar distributions as gameplay determinism.

## What the measurements mean

Raw CSV durations are integer nanoseconds from `std::chrono::steady_clock`.
Reports convert them to milliseconds and retain sample counts, mean, median,
p90, p95, p99 and maximum. Percentiles use linear interpolation between ordered
samples. Separate distributions have different sample counts because simulation
and drawing run at different rates.

| Series | Boundary and interpretation |
| --- | --- |
| `simulation` | One `update()` call; excludes input polling, packet exchange and turn pacing |
| `draw` | One `keeper_screen_redraw()` call drawing world and HUD into CPU pixels; excludes light-area setup, focus waiting, direct-message overlays and presentation |
| `replay` | GPU frame replay through `ResidentTarget`, including bridge flush/target lookup; zero for software drawing. Recorded as a child but subtracted from exported `presentation`, so `draw + replay + presentation` is additive. |
| `presentation` | Excludes `replay`. Per-frame cursor composition, palette/pixel processing and upload, rendering/present submission and cursor cleanup. Original SDL includes texture lock, indexed-to-RGBA blit, texture unlock/upload and clear/draw/present. Rust includes its surface acquisition, uploads, submission and polling. Excludes target setup and metadata queries. |
| `present_wait` | Nested SDL or Rust present call, including host work and blocking inside that call; **already included in presentation**. Backend implementations distribute work differently, so this is a diagnostic, not a common GPU/VSync-cost measurement. |
| `frame_interval` | Time between starts of successive measured presentation calls; includes simulation, drawing, event handling, pacing and scheduling between them |

All series measure elapsed **wall time**, including descheduling or waiting.
`draw` measures work implemented on the CPU, but is not a thread/process CPU-time
counter. `present_wait` is not a pure VSync wait: drivers can also block on texture
lock/upload or elsewhere. GPU timestamps are separately opt-in; these scopes do not measure GPU completion latency. Do not add nested series or call them GPU benchmarks.

Separate measured-window counters record process user and system CPU time using
`getrusage(RUSAGE_SELF)` on macOS/Linux or `GetProcessTimes` on Windows. They include
worker threads and exclude startup, warmup, sample-file output and shutdown. Reports
include CPU milliseconds per turn and presentation and average core equivalents
(process CPU time divided by measured wall time). These are process-wide counters,
not component CPU costs; they do not relabel the per-scope wall-time samples. Older
reports without these counters retain an explicit unavailable value.

Builds with live Rust support also record successful Rust global-allocator
allocation/reallocation calls and requested bytes over that window. They exclude
C/C++, SDL and driver/GPU allocations and do not measure retained memory. The
original backend normally does not activate Rust allocation paths; zero Rust
allocations there cannot establish a total-process memory advantage. Full heap
allocation and memory comparisons require additional measurement with explicit
coverage. Resource counter snapshots themselves add small unquantified overhead.

The existing on-screen timing display remains unchanged. Its logic scope includes
input and pacing (and can invoke drawing), while its draw scope includes
presentation. The exported scopes are narrower so those overlapping debug values
are not reused as component baselines.

## Coarse drawing breakdown

Add `--draw-breakdown` to collect four non-overlapping children of `draw`:

| Series | Boundary |
| --- | --- |
| `draw_scene` | `draw_view()` and `draw_frontview_engine()` setup and bucket construction before dispatch |
| `draw_raster` | Complete isometric/possession `display_drawlist()` dispatch, including terrain, sprites, shadows and bucket overlays |
| `draw_front_raster` | Complete front-view `display_fast_drawlist()` dispatch, including textured quads, sprites and bucket overlays |
| `draw_overlays` | View HUD, messages, hand and tooltip blocks, plus the common post-view overlay block in `redraw_display()` |
| `draw_unaccounted` | Derived per frame: `draw` minus the four children above |

Each child accumulates all visits within one draw and emits exactly one sample,
including zero for an unvisited scope. The engine rejects overlapping or unfinished
children. The report requires complete, ordered child blocks immediately preceding
their enclosing draw, and rejects a child sum exceeding its parent. These children
are already included in `draw`; do not add them to it. Zero does not prove a family
was absent: uninstrumented paths remain in `draw_unaccounted`.

These are whole dispatch timings, with no per-primitive or per-pixel clocks.
Terrain and sprites remain interleaved inside each dispatch; use a separate sampling
profile to investigate those families. Unaccounted time includes framebuffer setup,
view orchestration, smoothing/lens effects outside the hooks, other paths and timer
bookkeeping. Scene preparation also includes existing state updates at that boundary.
These scopes describe host work and do not establish GPU execution time.

The flag sets `KFX_PERF_DRAW_BREAKDOWN=1`; the default explicitly sets it to `0`.
Both modes keep the existing top-level timing boundaries and the run's frame cap. Compare
serial matched runs from the same executable with and without this flag to quantify
the added clock, bookkeeping and sample overhead before interpreting the breakdown.
The default mode still has inactive hook calls. Four extra records per drawn frame
share the existing 100,000-record limit; long runs can reach that limit sooner.

## Drawing-backend counters

Every report states the drawing backend that was actually active (`software`,
`wgpu`, or `wgpu-fallback` once the bridge has failed), so a silent fallback is
visible instead of being read as a GPU result. A backend that changes inside the
measured window invalidates the run.

With GPU drawing active the engine samples the bridge, drawing-context and frame
counters once per presented frame and keeps only per-frame deltas for the
measured window, emitted through the existing JSON sidecar; there is no per-frame
file I/O. The report gives per-frame min, mean, p95 and max plus the window total
for queue submits, full-target compute dispatches, blocking device polls and
their measured host wait time, frame checkpoints and GPU-to-GPU checkpoint copy
bytes, aggregate validation waits, kernel-flagged invalid frames, skipped status
publishes, command and asset upload bytes, GPU readback bytes, full-target
readbacks, buffer allocations and bytes, batches, commands and ordered sprites.
`checkpoint_copy_bytes` and `validation_waits` are structurally zero: a queued
frame records its batches straight into the root and publishes its validation flag
through a mapped ring instead of a blocking poll. `flagged_invalid_frames` counts
frames a raster kernel found an out-of-range lookup in — each one is presented as
drawn and then redrawn — and `status_stalls` counts publishes skipped because
every ring slot was still mapped, which defers a flag rather than losing it. `dispatches` counts every compute dispatch the drawing context
issues, including the single-workgroup ordered-sprite passes.
`host_staged_asset_bytes` is a gauge sampled at frame end holding the CPU copies
the drawing context stages, not GPU memory, so its window total is meaningless.

`arena_evictions`, `arena_overflows` and `arena_bytes_uploaded` cover the
persistent asset arena: LRU reclaims, allocations that could not be satisfied and
so rejected their batch, and bytes written into the arena.
`arena_bytes_resident` is a second gauge holding the arena extent suballocated so
far, including free-listed slots and power-of-two class padding, so it bounds the
live working set rather than tracking it exactly and its window total is
meaningless. It counts expanded arena bytes — one `u32` per source byte, the GPU
footprint — so dividing by four gives the real asset bytes that the arena's
source-byte capacity is stated in. `arena_scratch_bytes_peak` is a third gauge: the
widest extent the arena's transient regions reached inside one pinning scope, which
one submit per frame makes a whole frame rather than a batch. `upload_bytes` still
counts every asset write, arena or not.

The first presentation only establishes the counter baseline, so there is exactly
one fewer counter frame than presentation sample. `wait_ns` is host time blocked
inside device polls and is already contained in the enclosing `draw` and
`presentation` wall-clock scopes; it must not be added to them.

**GPU execution time is opt-in and per pass.** With `KFX_WGPU_GPU_TIMING=1` on an
adapter exposing `TIMESTAMP_QUERY`, each compute and render pass carries
`timestamp_writes` and the `gpu_*_ns` counters report GPU duration per pass kind.
`TIMESTAMP_QUERY_INSIDE_ENCODERS`, which this Metal adapter lacks, is needed only
to stamp outside a pass, so copy-only submissions stay untimed. Every other counter
here is host-side and none of them may be read as a GPU timing. `gpu_untimed_passes`
is nonzero when the resolve ring was saturated, and the per-pass totals then
under-report.

**A pass window is not exclusive.** It runs from that pass's own begin stamp to its
own end stamp, so it includes whatever the pass waited through; a `gpu_*_ns` figure
attributes cost rather than measuring it, and the sum of the windows decomposes
nothing. `gpu_pass_union_ns` is the union of the frame's timed pass intervals,
closed once per frame, so windows overlapping in time count once. It is an upper
bound on the frame's GPU occupancy and, measured, equals the window sum on this
Metal adapter: the windows are disjoint and each holds its own stall. Neither is
bounded by the `presentation` scope, which is host-side and ends at hand-off while
the GPU runs past it; the frame interval is the bound that holds.
`KFX_WGPU_GPU_TIMING=2` (`profile-game.py --serial-gpu-timing`) drains the queue
after every timed submission, so each window then holds its pass alone — the only
exclusive measurement here, and it serialises the frame, so it is a diagnostic and
not a performance baseline.

Counters cover the drawing context the bridge owns. Surface acquisition,
presentation by the Rust presenter, and a cursor that owns its own drawing context
rather than borrowing the bridge's are not counted, so the counters explain the
drawing scopes rather than the whole frame.

### Shadow asset reuse counters

`target_trig_geometry_bytes` and `target_trig_table_bytes` count actual expanded
uploads by the shadow target-triangle route. Their sum plus
`other_asset_upload_bytes` equals `asset_upload_bytes`; geometry stays asset data.
`target_trig_table_hits` / `target_trig_table_misses` count each table resolution,
including the second triangle of a pair. `shadow_pairs` counts replayed mask/pair
batches, including culled pairs. `target_trig_asset_buffers` counts only fallback
asset creations: zero with the arena, one per nonempty fallback batch. A fresh
pair uploads 480 geometry bytes; a resident table uploads zero. The disabled-arena
fallback deduplicates the pair's table but uploads it again next batch.

`arena_misses_{new_id,forget,size_class,generation,eviction}` and corresponding
`arena_miss_<cause>_bytes` partition successful arena resolutions that upload;
the bytes sum to `arena_bytes_uploaded`. Generation means the unchanged renderer
recovery epoch. `arena_explicit_forgets` counts resident entries removed on release,
separately from capacity-driven `arena_evictions`. A forgotten live ID can miss as
`forget`; release permanently drops its history, so a recreated resource's new ID
misses as `new_id`. Eviction history lasts only until reuse or resource release.

`preparer_buffers` / `preparer_buffer_bytes` separately account the three gpoly
setup buffers: vertices (96 bytes/triangle), row layout (20 bytes/triangle), and
viewport (16 bytes/encode). They do not change `buffers`, `buffer_bytes`, or the
legacy upload totals. Those totals still omit some uniform/staging traffic.

The additional gauges are `arena_capacity_bytes` (allocated buffer),
`arena_live_bytes` (resident size classes), `arena_retired_bytes` (released classes
still protected by an encoder), and `arena_growth_peak_bytes` (largest old+new
capacity during growth). They supplement the existing extent and scratch-peak
gauges; they do not measure whole-process or driver/GPU peak memory.

### Replay host attribution

`replay_{pack,upload,bind,encode,tile_index,other,submit_wait}_ns` partition host wall
time inside `frame_flush`: CPU packing/validation, staging and buffer writes/creation,
binding and pipeline creation, pass recording, tile binning, cleanup, and queue
submission/blocking GPU drains. Nested phases accumulate exclusively across batches;
serialized GPU waits are excluded from packing. `replay_bind_groups`, `replay_buffers`,
`replay_passes` and `replay_staged_bytes` count actual work inside that boundary, including
parameter buffers. Staged bytes are API payload bytes, not buffer capacity or GPU memory.
The report's **Replay host attribution** table uses `presenter.replay` snapshots around
that presentation's `ResidentTarget` call, including the first presentation, and gives
mean/p95/max plus signed residuals against its own `replay` scope; frames outside ±5%
are flagged. The earlier cumulative drawing samples include prior replay and checkpoint
flushes and cannot establish this residual. Legacy captures without the snapshots have
no replay attribution table. These are host wall times, including scheduling and waits,
and are distinct from GPU timestamps. No frame-time file I/O is added. Offscreen cells
work on a locked console and must be identified as offscreen when reporting results.

### Replay upload staging counters

Replay inputs use record, index and uniform rings, initially 2/8/1 MiB and capped
at 8/16/2 MiB. Reservations retain device-aligned bindings through their readers;
submit flushes pending writes before retiring the encoder. Overflow uses immutable
inputs, with a 32 MiB optimized overflow allowance before the compatibility path.
That inherited path has no new aggregate memory bound.

`presenter.replay` and `replay_host.counts` include:

- `upload_queue_writes`, `upload_queue_bytes`: actual flushed queue calls and bytes.
- `upload_queued_bytes`: logical payload reserved for staging, excluding padding;
  `upload_arena_dirty_bytes`: dirty interval bytes presented to arena coalescing.
- `upload_{records,indices,uniforms}_{capacity,used,high_water}`: byte gauges;
  `upload_padding_bytes`, `upload_ring_overflows`, `upload_overflow_bytes` and
  `upload_oversized_frames`: padding and fallback counters.
- `upload_<label>_{creates,create_bytes,reservations,payload_bytes,writes,write_bytes}`:
  label-level allocation and staging attribution. A reservation belongs to its
  input label; the resulting queue call belongs to its ring or `arena_flush`.
  `compatibility` collects other buffer labels. Field names are declared in
  [UploadCounters.h](../src/kfx/renderer/UploadCounters.h).

The arena retains a CPU image, validity bits and generation-tagged dirty intervals.
Coalescing never crosses GPU-owned scratch and spends at most 10% of dirty payload
on valid gaps. The retained CPU image is capped at 32 MiB. Larger inherited arenas use direct
compatibility writes; their GPU capacity is outside the normal-cell bound. The
normal path uses at most 32 MiB image, 1 MiB validity
bits and 6 MiB dirty metadata. Old upload ledgers still count requested resource
payload; `replay_staged_bytes` counts actual API payload, including alignment and
merged gaps, only when issued. Discard removes unflushed residency promises.
Upload staging and flushing remain inside `replay_upload_ns`; exceptional submits
retain the separate submit/wait phase. The profiler also accepts the previous
capture schema for a matched baseline. Performance and host parity numbers remain
pending; fixtures alone do not establish a speedup.

### Replay floor, measured 2026-09-15

[PR #44](https://github.com/zillakot/keeperfx/pull/44), source `15f8266ef`, binary
SHA256 `4dd145a150f02774ddd47579e5fed6187813591da1a8cbba7f8a3230b5927052`:
five Apple M5/Metal offscreen cells, locked console, guards enabled, Rust/wgpu,
VSync off, interpolation, 40 warmup/200 measured turns, nonserialized GPU timing.
This binary excludes PR #43. Means below are ms per presentation; capped cells have
600 samples, uncapped 2,563. Submit/wait within replay is zero in every cell.

| Cell | Replay | Upload | Pack | Tile index | Bind | Encode | Other | Residual |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| capped busy 640×480 | 2.504804 | 2.047045 | 0.318643 | 0.036235 | 0.054500 | 0.021620 | 0.026337 | 0.000425 |
| capped quiet 640×480 | 2.420650 | 1.945508 | 0.339768 | 0.038949 | 0.052044 | 0.020229 | 0.023545 | 0.000607 |
| capped busy 1920×1080 | 2.491032 | 1.964538 | 0.304512 | 0.108250 | 0.055521 | 0.031928 | 0.025863 | 0.000420 |
| capped quiet 1920×1080 | 2.487940 | 1.950395 | 0.315638 | 0.113182 | 0.053654 | 0.032553 | 0.022099 | 0.000419 |
| uncapped busy 1920×1080 | 1.736788 | 1.361645 | 0.208590 | 0.075169 | 0.040525 | 0.031014 | 0.019564 | 0.000281 |

Upload dominates replay at **78.39–81.72%**. Mean replay residual is 0.016–0.025%;
every frame is within 5%. Presentation residual is separate: 0.136–0.637%.
Busy HD replay buffers average **80.16**, with **93 at p95**; bind groups/passes
average 45.54, with 54 at p95. Staged payload is 5.260 MB mean, 6.104 MB p95.
The older drawing buffers counter omits three preparer buffers and is sampled
before the current replay; its 599 rows must not define replay residuals.

The code and counters reconcile about 12 raster uniforms, 10 shadow chains
(four buffers each), 2.6 ordered runs, 10 minimap calls (two buffers each), and
three preparer buffers in busy HD, plus roughly **138 queue writes per frame**.
The existing stream rings already avoid large command/tile allocations; small
serial inputs and uniforms still use create_buffer_init. Per-call buffer/staging
overhead is therefore the primary optimization target rather than more byte
reduction: earlier byte cuts did not produce proportional replay gains. This is
a code-supported inference, not an isolated allocation measurement; upload also
includes conversion and memcpy. Upload divided by buffers is 24.5–26.8 us capped,
17.0 us uncapped, an upper-bound attribution rather than a cost for each call.

Next: shared command/tile/uniform rings with coalesced writes, safe arena
coalescing, then bounded bind caching. Preserve byte contents to measure that
change; defer minimap/sprite byte work. One run per cell establishes no optimization
gain. All samples retain one submit, zero waits/checkpoints/errors and 32 MiB arena
capacity; uncapped reaches 256.321 FPS at 20.002772 turns/s. These are offscreen
observations, not acquired-surface performance or whole-process memory proof.

### Sprite asset interning, counters measured 2026-09-15

Branch binary SHA256
`82d7e6c516e350d7e855c2423dcf8a1aabe25a8d203d3fef69225e6a1782f163`, built with
`-DKFX_RUST_PRESENTER=ON`, against the reference run of source `c8687e108`, binary SHA256
`3359fe9a45a81a867e1cf48a274b795c9c6984acd00676764a2fd509b22ec0b4`
(`out/wgpu-migration/minimap-split-runs/drawing-coverage/`), which is master with the
minimap split and without this change. Identities are in
`out/wgpu-migration/sprite-interning-runs/sprite-interning-binaries.json`. Apple M5/Metal,
windowed control sessions under `scripts/drawing-coverage.py` with the drawing oracle on,
the timing lock held, packed arena. **Counters only: this run measures arena bytes and
residency, not frame time.** Both runs are 180 frames of `dungeon-busy`.

Per frame, `dungeon-busy`, reference → branch:

| Counter | Reference | Branch |
| --- | ---: | ---: |
| `gpu_sprite_commands` | 203.0 | 201.5 |
| `arena_sprite_hits` | 0 | 393.5 |
| `arena_sprite_misses` | 200.7 | 204.1 |
| `arena_sprite_bytes` | 481,176 | **97,654** |
| `arena_ordered_sprite_bytes` | 3,122 | 986 |
| `arena_cursor_bytes` | 2,119 | 2,119 |
| `arena_native_table_bytes` | 728 | 728 |
| `arena_bytes_uploaded` | 546,550 | 160,893 |
| `resource_snapshot_bytes` | 997,003 | 611,345 |
| `arena_bytes_resident` (gauge) | 27,166 | 33,727 |

Sprite arena bytes fall 4.93x, and total arena uploads fall 71% now that the minimap
split has already taken its own share out. A sprite command resolves two interned
resources, the artwork and the remap, and one per-call resource, the scaling ranges. Over
the run, **70,832 of 71,303 interned lookups hit, 99.3%**; counted instead against two
interned lookups for every one of the 36,271 sprite commands, 97.6%. The 36,742 misses are
36,271 per-call ranges plus 471 artwork and remap misses. `arena_misses_generation`,
`arena_misses_eviction`, `arena_misses_size_class`, `arena_evictions` and
`arena_overflows` are zero here and in the `front-view` and `dbc-text` scenes.
`arena_sprite_bytes` is 91,546 per frame in `front-view` (203.6 commands) and 9,260 in
`dbc-text` (7.6 commands).

`resource_snapshot_bytes` falls to 0.613 of the reference. That matches the prediction of
the slice's own spec, 0.605; the 0.35 acceptance bar contradicts it and was the figure
that was wrong. The drop, 385,657 bytes per frame, is this slice:
`arena_sprite_source_bytes` falls 383,521 over the same run. Subtracting every arena
kind's source bytes from the branch figure (160,893) leaves 450,453 per frame of host
copies that reach no arena kind at all, which nothing here touches.
`arena_bytes_resident` rises by the interned artwork, 4.89 → 6.07 MB against a 128 MiB
arena.

Parity: `scripts/drawing-coverage.py` on `dungeon-busy`, `front-view` and `dbc-text` is
complete with every gate counter zero — `failures`, `invalid_frames`,
`frame_flagged_invalid`, `verification_flagged_shades`, `rejected_commands`,
`rejected_spans`, `rejected_triangles`, `frame_rejected_checkpoints`,
`missing_cpu_barriers` — and 82,692 of 82,693 batches CPU-verified in `dungeon-busy`,
50,399 of 50,399 in `front-view` and 7,048 of 7,048 in `dbc-text`. No scene in the harness
draws a keepersprite with `water_source_cutoff != 0`, so the clipped-height key is covered
by the fixtures and not by a live scene. Run outputs are under
`out/wgpu-migration/sprite-interning-runs/drawing-coverage/`.

Timing was measured separately on the preceding head `3eb39f0d1`, recorded in
`out/wgpu-migration/sprite-interning-runs/timing-3eb39f0d1.md`: offscreen busy 1920x1080,
capped and uncapped matched pairs against a build of master `66c8e8764`, guards on, timing
lock held, 40 warmup and 200 measured turns, 20 turns/s, VSync off. Capped holds 60.00 FPS
in both pairs; uncapped is 288.1 and 265.8 FPS on the baseline against 289.9 and 292.1 on
the branch, inside a 22 FPS baseline spread, so no gain is claimed. `arena_sprite_bytes`
falls 543-561 KB to 69-72 KB per frame and `arena_bytes_uploaded` 63% capped and 79%
uncapped. Replay host time falls in all four pairs, 0.06 ms capped and 0.07-0.15 ms
uncapped, carried by the upload phase; consistent in sign but inside the capped pair
spread, so recorded rather than claimed.

### Minimap dispatch box, measured 2026-09-15

[PR #50](https://github.com/zillakot/keeperfx/pull/50), source `5ee89a7e9`, binary SHA256
`97088818a726beab08d860b17e52a020778d0fd36fbbb996afd5934f4e075aa0`, against baseline
source `b94fa46c7`, binary SHA256
`070e996cb0722b6a68490e61fdf36250db81679c1969c96fd9d139e2bd9d0637` (PR #47 head; PR #48
changes no pass behavior — a pure accessor rename in four modules and host-side dictionary
nibble encoding). Apple M5/Metal, offscreen
presentation (`--offscreen`, no swapchain), guards enabled, the timing lock held by the
schedule, VSync off, interpolation, 20 turns/s, 1920x1080, 40 warmup / 200 measured turns,
orders alternated within each pair. Capped and uncapped cells use `--gpu-timing`
(overlapped per-pass windows); the serial cells use `--serial-gpu-timing`, which drains
between passes and measures each family exclusively. Capped cells have 600 presented
frames, uncapped 2,465–2,799, serial 598 and 600. Every cell holds 20.000 turns/s, and
`frame_status_stalls`, `arena_overflows` and `gpu_untimed_passes` are zero throughout.

Overlapped cells, baseline → branch, `gpu_minimap_ns` and replay in ms per presented frame:

| Cell / pair | `gpu_minimap_ns` | `gpu_pass_union_ns` | Replay | FPS | Process CPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| capped-busy-1080 1 | 1.194 → 0.216 | 9.773 → 9.465 | 1.149 → 1.374 | 60.0 → 60.0 | 4.47 → 5.08 |
| capped-busy-1080 2 (cross-mode) | 1.101 → 0.212 | 9.455 → 9.294 | 1.355 → 1.394 | 60.0 → 60.0 | 5.48 → 5.11 |
| capped-quiet-1080 1 | 0.598 → 0.147 | 9.012 → 9.230 | 1.312 → 1.417 | 60.0 → 60.0 | 4.79 → 4.98 |
| capped-quiet-1080 2 | 0.611 → 0.148 | 9.193 → 9.125 | 1.317 → 1.496 | 60.0 → 60.0 | 4.84 → 5.16 |
| uncapped-busy-1080 1 | 0.453 → 0.080 | 3.909 → 3.429 | 1.054 → 1.044 | 246.5 → 279.9 | 4.16 → 3.95 |
| uncapped-busy-1080 2 | 0.441 → 0.082 | 3.807 → 3.523 | 1.047 → 1.053 | 253.0 → 272.4 | 4.18 → 4.01 |

The minimap pass falls 82% and 81% uncapped and 75–82% capped. Uncapped mean frame
interval improves in both pairs, 4.057 → 3.572 and 3.952 → 3.670 ms, with p95 5.289 →
4.717 and 5.061 → 4.871 ms, so the ceiling gain is not a mean-only artifact.
`arena_minimap_bytes` is byte-identical in every cell (559,427 busy, 557,507 quiet,
min = max), so the slice did not reach the upload plumbing, and
`upload_minimap_target_view_payload_bytes` is 160 B per dispatched command rather than the
spec's assumed ten commands.

**The capped-busy pair 2 row is cross-mode** and is not a matched comparison: its baseline
ran at 59.3 dispatches/frame against the branch's 44.6, the two workload modes
[the tooling plan](product/development-tooling-plan.md#2-offscreen-measurement-mode)
requires to be separated before anything is compared. Pair 1 is matched, 46.2 → 45.2.
Its minimap, union and replay figures are therefore reported but carry no comparison.

Serialized busy 1080p, exclusive per-pass means in ms per frame:

| Pass | Baseline | Branch |
| --- | ---: | ---: |
| raster | 1.147 | 1.029 |
| minimap | 0.992 | 0.914 |
| target triangles | 0.885 | 0.842 |
| shadow mask | 0.763 | 0.714 |
| ordered sprites | 0.223 | 0.241 |
| terrain prepare | 0.059 | 0.007 |
| union | 4.056 | 3.726 |

Four secondary gates are missed.

- The serialized union gate is "<= 4.000 ms **and** no other family worse than +5%". The
  union clause passes at 3.726 ms (-8.1%); the per-family clause fails on ordered sprites,
  0.223 → 0.241 ms (+8.1%).
- Serialized `gpu_minimap_ns` is 0.914 ms against a <= 0.700 ms gate: drained, the family
  is dominated by per-pass fixed cost, and the overlapped windows above carry the actual
  reduction.
- `dispatches` is gated at unchanged ±0.1/frame and falls about 1.0/frame in every matched
  cell (serial 45.53 → 44.57), because a zero-span overlay now submits no pass.
- Capped replay host means rise +19.6% busy pair 1 and +8.0% and +13.6% quiet against a
  <= +5% gate. The pack phase that computes the boxes grows only 0.007–0.036 ms and the
  rise sits in the untouched upload phase; the two busy baselines differ by 18% because
  they are in different workload modes, not because replay moved; and uncapped replay is
  unchanged (1.054/1.047 → 1.044/1.053 ms) with process CPU per frame lower.

Serialized terrain prepare also falls, a pass the slice does not touch, so treat that row
as drained-mode variation.

Parity on the same head: `KFX_WGPU_VERIFY` surface sessions verified 719 of 719 presented
frames at 640x480 and 712 of 712 at 1920x1080, 16 startup acquisition skips each, no
fallback; the `KFX_WGPU_DRAW_VERIFY` control session recorded 93,940 verified batches and
149,689 verified triangles at 0 failures, 0 invalid frames, 0 flagged shades and 0
rejected commands, with 1 `shadow_prior_divergence` event. That session skipped the video
mode cycle, which the control-tooling fault in
[native game control](native-game-control.md) makes unavailable. Run outputs are under
`out/wgpu-migration/minimap-box-runs/`. These are offscreen observations and establish no
windowed ceiling.

### Minimap payload split, measured 2026-09-15

[PR #54](https://github.com/zillakot/keeperfx/pull/54), branch
`perf/minimap-residency-split`, binary SHA256
`3359fe9a45a81a867e1cf48a274b795c9c6984acd00676764a2fd509b22ec0b4`, against the PR #50 head
`5ee89a7e9173f25e249b650415f3d9620401e55e`, binary SHA256
`97088818a726beab08d860b17e52a020778d0fd36fbbb996afd5934f4e075aa0`. Apple M5, macOS 26.6.2.

Oracle sessions. These are `scripts/drawing-coverage.py` control sessions with
`KFX_WGPU_DRAW_VERIFY=1`, one per binary, over the `dungeon-busy` and `front-view` scenes. Windowed, `VSYNC=ON`,
`TURNS_PER_SECOND=20`, `FRAMES_PER_SECOND=0` so draws interpolate between turns, smoothing
off, campaign `keeporig` level 20 in both scenes, `front-view` additionally launched with
`rotate_mode=2`. `dungeon-busy` runs at 640x480 and resizes through 800x600 and back;
`front-view` stays at 640x480. Both use `KFX_SCHEDULE_SKIP_CYCLE=1` and
`KFX_TIMING_LOCK_HELD=1` under a parent-held `/private/tmp/keeperfx-timing.lock`. They are
parity and upload-volume observations, **not timing runs**: no FPS, replay or per-pass
figure is established here.

Arena counters per presented frame, reference → branch:

| Scene | Frames | `arena_minimap_bytes` | `arena_minimap_misses` | `arena_minimap_hits` | `arena_bytes_uploaded` |
| --- | ---: | ---: | ---: | ---: | ---: |
| dungeon-busy | 179 → 180 | 475,046.3 → 12,079.7 | 8.497 → 5.261 | 0.000 → 14.294 | 1,004,132.5 → 546,549.8 |
| front-view | 129 → 130 | 485,707.0 → 16,442.2 | 8.690 → 6.546 | 0.000 → 13.454 | 990,895.7 → 521,475.6 |

**The miss and hit columns change unit between the two binaries and are not comparable.**
On the reference one minimap resolution is one command; on the branch it is one segment,
between 1 and 19 per command, so the reference could never hit — each command carried a
fresh `malloc` identity — and the branch's miss count has a different denominator. Only
the byte columns are unit-stable. Minimap is now the only arena kind whose resolutions are
renderer-private identities rather than public resources, so the per-kind description
below does not otherwise cover it.

Both sessions pass their gate on both binaries: 83,016 and 50,399 verified batches on
the branch, 0 failures, invalid frames, flagged shades and rejected commands, 0 arena
overflows and 0 arena evictions, with the documented single `shadow_prior_divergence`
event in `dungeon-busy` on both binaries. Arena capacity is
8,388,608 B on the branch against 4,194,304 B on the reference, live bytes 3,711,232
against 688,128, growth peak 12,582,912 against 6,291,456 — inside the 32 MiB capacity and
48 MiB growth-overlap gates.

Cache-memory scope. The split keeps a renderer-private content cache of immutable minimap
segments: one dictionary version, one cell version, 16 prefix versions and 4 versions per
style table. It is bounded in two dimensions — **5 MiB of arena source classes and 8 MiB
of the GPU bytes those classes occupy** — and the smaller of the two binds, so the packed
asset format runs against the 5 MiB source bound and the expanded format against the 8 MiB
GPU bound, which is 2 MiB of source classes and retains fewer style versions. Either way
the cache is at most 8 MiB of GPU bytes, a quarter of the arena's 32 MiB capacity gate;
both bounds are asserted at compile time. The source bound holds the full retention at the
validated maximum of 16 background colours **over a standard 256-subtile cell grid**; a
larger grid spends the budget on cells and retains fewer style versions until the split
gate keeps the command on the contiguous payload entirely. The budget is soft for the
single-version base roles: the dictionary and the cells are admitted even when they push
the total past it, so that table pressure cannot evict the base the next command needs, and
the overshoot is bounded by that base plus one segment.

Each entry also owns one host copy of its bytes, bounded by the same class budget, which is
separate from the whole-source copies C, the bridge and the renderer already stage per
command and separate from the arena's own host image. Three Rust-side gauges report the
cache — `minimap_cache_class_bytes` (live GPU bytes), `minimap_cache_cpu_bytes` (the owning
host copies) and `minimap_cache_evictions` (retirements the byte budget forced, not routine
version rotation) — and they are asserted in the drawing tests. Unlike PR #43's equivalents
they are **not** in the C counter ABI, so they do not appear in `KFX_WGPU_DRAW_STATS`
output: adding them there would touch the `DrawCounters` layout that PR #55 is changing.
The figures in this section are therefore a computed bound with test assertions behind it,
not a session measurement, and neither figure is process or driver memory.

Identity is complete byte equality, narrowed by role, layout, length and 16-byte samples at
each end of the segment. The samples reject a **changed** segment without reading it — the
routine style update rewrites entries 3, 4 and 10 of every table — but a **hit** still
compares the whole segment, and hits are the steady state: about 170 KB of comparison per
world command on exactly the frames that upload nothing. Whether that host cost is repaid
is what the deferred timing cells have to answer; these sessions do not.

The per-command minimap uniform grows from 40 to 60 words, so
`upload_minimap_target_view_payload_bytes` is 240 B per dispatched command rather than the
160 B recorded above for PR #50 — about 680 B/frame more at the busy scene's command rate.

The two sessions are separate control runs over the same scene scripts, so the frame counts
are not identical and these are matched scenes rather than matched frames. Run outputs are
under `out/wgpu-migration/minimap-split-runs/`.

Timing cells. Branch source `c8687e108385954ce040758bc0747ae5d3a8dcfb` (BUILD_NUMBER 5746),
binary SHA256 `3359fe9a45a81a867e1cf48a274b795c9c6984acd00676764a2fd509b22ec0b4`; reference
source `5ee89a7e9173f25e249b650415f3d9620401e55e` (PR #50 head), binary SHA256
`97088818a726beab08d860b17e52a020778d0fd36fbbb996afd5934f4e075aa0`. Apple M5, offscreen
presentation (`--offscreen`, no swapchain), busy scene at 1920x1080, capped and uncapped,
guards enabled, `KFX_SCHEDULE_SKIP_CYCLE=1`, the timing lock held by the schedule, 40 warmup
/ 200 measured turns, 600 replay-host frames per cell, `--gpu-timing`, two matched pairs per
cap mode with the order alternated. Per presented frame:

| Cell | FPS | Replay ms | Submit ms | Replay upload ms | `arena_bytes_uploaded` | `arena_minimap_bytes` | `arena_minimap_hits` | GPU union ms | Minimap pass ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| capped p1 reference | 60.0 | 1.402 | 0.636 | 0.877 | 1,132,642 | 559,427 | 0.0 | 9.352 | 0.231 |
| capped p1 branch | 60.0 | 1.430 | 0.622 | 0.846 | 775,290 | 214,002 | 14.1 | 8.611 | 0.208 |
| capped p2 reference | 60.0 | 1.394 | 0.641 | 0.872 | 1,122,157 | 559,427 | 0.0 | 9.416 | 0.228 |
| capped p2 branch | 60.0 | 1.442 | 0.643 | 0.860 | 779,767 | 213,996 | 14.1 | 9.303 | 0.235 |
| uncapped p1 reference | 298.1 | 0.932 | 0.385 | 0.572 | 1,118,125 | 559,427 | 0.0 | 3.213 | 0.083 |
| uncapped p1 branch | 277.9 | 1.034 | 0.502 | 0.584 | 628,327 | 46,164 | 21.0 | 3.458 | 0.084 |
| uncapped p2 reference | 294.9 | 0.954 | 0.394 | 0.587 | 1,112,593 | 559,427 | 0.0 | 3.246 | 0.083 |
| uncapped p2 branch | 297.5 | 0.917 | 0.380 | 0.516 | 608,907 | 43,135 | 21.1 | 3.229 | 0.084 |

All 8 cells complete, surface gates verified at 640 and 1080, drawing gate passed, waits 0
and `arena_overflows` 0 in every cell. Submits are 1.000/frame on the reference and 1.003
capped and 1.0004-1.0007 uncapped on the branch — one or two extra submits per 600-frame
cell, at session start.

Reading: capped stays on the 60 FPS cap in all four cells. `arena_bytes_uploaded` is down
31% capped and 45% uncapped, with `arena_minimap_bytes` 559,427 → 214,002/213,996 capped
and → 46,164/43,135 uncapped. FPS and replay are inside matched-run variability: the
uncapped FPS pairs disagree in sign (-20.2 and +2.6) and the two runs' reference cells span
278-298 FPS. No regression is shown and no gain is claimed; capped replay is +0.03 to
+0.05 ms on the branch in both pairs, within the 18% pair-to-pair spread recorded for
capped replay host time, and the minimap pass is unchanged. The first run, on the earlier
head `8c594954e33eddfba94eafaaf9969dcc02445a79`, reproduces the byte figures. Records:
`timing-c8687e1.md` and `timing-8c594954e.md` under `out/wgpu-migration/minimap-split-runs/`.

## Presenter host attribution

Rust-presenter runs include one `presenter.per_frame` sample per presentation;
SDL and older reports leave it unavailable. `acquire_ns` covers all of acquisition,
including polling and reconfiguration. `acquire_block_ns` covers only
`get_current_texture()` (all attempts), or offscreen ring-slot acquisition;
it is included in `acquire_ns`. `reconfigure_count` counts runtime reconfigurations.
`present_record_ns` covers `present_into` or `render_into`; `submit_ns` covers the
frame submit and present call and is included in `present_wait`. `replay_ns` times
`frame_flush` in the bridge; the `replay` scope also includes bridge orchestration.
Allocation calls and requested bytes are sampled across presentation, excluding
its replay child; they cover the Rust global allocator, not native or GPU memory.

`presentation_cpu = presentation - acquire_block_ns - present_wait` is computed
per frame before its mean and percentiles are taken, in milliseconds. It remains
host wall time, including scheduling, rather than process CPU time. The attribution
residual is `presentation - acquire_ns - present_record_ns - submit_ns`; report its
mean and fraction of presentation. Do not add the nested acquire block, present
wait, or the separately exported replay to that timer sum.

The frame-interval anchor and cursor composition/restore order are unchanged.
For comparisons with older GPU reports, use `presentation + replay` to recover
the previous presentation boundary. Drawing counters retain `upload_bytes` as
`asset_upload_bytes + command_upload_bytes`; existing gauges and GPU timestamps
keep their meanings. Counters are buffered and written only at capture completion.

### Presenter cost, PR A: measured 2026-09-14

Runtime `65e3ff9c2`, Apple M5/Metal, two 2560×1440 Acer displays at 75 Hz,
AC power, 200 turns/cell, guards enabled, load/core 0.169–0.325. Matched busy
1920×1080 capped pairs, all approximately 46 dispatches/frame; means in ms:

| Pair | Master presentation | Branch presentation | Branch replay | Buffers master → branch |
| --- | ---: | ---: | ---: | ---: |
| 1 | 3.859 | 0.657 | 3.186 | 90.40 → 88.39 |
| 2 | 3.871 | 0.656 | 3.051 | 90.41 → 88.39 |

Presentation + replay reconstructs the old boundary: this is chiefly attribution.
All capped matrix cells sustain 60 FPS and 20 turns/s; `presentation_cpu` is
0.013–0.025 ms. Mean attribution residual is 0.35–0.85% across branch timing
cells (worst individual frame 3.11%, below the 10% gate). Comparable pre-P4 HD
runs reduce scoped allocation calls 643 → 612/frame and present record
14.917 → 6.734–7.125 µs; requested allocation bytes do not uniformly fall.

Uncapped busy 1080p remains a separate comparison:

| Pair | Master FPS | Branch FPS |
| --- | ---: | ---: |
| 1 | 198.94 | 197.98 |
| 2 | 173.15 | 195.08 |

All sustain 20 turns/s; no ceiling increase is established from these two pairs.
Replay and submission remain: about 11 MB/frame asset uploads (94% of HD upload
bytes, P3) and `submit_ns` of 0.58–0.78 ms dominate the capped presenter work.
Drawable blocking is about 15 µs capped versus 0.33 ms mean / 1.47–1.79 ms p95
uncapped. These are host timings, not GPU execution costs.

Surface parity passes GPU/software at both resolutions: 715/710/717/716 frames
each equally verified, no mismatch or fallback. The corrected drawing oracle
verifies 171,918 batches and 202,743 triangles through camera input, pause/resume
and parchment return, with all required errors/rejections zero and one non-failing
shadow-prior divergence diagnostic. The first oracle attempt stopped on a helper
response-envelope bug at turn 5 and is excluded. Full tables, identities, tails,
load and both attempts are recorded in [PR #39](https://github.com/zillakot/keeperfx/pull/39).

Arena attribution adds `arena_<kind>_{bytes,misses,hits,source_bytes,distinct_lengths,length_overflows}`
for sprites, ordered sprites, cursor, general TRIG, terrain tiles/fades, native tables,
minimap, shadow descriptor/RLE, target-triangle geometry/tables, image/raw/tiled image,
movie, map view, bitmap, lens and other resources. Successful arena resolutions are
attributed to the calling packer; shared IDs charge bytes to the first missing use.
`bytes` counts uploaded GPU bytes and sums to `arena_bytes_uploaded` in every frame;
`source_bytes` counts unexpanded bytes on misses (the full packed sprite payload).
TRIG includes geometry and its packed texture tail, whose unexpanded miss bytes are
also `arena_trig_texture_source_bytes`. Distinct lengths count successful resolutions,
reset at queued-frame begin, and retain at most 64 lengths per kind; overflow counts
unretained observations, making distinct counts lower bounds when nonzero. Immediate
contexts without frame-begin retain that length set until destruction. Batch fallback
uploads are excluded. JSON and Markdown reports preserve all existing counters and
include the per-kind partition with a per-frame conservation check.

### Presenter cost, P3 slice 1: measured 2026-09-15

[PR #40](https://github.com/zillakot/keeperfx/pull/40), runtime `97402ca75`,
Apple M5/Metal, two unchanged 75 Hz displays. All 22 timing cells have one submit,
zero waits/checkpoints/invalid frames/arena evictions/overflows, and presentation_cpu
below 1 ms. Every branch sample has zero shadow-table uploads/misses and private
asset buffers; geometry is 480 bytes/pair. Remaining 3.9–4.5 MB/frame arena uploads
are all new-ID misses. The byte ledger conserves per frame.

Matched capped 1080p means; MB is decimal, replay in ms:

| Scene / pair | Asset MB baseline → branch | Replay baseline → branch | Buffers baseline → branch |
| --- | ---: | ---: | ---: |
| Busy 1 | 11.067 → 4.493 | 2.999 → 2.224 | 87.242 → 78.185 |
| Busy 2 | 11.080 → 4.482 | 2.757 → 1.958 | 87.242 → 77.228 |
| Quiet 1 | 11.400 → 3.911 | 3.122 → 2.107 | 81.159 → 69.694 |
| Quiet 2 | 11.400 → 3.911 | 3.108 → 2.033 | 81.127 → 69.716 |

Busy buffer creation bytes fall about 6.59 MB→15.1–15.7 KB/frame. Against PR39's
historical 640 cells, busy r1 changes 14.100→4.361 MB, replay 3.721→2.217 ms,
113.825→99.003 buffers and 9.743 MB→8.954 KB buffer bytes, with 14.8414 measured
shadow pairs/frame. Busy r2 emits fewer shadows (9.6194), so its 75.716 buffers
are a different workload. Quiet repeats upload 4.008 MB, replay in 2.163/2.143 ms.

Acceptance is partial: standalone busy HD and busy pair 1 slightly miss ≤78 buffers;
capped rates are 59.99–60.01 FPS with small strict-range misses. Busy pair 2 ends at
load/core 1.571 and 19.99435 turns/s. Guard interruptions and a manual session break
whole-schedule continuity; orders remain alternated within pairs. Capacity is
32 MiB with 48 MiB old+new growth overlap (16→32); transition-growth and driver/GPU
peak-memory gates remain untested. Creation bytes do not establish peak-memory savings.

Windowed uncapped is compositor/environment-paced this evening: three cells reach
74.93 FPS, the first baseline 84.75, with 7.31–10.03 ms acquire blocking despite
Immediate. These are not ceiling measurements; within-pair replay deltas are
−1.151/−1.089 ms. Keep the later offscreen comparison separate:

| Offscreen uncapped busy 1080p pair | FPS baseline → branch | Replay ms baseline → branch | Presentation ms baseline → branch |
| --- | ---: | ---: | ---: |
| 1 | 221.06 → 250.78 | 3.061 → 1.869 | 0.836 → 1.504 |
| 2 | 219.99 → 250.75 | 3.083 → 1.835 | 0.834 → 1.545 |

The faster branch waits more on the two-slot ring; these offscreen throughput gains
prove no windowed ceiling increase. Four surface gates pass (606/605 GPU and
610/605 software frames, each equally verified). Drawing attempt 1 ends in a
control-API SIGPIPE after DESKTOP switching, with zero drawing failures; attempt 2
verifies 135445 batches / 188535 triangles with zero errors and one non-failing
shadow-prior diagnostic. Its settle wait times out with zero presented frames and
247 acquisition skips (Fifo), then quit exits cleanly. Bridge readback verification
remains valid; surface proof comes from the separate gates. Video-mode round trip,
level reload and explicit table/palette mutation remain untested. Full target results,
tails, both attempts and interrupted-run history are retained in PR #40's delivery
record and the local `out/asset-reuse-report.md`.

## Collection bounds and outputs

The hook is inactive unless `KFX_PERF_OUTPUT` names a new CSV file whose parent
exists. The runner sets `KFX_PERF_TURN`, `KFX_PERF_TURNS` and
`KFX_PERF_SCENE=dungeon|possession` only for its child. It strips inherited capture
and profiling options, backend selectors and Rust fault/verification hooks;
simultaneous frame capture is rejected by the engine.
The runner rejects existing output directories and output outside ignored `out`.

The hook reserves at most 100,000 records (about 2.4 MB on a 64-bit build), takes
clock readings only while active, and performs no sample file I/O until the run
ends. Population/seed snapshots occur outside timed work. This is low-overhead
instrumentation by design, not a measured zero-overhead guarantee: timer calls,
record insertion and metadata queries still perturb execution. A future backend
must use the same boundaries and collector for a fair comparison.

Warmup is bounded to 1–600 turns and measurement to 20–1,200 turns. The runner
also enforces a wall-clock process timeout, covering startup failures, menus,
pauses, missing possession candidates or stalled turns. Sample overflow, invalid
options, wrong local-game/view state and failed presentation invalidate the run.
Only a complete engine sidecar plus validated samples can produce a report.

The output directory contains raw CSV, its engine JSON sidecar, JSON and Markdown summaries,
the isolated settings and process/game logs. The JSON summary includes the input
identity and request manifest. Keep complete local
results for later comparison; original game data and captured results stay under
ignored `out`. PRs may record aggregate timings and conditions without uploading
original assets.

## Uncapped measurement mode

```sh
python3 scripts/profile-game.py --scene busy --resolution 1920x1080 --uncapped --out out/perf-uncapped
```

`--uncapped` writes `FRAMES_PER_SECOND=0` into the isolated configuration instead
of `60`. Everything else is unchanged: `DELTA_TIME=ON`, `TURNS_PER_SECOND=20`,
`VSYNC=OFF`, the same scenes, warmup, durations and collector.

### What uncapped means in this engine

`FRAMES_PER_SECOND` is parsed by
[`parse_draw_fps_config_val`](../src/config_keeperfx.c) into
`start_params.num_fps_draw_main`, which
[`redetect_screen_refresh_rate_for_draw`](../src/main.cpp) turns into
`fps_limit_current`: `-1` means the display refresh rate, a positive value is that
value, and `0` leaves `fps_limit_current` at zero. The frame limiter in
[`gameplay_loop_draw`](../src/game_loop.c) is guarded by `fps_limit_current > 0`;
with zero it is skipped entirely, so nothing accumulates `process_frame_time`, the
`SDL_Delay(1)` in that block never runs and no frame is suppressed by the limiter.

The simulation rate is independent of it. With interpolation on,
`gameplay_loop_timestep` skips `keeper_wait_for_next_turn` entirely, so the only
pacing is `game.process_turn_time`, which accumulates elapsed seconds times
`turns_per_second`. Drawing fills the gap: `gameplay_loop_logic` spins
`gameplay_loop_draw` until the next turn is due. Uncapped therefore means drawing
as fast as the host allows while the simulation still targets 20 turns per second.

VSync is off on both presenters and independently confirmed. `vsync_enabled` is
false, so the SDL path calls `SDL_SetRenderVSync(renderer, 0)`
([`RendererSoftware`](../src/kfx/renderer/RendererSoftware.cpp)) and the wgpu path
selects `Immediate`, falling back to `Mailbox` and otherwise failing rather than
silently configuring `Fifo` ([`live.rs`](../tools/frame-replay/src/live.rs)). The
runner already rejects a Rust run whose reported present mode is neither.

### Runner and report contract

The engine reports `fps_limit_current` as `fps_limit` in its metadata sidecar. The
runner requires it to equal the requested cap: `60` by default, `0` under
`--uncapped`. A run whose engine did not apply the requested cap is rejected, in
both directions, so a stale configuration cannot be read as an uncapped result.

`report.json` carries a `frame_cap` object (`uncapped`, `requested_fps_limit`,
`engine_fps_limit`, `label`) and an `observed` object with the derived
frames-per-second and turns-per-second figures. `report.md` states the cap beside
its title, beside the engine frame limit, and after every frame-rate figure, and
reports observed turns per second beside observed FPS. The uncapped limitation
list replaces the capped-FPS caveat with its own.

For uncapped swapchain runs with presenter counters, `presentation_paced` is true
when mean `acquire_block_ns` exceeds 50% of the mean `frame_interval` (converted
from milliseconds to nanoseconds). `presentation_pacing.acquire_block_fraction`
records the ratio and `presentation_pacing.threshold` records `0.5`. This threshold
separates the 2026-09-15 compositor-paced evidence (7.3–10.0 ms acquisition in a
13.35 ms frame, about 55–75%) from the faster 195–199 FPS evidence (0.33 ms,
about 6.5%). It is a diagnostic heuristic; a false value does not prove an engine
ceiling, and `Immediate` present mode does not prevent compositor pacing.

A flagged cell remains `status: "complete"`. Its Markdown limitations explicitly
say it is compositor-paced and is not a ceiling; it remains useful for matched
host-work comparisons. Capped and offscreen reports, and reports without presenter
counters, omit these fields. A capped run already makes no ceiling claim.
For ceiling comparisons, use `--uncapped --backend rust --offscreen` on both sides
to remove drawable acquisition. Keep this a separate experiment: offscreen frame
intervals, presentation costs and FPS are not interchangeable with swapchain
results; see [offscreen measurement mode](#offscreen-measurement-mode).

`benchmark-presenters.py` accepts the same `--uncapped` and applies it to every
run in the experiment. `compare` refuses a set of reports that mixes capped and
uncapped runs, and the cap is part of each scene's recorded identity, so an
uncapped run can never be paired against a capped one.

### Limits

An uncapped figure is host wall-clock pacing of this process on this host under
the conditions of that run. It is not a portable frame-rate claim, and uncapped
numbers must never be compared with capped ones.

Observed turns per second is the load-bearing check. The measured window is paced
by the simulation, so a value below 20.00 means the host could not sustain the
simulation while drawing; such a run measures a degraded loop, not a drawing
ceiling, and its FPS figure describes that degraded loop.

Uncapped runs present far more frames than capped ones for the same turn count, so
the 100,000-sample limit is reached sooner; keep `--turns` bounded and check the
sample counts in the report. Presentation tails also widen, because the swapchain
is queried continuously rather than once per vertical interval.

### Recorded uncapped matrix

Source `56a5eee8b`, Apple M5, Metal, `Bgra8Unorm`, present mode `Immediate`, one
engine and asset set across every run, 200 measured turns each, collected serially
under the timing lock with audio, control/API and readback disabled. One run per
cell plus four repeats: descriptive, not a repeated experiment.

First, the cap semantics themselves, busy 640×480 on the SDL presenter, measured
on the earlier `e00fcd322` source:

| Frame cap | Frame interval mean / p95 ms | Observed FPS | Turns/s | Presentations |
| --- | ---: | ---: | ---: | ---: |
| 60 FPS | 16.665 / 17.491 | 60.01 | 20.00 | 600 |
| uncapped | 2.901 / 8.699 | 344.67 | 20.00 | 3,447 |

The cap holds the frame interval at the 16.67 ms period; removing it drops the
interval to 2.9 ms and leaves the simulation at exactly 20.00 turns per second.

| Presenter | Drawing | Scene | Logical | Frame interval mean / p95 ms | FPS | Turns/s | Draw mean | Presentation mean |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| wgpu | wgpu | busy | 640×480 | 7.011 / 14.012 | 142.64 | 20.01 | 0.643 | 6.288 |
| wgpu | wgpu | quiet | 640×480 | 4.925 / 8.166 | 203.05 | 20.00 | 0.605 | 4.280 |
| wgpu | wgpu | busy | 1920×1080 | 5.272 / 6.868 | 189.69 | 20.00 | 0.618 | 4.593 |
| wgpu | wgpu | quiet | 1920×1080 | 4.967 / 7.844 | 201.32 | 20.00 | 0.569 | 4.361 |
| wgpu | software | busy | 640×480 | 5.126 / 15.874 | 195.08 \* | 20.00 | 0.887 | 4.153 |
| wgpu | software | quiet | 640×480 | 4.845 / 15.541 | 206.41 \* | 20.00 | 0.814 | 3.989 |
| wgpu | software | busy | 1920×1080 | 4.438 / 12.575 | 225.34 | 20.01 | 1.970 | 2.412 |
| wgpu | software | quiet | 1920×1080 | 4.957 / 13.641 | 201.73 | 20.00 | 1.942 | 2.982 |
| SDL | software | busy | 640×480 | 3.462 / 12.784 | 288.85 \* | 20.00 | 0.695 | 2.712 |
| SDL | software | quiet | 640×480 | 3.445 / 12.275 | 290.29 \* | 20.00 | 0.706 | 2.707 |
| SDL | software | busy | 1920×1080 | 3.201 / 7.594 | 312.42 | 20.00 | 1.762 | 1.406 |
| SDL | software | quiet | 1920×1080 | 3.142 / 3.057 | 318.28 | 20.00 | 1.863 | 1.258 |

**Every cell measures a ceiling.** Observed turns per second is 19.995 to 20.011
throughout and FPS exceeds it by a factor of 7 to 16, so no row carries the
degraded-loop signature that made the two 1080p GPU-drawing rows of the earlier
`e00fcd322` matrix unusable.

**\* Contention-affected lower bounds.** The simulation scope is
resolution-independent, so matched scenes must agree; in these four cells it reads
1.7–2.0x its 1080p partner (0.325–0.383 ms against 0.191–0.212 ms busy), which is
host contention landing on them rather than a resolution effect. The four
GPU-drawing cells agree to 0.002 ms and all four 1080p software cells are clean.
Repeats of the 1080p SDL cell reproduce to 0.5%; repeats of the three 640×480
cells vary by 6.6–10.5%.

What bounds each measured ceiling:

- **GPU drawing is not bound by GPU execution.** A serialized capped run
  (`KFX_WGPU_GPU_TIMING=2`) puts exclusive GPU work at 1.60 ms busy at 640×480 and
  3.68 ms busy at 1920×1080, against frame intervals of 7.01 and 5.27 ms here, and
  1080p — with 2.3x the GPU work — reaches the higher ceiling. `draw` is
  0.57–0.64 ms, blocking device polls are zero, and the host spends 4.28–6.29 ms in
  `presentation`: surface acquisition, the per-frame uploads, the frame replay
  itself and the palette pass, not device execution and not the present call.
- **Software drawing on the wgpu presenter is bound by the presenter**, not by CPU
  drawing: at 1080p `draw` is 1.94–1.97 ms while `presentation` adds 2.41–2.98 ms,
  and at 640×480 `draw` falls to 0.81–0.89 ms with presentation still at
  3.99–4.15 ms.
- **Software drawing on the SDL presenter is the cheapest host-side path** in every
  cell (presentation 1.26–2.71 ms) and gives the highest ceilings here.

The resolution ordering inverted relative to the `e00fcd322` matrix, where 640×480
was far faster than 1080p. The 1080p side of that change reproduces to 0.5%; the
640×480 side carries both the contention canary and the repeat spread. **No cause
is claimed**: the merged drawing work and a changed display configuration (two
displays rather than one) are both candidates and were not separated.

PR #35 (one encoder and one submit per frame) landed after these runs and
supersedes them wherever its own body gives a figure: over three matched uncapped
pairs it moves the busy 640×480 GPU-drawing ceiling from 141 to 190 FPS, with the
1080p pair inside its run-to-run spread.

Every figure here is uncapped host wall-clock timing on this host.

## Offscreen measurement mode

```sh
python3 scripts/profile-game.py --scene busy --resolution 1920x1080 --backend rust \
  --offscreen --gpu-timing --out out/perf-offscreen
```

`--offscreen` selects `KFX_PRESENT_BACKEND=wgpu-offscreen`. The presenter builds its
adapter, device and palette pipeline exactly as the windowed path does, but renders
into a two-slot `Bgra8Unorm` texture ring instead of a swapchain: there is no surface,
no drawable and no dependence on an unlocked, unoccluded display. The SDL window is
still created — the mode, event and mouse layer hangs off it — and then hidden. The
output size comes from the logical framebuffer (`lbDrawSurface`), not the window, so
it is independent of window size, backing scale and visibility. The engine reports
itself as `wgpu-metal-offscreen` with present mode `Offscreen`, and the runner rejects
either half of that pair appearing without the other. It requires `--backend rust` on
native macOS and is incompatible with `--headless`.

Pacing is unchanged for capped runs: the frame limiter is engine-side and unrelated to
presentation. Uncapped runs are throttled by the ring instead of by `nextDrawable` —
before a slot is rendered into again the host waits for that slot's last submission, so
it stays at most two frames ahead of the GPU, the same depth as the swapchain's
`desired_maximum_frame_latency`. The ring waits on the frame's own submission, taken
from whichever renderer made it, and issues no submission of its own, so `submits` is
one per frame on both paths and remains comparable.

**Comparable with a windowed run:** `simulation`, `draw` and the whole
`--draw-breakdown` series; every drawing volume counter (`submits`, `dispatches`,
`upload_bytes`, `buffers`, `batches`, `commands`, `tile_entries*`, `arena_*`,
`prepared_row_*`); and the per-pass `gpu_*_ns` timestamps with `gpu_pass_union_ns`,
which live in the drawing context and never touched the surface.

**Not comparable:** `presentation` (it loses the `nextDrawable` wait and gains the ring
wait), `present_wait`, `frame_interval`, and therefore observed FPS on uncapped runs;
process CPU to the extent it tracks blocking. `gpu_present_ns` is the same shader at
the same size but writes a plain texture rather than a drawable: treat it as
same-order, not identical.

This is a measurement mode, not a player feature — nothing reaches the screen.
`report.json` carries `presentation_mode`, `report.md` states it beside the backend
line, and the limitation list gains the offscreen caveat.

### Environment guards and the timing lock

Both runners sample the environment and refuse a run they cannot measure. A refusal is
`status: "refused"` with `refusal: {reason, detail}` in `report.json`, distinct from
`status: "failed"`, and the sampled values are kept under `environment_guards`.

- `console_locked` — a locked console session (`CGSSessionScreenIsLocked` via `ioreg`)
  refuses the swapchain path, which cannot acquire a drawable behind it, and is allowed
  under `--offscreen`, which is the point of that mode. A probe that fails records
  `null`, which is not a refusal.
- `background_load` — the one-minute load average per core above `--max-load`
  (default `0.5`). It is a cheap guard, not a scheduler: a one-minute average both lags
  a job that just started and trails one that just finished by about a minute, so a
  build that ended moments ago still shows in it. Wait for it to fall rather than
  raising the threshold. It is sampled again at the end, and a late breach annotates the
  limitations rather than discarding a completed run.
- `occluded` — checked after the engine exits, over the measured window only: the
  `Rust surface acquisition skipped` marker in stderr, or zero presentation samples. A
  skipped acquisition inside the window calls `performance_failed`, which writes that
  marker, so the two cover the window between them. The presenter's own counts are not
  consulted: `renderer_details` is captured once on the first frame and must stay
  constant for the run, so it only ever carries a startup snapshot. End-of-run counts
  appear in the `Rust presenter shutdown after N frames, M acquisition skips` line in
  `keeperfx.log`.

`--ignore-guards` records `"ignored": true` and the findings, adds a limitation line,
and runs anyway. It never applies to the lock.

Every run holds `/private/tmp/keeperfx-timing.lock` from before the isolated tree is
prepared until `report.json` is written, so no build can start inside a timing window.
A second invocation prints the holder and blocks; its `report.json` records
`timing_lock` with the `waited_seconds` it spent waiting.
`benchmark-presenters.py` takes the lock once for the whole schedule and exports
`KFX_TIMING_LOCK_HELD=1` to each child, which records `held_by_parent: true` and does
not re-acquire — `flock` is per open file description, so a nested acquire would
self-deadlock. The lock file is never unlinked, because that would race a waiter that
has already opened it. It is advisory: a build started by hand still overlaps a timing
window.

## Repeated native A/B comparison

After all builds and other performance work stop, collect five serial pairs for
each scene with one executable containing both presenters:

```sh
python3 scripts/benchmark-presenters.py --out out/presentation-ab \
  --conditions 'Record display/scaling, power mode, window visibility and background load here'
```

Replace the conditions text with actual observations. The driver runs 30 separate
processes, alternates original/Rust order between pairs and rotates scene order.
Every run uses the same default warmup, duration and settings; optional
`--warmup-turns`, `--turns` and `--resolution` apply to both paths. Keep the window
visible and unobscured throughout. Never run simultaneous benchmark windows.

The driver rejects mismatched binary, asset, configuration, host or actual output
identities within each scene. It preserves raw per-run artifacts beneath `runs/`
and records order and conditions in `manifest.json`. Failures stop collection and
remain in place. Inspect state/population snapshots for workload differences;
the runner does not make startup randomness deterministic.

`comparison.json` and `comparison.md` give each run equal weight. They report the
median and range of run medians and run p95s, and paired percentage changes.
The median of run p95s is labelled as such; it is not a pooled p95. With original
duration O and Rust duration R, time saved is `100 * (O - R) / O` and speedup factor
is `O / R`. Negative time saved means a regression. Keep absolute milliseconds
beside percentages and report scenes separately. Five pairs support descriptive
results; varied signs or wide ranges warrant an inconclusive result or further
complete rounds, not a significance claim.

Both paths remain capped at 60 FPS by default. Lower presentation duration means
reduced host overhead; 60 versus 60 FPS does not prove an uncapped gameplay
speedup. `--uncapped` collects a matched uncapped experiment instead, and the
driver refuses to mix the two; its results remain a separate matrix and do not
reinterpret the capped one. Rust VSync-off measurements require actual Immediate
or Mailbox present mode; silent FIFO fallback is rejected.

The timing matrix does not replace the live milestone's pixel, lifecycle,
gameplay, audio and save/reload checks in the [Rust port plan](product/rust-port-plan.md).
The profiling runner disables sound and gameplay commands, so run those checks
separately. Scoped Rust allocation counts also leave total-process allocation
and memory comparisons outstanding.

## Recorded live presentation result

[PR #9](https://github.com/zillakot/keeperfx/pull/9) records the final 30 serial runs
on source `506b703a35b84f4adb1bbe92d5f109166b3f8481`: five pairs per scene,
200 simulation updates and 600 presentations per run. Both paths used Metal,
640×480 framebuffer/output, VSync off, 20 turns/s, interpolation on and a 60 FPS
cap. Control/API, audio and verification/readback were disabled. Rust used
`Bgra8Unorm` and Immediate mode; no run fell back or failed collection.

| Scene | Presentation median ms, SDL → Rust | Paired median reduction, median (range) | Paired p95 increase, median |
| --- | ---: | ---: | ---: |
| Quiet | 0.644 → 0.623 | 3.811% (-0.940 to 6.107%) | 6.636% |
| Busy | 0.634 → 0.604 | 1.038% (-15.914 to 17.456%) | 9.358% |
| Possession | 0.676 → 0.627 | 3.072% (-11.269 to 9.868%) | 9.336% |

Absolute values are medians of per-run statistics; percentages are calculated
within pairs before aggregation. Every scene's median-reduction range crosses
zero, and presentation p95 increased in all 15 pairs. Process CPU savings also
had ranges crossing zero in every scene. Both paths stayed near the cap; there is
no demonstrated uncapped FPS gain. These results supersede earlier pre-control
9–13% presentation improvement claims and do not justify changing the SDL default.

All 120 recorded artifact hashes, common engine/assets/config identities, sample
counts, scheduled order and aggregate recomputation were independently checked.
The experiment remains descriptive: startup RNG/populations varied, and background
load, temperature, visibility/occlusion and display placement were not continuously
controlled or independently verified. Rust allocation counters do not cover the
whole process or GPU. PR #9 retains the full tables and evidence boundaries.

The [active graphics migration](product/rust-port-plan.md#active-delivery-full-wgpu-drawing)
starts by separating CPU scene preparation, rasterization and overlays, evaluating
higher framebuffer resolution and choosing a pre-rasterization command boundary,
then continues through GPU implementation and complete drawing coverage. The
runner's [uncapped mode](#uncapped-measurement-mode) supplies the ceiling figures
that migration needs, in a matrix kept separate from the capped one. Existing
presenter results do not validate the drawing migration.

## Asset-free checks and headless validation

```sh
python3 -m unittest discover -s scripts/tests -v
python3 scripts/profile-game.py --headless --scene quiet --turns 20 --out out/perf-smoke
```

Unit tests use synthetic CSV and fake engines. The second command needs local
original assets and validates the collection pipeline using SDL dummy/software;
its report is explicitly labelled headless. It is **not native window performance**
and must not be compared to a live SDL/Metal or Rust surface baseline.
CI runs redistributable checks without original game assets. Native performance
runs remain local, and their results are not CI performance thresholds.

### Packed-arena measurement schema

Capture schema 2 records representation (`arena_representation`: 1 expanded_u32,
2 packed_u8) and transport (`upload_transport`: 1 queue, 2 mapped_copy) as gauges.
The same `presenter.replay` interval contains `arena_source_bytes`,
`arena_logical_upload_bytes` and `arena_transfer_bytes`. Logical bytes exclude
padding/gaps; transfer bytes include actual arena queue traffic. Snapshot GPU
copies are separate. Physical staging reconciles as queue + explicit copy +
initialized-buffer bytes. Older captures leave new measurements unavailable.

`upload_cpu_copy_ns` includes expansion and renderer staging-copy submetrics;
`upload_api_ns` measures wgpu calls. These are nested within Upload. CPU copy bytes
count renderer writes; wgpu's hidden memcpy is not directly timed. Direct staging
pool/copy counters remain zero on the queue baseline. Snapshot raster and future
snapshot packing have separate GPU categories. Optional `KFX_WGPU_PASS_TRACE`
writes bounded pass-instance durations at renderer destruction; use it only for
separate diagnostics, with zero dropped/untimed instances required for coverage.
Instrumentation overhead and host parity remain host measurement gates.

### Byte-packed arena format

Packed assets keep source byte offsets in the 112-byte command record. WGSL reads
four source bytes per storage word, including unaligned LE16/LE32 records. Targets,
command/index/uniform rings, rows and shadow slots remain u32. Independent arena
allocations own their aligned tail; GPU snapshots are packed into separate scratch
words before sampling. Queue transport and the retained HostImage remain: one
renderer copy plus wgpu's staging copy, with no direct staging pool.

`arena_bytes_uploaded`, miss/family bytes and asset upload bytes count represented
payload, excluding tail padding and coalesced gaps. Physical transfer, source bytes
and representation metadata provide the comparison. The normal matched 32 MiB
arena/image should become 8 MiB; this is not a process/GPU-memory measurement.
The existing source-validation ceiling is retained. The `packed-arena` Cargo feature
is enabled by default after both complete fixture matrices passed.
`--no-default-features` selects the expanded control; CI retains both builds.
Performance, snapshot operation budgets and acquired-surface parity remain host gates.
PR #43 is not included; it was closed as superseded by PR #54, whose split segments use
these byte accessors and whose cache budget is counted in arena source bytes so that the
same commands are admitted in both controls.

Packed LE16/LE32 reads fetch one storage word, or two when crossing a word
boundary; sprite colour/coverage pairs share the LE16 fetch. Minimap dictionary
inversion happens once per dispatch. Its private view is 144 bytes: four target
words followed by 32 words holding eight four-bit dictionary indices each.
Unknown background colours retain dictionary index zero. This uses the existing
uniform ring and adds no GPU pass or barrier; raw minimap sources stay unchanged.

Matched offscreen pairs, 2026-09-15 08:30–08:50. Word-once binary `b94fa46c7`,
SHA256 `070e996cb0722b6a68490e61fdf36250db81679c1969c96fd9d139e2bd9d0637`;
baseline [PR #45](https://github.com/zillakot/keeperfx/pull/45) head `416a46d59`,
SHA256 `fb4cd2c743a95ae2bf6ae338dc8918f5f72f9989fe8ca2d566db03570bda8901`.
Apple M5/Metal, offscreen presentation (`--offscreen`, no swapchain), guards
enabled, the timing lock held by the schedule, VSync off, interpolation,
20 turns/s, 1920x1080 or 640x480, 40 warmup / 200 measured turns,
`--gpu-timing` (overlapped per-pass timestamps), orders alternated within each
pair. Capped cells hold the 60 FPS limit over 599–600 presented frames; the
uncapped cells have 2,412–2,675. Every cell below is baseline → word-once;
replay, upload, pack and process CPU are means in ms per presented frame and
FPS is observed.

| Cell / pair | Replay | Upload | Pack | FPS | Process CPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| capped-busy-1080 1 | 1.976 → 1.360 | 1.476 → 0.838 | 0.278 → 0.293 | 60.0 → 60.0 | 6.13 → 5.67 |
| capped-busy-1080 2 | 1.960 → 1.323 | 1.464 → 0.807 | 0.277 → 0.290 | 60.0 → 60.0 | 6.09 → 5.54 |
| capped-quiet-1080 1 | 1.855 → 1.266 | 1.368 → 0.765 | 0.283 → 0.291 | 60.0 → 60.0 | 5.39 → 4.76 |
| capped-quiet-1080 2 | 1.837 → 1.248 | 1.355 → 0.743 | 0.279 → 0.293 | 60.0 → 60.0 | 5.37 → 4.78 |
| capped-busy-640 1 | 1.893 → 1.196 | 1.500 → 0.751 | 0.267 → 0.295 | 60.0 → 60.0 | 5.54 → 5.32 |
| capped-busy-640 2 | 1.945 → 1.301 | 1.540 → 0.832 | 0.275 → 0.310 | 60.0 → 60.0 | 5.62 → 5.63 |
| capped-quiet-640 1 | 1.818 → 1.084 | 1.390 → 0.669 | 0.292 → 0.288 | 60.0 → 60.0 | 5.36 → 4.31 |
| capped-quiet-640 2 | 1.774 → 1.193 | 1.357 → 0.743 | 0.283 → 0.311 | 60.0 → 60.0 | 5.24 → 4.74 |
| uncapped-busy-1080 1 | 1.722 → 0.992 | 1.313 → 0.598 | 0.228 → 0.224 | 241.1 → 253.2 | 5.13 → 4.06 |
| uncapped-busy-1080 2 | 1.612 → 0.904 | 1.247 → 0.547 | 0.216 → 0.210 | 255.8 → 267.5 | 4.41 → 3.46 |

Replay falls 0.58–0.73 ms in every cell and the capped cells stay on the 60 FPS
cap. Pack time falls in three rows: capped-quiet-640 pair 1 by 1.4% and both
uncapped pairs by 1.8% and 2.8%. It rises within +5% in two rows,
capped-quiet-1080 pair 1 by 2.8% and capped-busy-1080 pair 2 by 4.7%, and above
+5% in the remaining five: capped-quiet-1080 pair 2 by 5.0%, capped-busy-1080
pair 1 by 5.4%, capped-quiet-640 pair 2 by 9.9% and the busy 640x480 pairs by
10.5% and 12.7%. The largest absolute pack change is +0.035 ms, in
capped-busy-640 pair 2; no row moves more than 0.035 ms either way. The uncapped
1080p ceiling rises about 12 FPS in both pairs.

The serialized exclusive GPU union is still about +7%, 4.02 → 4.30 ms, so GPU work
per frame is higher even though host time is lower. Every timed drawing pass but
terrain prepare (0.065 → 0.059 ms) grows, the scene reaching no lens pass: shadow
mask 0.765 → 0.844 ms is the largest absolute growth at +0.079 ms, ordered sprites
0.220 → 0.265 ms the largest relative one at +20.1%, then raster 1.153 → 1.221 ms
(+5.9%), target triangles 0.865 → 0.931 ms (+7.6%) and the minimap 0.960 → 1.012 ms
(+5.4%), which stays the second most expensive pass. That figure is the separate `serial-busy-1080-baseline` /
`serial-busy-1080-branch` cells: busy 1920x1080, capped, `--serial-gpu-timing`,
which drains between passes and so measures each family exclusively over 587 and
581 frames. It is not comparable with the overlapped `gpu_pass_union_ns` of the
paired cells above.

Windowed parity on the word-once binary: KFX_WGPU_VERIFY surface sessions
verified every presented frame at 640x480 and 1920x1080 on both wgpu and
software drawing, 717–719 frames each with 16–18 startup acquisition skips.
Run outputs are under `out/wgpu-migration/byte-arena-runs/word-once/`.
