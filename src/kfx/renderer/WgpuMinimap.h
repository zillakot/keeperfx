#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#define KFX_WGPU_DRAW_MINIMAP 12u
/* Minimap source: 24 little-endian u32 parameters, immutable draw-square offsets,
 * optional background palette dictionary, semantic u16 world cells, palette tables.
 * Setup captures a GPU target snapshot; slab sampling uses that retained version. */
