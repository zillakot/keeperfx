#include "GpolyCapture.h"

#include <stdlib.h>
#include <string.h>

KfxGpolySink kfx_gpoly_sink;
void *kfx_gpoly_sink_context;

void kfx_gpoly_set_sink(KfxGpolySink sink, void *context)
{
    kfx_gpoly_sink = sink;
    kfx_gpoly_sink_context = context;
}

int kfx_gpoly_capture_init(struct KfxGpolyCapture *capture,
    uint32_t span_limit, uint32_t resource_limit)
{
    memset(capture, 0, sizeof(*capture));
    if (!span_limit || !resource_limit)
        return 0;
    capture->spans = calloc(span_limit, sizeof(*capture->spans));
    capture->textures = calloc(resource_limit, sizeof(*capture->textures));
    capture->fades = calloc(resource_limit, sizeof(*capture->fades));
    if (!capture->spans || !capture->textures || !capture->fades) {
        kfx_gpoly_capture_free(capture);
        return 0;
    }
    capture->span_limit = span_limit;
    capture->resource_limit = resource_limit;
    return 1;
}

void kfx_gpoly_capture_free(struct KfxGpolyCapture *capture)
{
    for (uint32_t i = 0; i < capture->texture_count; ++i)
        free(capture->textures[i]);
    for (uint32_t i = 0; i < capture->fade_count; ++i)
        free(capture->fades[i]);
    free(capture->textures);
    free(capture->fades);
    free(capture->spans);
    memset(capture, 0, sizeof(*capture));
}

static int snapshot(uint8_t **resources, uint32_t *count, uint32_t limit,
    const uint8_t *bytes, size_t length)
{
    for (uint32_t i = 0; i < *count; ++i) {
        if (memcmp(resources[i], bytes, length) == 0)
            return (int)i;
    }
    if (*count == limit || *count > INT32_MAX)
        return -1;
    uint8_t *copy = malloc(length);
    if (!copy)
        return -1;
    memcpy(copy, bytes, length);
    resources[*count] = copy;
    return (int)(*count)++;
}

int kfx_gpoly_capture_sink(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *texture, const uint8_t *fade)
{
    struct KfxGpolyCapture *capture = context;
    if (capture->failed)
        return KFX_GPOLY_ERROR;
    if (!target->pixels || !texture || !fade || !target->width || !target->height ||
        target->pitch < target->width || span->x < 0 || span->y < 0 ||
        (uint32_t)span->y >= target->height || (uint32_t)span->x > target->width ||
        span->count > target->width - (uint32_t)span->x ||
        capture->span_count == capture->span_limit)
        goto failed;
    if (capture->span_count && (capture->target.pixels != target->pixels ||
        capture->target.width != target->width || capture->target.height != target->height ||
        capture->target.pitch != target->pitch))
        goto failed;

    uint8_t packed_texture[KFX_GPOLY_TEXTURE_BYTES] = {0};
    for (uint32_t y = 0; y < 32; ++y)
        memcpy(packed_texture + y * 256, texture + y * 256, 32);
    int texture_id = snapshot(capture->textures, &capture->texture_count,
        capture->resource_limit, packed_texture, sizeof(packed_texture));
    int fade_id = snapshot(capture->fades, &capture->fade_count,
        capture->resource_limit, fade, KFX_GPOLY_FADE_BYTES);
    if (texture_id < 0 || fade_id < 0)
        goto failed;
    capture->target = *target;
    capture->spans[capture->span_count++] = (struct KfxGpolyCapturedSpan){
        *span, (uint32_t)texture_id, (uint32_t)fade_id};
    return KFX_GPOLY_DECLINED;

failed:
    capture->failed = 1;
    return KFX_GPOLY_ERROR;
}

int kfx_gpoly_capture_replay(const struct KfxGpolyCapture *capture,
    uint8_t *pixels, size_t length)
{
    if (capture->failed || !pixels ||
        (uint64_t)capture->target.pitch * capture->target.height > length)
        return 0;
    for (uint32_t i = 0; i < capture->span_count; ++i) {
        const struct KfxGpolySpan *span = &capture->spans[i].span;
        uint32_t low = span->start_low;
        for (uint32_t x = 0; x < span->count; ++x) {
            if ((low & 0xff00) >= KFX_GPOLY_FADE_BYTES)
                return 0;
            low += span->step_low;
        }
    }
    for (uint32_t i = 0; i < capture->span_count; ++i) {
        const struct KfxGpolyCapturedSpan *command = &capture->spans[i];
        const struct KfxGpolySpan *span = &command->span;
        uint32_t low = span->start_low, high = span->start_high;
        for (uint32_t x = 0; x < span->count; ++x) {
            uint32_t uv = ((high << 8) | (high >> 24)) & 0x1f1f;
            uint32_t shade = low & 0xff00;
            pixels[(size_t)span->y * capture->target.pitch + span->x + x] =
                capture->fades[command->fade][shade | capture->textures[command->texture][uv]];
            uint32_t next = low + span->step_low;
            high += span->step_high + (next < low);
            low = next;
        }
    }
    return 1;
}
