#pragma once
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Views retain the canonical target's storage. Coordinates remain view-relative.
 * During an active frame submit copies commands and accepts them for validation.
 * Flush/end validate and commit atomically; failure preserves the prior checkpoint.
 * An invalid frame requires abort. Resource releases retain queued versions.
 * Readback and GPU snapshot operations are explicit checkpoints. */
uint64_t kfx_wgpu_draw_target_view(void *drawing, uint64_t parent,
    uint32_t x, uint32_t y, uint32_t width, uint32_t height, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_begin(void *drawing, uint64_t root, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_flush(void *drawing, char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_end(void *drawing, char *error, size_t capacity);
struct KfxWgpuFrameCounters {
    uint64_t queued_commands, checkpoints, validation_waits, validation_bytes;
    uint64_t checkpoint_copy_bytes, rejected_checkpoints;
};
int32_t kfx_wgpu_draw_frame_counters(void *drawing, struct KfxWgpuFrameCounters *output,
    char *error, size_t capacity);
int32_t kfx_wgpu_draw_frame_abort(void *drawing, char *error, size_t capacity);
#ifdef __cplusplus
}
#endif
