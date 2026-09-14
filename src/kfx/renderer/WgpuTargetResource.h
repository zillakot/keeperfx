#pragma once

#include "WgpuDraw.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Snapshot handles are distinct from CPU asset handles. Region bounds must fit the
 * target; pitch is in indices and padding is zero. Captures prior queued draws by
 * flushing the frame's recorded work, using GPU copies only. Each handle is an immutable version, surviving target writes and
 * release. Release never cancels submitted sampling. All calls require serialization.
 * These versions have no CPU reconstruction data: terminal device loss invalidates
 * them. Callers must retain reconstructible inputs before adopting software fallback.
 */
uint64_t kfx_wgpu_draw_target_snapshot(void *drawing, uint64_t target,
    uint32_t x, uint32_t y, uint32_t width, uint32_t height, uint32_t pitch,
    char *error, size_t capacity);
int32_t kfx_wgpu_draw_target_snapshot_release(void *drawing, uint64_t snapshot,
    char *error, size_t capacity);

/* IMAGE or single TRANSITION command: source is a snapshot; table is a CPU asset.
 * TRANSITION source_x=0 selects map fade (start_low/high hold the second snapshot,
 * step_low is progress 0..32; table is 33 fade rows followed by the ghost table).
 * source_x=1 smooths the specified rectangle from the source snapshot and ghost table.
 * Uses IMAGE nearest sampling, clip, transparency and blend semantics. Overlap reads
 * immutable snapshot pixels, never earlier destination writes. Calls order with other
 * draw submissions. Host-rejected batches leave the target unchanged; terminal GPU failure
 * requires reconstruction. Returns 1 on acceptance, -1 on error.
 */
int32_t kfx_wgpu_draw_submit_target_images(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *commands, size_t count, char *error, size_t capacity);

/* TRIG sources contain 60 geometry bytes; the texture is a resident 256x256 mask slot.
 * Tables remain CPU asset handles.
 */
int32_t kfx_wgpu_draw_submit_target_triangles(void *drawing, uint64_t target,
    const struct KfxWgpuDrawCommand *commands, size_t count, uint32_t slot,
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
