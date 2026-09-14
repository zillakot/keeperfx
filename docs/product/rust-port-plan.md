---
type: product
description: Phased plan for incrementally porting KeeperFX to Rust while preserving pixel art, gameplay and a playable reference build, with validation criteria for each stage.
---

# Rust port plan

The aim is to learn Rust and modernize KeeperFX incrementally while preserving the
original pixel-art appearance and game behavior. The long-term target is a
Rust-owned application and game implementation. Third-party libraries may still
contain C/C++; rewriting those libraries is outside this plan.

The graphics migration below is authorized for execution; later language and
gameplay phases remain proposals. Completed foundations are listed separately.
Delivery scope, findings and validation belong in the
[fork's pull requests](https://github.com/zillakot/keeperfx/pulls). Issues are
currently disabled in the fork, so this plan uses PRs as delivery records.

## Starting point

The [project overview](../architecture/project-overview.md) describes the current
implementation. These foundations already exist:

| Foundation | Evidence and limits |
| --- | --- |
| Native Apple Silicon C/C++ game | [PR #1](https://github.com/zillakot/keeperfx/pull/1): native launch, initial playtest and save/reload checks; broad campaign and cross-platform compatibility remain unverified. |
| Standalone Rust/wgpu frame replay | [PR #2](https://github.com/zillakot/keeperfx/pull/2): exact captured-frame comparisons and regression tests; supplies frozen pixel input, not world scene data. |
| Optional live Rust/wgpu presentation and native control | [PR #9](https://github.com/zillakot/keeperfx/pull/9): merged live Metal integration, exact surface comparisons, isolated game control and matched presenter measurements; CPU world drawing and the SDL default remain. |

The [frame-feedback workflow](../frame-feedback.md) is our visual comparison tool.
It establishes pixel correctness for captured input, not gameplay performance or
simulation equivalence.

## Proposed phases

| Phase | Outcome | Evidence required before expanding scope |
| --- | --- | --- |
| 1. Broaden the reference cases | Reusable visual fixtures and measurements for representative scenes | Menu, dungeon, possession, palette changes and interface cases; separately measured simulation, drawing and presentation costs |
| 2. Present live frames through Rust | Bounded Apple Silicon integration delivered in PR #9; broaden coverage separately | Preserve exact comparisons and SDL fallback; close the remaining coverage items below before claiming comprehensive platform validation |
| 3. Replace one bounded utility | A small live engine component implemented in safe Rust behind a narrow interface | Valid-input equivalence, malformed-input tests, bounded allocations and verified ownership at the language boundary |
| 4. Establish world-state ownership | Typed Rust data for a bounded part of the world, with explicit adapters to legacy state | One authoritative writer, stable identifiers and verified save/network conversion for the affected data |
| 5. Migrate gameplay systems | Move rules and simulation one system at a time | Replayed command sequences produce equivalent gameplay state; save/load, Lua and multiplayer checks cover each migrated system |
| 6. Move application ownership | Rust owns startup, game orchestration and the remaining game implementation | Agreed campaign, input, audio, rendering, save/load and multiplayer coverage; a distributable build without the KeeperFX C/C++ implementation |

The [audio modernization plan](audio-modernization-plan.md) defines a parallel
track for sound remastering and replacement, audio compatibility fixtures and
gradual Rust ownership. Its initial audition pack can use the existing engine;
live audio and graphics integration share explicit lifecycle checks.

The active graphics task is full wgpu drawing, delivered through the gates below.
Utility and gameplay migration remain separate tracks; GPU world drawing does not
require transferring simulation ownership to Rust. Later phases require their own scoped designs.

## Delivered milestone: optional live Rust presentation

[PR #9](https://github.com/zillakot/keeperfx/pull/9) merged the optional live adapter
and stable [native game control CLI](../native-game-control.md). SDL owns the window
and event handling; Rust presents borrowed indexed rows and palette through the
shared pipeline. The existing C/C++ renderer still draws the world, menus and HUD.
SDL remains the default and terminal fallback; the Rust build and selection are
explicitly opt-in. Both presenters already use Metal on the measured Mac.

The final measured source was `506b703a35b84f4adb1bbe92d5f109166b3f8481`, merged as
`ccaff3f52f4f9eda98c5cbb0a0c7d187ff27b4af`. The
[final 30-run results](../performance-baselines.md#recorded-live-presentation-result)
supersede earlier 9–13% presentation improvement claims. Small median reductions
had ranges crossing zero, every pair regressed at p95, and neither consistent CPU
savings nor uncapped FPS gains were established. This proves Rust integration;
it does not justify changing the default or claiming a graphics speedup.

Final-source native-event validation completed menu/options/load navigation,
fullscreen changes, arbitrary resize, pause, minimize/restore, focus changes and
normal quit. All 833 acquired frames matched live GPU comparison without fallback.
Control authentication, cancellation and subscription teardown regressions passed.
Separate earlier audits covered palette/alpha/padded-row and scaling comparisons,
initial/runtime SDL fallback, API-driven digging, possession and save/reload, plus
nonzero game-generated PCM. Those audits have distinct provenance; they are not
all final-source physical-input or complete audio coverage.

Remaining coverage includes physical OS input delivery, transitions between distinct
physical backing scales, additional targeted surface/device recovery failures,
complete music/speaker behavior, total-process/GPU allocation measurement and live
Rust support beyond the tested Apple Silicon configuration. The broad phase-2
criteria are therefore not all closed. These items constrain support/default-change
claims; they do not block a separate rendering investigation. See the
[live guide](../live-rust-presentation.md) for the implemented ownership and failure
contracts and PR #9 for the detailed evidence.

## Language boundaries and small components

Use a C-compatible function boundary between the C/C++ engine and Rust. The
[Rust FFI guidance](https://doc.rust-lang.org/nomicon/ffi.html) explains the
interoperability and safety obligations. For each boundary, define:

- C-compatible record layout (`#[repr(C)]` in Rust), fixed-width data fields, buffer lengths, alignment and valid identifier ranges.
- Which side allocates, owns and frees each resource; use opaque handles for Rust-owned objects.
- How long borrowed input is valid, and whether the callee copies or retains it.
- Error results, panic behavior, callback lifetime and threading requirements. Rust panics and C++ exceptions must not unwind across the C ABI; ordinary errors need explicit return values.

Keep unsafe operations in a small adapter with safe Rust interfaces behind it.
Do not expose `Vec`, `String`, Rust references or C++ containers as ABI types, or
create references to potentially unaligned packed records. A Rust wrapper alone
does not make the remaining C/C++ implementation memory-safe.

A candidate first utility is the RNC decompression code in
[bflib_dernc.c](../../src/bflib_dernc.c). Confirm the boundary before choosing it:
the existing interface includes in-place buffers and error/ignore flags. A port
must preserve valid asset decoding while specifying safe behavior for truncated,
corrupt or oversized inputs. Compare against known valid outputs; do not reproduce
undefined behavior from the old implementation as a compatibility requirement.

Keep the original implementation available for comparison until the replacement
passes its focused tests and game-data loading checks. Synthetic or redistributable
fixtures belong in CI; original game assets remain local.

## World state, gameplay and compatibility

The global [Game structure](../../src/game_legacy.h) is the largest coupling point.
Do not translate it wholesale into shared mutable Rust globals. Start with explicit
read-only views or copies, then transfer ownership of one bounded part at a time.
During a transition, exactly one implementation commits state changes.

Before changing state layout, inventory the relevant
[save code](../../src/game_saves.c), [packet code](../../src/packets.c) and
[network resynchronization](../../src/net_resync.cpp). Saves currently write some
C structures directly. Rust's internal memory layout must not become a file format:
use explicit encoding or compatibility adapters. Any intentional save/protocol
change needs a version and migration policy in its implementation design.

Build deterministic simulation comparisons before migrating gameplay. Fix the
starting state, command sequence and relevant random seeds, and compare semantic
state such as positions, health, resources, jobs and timers after each turn.
Exclude addresses, padding and purely local rendering state from those comparisons.
Shadow runs must suppress external side effects such as duplicate audio, file
writes and network sends.

Begin with a small rule system, such as research or manufacturing, after checking
its dependencies. Leave creature job selection, combat and navigation until the
comparison harness can expose divergence reliably. Preserve update ordering,
integer arithmetic, random-number consumption and Lua behavior. Rust borrowing
and threading changes must not silently change simulation order.

## Graphics and performance track

Preserve the original sprites, textures, palette lookup, nearest sampling, draw
order and pixel coverage. GPU world drawing needs scene or draw-command data
**before rasterization**. Replaying the finished framebuffer only changes
presentation; it cannot remove CPU terrain or sprite drawing. The implemented
foundation uses ordered indexed commands and integer compute drawing, with one
GPU-owned `u32` palette index per pixel. CPU tile lists retain submission order;
each GPU invocation exclusively owns a destination pixel. The live integration
currently synchronizes that target with remaining CPU drawing. Full GPU target
ownership now has a queued-frame implementation; complete native coverage and
performance acceptance remain open. The target architecture for that path — one
command stream, one asset arena, one encoder and one submit per frame — is the
[single-stream wgpu renderer design](../architecture/wgpu-single-stream-renderer.md).
The tooling and process improvements that the measurement and review work
identified are tracked separately in the
[development tooling plan](development-tooling-plan.md).

### Active delivery: full wgpu drawing

The user authorized the full drawing migration on 2026-09-13. Inventory and
measurement are the first gate, followed by command extraction, GPU implementation,
all drawing paths and native validation. A single family or framebuffer presenter
does not complete this scope. The foundation lands opt-in so the software path
stays default while the remaining gates are worked; record exact source and
evidence at each gate without marking untested paths complete.

### Status: 2026-09-14

The objective is full Rust/wgpu ownership of drawing, with the C/C++ drawing
retired afterwards, exact indexed parity and 60 FPS at a 1920×1080 framebuffer.
[PR #16](https://github.com/zillakot/keeperfx/pull/16) merges to `master` as the
opt-in foundation of that work: software drawing and SDL presentation stay the
default, `KFX_RUST_PRESENTER` and `KFX_DRAW_BACKEND=wgpu` select the GPU path,
and the restructuring below ships as small PRs against `master`.

The tested runtime is `33ff16a4f65d1c310194d1ca7dd367f027d7ad0f`; executable SHA256
`045ba991be5ebc193e659c4ba592a06e7418cdfed73fb7918b50dadf872f7de8`.
It was built with AppleClang 21, release Rust 1.98.1 and native RelWithDebInfo.
The later `2afba22c2` changes only fixture prerequisites and generated audio cue
references. Original assets and the earlier playable binary remain unchanged.

Native validation found and fixed resident presentation bypassing
`KFX_WGPU_VERIFY`. The rebuilt runtime passed 94/94 acquired-surface comparisons
and 76,859 independent drawing-batch comparisons, with zero drawing failures,
invalid frames or missing barriers. A separate production queued-cursor run
passed 21/21 surface comparisons. Parchment, resize and cursor checks passed;
save/reload and compound-lens evidence also exist at their separately recorded
runtime identities. This is not complete coverage of every view, language, asset
or failure path.

Clean serial runs used the same binary and wgpu Metal presenter, 640×480 framebuffer
and output, VSync off, 20 turns/s and a 60 FPS cap. Verification, control/API,
audio and per-frame statistics-file I/O were disabled. Each run measured 200 turns.

| Scene | Software draw mean | GPU draw mean / p95 | Software / GPU observed FPS |
| --- | ---: | ---: | ---: |
| Quiet | 0.894 ms | 23.535 / 25.494 ms | 60.01 / 31.62 |
| Busy | 0.949 ms | 20.179 / 23.175 ms | 60.00 / 33.75 |

The earlier synchronous prototype reached only about 7.6–7.7 FPS in its recorded
clean runs. Batching removed ordinary per-command framebuffer transfers in the
exercised paths, but the current GPU path still misses the software reference.
These are host wall timings, not GPU timestamps or uncapped throughput; seeds,
evolved populations and host conditions differ. Continuous unobscured window
visibility was not independently established. Three zero-presentation surface
acquisition attempts were rejected and contribute no timing claims.

#### Terrain binning, 2026-09-14

Serial matched pairs on the wgpu presenter with `KFX_DRAW_BACKEND=wgpu`, busy scene,
200 measured turns, VSync off and a 60 FPS cap, before and after terrain tile binning on
the same host and assets. Per-pass GPU time is from `TIMESTAMP_QUERY`; every other figure
is host wall clock. Simulation stayed at 0.343/0.317 ms and 0.325/0.312 ms across the
pairs, so host contention did not move between runs.

| | 640x480 before | 640x480 after | 1080p before | 1080p after |
| --- | ---: | ---: | ---: | ---: |
| GPU raster | 4.489 ms | 4.437 ms | 44.891 ms | 56.890 ms |
| GPU terrain setup | 1.164 ms | 0.042 ms | 1.737 ms | 0.036 ms |
| GPU terrain raster | 4.076 ms | folded | 24.656 ms | folded |
| GPU all passes | 14.005 ms | 9.292 ms | 88.070 ms | 87.807 ms |
| Terrain iterations | 308 M *derived* | 1.85 M | 2,065 M *derived* | 7.85 M |
| Dispatches / submits | 128.7 / 83.9 | 59.1 / 45.2 | 113.1 / 76.1 | 48.6 / 39.1 |
| Draw mean | 1.550 ms | 1.484 ms | 1.409 ms | 1.332 ms |
| Presentation mean | 16.258 ms | 5.706 ms | 103.163 ms | 76.825 ms |
| Frame interval / FPS | 18.197 ms / 54.95 | 16.667 ms / 60.00 | 105.046 ms / 9.52 | 79.030 ms / 12.65 |
| Turns/s over the window | 20.02 | 20.03 | 9.57 | 13.98 |

A quiet 1080p pair on the same builds: GPU all passes 76.05 → 59.57 ms, presentation
91.66 → 62.06 ms, 10.74 → 15.75 FPS and 10.79 → 15.83 turns/s, with `terrain_tile_entries` 33,588, i.e. 8.60 M
iterations. Terrain is a larger share of a quiet scene, so it gains more there.

640x480 reaches the 60 FPS cap with 20.03 turns/s. 1080p gains a third but stays 4.7x
short. The 1080p GPU total did not fall: the terrain pass's 26.4 ms became about 12 ms of
extra raster time, and the minimap and ordered-sprite passes took the rest back — those
passes are unchanged by this work, so the shift is either scheduling or an effect of the
halved submission count, and it is not explained here. Per-pass GPU windows include time a
pass spends stalled on its dependencies, so they attribute cost rather than decompose it.

#### Tight bin boxes, 2026-09-14

Serial matched pairs on the wgpu presenter with `KFX_DRAW_BACKEND=wgpu`, busy scene,
200 measured turns, VSync off, a 60 FPS cap and `--gpu-timing`, before (`dc31cabe0`) and
after tight bin boxes, on the same host and assets. Simulation stayed at 0.343/0.362 ms
and 0.315/0.334 ms across the pairs, so host contention did not move between runs.

| | 640x480 before | 640x480 after | 1080p before | 1080p after |
| --- | ---: | ---: | ---: | ---: |
| `tile_entries` | 126,750 | 9,912 | 897,385 | 46,196 |
| Pixel-command evaluations | 32.4 M | 2.5 M | 229.7 M | 11.8 M |
| `tile_entries` sprite / trig / terrain / clear | — | 1,135 / 347 / 7,230 / 1,200 | — | 6,184 / 1,193 / 30,660 / 8,160 |
| GPU raster | 5.223 ms | 1.287 ms | 46.124 ms | 3.069 ms |
| GPU shadow triangles | 1.314 ms | 0.685 ms | 7.876 ms | 0.982 ms |
| GPU minimap | 3.175 ms | 0.869 ms | 16.901 ms | 2.436 ms |
| Sum of pass windows | 11.033 ms | 4.060 ms | 76.804 ms | 7.779 ms |
| `gpu_frame_ns` | absent | 4.060 ms | absent | 7.779 ms |
| Presentation mean | 4.748 ms | 5.473 ms | 76.369 ms | 4.681 ms |
| Frame interval / FPS | 16.666 ms / 60.00 | 16.667 ms / 60.00 | 78.133 ms / 12.80 | 16.667 ms / 60.00 |
| Turns/s over the window | 20.02 | 20.02 | 12.86 | 20.04 |

A quiet 1080p pair on the same builds: `tile_entries` 755,382 → 48,938, raster
30.874 → 2.861 ms, sum of pass windows 51.530 → 6.741 ms, presentation 60.340 → 4.476 ms,
16.16 → 60.00 FPS and 10.78 → 20.03 turns/s.

Both 1080p scenes now reach the 60 FPS cap and 20 turns/s, so the remaining GPU cost is no
longer what bounds the frame; the cap is. The minimap window fell with everything else
without any minimap change, which confirms that its earlier 16.9 ms was attribution rather
than cost. A serialised repeat of the busy 1080p run (`--serial-gpu-timing`) puts the sum
of pass windows at 3.648 ms against 7.779 ms unserialised — over half of the per-pass
window total is dependency stall, not work — while presentation rises to 15.375 ms because
the host now blocks on the queue, so that mode measures attribution, not throughput.

Validation: 99,298 GPU batches verified against the CPU oracle with 0 failures under
`KFX_WGPU_DRAW_VERIFY=1` on level 20, and a `KFX_WGPU_VERIFY=1` session presented and
verified 546 of 546 acquired surfaces, which earlier sessions could not reach.

#### HD measurement

The same binary, presenter and settings, capped host-wall timing at a 1920×1080
framebuffer:

| Scene | Software draw mean / FPS | GPU draw mean / FPS | GPU presentation mean |
| --- | ---: | ---: | ---: |
| Quiet | 3.463 ms / 60.00 | 68.266 ms / 8.76 | 45.7 ms |
| Busy | 3.318 ms / 60.00 | 70.442 ms / 8.00 | 54.2 ms |

The GPU runs did not sustain 20 turns/s at 1280×800 or 1920×1080. Process CPU
stayed flat while wall time grew, so the added cost is waiting rather than
computation.

#### Diagnosis and next step

The current path performs about 125 queue submits, 17 blocking waits and 9
checkpoints per frame, re-uploads immutable assets every frame (about 20 MB) and
runs shadows as synchronous readback chains. The measured presentation cost is a
checkpoint drain inside the presentation scope. Removing scaffolding cannot reach
the target: the next step is a restructure to one ordered command stream per
frame, a persistent asset arena, one or two submits and zero blocking waits,
keeping the existing exact kernels.

Per-frame acceptance metrics for that restructure: at most 4 submits; zero
blocking waits outside verification and screenshots; at most 1 checkpoint;
steady-state asset upload under 1 MB; allocation traffic under 1 MB; at most 3
full-target dispatches; parity unchanged.

Next PRs, in order:

1. Measured-window drawing-backend, submit, wait and transfer counters, plus the
   two free fixes (in progress).
2. Shadows as queued commands rather than synchronous readback chains (landed: with
   the counters PR and bridge batching, per-frame checkpoints fall 9.4 → 1.0, blocking
   waits 29.8 → 2.9 and shadow readback bytes 2.43 MB → 0 at busy 640x480; the two
   remaining waits are the bridge's full-target readbacks for CPU presentation).
3. Non-blocking validation and no double copy (landed: the validation flag lives in the
   raster kernels and reaches the CPU through a mapped ring one or two frames later, and
   queued frames write the root directly). At busy 640x480 with GPU drawing behind the SDL
   presenter: blocking waits 3.0 → 2.0 per frame, aggregate validation waits 1.0 → 0,
   checkpoint copy bytes 2.46 MB → 0, submits 116.7 → 76.4, buffer allocations 241.6 → 200.6,
   frame interval 19.75 ms → 18.58 ms and observed 50.6 → 53.8 frames/s. The two remaining
   waits are the CPU presenter's full-target readbacks; the wgpu-presenter pair is outstanding.
4. One command stream per frame in root space with one tile index (landed: every record
   names the view it was issued against, so views are offset aliases of the root and a
   target change no longer flushes; one counting sort over renderer-owned scratch bins the
   whole frame, and each serial segment rasters once over the tiles its own records reach).
   At busy 640x480 with GPU drawing behind the SDL presenter, matched pairs: bridge
   target-change flushes 39.3 → 0 per frame, tile-list allocations 0, buffer allocations
   198.5 → 161.3, Rust allocator calls 30.0 M → 5.7 M per measured window, CPU drawing
   0.506 → 0.470-0.531 ms and presentation 0.943 → 0.884-0.903 ms. The allocator drop is the
   per-batch tile lists this step deletes: a wgpu-presenter profile of `master` attributes 15%
   of `frame_flush` samples to `RawVec` growth, and the frame's record buffers are now reused
   across frames as well. **Rust batches did not
   fall** (72.5 → 71.7-75.1): ~38 creature shadows per frame each close a raster segment, so
   the design's "≈ 4" needs the mask chain hoisted as well as terrain, ordered sprites and
   the lens/minimap folds. GPU blocking wait rose 9.08 → 10.91 ms per frame and observed FPS
   fell 56.5 → 54.5-56.1. That cost is unattributed: the record layout and root-space tile
   misalignment were both measured and rejected (the latter at 3.5% more tile entries than a
   view-space binning of the same frame). Under the wgpu presenter on `master` the frame is
   GPU-bound — 82% of main-thread samples in the swapchain wait at 1920x1080, 9.8 FPS, with
   drawing at 0.5-0.7 ms and no blocking waits — so the presenter pair is the measurement that
   decides whether this matters, and it is outstanding.
5. The rest of the single-stream restructure, guided by the design document under
   [`docs/architecture/`](../architecture/).

Coverage work remains independent of performance: arbitrary Lua pixel/batch
drawing, general striped-line coverage, remaining valid-input/alias domains and
persistent offscreen/scratch ownership are still open. Passing current fixtures
or reaching a frame-rate target does not complete the full drawing goal.

### Inventory and measurement

Entry: start from the latest fork `master`, retain PR #9 as the presentation
baseline, and record the exact source/binary/assets/settings used. Reuse the
[performance collector](../performance-baselines.md) and isolate original assets
and saves. Use [native game control](../native-game-control.md) for separate
correctness checks; disable control, API, audio and verification/readback during
performance collection.

Work and deliverables:

1. Inspect the existing simulation/draw/presentation distributions before adding
   measurement. Break down CPU drawing into scene preparation, rasterization and
   HUD/overlays where the actual call graph allows it. Start at
   `keeper_screen_redraw()` / `redraw_display()` in
   [engine_redraw.c](../../src/engine_redraw.c), then `draw_view()` and
   `display_drawlist()` in [engine_render.c](../../src/engine_render.c).
   `draw_view()` prepares bucketed work before `display_drawlist()` dispatches
   polygons and sprites into the [software rasterizers](../../src/kfx/renderer/software/).
   Front view has a separate `draw_frontview_engine()` path. Check each view;
   do not assume one boundary covers every camera or effect.
2. Use bounded coarse timing scopes and, if needed, a sampling profiler to locate
   expensive rasterizer families without timing every pixel/primitive. Preserve
   existing scope meanings in [performance_capture.cpp](../../src/performance_capture.cpp)
   and the reports. Label inclusive/nested times, report unaccounted drawing work
   and measure instrumentation overhead with a matched control. CPU drawing wall
   time, process CPU time and GPU timing are different measurements.
3. Collect serial matched quiet/busy/possession runs at the existing 640×480
   baseline and one validated higher framebuffer resolution (candidate 1280×800).
   Record actual framebuffer and output dimensions separately: enlarging only the
   window does not increase world rasterization work. Reuse the five-pair protocol
   when comparing presenters; inspect camera/population/RNG differences and record
   display, visibility, power and background conditions. Keep capped results
   separate. The current runner fixes 60 FPS; an uncapped experiment requires an
   explicit, tested runner/metadata extension and validation of the engine's cap
   semantics first. Run a complete matched uncapped matrix before any FPS claim,
   or record that experiment as deferred with a reason.
4. Trace one costly terrain or sprite family from command creation to pixel writes.
   Sketch the smallest immutable command input: geometry or screen-space vertices,
   camera/clip state, texture/sprite identifiers, palette/shade tables, ordering,
   dimensions and referenced data lifetimes. Identify global-state reads and
   effects that escape the buckets. Existing bucket records contain legacy
   pointers; they are candidates to adapt, not an approved Rust ABI or retained
   scene representation. Compare extraction at that boundary with a narrower
   rasterizer input; choose only after measuring cost and compatibility needs.

Inventory acceptance: identify which measured work limits each scene, include
absolute costs and distributions with workload/identity limits, and record every
drawing family's callers, input, ownership and pixel rules. Choose the first
extraction boundary and fixtures from this evidence, then continue through the
remaining gates. A lack of measured speedup must be reported; it does not establish
migration completion or justify an FPS claim.

Validation for changed measurement code must cover sample completeness, nesting,
metadata/config mismatch rejection and preservation of default capped behavior.
Run relevant collector/runner tests and existing CI; use an isolated native smoke
check for changed hooks. Do not treat dummy/headless runs as native performance or
run builds/profilers concurrently with benchmark collection. Keep original artwork,
raw captures, session descriptors and private host details out of the PR; publish
portable procedures, aggregate evidence and redistributable fixtures.

### Execution and coverage ledger

This ledger records the completed 2026-09-13 implementation slices and their
separate validation identities. It does not certify the combined development head.
The [live guide](../live-rust-presentation.md#partial-gpu-drawing) describes selection,
ownership, synchronization, counters and failure behavior.

| Gate | Status | Evidence and remaining work |
| --- | --- | --- |
| Inventory and measurement | Partial | Per-pass GPU execution time is collected on request (`KFX_WGPU_GPU_TIMING=1`, `profile-game.py --gpu-timing`), and `tile_entries_<kind>` splits the binning index per record kind. A pass window includes the stalls that pass waited through, so the windows overlap and their sum is not a decomposition of the frame; `gpu_frame_ns` is the union of the frame's pass intervals and `KFX_WGPU_GPU_TIMING=2` (`--serial-gpu-timing`) drains the queue between timed submissions to make the per-pass windows exclusive at a throughput cost. A serialised busy 1080p run puts the pass-window sum at 3.648 ms against 7.779 ms unserialised. Clean 640×480 measurements exposed the synchronous prototype regression: software drawing about 1.3–1.5 ms versus GPU drawing about 129–131 ms, with verifiers disabled. Full-frame readback dominated the separate stack sample. Queued frames are now measured at 640×480 and 1920×1080 (see the status section); the front-view matrix remains open. |
| Extract commands | Partial | [Gpoly capture](../../src/kfx/renderer/GpolyCapture.h) owns span/resource snapshots; reviewed CPU oracle at `beec45800`: 615 fixtures and 20,389 spans matched native indices. Original-vertex native routing at `17e993a84` copies vertices before CPU setup and retains immutable texture/fade versions. Other families need immutable commands. |
| Implement GPU drawing | Partial | [Indexed backend](../../tools/frame-replay/src/draw.rs) and [C ABI](../../src/kfx/renderer/WgpuDraw.h) cover the implemented families below. General triangles have all 27 kernels and deterministic thin-triangle setup. Queued frames, alias views, resource ownership and borrowed cursor integration are implemented; final combined runtime and performance evidence must match their exact source. |
| Cover every drawing path | Open | Accepted original-vertex terrain bypasses CPU setup and rasterization; bounded 2D hooks suppress selected CPU pixel loops. The remaining families below and routine upload/readback bridges prevent complete GPU coverage. |
| Native validation | Partial | The tight-bin-box build passed an isolated native session on busy level 20 through `game-control.py` — camera movement, parchment open and return, pause and resume, two resizes and a clean quit, every state predicate reached, 1,758 GPU batches and no failure, invalid frame or fallback — a `KFX_WGPU_DRAW_VERIFY=1` run with 99,298 verified batches at zero failures, and a `KFX_WGPU_VERIFY=1` session that presented and verified 546 of 546 acquired surfaces, which earlier sessions could not reach. The terrain-binning build before it passed an isolated native session on busy level 20 through `game-control.py` — camera movement, parchment open and return, pause and resume, two resizes and a clean quit, every state predicate reached, 2,424 GPU terrain batches and no failure, invalid frame or fallback — and a separate `KFX_WGPU_DRAW_VERIFY=1` run against the CPU oracle with 5,986 verified batches and 21,223 verified triangles at zero failures. Exact `e19ff26f7` sessions passed gameplay, parchment, save/reload and compound-lens possession, with 788 surface-verified presentations and no drawing failures. A real parchment oracle-recursion crash was fixed and retested. Later queued-frame source requires its own acceptance; complete views, languages, assets and failure coverage remain open. |
| Performance and delivery | Open | Tight bin boxes are the step that closes the 1080p gap: on the wgpu presenter with GPU drawing, observed FPS rose 12.80 → 60.00 at busy 1920x1080 and 16.16 → 60.00 on a quiet 1080p pair, both reaching the cap at 20 turns/s, with `tile_entries` 897,385 → 46,196 and the raster pass 46.12 → 3.07 ms. 640x480 was already capped and stays there. The frame cap, not the GPU, now bounds both scenes, so no uncapped-throughput claim follows. Terrain binning before it was the first step that paid: on the wgpu presenter with GPU drawing, observed FPS rose 54.95 → 60.00 at busy 640x480, hitting the cap, and 9.52 → 12.65 at 1920x1080, with per-frame GPU time 14.00 → 9.29 ms and 88.07 → 87.81 ms. 1080p is still 4.7x off the target. The single command stream before it moved its structural counters without paying for itself: observed FPS fell about 2 at busy 640x480 while CPU drawing and presentation improved. The synchronous prototype is unsuitable for regular play. Queued native drawing replaces per-command framebuffer transfers; verify the actual improvement with clean matched runs and active GPU counters. No complete-renderer or speedup claim follows from fixtures. The foundation merges opt-in with the software path default; the single-stream restructure and its acceptance metrics gate any default switch. Exact-head CI and merge verification remain required for each PR. |

| Drawing family and source boundary | Implemented coverage | Remaining GPU work / validation |
| --- | --- | --- |
| Dungeon/possession terrain: [world dispatch](../../src/engine_render.c), [gpoly](../../src/kfx/renderer/software/bflib_render_gpoly.c) | Original vertices → GPU setup, clipping, scan conversion and texture/shade stores; every triangle is a binned stream record sharing one raster pass with the other families, with setup into a compressed renderer-owned row arena; general-triangle kernels cover the other implemented modes | Broader scene/resource and allocation/alias coverage; full offscreen possession target ownership |
| Front view: `display_fast_drawlist()` in [engine_render.c](../../src/engine_render.c) | `QK_TextureQuad` original-vertex terrain batching is wired and inherits binning through the same sink | Independent front-view runtime proof and its own `terrain_tile_entries` read; sprites and interleaved overlays |
| General triangles: [trig](../../src/kfx/renderer/software/bflib_render_trig.c), world dispatch | Original vertices feed GPU kernels for modes 0–26; native adapter covers 26 modes and dedicated shadows provide mode10 | Allocation/alias fallback, combined-head native coverage and resident target integration |
| Creature shadows: [shadow adapter](../../src/kfx/renderer/software/WgpuShadow.h), world dispatch | Original RLE/frame metadata → GPU silhouette in a resident 256x256 scratch → one of two resident mask slots → both native-order mode10 triangles in the next submission; partial scratch clear preserved | The resident chain and the shared `big_scratch` are unreconciled in both directions, measured by `shadow_prior_divergence` under verification; scratch-alias residency, broader assets and combined-head gameplay |
| World sprites, creatures, objects and effects: [sprite adapter](../../src/kfx/renderer/software/WgpuSprite.c) | RLE index/coverage assets and scale ranges feed GPU source selection, flips, clipping, remap/ghost/alpha; source-frame offsets and water-truncated height preserved; accepted calls bypass native stores. Consecutive ordered sprites with disjoint scaling-range rectangles share one dispatch of *M* workgroups, reported by `ordered_sprite_layers` and `ordered_sprite_passes` | Ordered GPU run copies cover solid scaled-up horizontal flips, including native alignment/chunk behavior; asset/destination aliases and custom-asset/gameplay coverage remain open. Layering measured 2.68 layers against 2.69 ordered sprites on a busy 640x480 pair, so it pays nothing today: ordered sprites almost never land consecutively in the stream |
| Pixels, boxes, HV lines and circles: [bflib_vidraw.c](../../src/kfx/renderer/software/bflib_vidraw.c) | Reviewed native GPU hooks and 1,116 exact fixtures; circles preserve repeated blend hits | Circle radii above 8,191 and other unsupported inputs decline to CPU; complete runtime coverage remains open |
| General lines, world overlays, HUD/menu sprites: [engine_render.c](../../src/engine_render.c), [UI interface](../../src/kfx/renderer/IUIRenderer.h) | Selected low-level primitives; scaled normal/remap/one-colour/alpha and immediate normal/one-colour sprites | General-line coverage/color selection, unsupported sprite modes and full interleaving validation |
| Text, including Asian fonts: [bflib_sprfnt.c](../../src/bflib_sprfnt.c) | Sprite glyphs and direct DBC bitmap GPU hooks; CPU layout retained; huge/DBC native fixture group has 849 exact Metal cases | Actual language/font runtime coverage, oversized custom inputs and mutable-source aliases |
| Raw/tiled images, frontend backgrounds, landview/torture/zoom: [raw adapter](../../src/kfx/renderer/software/WgpuRawImage.c), [raw helper](../../src/front_simple.c), [slab helper](../../src/gui_draw.c) | Raw8 scaling/letterbox, tiled slabs, static backgrounds, huge sprite and campaign zoom GPU paths | Mutable source/destination aliases, noncanonical huge steps and source footprints above 1,048,576 pixels; full asset/runtime coverage |
| Minimap, parchment and overhead/zoom maps: [frontmenu_ingame_map.c](../../src/frontmenu_ingame_map.c), [gui_parchment.c](../../src/gui_parchment.c) | Semantic GPU cells, setup fills, markers and map/zoom transforms; 568 minimap and 1,358 map-view native/Metal fixtures | Minimap background dictionary still needs an explicit CPU read checkpoint; broader states and complete offscreen ownership |
| Built-in possession lenses: [lens implementations](../../src/kfx/lense/) | Indexed displacement/flyeye remaps, mist and overlay GPU kernels preserve sequential source/target aliases; CPU map preparation and palette lifecycle remain | Resident GPU target views; lightness 32–63 mist, out-of-viewport maps, asset/destination aliases and oversized inputs still decline; full LensManager lifecycle/gameplay validation |
| Custom Lua lenses: [LuaLensEffect.cpp](../../src/kfx/lense/LuaLensEffect.cpp), [lua_api_lens.c](../../src/lua_api_lens.c) | CPU reference | Ordered GPU writes/copies and exact read-after-write compatibility for arbitrary pixel-dependent Lua control flow; CPU-script readback is explicit, never hidden CPU-rendered lens upload |
| Smoothing and map fades/transitions: [engine_redraw.c](../../src/engine_redraw.c) | GPU snapshots and exact indexed effects; 271 native/Metal cases plus failed-preparation/normal-exit state tests | Retained CPU recovery checkpoints, valid alias cases and broader lifecycle coverage |
| Movies: [bflib_fmvids.cpp](../../src/bflib_fmvids.cpp) | CPU decoding feeds GPU frame scaling/copy, packed doubling/interlace and palette-index writes; 105 exact native fixtures | Visible playback/audio timing, uncommon source domains and recording/readback ownership |
| Cursor, clears, screenshots and recording: [bflib_mspointer.cpp](../../src/bflib_mspointer.cpp), [RendererSoftware.cpp](../../src/kfx/renderer/RendererSoftware.cpp), [scrcapt.c](../../src/scrcapt.c) | GPU indexed clear for full SDL surface clips, preserving row padding; direct cursor scaling and GPU snapshot backup/keyed draw/opaque restore, now recorded with the palette pass into one present-tail encoder submitted by `kfx_wgpu_present`; native captures still consume the synchronized image | Nonfull SDL clip clears; retire cursor wrapper transfers and integrate authoritative GPU capture with matching frame/palette/cursor semantics |
| Cross-family palette, transparency, clipping and scaling | Bounded command and offscreen palette-output fixtures pass | Full-family index/RGBA comparisons; table versions, target aliases and strict CPU-writer/readback audit |

The reviewed original-vertex [GPU preparation test](../../tools/frame-replay/tests/gpoly_gpu.rs)
compared 597,800 setup words and 6,202,175 palette indices, including pitch padding,
against independent native output on Metal. It consumes GPU-produced rows directly
in a subsequent GPU pass. The production [triangle test](../../tools/frame-replay/tests/draw_triangles_gpu.rs)
at `17e993a84` separately checks all 1,225 triangles in ordered overlapping batches,
immutable texture/fade versions, host resource/coordinate rejection and the flagged
out-of-range shade that skips its own pixels.
Independent review fixes at `ab2300ea1` add allocation and pixel-dispatch limits;
three limited-device Metal cases preserve the target and a usable device. Counter
hardening at `10eb96a35` counts verified triangles only after a complete exact
comparison; matching and forced-mismatch/recovery mock cases pass ASan.
The [native bridge fixture](../../tests/terrain-vertices/bridge_test.cpp) independently
checks CPU setup bypass, resource mutation, CPU interleaving and original-input
reconstruction with ASan/Metal. None establishes whole-frame GPU ownership or speedup.
The 2D [native fixture generator](../../tests/primitives/fixture.c) compares actual
legacy output for the supported primitives; it does not establish full HUD/text
coverage.

The sprite slice through `c20303633` has 12,386 exact native-reference Metal cases
from the [ASan native fixture](../../tests/sprites/fixture.c), including all 588 former
scaled solid horizontal-flip declines. Ordered GPU run copies preserve the extra
left pixel, RLE segmentation and overlapping destination writes. Independent review
added 576 [narrow-pitch cases](../../tests/sprites/copy_fixture.c): actual native
four-byte copy grouping depends on destination alignment, which the command now
preserves. All 12,962 cases match exact Metal indices, including 2,029 ordered commands.
The verification oracle preserves native target alignment in its temporary buffer.
Separate cursor and shadow adapters now handle their direct native paths.
Trusted native RLE pointers have no encoded-length contract, and mutable artwork/table
aliases with the destination explicitly decline. Three native alias regressions verify
exact fallback for decoded RLE, remap and blend-table overlap; GPU alias support is open.
The raw slice through `8a38178ef` has 270 exact native-reference Metal cases from
the [raw fixture](../../tests/raw-images/fixture.c), including tile clipping/phase and
padded clears. Both suites check isolated recursive oracles and source snapshots.
These bounded fixtures do not replace combined-head gameplay, complete asset coverage or performance evidence.
The native build at `8a38178ef` passed with binary SHA-256
`63a0c73186aae4ef5b53470bdb0e9de7f6cf7e043689b295aae7e0c302e58228`;
that is compilation/linking evidence only. Sprite/raw commands still use the
synchronous full-target upload/readback bridge, including clears and backgrounds.

The shadow slice at `3add2d680` uses the [actual native mask and mode10 oracle](../../tests/shadows/fixture.c).
Its Metal evidence covers 192 complete 65,536-byte masks and 384 native-order triangles,
all 64 constant shades, four scratch alignments, partial clears, offsets, flip scanline
crossings and padded cumulative targets. Separate selection checks cover 72 native
frame/orientation/base/custom choices. The ASan native bridge verifies 192 accepted
calls and exact fallback after initialization failure or one successful batch.
Independent review also requires the same complete target hash from 192 production
calls with verification disabled and zero CPU oracle commands.
Accepted production calls perform no CPU mask rasterization and no readback: the mask
chain lives in a persistent GPU scratch buffer, each mask is stamped into one of two
resident slots, and the mode10 triangles sample that slot. Mask *i* is submitted ahead of
the triangles that read slot *i*, and queue submissions execute in order, so no later mask
can overwrite a slot an earlier submission still reads. The asset carries only header,
geometry and RLE. Under `KFX_WGPU_DRAW_VERIFY` the CPU oracle runs on the game's own
scratch and one blocking scratch read per shadow compares the two masks. `FullRedraw`
resets the cross-frame scratch; a rejected queued frame does not roll the scratch back,
but it invalidates the frame, and the invalidation forces the `FullRedraw` that resets it
before the next accepted shadow.

The resident chain and the legacy `big_scratch` chain are now unreconciled in both
directions, and nothing detects a divergence beyond counting it. Simulation code writes
[`big_scratch`](../../src/custom_sprites.c) from offset 0 — inside the 64 KiB mask window —
in `spdigger_stack.c`, `player_complookup.c`, `room_lair.c`, `player_utils.c` and
`power_specials.c`; a declined shadow CPU-rasterizes into it without telling the GPU
(`engine_render.c`); a rejected queued frame's mask writes are not rolled back, and are
undone only by the `FullRedraw` its invalidation forces; a CPU fallback after accepted GPU
shadows rebuilds its mask over
whatever that buffer holds; and a level change that detaches the presenter leaves the
scratch resident while loading clobbers `big_scratch`. Nothing bounds the sampled region
to the cleared rectangle either — neither the Rust descriptor checks nor the C guard — so
carried bytes can reach the triangles. `KFX_WGPU_DRAW_VERIFY` runs its oracle on the
game's own scratch and reports `shadow_prior_divergence` when the two priors differ,
skipping the mask and pixel comparison for that shadow rather than failing it. The counter
counts events, not shadows after the first: verification re-seeds the CPU scratch from the
GPU prior after each event, so the following shadows are checked again for their own mask
and pixel correctness. That counter is the measurement, not a fix. Reproducing the legacy scratch contents on the GPU
is explicitly not a goal. Generic scratch aliases remain open.

The cursor slice at `95c4ec603` has [actual native pointer and SDL surface oracles](../../tests/cursor/cursor_test.cpp),
including 81 backup/draw/restore cycles and 12 scale/hotspot/position/begin-end-swap
traces. Independent review expands direct cases to all four pointer alignments with
allocation guards and padded pitch. Enabled traces require zero CPU cursor raster,
backup or composition calls. Borrowed targets, immutable artwork, resize/absence,
release order and invalid-target checkpoint recovery are covered. Native wrappers
still upload/read back the screen and maintain sprite/backup checkpoints; the borrowed
context must outlive its cursor and does not provide automatic CPU reconstruction.
Captures retain their position between begin-swap composition and end-swap restoration
by source inspection, not a new visible gameplay capture. Twenty-four swap traces additionally
run a presented frame's repaint and its single flush before the swap, across an
interrupted pointer, a hidden pointer and a surface resize, and assert that the
backup, composition and restore add no queued-frame checkpoint. The frame-replay workflow
builds real SDL3 surface code and the portable drawing C ABI for a required Vulkan
cursor fixture. Exact-head CI, physical input, visible presentation and device-loss
reconstruction remain separate validation gates. These offscreen proofs do not establish
combined-head gameplay, complete GPU frame ownership or a performance improvement.

The built-in lens slice at `7a865cc43`, combined with shared dispatch at `d18840c6b`,
has 54 [native fixture cases](../../tests/lens/lens_test.cpp) covering padded/different
pitches, in-place and partial aliases, signed alpha, wrapped mist phases, transparent
index 255 and signed remaps. Its extracted-loop native oracle is separate from the
GPU shader; it does not run the full LensManager. Review additionally verifies source
snapshot lifetime, mixed-lens batch rejection, and limited-device rejection without
target changes or device loss. Mist animation remains once per Draw by source review;
palette effects have no pixel loop. Native wrappers still upload the current target
and source assets, execute GPU pixels, then read back before committing. Dimensions
above 8192, pitches above 1 MiB, extents above 32 MiB, packed assets above 16 MiB,
out-of-viewport map entries and asset/destination aliases retain native fallback.
Mist requires 33 readable fade rows; configured lightness 32–63 remains native,
including valid narrower-shade cases. No visible presentation, final linked-game
lifecycle result, resident target ownership or speedup follows from these fixtures.

The original-vertex native smoke used original campaign level 1, a 640×480 indexed
target, isolated assets/settings/saves, SDL presentation and drawing verification:
438,568 GPU original triangles in 4,992 exact bridge batches, with zero GPU spans,
declined/replayed triangles or failures. Native movement, capture and quit succeeded.
Its binary SHA-256 was
`b3b08abd28fb25141fcd7211758a70dc401ef4686709fd4239197ee63dbef6ba`;
it predates final invalid-shade fallback hardening in `17e993a84`. The hardened
native ASan/Metal fixture passed separately; the final native build hash was
`15a2930d9acf6bac84d891a62abb9a408fde14c30206f79c1bf058d406025d62`,
before a warning-text-only edit. The later `ab2300ea1` device-limit fix has focused
offscreen Metal proof, not another native game run. These sources do not establish
subsequent sprite/raw-image changes or combined-head gameplay.

Earlier span-only gameplay (`7fde9463f`/`489c2e888`, binary
`da428a86f6c5aae579b26217f937654d09a2920d602980e0b3151e728fd36a2f`)
verified 381 batches and 356,372 spans; separate failure injection reconstructed
110 spans. Its wgpu-presenter attempt acquired/presented zero surface frames while
the display was locked. Visible output remains unproven for these drawing changes;
prior PR #9 surface evidence stays separate.

Each fallback or unsupported input remains uncovered GPU work. Zero declined
gpoly spans measures one sink, not every software writer. Completion requires
an audit of all targets and aliases, no routine built-in CPU rasterization or
completed-frame upload in GPU mode, and explicit handling of CPU pixel reads.
Keep original assets, raw captures and private session descriptors outside the PR;
publish synthetic fixtures, source identities and portable aggregate evidence.

Keep exact presentation comparisons unchanged. Define each drawing family's visual
acceptance before implementation and investigate any rasterization differences;
do not silently accept smoothing, filtering, changed palette behavior or new art.
SDL remains default until comparable correctness, coverage and performance evidence
justifies a separate default-change decision.

AI, pathfinding, creature behavior and other simulation costs stay on the CPU in
this graphics track. If simulation dominates, report that bottleneck and scope a
separate simulation investigation; a GPU world renderer or Rust utility port does
not by itself speed those systems up. The utility/world-state/gameplay phases above
continue independently of this graphics sequence.

## Release and retirement criteria

Each migration should leave a playable build and a way to compare or revert the
changed subsystem. Keep upstream synchronization separate from a Rust migration
PR so regressions can be attributed to a bounded change.

Complete the remaining drawing, menu, input, audio and resource orchestration
before handing application ownership fully to Rust. Complete the authorized wgpu
drawing track while retaining its pixel rules and reference fallback. SDL, audio
codecs and other external libraries can remain behind explicit bindings.

Retire a legacy component only after its callers have migrated and its agreed
behavioral checks pass. Keep a pinned baseline build for comparison. A full port
is complete when the shipped game implementation no longer depends on KeeperFX's
C/C++ code, the agreed compatibility matrix passes, and packaging works on each
claimed platform. External libraries and original game-data requirements remain
explicit dependencies.

Use phase completion and behavioral coverage to measure progress, not translated
line counts. Estimate components after inspecting their callers and fixtures, and use those
results to refine the scope of later phases.
