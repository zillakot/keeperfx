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

### Status: 2026-09-14, closing

The objective is unchanged: full Rust/wgpu ownership of drawing, the C/C++ drawing
retired afterwards, exact indexed parity and 60 FPS at a 1920×1080 framebuffer. It
stays opt-in: `KFX_RUST_PRESENTER` and `KFX_DRAW_BACKEND=wgpu` select it.

The morning's HD measurement had GPU drawing at 8.0 FPS busy at 1080p against
60.00 FPS software, dominated by 45.7–54.2 ms of presentation. The diagnosis was
structural: about 125 queue submits, 17 blocking waits and 9 checkpoints per
frame, 20 MB of immutable assets re-uploaded every frame and shadows as
synchronous readback chains — waiting rather than computation.

[PRs #16 through #35](https://github.com/zillakot/keeperfx/pulls?q=is%3Apr+is%3Amerged)
carried out the restructure: the opt-in foundation and guards
(#16, #17), the [design](../architecture/wgpu-single-stream-renderer.md) (#18) and
its counters (#19), bridge batching and recovery (#20, #22), fixture CI (#23), the
persistent asset arena (#21), shadow residency (#24), non-blocking validation
(#25, #26), one command stream in root space (#27), terrain tile binning with
per-pass GPU timestamps (#28), the cursor present tail (#29), ordered-sprite layers
(#30), tight bin boxes (#31), the [tooling plan](development-tooling-plan.md),
parity policy and agent workflow (#32), the uncapped runner (#33), the status-ring
test (#34) and one encoder and one submit per frame (#35).

Final measurement on `56a5eee8b`, wgpu presenter, host wall clock, 200 measured
turns per cell, one run per cell. Capped at 60 FPS, milliseconds, busy / quiet:

| Drawing | Logical | draw mean | presentation mean | FPS | Turns/s |
| --- | --- | ---: | ---: | ---: | ---: |
| GPU | 640x480 | 0.787 / 0.775 | 5.622 / 4.649 | 60.00 | 20.00 |
| GPU | 1920x1080 | 0.848 / 0.813 | 5.014 / 4.677 | 60.00 | 20.00 |
| software | 640x480 | 1.313 / 1.097 | 0.445 / 0.449 | 59.99 / 60.00 | 20.00 |
| software | 1920x1080 | 2.400 / 2.668 | 0.519 / 0.465 | 59.99 / 60.00 | 20.00 |

All eight cells hold the cap at 20.000 ± 0.004 turns/s, both 1080p GPU cells
included, with blocking device polls, status stalls, invalid frames and arena
evictions zero. Uncapped ceilings, same source, observed FPS, busy / quiet:

| Presenter / drawing | 640x480 | 1920x1080 |
| --- | ---: | ---: |
| wgpu / GPU | 142.6 / 203.1 | 189.7 / 201.3 |
| wgpu / software | 195.1\* / 206.4\* | 225.3 / 201.7 |
| SDL / software | 288.9\* / 290.3\* | 312.4 / 318.3 |

\* Lower bounds: background CPU contention, simulation canary 1.7–2.0x their 1080p
partners. Every cell held 20.000 ± 0.005 turns/s, so none is a degraded loop.

Parity: 1,857,396 verified batches and 1,018,767 verified triangles over 2,117
frames of an isolated control session against the CPU oracle, at 0 comparison
failures, 0 invalid frames and 0 flagged shades, with 1 `shadow_prior_divergence`
event in 106 creature shadows; a separate run verified 718 of 718 surfaces. That
volume is concentrated in terrain, sprites, primitives and the minimap. In the same
samples `arena_trig_*` (generic triangle textures), the movie, built-in lens and
huge-bitmap arena kinds and `transition_commands` are zero, and DBC text and Lua
lenses have no dedicated counter at all, so their coverage is unmeasured rather than
zero. This is parity evidence for the families the control sessions reach, not for
all of them.

**Presentation now bounds the ceiling, not GPU execution.** Serialized timing puts
exclusive GPU work at 1.60 ms busy at 640x480 and 3.68 ms busy at 1080p, well
inside frame intervals of 7.01 and 5.27 ms, and 1080p — with 2.3x the GPU work —
reaches a higher uncapped FPS than 640x480. The host spends 4.28–6.29 ms in
`presentation` against 0.57–0.64 ms in `draw`, with blocking polls at zero:
surface acquisition, the per-frame uploads, the frame replay itself and the
palette pass, not device execution and not the present call.

[PR #35](https://github.com/zillakot/keeperfx/pull/35) merged after that
measurement and supersedes it where it gives a figure. A presented frame is now
one command buffer: `submits` 37 → 1.0 and `checkpoints` 1 → 0 per frame, capped
presentation 5.15 → 3.92 ms at 640x480 and 6.17 → 4.73 ms at 1080p, uncapped busy
640x480 141 → 190 FPS with 1080p inside its spread, and `buffers` unchanged at
about 87. Three review items stay open: arena headroom can still refuse a batch
inside a frame, because `fits()` counts only resource bytes, taking the rare
`OVERFLOW` then full-redraw path; `shadow_scratch_reset` on that path holds the
arena until the next `frame_begin`; and peak GPU memory is unmeasured and higher by
construction, about 38 shadow arenas coexisting.

**Presenter cost, PR A ([measurement](../performance-baselines.md#presenter-cost-pr-a-measured-2026-09-14)):**
On `65e3ff9c2`, capped busy 1080p presentation is 0.657/0.656 ms plus
3.186/3.051 ms replay, against 3.859/3.871 ms in master’s old combined scope.
Buffers fall 90.40 → 88.39/frame; comparable scoped allocations 643 → 612 and
present record 14.917 → 6.734–7.125 µs. Attribution residual is below 1% of
the mean and native surface/drawing gates pass. Uncapped 197.98/195.08 FPS
establishes no ceiling increase. Replay’s roughly 11 MB/frame asset uploads
(P3) and 0.58–0.78 ms host submission remain the next costs to address.

**Presenter cost, P3 slice 1 ([measurements](../performance-baselines.md#presenter-cost-p3-slice-1-measured-2026-09-15)).**
Runtime `97402ca75` removes warm shadow-table uploads/private asset buffers;
matched capped busy HD asset uploads fall 11.07–11.08→4.48–4.49 MB/frame and replay
2.999/2.757→2.224/1.958 ms. One submit and zero drawing errors remain; capacity is
32 MiB, old+new growth overlap 48 MiB. Acceptance is partial: small buffer/cadence
target misses, one high-load pair and untested mutation/transition coverage remain.
Windowed uncapped is compositor-paced near 75 Hz this time, not a ceiling.
Separate offscreen pairs improve 221.06/219.99→250.78/250.75 FPS while presentation
rises 0.83→1.50–1.55 ms. Surface gates pass; the drawing oracle passes exercised
operations despite separate control-tooling failures. No windowed ceiling or
whole-process/GPU peak-memory gain is established.

**Replay upload rings ([PR #45](https://github.com/zillakot/keeperfx/pull/45)) and the
byte-packed arena ([PR #47](https://github.com/zillakot/keeperfx/pull/47)) are delivered
([measurements](../performance-baselines.md#byte-packed-arena-format)).** Word-once source
`b94fa46c7` against the PR #45 baseline `416a46d59`, matched offscreen pairs, binary
identities recorded with the measurements. Replay falls 0.58–0.73 ms in every cell, offscreen
uncapped busy 1080p rises to 253/268 FPS, and the serialized exclusive GPU union rises 7%
to 4.30 ms/frame, of which the minimap pass is 1.01 ms. These are offscreen cells, so they
establish no windowed ceiling; the
[replay floor](../performance-baselines.md#replay-floor-measured-2026-09-15) remains the
control for replay attribution.

**The minimap dispatch extent is delivered**
([PR #50](https://github.com/zillakot/keeperfx/pull/50),
[measurements](../performance-baselines.md#minimap-dispatch-box-measured-2026-09-15)):
dispatching each command's written rectangle instead of the whole `MapDiagonalLength`
square cuts the overlapped minimap pass 82% offscreen uncapped busy 1080p, 0.453/0.441 →
0.080/0.082 ms, for 246.5/253.0 → 279.9/272.4 FPS with `arena_minimap_bytes` byte-identical.

Next, in order:

1. **Drawing-family scene matrix** (in flight, `tooling/drawing-coverage-matrix`). A
   control-session harness with no engine change, so it carries no parity risk. The coverage
   slices below cannot produce an acceptance number until a scene reaches their family, and
   the performance items need the same scenes, so this gates both. Acceptance: every drawing
   family has a non-zero arena/command counter in at least one scene at 0 comparison failures.
2. **Sprite asset interning**, in parallel with 1: `arena_sprite_hits` is 0 against 39,267
   misses. Sprites and the minimap are the two largest remaining per-frame uploads, about
   475 and 479 KB/frame; the minimap stays non-resident while PR #43 is unmerged.
3. **Coverage and ownership before C/C++ drawing can be retired**: general-triangle runtime
   coverage and its alias fallback, possession-lens offscreen target residency, transition
   checkpoint removal, shadow scratch decoupling from `big_scratch`, the remaining declines,
   an ordered contract for arbitrary Lua pixel drawing, same-frame recovery for every command
   kind, and an audit of all targets and aliases — including
   [frontend.cpp](../../src/frontend.cpp) lines 1044–1047, which write the screen with no hook
   and no barrier. Reaching a frame-rate target does not complete the drawing goal.
4. **The default switch**, then per-family deletion of the C/C++ rasterizers with the CPU
   oracle preserved as a test-only library.

Deferred: minimap residency ([PR #43](https://github.com/zillakot/keeperfx/pull/43)) is not
merged; its evidence predates the packed arena and it conflicts with the current tree.
Remaining presenter cost — host submission after replay, software index upload, late cursor
restoration — sits behind the items above and must preserve `presentation_cpu` at most 1.0 ms
and simulation cadence in uncapped comparisons. Reusable mapped staging is a later PR.

In parallel, work the tooling plan's
[recommended order](development-tooling-plan.md#recommended-order) on what each item still
owes: the offscreen mode's locked/unlocked per-pass equivalence; the trace profiler under
GPU time attribution; the bounds-superset property test; command-stream capture with offline
replay; deterministic scene mode; then the per-suite frame-replay CI matrix, which today
splits only the two arena formats. Control tooling has one open fault: after
[PR #41](https://github.com/zillakot/keeperfx/pull/41) the API server no longer dies on the
video-mode switch, but the `scripts/game-control.py cycle-mode` reply and the following
quit timed out while the game kept running in desktop mode, so oracle sessions skip the
mode round trip until it is fixed. Evidence:
`out/wgpu-migration/byte-arena-runs/word-once/measure-schedule.log` lines 113-114 for the
timed-out replies, and the `drawing-busy-control.incomplete-*/` session directory beside
it for the game state.

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
| Inventory and measurement | Partial | Per-pass GPU execution time is collected on request (`KFX_WGPU_GPU_TIMING=1`, `profile-game.py --gpu-timing`), and `tile_entries_<kind>` splits the binning index per record kind. A pass window includes the stall that pass waited through, so it attributes cost rather than measuring it and the window sum decomposes nothing. `gpu_pass_union_ns` is the union of the frame's pass intervals and bounds GPU occupancy from above; measured, it equals the window sum, so the windows are disjoint and the stall sits inside them. Only `KFX_WGPU_GPU_TIMING=2` (`--serial-gpu-timing`) is exclusive: a serialised busy 1080p run reports 3.549 ms of GPU work against an 8.006 ms window sum, and that serialised figure is what acceptance tables cite. Clean 640×480 measurements exposed the synchronous prototype regression: software drawing about 1.3–1.5 ms versus GPU drawing about 129–131 ms, with verifiers disabled. Full-frame readback dominated the separate stack sample. Queued frames are now measured at 640×480 and 1920×1080 (see the status section); the front-view matrix remains open. |
| Extract commands | Partial | [Gpoly capture](../../src/kfx/renderer/GpolyCapture.h) owns span/resource snapshots; reviewed CPU oracle at `beec45800`: 615 fixtures and 20,389 spans matched native indices. Original-vertex native routing at `17e993a84` copies vertices before CPU setup and retains immutable texture/fade versions. Other families need immutable commands. |
| Implement GPU drawing | Partial | [Indexed backend](../../tools/frame-replay/src/draw.rs) and [C ABI](../../src/kfx/renderer/WgpuDraw.h) cover the implemented families below. General triangles have all 27 kernels and deterministic thin-triangle setup. Queued frames, alias views, resource ownership and borrowed cursor integration are implemented; final combined runtime and performance evidence must match their exact source. |
| Cover every drawing path | Open | Accepted original-vertex terrain bypasses CPU setup and rasterization; bounded 2D hooks suppress selected CPU pixel loops. The remaining families below and routine upload/readback bridges prevent complete GPU coverage. |
| Native validation | Partial | The one-encoder build passed an isolated native session on busy level 20 through `game-control.py` — camera movement, parchment open and return, pause and resume, two resizes and a clean quit, every state predicate reached, 4,007 GPU batches and no failure, invalid frame or fallback — a `KFX_WGPU_DRAW_VERIFY=1` run with 216,189 verified batches at zero failures, and a `KFX_WGPU_VERIFY=1` session that presented and verified 250 of 250 acquired surfaces. The tight-bin-box build before it passed an isolated native session on busy level 20 through `game-control.py` — camera movement, parchment open and return, pause and resume, two resizes and a clean quit, every state predicate reached, 1,758 GPU batches and no failure, invalid frame or fallback — a `KFX_WGPU_DRAW_VERIFY=1` run with 99,298 verified batches at zero failures, and a `KFX_WGPU_VERIFY=1` session that presented and verified 546 of 546 acquired surfaces, which earlier sessions could not reach. The terrain-binning build before it passed an isolated native session on busy level 20 through `game-control.py` — camera movement, parchment open and return, pause and resume, two resizes and a clean quit, every state predicate reached, 2,424 GPU terrain batches and no failure, invalid frame or fallback — and a separate `KFX_WGPU_DRAW_VERIFY=1` run against the CPU oracle with 5,986 verified batches and 21,223 verified triangles at zero failures. Exact `e19ff26f7` sessions passed gameplay, parchment, save/reload and compound-lens possession, with 788 surface-verified presentations and no drawing failures. A real parchment oracle-recursion crash was fixed and retested. Later queued-frame source requires its own acceptance; complete views, languages, assets and failure coverage remain open. |
| Performance and delivery | Open | One encoder and one submit per frame is delivered and is the first step whose acceptance counter is met exactly: `submits` 37.2 → 1.00 at busy 640x480 and 38.3 → 1.00 at 1080p, `checkpoints` 1.0 → 0 and blocking waits 0, with presentation falling about 23% at both. Both capped scenes were already on the 60 FPS cap, so the gain shows only uncapped, where 640x480 rises 141.1 → 189.6 FPS over three matched pairs while 1080p stays inside its spread. Buffer allocations did not move; the next slice is shared upload rings and coalescing, as ordered above. Tight bin boxes before it are the step that closes the 1080p gap: on the wgpu presenter with GPU drawing, observed FPS rose 12.80 → 60.00 at busy 1920x1080 and 16.16 → 60.00 on a quiet 1080p pair, both reaching the cap at 20 turns/s, with `tile_entries` 897,385 → 46,196 and the raster pass 46.12 → 3.07 ms. 640x480 was already capped and stays there. The frame cap, not the GPU, now bounds both scenes, so no uncapped-throughput claim follows. Terrain binning before it was the first step that paid: on the wgpu presenter with GPU drawing, observed FPS rose 54.95 → 60.00 at busy 640x480, hitting the cap, and 9.52 → 12.65 at 1920x1080, with per-frame GPU time 14.00 → 9.29 ms and 88.07 → 87.81 ms. 1080p is still 4.7x off the target. The single command stream before it moved its structural counters without paying for itself: observed FPS fell about 2 at busy 640x480 while CPU drawing and presentation improved. The synchronous prototype is unsuitable for regular play. Queued native drawing replaces per-command framebuffer transfers; verify the actual improvement with clean matched runs and active GPU counters. No complete-renderer or speedup claim follows from fixtures. The foundation merges opt-in with the software path default; the single-stream restructure and its acceptance metrics gate any default switch. Exact-head CI and merge verification remain required for each PR. |

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
| Minimap, parchment and overhead/zoom maps: [frontmenu_ingame_map.c](../../src/frontmenu_ingame_map.c), [gui_parchment.c](../../src/gui_parchment.c) | Semantic GPU cells, setup fills, markers and map/zoom transforms; 568 minimap and 1,358 map-view native/Metal fixtures | Minimap dictionary, cells and style tables are re-uploaded every frame rather than resident, and the PR #43 counters measured style tables churning about 30 times a second; the dispatch extent is done, each command now dispatching only its written rectangle; broader states and complete offscreen ownership |
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
