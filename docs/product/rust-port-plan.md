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
implementation. Two useful foundations already exist:

| Foundation | Evidence and limits |
| --- | --- |
| Native Apple Silicon C/C++ game | [PR #1](https://github.com/zillakot/keeperfx/pull/1): native launch, initial playtest and save/reload checks; broad campaign and cross-platform compatibility remain unverified. |
| Standalone Rust/wgpu frame replay | [PR #2](https://github.com/zillakot/keeperfx/pull/2): exact captured-frame comparisons and regression tests; Rust does not yet render the running game. |

The [frame-feedback workflow](../frame-feedback.md) is our visual comparison tool.
It establishes pixel correctness for captured input, not gameplay performance or
simulation equivalence.

## Proposed phases

| Phase | Outcome | Evidence required before expanding scope |
| --- | --- | --- |
| 1. Broaden the reference cases | Reusable visual fixtures and measurements for representative scenes | Menu, dungeon, possession, palette changes and interface cases; separately measured simulation, drawing and presentation costs |
| 2. Present live frames through Rust | Optional Rust/wgpu presentation in the playable game | Exact palette/frame comparisons, working window lifecycle, successful gameplay checks and a working original-renderer fallback |
| 3. Replace one bounded utility | A small live engine component implemented in safe Rust behind a narrow interface | Valid-input equivalence, malformed-input tests, bounded allocations and verified ownership at the language boundary |
| 4. Establish world-state ownership | Typed Rust data for a bounded part of the world, with explicit adapters to legacy state | One authoritative writer, stable identifiers and verified save/network conversion for the affected data |
| 5. Migrate gameplay systems | Move rules and simulation one system at a time | Replayed command sequences produce equivalent gameplay state; save/load, Lua and multiplayer checks cover each migrated system |
| 6. Move application ownership | Rust owns startup, game orchestration and the remaining game implementation | Agreed campaign, input, audio, rendering, save/load and multiplayer coverage; a distributable build without the KeeperFX C/C++ implementation |

The order prioritizes our existing graphics work. A bounded utility can be tackled
independently if platform integration delays phase 2. Later phases require their
own scoped designs; success with frame replay is not evidence that the whole port
will be small or quick.

## Next milestone: optional live Rust presentation

First expand the fixtures enough to cover palettes, transparent entries, interface
screens, different resolutions and possession. Then extract reusable rendering
logic from [the replay tool](../../tools/frame-replay/src/gpu.rs) into a library
that both the offline tool and a live adapter can use.

The current tool creates GPU resources and reads back the result for each run.
The live path must instead retain the device, queue, surface and reusable textures
across frames, and avoid routine GPU-to-CPU readback. Continue drawing the world
with the existing CPU renderer and upload its indexed pixels and palette.

Use the [renderer interfaces](../../src/kfx/renderer/IRenderer.h) and
[SDL window implementation](../../src/kfx/platform/WindowSystemSDL.cpp) to define
ownership of the native window and presentation surface. Keep SDL input handling
in place. The two presentation backends must not compete for the same window.

Completion criteria for this milestone:

- The existing presentation path remains the default; the Rust path is explicitly selectable.
- Captured output remains byte-identical at the supported integer scales and palette transitions.
- Resize, minimize/restore, focus changes, fullscreen, display scaling, shutdown and surface/device errors have defined, tested behavior.
- The original path remains usable when Rust initialization fails or the option is disabled; error recovery is exercised rather than assumed.
- Menu navigation, digging, creature possession, sound and save/reload work during an Apple Silicon playtest.
- CI retains Windows and Linux builds. Live Rust support on another platform is claimed only after runtime checks there; it can continue using the original path meanwhile.
- CPU time, allocations and frame-time distributions are compared with the original presentation path under the same settings. Any regression is explained before making Rust the default.

This milestone proves live integration. It is unlikely to create a large speedup
because the existing path already presents through SDL/Metal on the tested Mac.

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

Preserve the original sprites, textures and palette behavior throughout the port.
Moving terrain, sprites or lighting to GPU drawing is a separate graphics project,
not a prerequisite for using Rust. It needs scene data before rasterization, which
the current framebuffer replay does not supply.

Profile representative workloads first: a quiet level, a busy dungeon and
possession, with recorded resolution, frame limits and VSync settings. Measure
simulation, CPU drawing, presentation, memory and frame-time distributions.
Choose a GPU rendering milestone only when those measurements justify it. Specify
visual acceptance criteria before changing rasterization; do not relax the exact
presentation comparison merely to make a new backend pass.

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
