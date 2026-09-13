#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#ifdef __cplusplus
extern "C" {
#endif
int kfx_wgpu_raw_image(uint8_t *dst, int pitch, int height, int dw, int dh, int x, int y,
    const uint8_t *source, int sw, int sh, KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_raw_tile(uint8_t *dst, int pitch, int height, int x, int y, int width, int rows,
    const uint8_t *source, int size, KfxWgpuNativeOracle oracle, void *context);
int kfx_wgpu_raw_clear(uint8_t *dst, int pitch, int width, int height, uint8_t colour);
#ifdef __cplusplus
}
#endif
