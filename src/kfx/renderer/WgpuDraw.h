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
 * Ordered sprite source_y records the native target base byte alignment modulo 4.
 * Bit 3 preserves solid-RL row-copy order; coverage 2 ends each positive RLE run.
 * Scaling ranges are target-relative; table uses the ordinary blend axes. */
#define KFX_WGPU_DRAW_RAW_IMAGE 7u
#define KFX_WGPU_DRAW_TILED_IMAGE 8u
#define KFX_WGPU_DRAW_TRIG 9u
#define KFX_WGPU_DRAW_LENS_EFFECT 10u
#define KFX_WGPU_DRAW_MOVIE 13u
#define KFX_WGPU_DRAW_MAP_VIEW 14u
#define KFX_WGPU_DRAW_BITMAP 15u
#define KFX_WGPU_DRAW_TRANSITION 16u
/* MOVIE source_x bits select width doubling, line doubling and interlace;
 * start is the signed native image origin. Packed doubling omits width tails. */
/* TRIG source begins with three x/y/u/v/shade little-endian i32 vertices,
 * then source_y texture bytes. source_x is the native mode, source_width is
 * the native long width (64). Table is 64 fade rows followed by 256 ghost rows. */
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
 * Submit returns 1 when accepted, -1 on error; host-rejected batches do not mutate targets.
 * A GPU-detected invalid lookup is not a rejection: the command's own write is skipped and
 * the frame flag is raised for kfx_wgpu_draw_frame_status.
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

#define KFX_WGPU_DRAW_PASS_KINDS 8

struct KfxWgpuDrawCounters {
    uint64_t batches, commands, asset_upload_bytes, command_upload_bytes, readback_bytes;
    /* wait_ns is host stall time inside blocking device polls; it is host-side.
     * Only pass_ns below is GPU execution time, and only when timing is enabled. */
    uint64_t submits, dispatches, waits, wait_ns, buffers, buffer_bytes;
    uint64_t arena_evictions, arena_overflows, arena_bytes_uploaded;
    /* Host-side staged asset bytes the context holds, not GPU memory; a gauge.
     * arena_bytes_resident is the suballocated GPU arena extent, also a gauge. */
    uint64_t host_staged_asset_bytes, arena_bytes_resident;
    /* tile_allocations counts growths of the persistent binning scratch; zero after warm-up. */
    uint64_t tile_allocations, tile_entries;
    /* The terrain share of tile_entries; terrain inner-loop iterations are 256 times it. */
    uint64_t terrain_tile_entries;
    /* Words the compressed prepared-terrain row arena carried, and its growths. */
    uint64_t prepared_row_words, prepared_row_allocations;
    /* Opt-in per-pass GPU execution time, in KFX_WGPU_DRAW_PASS_KINDS order; zero
     * unless KFX_WGPU_GPU_TIMING=1 and the adapter supports timestamp queries. */
    uint64_t pass_ns[KFX_WGPU_DRAW_PASS_KINDS];
    uint64_t timed_passes, untimed_passes;
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
