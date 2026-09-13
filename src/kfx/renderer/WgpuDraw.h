#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define KFX_WGPU_DRAW_ABI_VERSION 1u
#define KFX_WGPU_DRAW_CLEAR 0u
#define KFX_WGPU_DRAW_RECT 1u
#define KFX_WGPU_DRAW_IMAGE 2u
#define KFX_WGPU_DRAW_GPOLY_SPAN 3u
#define KFX_WGPU_DRAW_CIRCLE_FILLED 4u
#define KFX_WGPU_DRAW_CIRCLE_OUTLINE 5u
#define KFX_WGPU_DRAW_SPRITE 6u
/* SPRITE source: index/coverage byte pairs, x then y little-endian u32
 * start/count pairs, and a 256-byte remap. source_width/height give decoded size;
 * source_x bits 0/1/2 select horizontal flip, vertical flip, and one-colour.
 * Bit 3 preserves solid-RL row-copy order; coverage 2 ends each positive RLE run.
 * Scaling ranges are target-relative; table uses the ordinary blend axes. */
#define KFX_WGPU_DRAW_RAW_IMAGE 7u
#define KFX_WGPU_DRAW_TILED_IMAGE 8u
/* RAW_IMAGE covers the target; start is signed image origin and step is destination size. */
/* Circle radius is source_width; bounds are the inclusive diameter square. */
#define KFX_WGPU_DRAW_REPLACE 0u
#define KFX_WGPU_DRAW_SOURCE_DESTINATION 1u
#define KFX_WGPU_DRAW_DESTINATION_SOURCE 2u
#define KFX_WGPU_DRAW_OPAQUE 256u

/* Commands and resource bytes are copied before return. Calls require serialization.
 * Handles belong to their creating drawing context. Release does not cancel submitted work.
 * Coordinates and clips are target-relative; rectangles use half-open bounds.
 * GPOLY starts are already clipped/truncated by legacy setup; shader steps wrap at 64 bits.
 * Submit returns 1 when accepted, -1 on error; rejected batches do not mutate targets.
 * A terminal GPU error requires reconstruction before software fallback can display pixels.
 */
#pragma pack(push, 8)
struct KfxWgpuDrawCommand {
    uint32_t abi_version, kind, blend, colour;
    int32_t x, y;
    uint32_t width, height;
    int32_t clip_x, clip_y;
    uint32_t clip_width, clip_height;
    uint64_t source, table;
    uint32_t source_x, source_y, source_width, source_height;
    uint32_t start_low, start_high, step_low, step_high;
    uint32_t transparent, reserved[3];
};

struct KfxWgpuDrawCounters {
    uint64_t batches, commands, asset_upload_bytes, command_upload_bytes, readback_bytes;
};
#pragma pack(pop)

int32_t kfx_wgpu_draw_counters(void *drawing, struct KfxWgpuDrawCounters *output,
    char *error, size_t capacity);

/* A headless context is owned; a presenter's context is borrowed until presenter destruction. */
void *kfx_wgpu_draw_create(char *error, size_t capacity);
void kfx_wgpu_draw_destroy(void *drawing);
void *kfx_wgpu_draw_context(void *presenter, char *error, size_t capacity);

uint64_t kfx_wgpu_draw_target_create(void *drawing, uint32_t width, uint32_t height,
    char *error, size_t capacity);
int32_t kfx_wgpu_draw_target_release(void *drawing, uint64_t target,
    char *error, size_t capacity);
uint64_t kfx_wgpu_draw_resource_create(void *drawing, const uint8_t *bytes, size_t length,
    uint32_t width, uint32_t height, uint32_t pitch, char *error, size_t capacity);
int32_t kfx_wgpu_draw_resource_release(void *drawing, uint64_t resource,
    char *error, size_t capacity);
int32_t kfx_wgpu_draw_submit(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *commands, size_t count, char *error, size_t capacity);
int32_t kfx_wgpu_draw_readback(void *drawing, uint64_t target,
    uint8_t *indices, size_t length, uint32_t pitch, char *error, size_t capacity);
/* Returns 0 for an occluded/timed-out surface, 1 when ready for kfx_wgpu_present. */
int32_t kfx_wgpu_draw_prepare_present(void *presenter, uint64_t target,
    const uint8_t *palette, size_t palette_length, uint32_t output_width,
    uint32_t output_height, int32_t vsync, char *error, size_t capacity);

#ifdef __cplusplus
}
#endif
