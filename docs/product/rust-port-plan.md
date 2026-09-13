---
type: product
description: Phased plan for incrementally porting KeeperFX to Rust while preserving pixel art, gameplay and a playable reference build, with validation criteria for each stage.
---

# Rust port plan

The aim is to learn Rust and modernize KeeperFX incrementally while preserving the
original pixel-art appearance and game behavior. The long-term target is a
Rust-owned application and game implementation. Third-party libraries may still
contain C/C++; rewriting those libraries is outside this plan.

The phases below are proposals; the completed foundations are listed separately. Delivery scope, findings and validation belong in the
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

The next graphics task is the investigation below. Utility and gameplay migration
remain separate tracks; GPU world drawing does not require transferring simulation
ownership to Rust. Later phases require their own scoped designs.

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
presentation; it cannot remove CPU terrain or sprite drawing. No GPU world drawing
has been implemented, and no new renderer architecture has been selected.

### Next session: measure CPU drawing and define one extraction boundary

This is the first bounded follow-up task, deferred to the next session. Deliver
one investigation PR with a rendering-cost report, a source-backed scene/command
boundary sketch and a go/no-go recommendation for one small GPU experiment.
Instrumentation and runner changes needed for that investigation belong in that
PR; implementing a GPU world renderer does not.

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

Exit/acceptance: the PR identifies which measured work limits each scene, includes
absolute costs and distributions with workload/identity limits, and names one
candidate's callers, required data, ownership, exclusions and expected removable
CPU work. It proposes synthetic pixel fixtures and a local game comparison for that
candidate. If the data do not justify GPU work, record that conclusion and the
next measured question instead of promising an FPS improvement.

Validation for changed measurement code must cover sample completeness, nesting,
metadata/config mismatch rejection and preservation of default capped behavior.
Run relevant collector/runner tests and existing CI; use an isolated native smoke
check for changed hooks. Do not treat dummy/headless runs as native performance or
run builds/profilers concurrently with benchmark collection. Keep original artwork,
raw captures, session descriptors and private host details out of the PR; publish
portable procedures, aggregate evidence and redistributable fixtures.

### Gates after the investigation

| Gate | Small delivery and acceptance |
| --- | --- |
| Select a bounded slice | Measured cost and command-boundary feasibility justify one terrain or sprite family and one initial view. Define coverage, unsupported cases and numerical/pixel rules before choosing raster versus compute or a broader scene architecture. |
| Extract and compare commands | Add a bounded read-only adapter and synthetic fixtures in a separate PR. Specify ownership, limits and errors; preserve C/C++ simulation authority. Capture input before rasterization and prove that the legacy path still produces the reference pixels. |
| Prototype GPU world drawing | Implement only the selected family behind an opt-in path. Compare identical command input against CPU output, including clipping, ordering, transparency, palette/shade behavior and integer scaling. Explain CPU/GPU composition and synchronization costs; fall back for unsupported input. |
| Integrate and measure | Run exact pixel and native lifecycle/gameplay checks, then matched end-to-end measurements with verification disabled. Account for command extraction, upload, synchronization, remaining CPU drawing and frame-time tails. Expand only when evidence supports the next slice. |

Keep exact presentation comparisons unchanged. Define the new slice's visual
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
before handing application ownership fully to Rust. If GPU world rendering has
not been selected, port the CPU drawing routines to Rust while retaining their
pixel rules. SDL, audio codecs and other external libraries can remain behind
explicit bindings.

Retire a legacy component only after its callers have migrated and its agreed
behavioral checks pass. Keep a pinned baseline build for comparison. A full port
is complete when the shipped game implementation no longer depends on KeeperFX's
C/C++ code, the agreed compatibility matrix passes, and packaging works on each
claimed platform. External libraries and original game-data requirements remain
explicit dependencies.

Use phase completion and behavioral coverage to measure progress, not translated
line counts. Estimate components after inspecting their callers and fixtures, and use those
results to refine the scope of later phases.
