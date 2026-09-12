---
type: reference
description: Explains KeeperFX world entities and terrain structures, with links to their current C definitions and navigation code.
---

# World-data reference

For the game loop and component relationships, read the
[project overview](architecture/project-overview.md).

## Entities and ownership

| Structure | Role | Definition |
| --- | --- | --- |
| `Thing` | World entity with a class, model, owner, position and state; examples include creatures, objects, shots, effects, traps and doors | [thing_data.h](../src/thing_data.h) |
| `CreatureControl` | Additional creature behavior and control state | [creature_control.h](../src/creature_control.h) |
| `Room` | Room state and its associated slabs | [room_data.h](../src/room_data.h) |
| `Dungeon` | A player's dungeon state, resources and statistics | [dungeon_data.h](../src/dungeon_data.h) |
| `Game` | Shared state containing arrays of entities, players, terrain and other systems | [game_legacy.h](../src/game_legacy.h) |

A Thing's class selects its broad kind; its model identifies a configured type
within that class. Its owner identifies the associated player. Many relationships
use indices into shared arrays, resolved through helpers such as `thing_get()`.
Terrain and rooms have their own structures, so not every game concept is a Thing.

## Terrain hierarchy

A slab is a tile-sized unit used for terrain kinds, ownership and room membership.
Each slab covers 3 × 3 subtiles, as defined by `STL_PER_SLB` in
[map_data.h](../src/map_data.h).

| Level | Representation | Purpose |
| --- | --- | --- |
| Slab | `SlabMap` in [slab_data.h](../src/slab_data.h) | Terrain kind, owner, health and room membership |
| Subtile | `Map` in [map_data.h](../src/map_data.h) | Finer map cell, including a column index, entity-list reference and reveal flags |
| Column | `Column` in [map_columns.h](../src/map_columns.h) | Vertical cube stack and floor/solid information; the current stack capacity is eight cubes |
| Cube | Definitions in [cubes.cfg](../config/fxdata/cubes.cfg) | Cube surface textures used to draw terrain |

[terrain.cfg](../config/fxdata/terrain.cfg) defines terrain properties.
[slabset.toml](../config/fxdata/slabset.toml) and
[columnset.toml](../config/fxdata/columnset.toml) describe slab/column layouts;
[textureanim.toml](../config/fxdata/textureanim.toml) defines texture animations.

## Navigation

Navigation uses more than a slab grid. The Ariadne system includes a triangle
navigation representation and wall-hugging behavior. Start with
[ariadne.h](../src/ariadne.h), [ariadne_tringls.h](../src/ariadne_tringls.h) and
[ariadne_wallhug.c](../src/ariadne_wallhug.c) when tracing a creature's route.

## Layout and compatibility

Some records are explicitly packed and participate in saved or networked state.
Before changing a field or its alignment, inspect its uses in
[game_saves.c](../src/game_saves.c), [packets.c](../src/packets.c) and the network
code. Reaching gameplay is useful runtime evidence, but does not establish
cross-platform save or multiplayer compatibility.
