#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
struct TbHugeSprite;
#ifdef __cplusplus
extern "C" {
#endif
int kfx_wgpu_bitmap_huge(uint8_t *dst, int pitch, int height, const int32_t *xs,
    const int32_t *ys, const struct TbHugeSprite *sprite, size_t length,
    KfxWgpuNativeOracle oracle, void *context);
/* glyph_id names the font and character the bits belong to; zero draws unnamed, which
 * costs a fresh asset per call. The colours are part of the asset, so they join the key. */
int kfx_wgpu_bitmap_font(const struct KfxGpolyTarget *target, int wx, int wy,
    int ww, int wh, int x, int y, const uint8_t *bits, int sw, int sh,
    int dw, int dh, int scaled, int foreground, int background, int shadow,
    uint64_t glyph_id, KfxWgpuNativeOracle oracle, void *context);
#ifdef __cplusplus
}
#endif
