---
type: index
description: Entry point for understanding this KeeperFX fork, building the Mac game, and developing graphics and audio.
---

# KeeperFX documentation

This fork supports learning and incremental modernization while preserving the
original pixel-art appearance. Start with the project overview, then use the
practical guide for the work you want to do.

| Read this | For |
| --- | --- |
| [Understanding KeeperFX](architecture/project-overview.md) | Engine structure, game loop, shared state, graphics, content and the role of Rust |
| [Rust port plan](product/rust-port-plan.md) | Proposed migration phases, delivered live presentation, active graphics migration and migration gates |
| [Single-stream wgpu renderer](architecture/wgpu-single-stream-renderer.md) | Target wgpu drawing architecture, per-frame acceptance counters, C ABI changes, fixtures and the PR-sized migration sequence |
| [Development tooling plan](product/development-tooling-plan.md) | Tooling and process improvements for the graphics track: measurement, determinism, offline replay, property tests, CI structure and the parity policy |
| [Audio modernization plan](product/audio-modernization-plan.md) | Sound remastering and replacement, Rust audio ownership, audition pack and compatibility criteria |
| [Audio cue inventory](audio/inventory.md) | Reproducible definitions, playback references, override rules and unresolved source evidence |
| [Audio command references](audio-reference.md) | Isolated audio-enabled scenes, bounded playback tracing and remaining baseline checks |
| [Audio audition pack](audio-audition.md) | Opt-in procedural cues, private restoration recipe, comparison clips and listening criteria |
| [World-data reference](data_structure.md) | Things, creature controls, rooms, slabs, subtiles, columns and cubes |
| [macOS development](macos.md) | Build and run the native Apple Silicon game with the required assets |
| [Frame capture and Rust replay](frame-feedback.md) | Obtain quick visual feedback, compare exact pixels and interpret the timings |
| [Native game control](native-game-control.md) | Isolated game-event input, state predicates, screenshots and real SDL window operations |
| [Live Rust presentation](live-rust-presentation.md) | Optional Metal surface integration, ownership and validation |
| [Performance baselines](performance-baselines.md) | Isolated native simulation, drawing and presentation measurements and their limits |
| [Original game files](files_required_from_original_dk.txt) | Files to copy from an original Dungeon Keeper installation |
| [Steam Deck development](steam-deck.md) | Existing Linux build and remote deployment workflow |

## Implementation and evidence

Use `origin` for the personal fork and `upstream` for `dkfans/keeperfx`. Work on
feature branches and merge through PRs targeting this fork's `master` branch.

The PRs are the durable records of changes and their validation:

- [PR #1: Native Apple Silicon build](https://github.com/zillakot/keeperfx/pull/1)
  records the native build, launch, initial gameplay and save/reload checks.
- [PR #2: Frame capture and Rust GPU replay](https://github.com/zillakot/keeperfx/pull/2)
  records exact image comparisons, deliberate-mismatch checks and CI coverage.
- [PR #9: Live Rust presentation and native control](https://github.com/zillakot/keeperfx/pull/9)
  records live integration, final-source UI/window checks and the final 30-run
  presentation comparison.

The shared Rust palette pipeline supports offline replay and optional live Metal
presentation. CPU world drawing remains unchanged; use paired live measurements
to assess presentation costs. The final comparison established no reliable overall
performance win; SDL remains the default. Continue with the
[active graphics migration](product/rust-port-plan.md#active-delivery-full-wgpu-drawing).

The audio foundations are recorded in [PR #11](https://github.com/zillakot/keeperfx/pull/11)
(command references), [PR #12](https://github.com/zillakot/keeperfx/pull/12)
(cue inventory) and [PR #13](https://github.com/zillakot/keeperfx/pull/13)
(audition pack). Two representative replacement cues have reached OpenAL in an
isolated run; listening acceptance, full baseline coverage and Rust audio remain
open. Follow the [audio plan](product/audio-modernization-plan.md) for the next
work and each guide for the limits of its evidence.

## Upstream resources

The [KeeperFX wiki](https://keeperfx.net/wiki/home) covers playing and modding.
The [upstream development guide](https://github.com/dkfans/keeperfx/wiki/Building-KeeperFX)
covers upstream build workflows. Additional inherited notes remain in this
folder; use the guides above for this fork's Mac and Rust workflows.

Track implementation and validation in [this fork's pull requests](https://github.com/zillakot/keeperfx/pulls).
Issues are currently disabled in the fork.
