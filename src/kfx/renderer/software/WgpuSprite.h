#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "bflib_vidraw.h"
#include "bflib_sprite.h"

int kfx_wgpu_sprite(long posx, long posy, const struct TbSourceBuffer *source,
    const struct TbSprite *sprite, const TbPixel *remap, TbPixel colour, unsigned mode);
/* Observation of the expanded-artwork cache, for tests that assert its bounds; the
 * emitter never reads these back. Either pointer may be NULL. */
void kfx_wgpu_sprite_cache_stats(size_t *entries, size_t *bytes);
