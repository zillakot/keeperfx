#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#ifdef __cplusplus
extern "C" {
#endif
int kfx_wgpu_map_fade(uint8_t* dst, int pitch, int width, int height,
    const uint8_t* first, const uint8_t* second, uint64_t first_snapshot, uint64_t second_snapshot,
    const uint8_t* fade, const uint8_t* ghost, int progress,
    KfxWgpuNativeOracle oracle, void* context);
int kfx_wgpu_smooth(uint8_t* dst, int pitch, int height, int x, int y, int right, int bottom,
    const uint8_t* ghost, KfxWgpuNativeOracle oracle, void* context);
#ifdef __cplusplus
}
#endif
