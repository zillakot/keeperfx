#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#ifdef __cplusplus
extern "C" {
#endif
int kfx_wgpu_map_row(uint8_t *dst, int pitch, int height, int x, int y, int block_size,
    const int *styles, int count, const uint8_t *ghost, const uint8_t *abyss,
    KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_map_texture(uint8_t *dst, int pitch, int width, int height, int x, int y,
    int dw, int dh, int flags, const uint8_t *texture, const uint8_t *fade,
    KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_map_zoom(uint8_t *dst, int pitch, int width, int height, int x, int y,
    int map_x, int map_y, int delta, const uint8_t *source, int sw, int sh,
    KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_map_marker(uint8_t *dst, int pitch, int height, int x, int y,
    const int32_t *pattern, int count, int spread, int cross, uint8_t colour,
    KfxWgpuNativeOracle oracle, void *context);
#ifdef __cplusplus
}
#endif
