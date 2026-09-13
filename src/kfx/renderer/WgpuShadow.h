#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#define KFX_WGPU_DRAW_SHADOW 11u

#ifdef __cplusplus
extern "C" {
#endif

/* Source: eight LE u32 fields (clear width/height, sprite width/height, x/y offset,
 * horizontal flip, RLE byte length), two 60-byte triangle geometries, 65536 prior
 * scratch bytes, then immutable signed-run sprite data. GPU produces the mask and
 * samples its snapshot before copying the resulting scratch to the compatibility mirror.
 */
int32_t kfx_wgpu_draw_submit_shadow(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *command, uint8_t *mirror, size_t length,
    char *error, size_t capacity);
int kfx_wgpu_native_shadow(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, uint8_t *scratch,
    KfxWgpuNativeOracle oracle, void *oracle_context);
#ifdef __cplusplus
}
#endif
