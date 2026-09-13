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
ownership across the game frame remains open.

### Active delivery: full wgpu drawing

The user authorized the full drawing migration on 2026-09-13. Inventory and
measurement are the first gate, followed by command extraction, GPU implementation,
all drawing paths and native validation. A single family or framebuffer presenter
does not complete this scope. Keep the delivery PR in draft while implementation
or required validation remains incomplete; record exact source and evidence at each
gate without marking untested paths complete.

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
| Inventory and measurement | Inventoried; measurement incomplete | Drawing families and source boundaries below; opt-in coarse timing at `02f4587bc`. Native quiet/possession/default hook smokes passed. Instrumentation overhead, busy/front-view coverage, resolution matrix and matched measurements remain open. |
| Extract commands | Partial | [Gpoly capture](../../src/kfx/renderer/GpolyCapture.h) owns span/resource snapshots; reviewed CPU oracle at `beec45800`: 615 fixtures and 20,389 spans matched native indices. Original-vertex native routing at `17e993a84` copies vertices before CPU setup and retains immutable texture/fade versions. Other families need immutable commands. |
| Implement GPU drawing | Partial | [Indexed backend](../../tools/frame-replay/src/draw.rs) and [C ABI](../../src/kfx/renderer/WgpuDraw.h): ordered clear/rectangle/image/span commands, palette lookup composition and nearest palette presentation; reviewed at `c2e594d95`. Original-vertex setup at `f0c719bc6` and production row consumption at `17e993a84` each match 1,225 native triangles; independent integration review and limited-device fixes at `ab2300ea1` pass. Native 2D hooks at `e203dbd25` have 1,072 exact GPU fixture cases; independent review is pending. |
| Cover every drawing path | Open | Accepted original-vertex terrain bypasses CPU setup and rasterization; bounded 2D hooks suppress selected CPU pixel loops. The remaining families below and routine upload/readback bridges prevent complete GPU coverage. |
| Native validation | Partial | Original-vertex integration `17e993a84` has isolated exact gameplay evidence and native ASan/Metal recovery fixtures, with separate source/binary limits below. Visible wgpu surface proof for this drawing candidate requires an unlocked display; combined-head gameplay, all views, save/reload and lifecycle coverage remain open. |
| Performance and delivery | Open | No drawing speedup measured. Correctness bridge readbacks remain mandatory. Collect final serial matched runs and absolute costs/tails after coverage and synchronization work; exact-head CI and final merge verification remain required. |

| Drawing family and source boundary | Implemented coverage | Remaining GPU work / validation |
| --- | --- | --- |
| Dungeon/possession terrain: [world dispatch](../../src/engine_render.c), [gpoly](../../src/kfx/renderer/software/bflib_render_gpoly.c) | Original unsorted vertices → GPU setup, clipping, scan conversion and ordered texture/shade stores for `QK_PolygonStandard`, `QK_PolyMode5`, near-FP textured subtypes 0–11; immutable texture/fade snapshots | Other polygon modes and near-FP solid subtypes 12–23; broader scene/resource coverage |
| Front view: `display_fast_drawlist()` in [engine_render.c](../../src/engine_render.c) | `QK_TextureQuad` original-vertex terrain batching is wired | Independent front-view runtime proof; sprites and interleaved overlays |
| General triangles and creature shadows: [trig](../../src/kfx/renderer/software/bflib_render_trig.c), world dispatch | CPU reference | All modes, destination-dependent blending and GPU shadow-mask generation |
| World sprites, creatures, objects and effects: [sprite adapter](../../src/kfx/renderer/software/WgpuSprite.c) | RLE index/coverage assets and scale ranges feed GPU source selection, flips, clipping, remap/ghost/alpha; source-frame offsets and water-truncated height preserved; accepted calls bypass native stores | Solid scaled-up horizontal flips with duplicated rows remain native (588 fixture cases); creature shadow masks, custom-asset/gameplay coverage and direct cursor kernels remain open |
| Pixels, boxes, HV lines and circles: [bflib_vidraw.c](../../src/kfx/renderer/software/bflib_vidraw.c) | Native GPU hooks; circles use original center/radius and preserve repeated blend hits | Independent review and combined-head native checks; circle radii above 8,191 and other unsupported inputs decline to CPU |
| General lines, world overlays, HUD/menu sprites: [engine_render.c](../../src/engine_render.c), [UI interface](../../src/kfx/renderer/IUIRenderer.h) | Selected low-level primitives; scaled normal/remap/one-colour/alpha and immediate normal/one-colour sprites | General-line coverage/color selection, unsupported sprite modes and full interleaving validation |
| Text, including Asian fonts: [bflib_sprfnt.c](../../src/bflib_sprfnt.c) | Ordinary sprite-based glyphs reach GPU sprite wrappers; CPU layout retained | Direct DBC bitmap glyphs, unsupported sprite modes and complete underline/shadow/font validation |
| Raw/tiled images, frontend backgrounds, landview/torture/zoom: [raw adapter](../../src/kfx/renderer/software/WgpuRawImage.c), [raw helper](../../src/front_simple.c), [slab helper](../../src/gui_draw.c) | Original raw8 assets with exact floor-endpoint scaling, clipping and black letterbox; 64×64 tiled slabs with native clipped phase; shared static frontend/loading/landview/torture/parchment backgrounds | Huge compressed sprites, landview/parchment zoom and surrounding transforms; aliased source/destination inputs remain native |
| Minimap, parchment and overhead/zoom maps: [frontmenu_ingame_map.c](../../src/frontmenu_ingame_map.c), [gui_parchment.c](../../src/gui_parchment.c) | Static raw parchment background uses GPU image helper; map generation/transforms remain CPU | Semantic map commands, rotation/masks and framebuffer-derived minimap background state |
| Built-in possession lenses: [lens implementations](../../src/kfx/lense/) | CPU reference | GPU target views and indexed displacement, flyeye, mist, overlay and palette effects; preserve alias/order behavior |
| Custom Lua lenses: [LuaLensEffect.cpp](../../src/kfx/lense/LuaLensEffect.cpp), [lua_api_lens.c](../../src/lua_api_lens.c) | CPU reference | Ordered GPU writes/copies and exact read-after-write compatibility for arbitrary pixel-dependent Lua control flow; CPU-script readback is explicit, never hidden CPU-rendered lens upload |
| Smoothing and map fades/transitions: [engine_redraw.c](../../src/engine_redraw.c) | CPU reference | GPU target snapshots and exact indexed effects, including traversal/truncation quirks |
| Movies: [bflib_fmvids.cpp](../../src/bflib_fmvids.cpp) | CPU decode and screen drawing | Upload decoded source assets; GPU centering/scaling/interlace and palette timing |
| Cursor, clears, screenshots and recording: [bflib_mspointer.cpp](../../src/bflib_mspointer.cpp), [RendererSoftware.cpp](../../src/kfx/renderer/RendererSoftware.cpp), [scrcapt.c](../../src/scrcapt.c) | GPU indexed clear for full SDL surface clips, preserving row padding; CPU cursor composition/capture of the synchronized native image | Nonfull SDL clip clears, direct cursor kernel and backup/restore; authoritative GPU capture with matching frame/palette/cursor semantics |
| Cross-family palette, transparency, clipping and scaling | Bounded command and offscreen palette-output fixtures pass | Full-family index/RGBA comparisons; table versions, target aliases and strict CPU-writer/readback audit |

The reviewed original-vertex [GPU preparation test](../../tools/frame-replay/tests/gpoly_gpu.rs)
compared 597,800 setup words and 6,202,175 palette indices, including pitch padding,
against independent native output on Metal. It consumes GPU-produced rows directly
in a subsequent GPU pass. The production [triangle test](../../tools/frame-replay/tests/draw_triangles_gpu.rs)
at `17e993a84` separately checks all 1,225 triangles in ordered overlapping batches,
immutable texture/fade versions and atomic shade/resource/coordinate rejection.
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

The sprite slice through `a7be8ce6b` has 10,789 exact native-reference Metal cases
from the [ASan native fixture](../../tests/sprites/fixture.c), covering RLE transparency
versus opaque indices 0/255, scale/flip/table combinations, clipping and source-frame
offsets. Its 588 scaled solid horizontal-flip declines preserve a native duplicated-row
copy quirk; they are outstanding GPU work, not passing GPU coverage. The cursor's
direct scaling kernel and creature-shadow mask loop bypass the sprite wrappers.
The raw slice through `8a38178ef` has 270 exact native-reference Metal cases from
the [raw fixture](../../tests/raw-images/fixture.c), including tile clipping/phase and
padded clears. Both suites check isolated recursive oracles and source snapshots.
Independent source review found no additional defect; these bounded fixtures do
not replace combined-head gameplay, complete asset coverage or performance evidence.
The native build at `8a38178ef` passed with binary SHA-256
`63a0c73186aae4ef5b53470bdb0e9de7f6cf7e043689b295aae7e0c302e58228`;
that is compilation/linking evidence only. Sprite/raw commands still use the
synchronous full-target upload/readback bridge, including clears and backgrounds.

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
