---
type: index
description: Entry point for understanding this KeeperFX fork, building the Mac game, and developing graphics with Rust frame replay.
---

# KeeperFX documentation

This fork supports learning and incremental modernization while preserving the
original pixel-art appearance. Start with the project overview, then use the
practical guide for the work you want to do.

| Read this | For |
| --- | --- |
| [Understanding KeeperFX](architecture/project-overview.md) | Engine structure, game loop, shared state, graphics, content and the role of Rust |
| [Rust port plan](product/rust-port-plan.md) | Proposed migration phases, the next live presentation milestone and validation criteria |
| [Audio modernization plan](product/audio-modernization-plan.md) | Sound remastering and replacement, Rust audio ownership, audition pack and compatibility criteria |
| [World-data reference](data_structure.md) | Things, creature controls, rooms, slabs, subtiles, columns and cubes |
| [macOS development](macos.md) | Build and run the native Apple Silicon game with the required assets |
| [Frame capture and Rust replay](frame-feedback.md) | Obtain quick visual feedback, compare exact pixels and interpret the timings |
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

The Rust tool currently renders captured frames offscreen. It does not replace
the live game renderer or demonstrate a gameplay performance improvement.

## Upstream resources

The [KeeperFX wiki](https://keeperfx.net/wiki/home) covers playing and modding.
The [upstream development guide](https://github.com/dkfans/keeperfx/wiki/Building-KeeperFX)
covers upstream build workflows. Additional inherited notes remain in this
folder; use the guides above for this fork's Mac and Rust workflows.

Track implementation and validation in [this fork's pull requests](https://github.com/zillakot/keeperfx/pulls).
Issues are currently disabled in the fork.
