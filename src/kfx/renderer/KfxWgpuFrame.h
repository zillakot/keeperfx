#pragma once
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Views retain the canonical target's storage. Coordinates remain view-relative.
 * During an active frame submit copies commands and appends them to the frame's stream;
 * accepted commands write the canonical target as the frame is recorded, with no
 * intermediate copy. Host validation still rejects a batch before any target write, and
 * a frame it invalidates requires abort. A lookup the GPU finds out of range skips its
 * own write, leaves earlier writes intact and raises a frame flag; the frame presents as
 * drawn, kfx_wgpu_draw_frame_status reports the flag without blocking, typically within two
 * frames, and recovery is a full redraw, not a rollback. The bound is typical rather than
 * guaranteed: a publish whose ring slot is still mapped defers to the next flush without
 * losing the flag. Recovery does not reach target snapshots already taken from a flagged
 * frame; their owner must release them. Resource releases retain queued versions.
 * Readback and GPU snapshot operations flush the frame's recorded work. */
uint64_t kfx_wgpu_draw_target_view(void *drawing, uint64_t parent,
    uint32_t x, uint32_t y, uint32_t width, uint32_t height, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_begin(void *drawing, uint64_t root, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_flush(void *drawing, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_end(void *drawing, char *error, size_t capacity);
struct KfxWgpuFrameCounters {
    uint64_t queued_commands, checkpoints, validation_waits, validation_bytes;
    uint64_t checkpoint_copy_bytes, rejected_checkpoints;
    uint64_t invalid_frames, status_reads, status_stalls;
};
int32_t kfx_wgpu_draw_frame_counters(void *drawing, struct KfxWgpuFrameCounters *output,
    char *error, size_t capacity);
/* Non-blocking. Reports and clears the flags raised by the most recent frame whose
 * staging read has completed, so a flag reaches the caller within two frames.
 * Bit 0 is the frame flag; the remaining bits name the kernel check that raised it. */
int32_t kfx_wgpu_draw_frame_status(void *drawing, uint32_t *flags, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_abort(void *drawing, char *error, size_t capacity);
#ifdef __cplusplus
}
#endif
