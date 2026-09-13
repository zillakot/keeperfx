#ifndef KFX_GPOLY_CAPTURE_H
#define KFX_GPOLY_CAPTURE_H

#include <stddef.h>
#include <stdint.h>

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

/* A consumed sink owns copied inputs before returning; every other result uses the legacy loop. */
typedef int (*KfxGpolySink)(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *texture, const uint8_t *fade);

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
int kfx_gpoly_capture_replay(const struct KfxGpolyCapture *capture,
    uint8_t *pixels, size_t length);

#ifdef __cplusplus
}
#endif
#endif
