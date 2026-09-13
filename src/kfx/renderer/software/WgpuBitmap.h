#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
struct TbHugeSprite;
#ifdef __cplusplus
extern "C" {
#endif
int kfx_wgpu_bitmap_huge(uint8_t *dst, int pitch, int height, const int32_t *xs,
    const int32_t *ys, const struct TbHugeSprite *sprite, size_t length,
    KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_bitmap_font(const struct KfxGpolyTarget *target, int wx, int wy,
    int ww, int wh, int x, int y, const uint8_t *bits, int sw, int sh,
    int dw, int dh, int scaled, int foreground, int background, int shadow,
    KfxWgpuNativeOracle oracle, void *context);
#ifdef __cplusplus
}
#endif
