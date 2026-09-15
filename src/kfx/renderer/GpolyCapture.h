#ifndef KFX_GPOLY_CAPTURE_H
#define KFX_GPOLY_CAPTURE_H

#include <stddef.h>
#include <stdint.h>
#include "kfx/renderer/WgpuTriangle.h"

#ifdef __cplusplus
extern "C" {
#endif

enum { KFX_GPOLY_TEXTURE_BYTES = 8192, KFX_GPOLY_FADE_BYTES = 16384 };
enum { KFX_GPOLY_DECLINED = 0, KFX_GPOLY_CONSUMED = 1, KFX_GPOLY_ERROR = -1 };

struct KfxGpolySpan {
    int32_t x, y;
    uint32_t count, start_low, start_high, step_low, step_high;
};

struct KfxGpolyTarget {
    uint8_t *pixels;
    uint32_t width, height, pitch;
};

/* Rasterizers may write scratch pixels before reporting an invalid shade. */
typedef int (*KfxGpolyRasterizer)(const struct KfxGpolyTarget*,
    const struct KfxWgpuTriangle*, const uint8_t*, const uint8_t*);
typedef int (*KfxGpolyTriangleSink)(void*, const struct KfxGpolyTarget*,
    const struct KfxWgpuTriangle*, const uint8_t*, const uint8_t*, KfxGpolyRasterizer);
extern KfxGpolyTriangleSink kfx_gpoly_triangle_sink;
extern void* kfx_gpoly_triangle_context;

/* A consumed sink owns copied inputs before returning; every other result uses the legacy loop. */
typedef int (*KfxGpolySink)(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *texture, const uint8_t *fade);

/* Identity generation for immutable drawing assets (texture pages, fade and ghost
 * tables). Bump it after the bytes behind a stable pointer are rewritten, and also
 * before a rewrite that can fail part-way, so a resource cached mid-write is
 * invalidated too. Resource caches key on pointer plus this value instead of
 * comparing content, but only inside a range registered as stable; anything else
 * still compares content. */
extern uint64_t kfx_render_asset_generation;
void kfx_render_assets_changed(void);
/* Identity generation for sprite artwork, separate so that adding one Lua sprite does
 * not drop every terrain texture and fade table. Bump it whenever expanded artwork
 * behind a live sprite name can change: a sheet or font loaded, freed or extended, a
 * keepersprite frame loaded into the graphics heap, a heap reset, or a level load. */
extern uint64_t kfx_render_sprite_generation;
void kfx_render_sprites_changed(void);
/* Registers storage whose bytes only change with a generation bump. At most
 * KFX_RENDER_ASSET_RANGES ranges; re-registering the same base replaces it. */
enum { KFX_RENDER_ASSET_RANGES = 8 };
void kfx_render_asset_range(const void *base, size_t length);
/* Registrations refused because every slot was taken. Losing a name is safe — the asset
 * falls back to a per-call resource — but it is silent, so it is counted. */
extern uint64_t kfx_render_asset_range_drops;
/* Drops a registration before its storage is freed, so a later allocation at the same
 * address is not mistaken for the asset that used to live there. */
void kfx_render_asset_range_forget(const void *base);
int kfx_render_asset_stable(const void *bytes, size_t length);

void kfx_gpoly_set_sink(KfxGpolySink sink, void *context);
extern KfxGpolySink kfx_gpoly_sink;
extern void *kfx_gpoly_sink_context;

struct KfxGpolyCapturedSpan {
    struct KfxGpolySpan span;
    uint32_t texture, fade;
};

struct KfxGpolyCapture {
    struct KfxGpolyTarget target;
    struct KfxGpolyCapturedSpan *spans;
    uint8_t **textures, **fades;
    uint32_t span_count, texture_count, fade_count;
    uint32_t span_limit, resource_limit;
    int failed;
};

int kfx_gpoly_capture_init(struct KfxGpolyCapture *capture,
    uint32_t span_limit, uint32_t resource_limit);
void kfx_gpoly_capture_free(struct KfxGpolyCapture *capture);
int kfx_gpoly_capture_sink(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *texture, const uint8_t *fade);
/* Failure leaves pixels unchanged; success replays onto caller-provided initial pixels. */
int kfx_gpoly_capture_replay(const struct KfxGpolyCapture *capture,
    uint8_t *pixels, size_t length);

#ifdef __cplusplus
}
#endif
#endif
