#pragma once

#include "WgpuDraw.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Snapshot handles are distinct from CPU asset handles. Region bounds must fit the
 * target; pitch is in indices and padding is zero. Captures prior queued draws using
 * GPU copies only. Each handle is an immutable version, surviving target writes and
 * release. Release never cancels submitted sampling. All calls require serialization.
 * These versions have no CPU reconstruction data: terminal device loss invalidates
 * them. Callers must retain reconstructible inputs before adopting software fallback.
 */
uint64_t kfx_wgpu_draw_target_snapshot(void *drawing, uint64_t target,
    uint32_t x, uint32_t y, uint32_t width, uint32_t height, uint32_t pitch,
    char *error, size_t capacity);
int32_t kfx_wgpu_draw_target_snapshot_release(void *drawing, uint64_t snapshot,
    char *error, size_t capacity);

/* IMAGE commands only: source is a snapshot; table is an ordinary CPU asset.
 * Uses IMAGE nearest sampling, clip, transparency and blend semantics. Overlap reads
 * immutable snapshot pixels, never earlier destination writes. Calls order with other
 * draw submissions. Invalid batches leave the target unchanged; terminal GPU failure
 * requires reconstruction. Returns 1 on acceptance, -1 on error.
 */
int32_t kfx_wgpu_draw_submit_target_images(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *commands, size_t count, char *error, size_t capacity);

/* TRIG sources contain 60 geometry bytes; texture is a 256x256 GPU snapshot.
 * Tables remain CPU asset handles. Entire batches validate before target writes.
 */
int32_t kfx_wgpu_draw_submit_target_triangles(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *commands, size_t count, uint64_t texture,
    char *error, size_t capacity);

#pragma pack(push, 8)
struct KfxWgpuTargetResourceCounters {
    uint64_t snapshots, snapshot_copy_bytes, sampling_copy_bytes;
};
#pragma pack(pop)

int32_t kfx_wgpu_draw_target_resource_counters(void *drawing,
    struct KfxWgpuTargetResourceCounters *output, char *error, size_t capacity);

#ifdef __cplusplus
}
#endif
