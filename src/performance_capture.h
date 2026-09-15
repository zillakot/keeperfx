#pragma once
#include "kfx/renderer/ArenaKindCounters.h"

#ifdef __cplusplus
extern "C" {
#endif

enum PerformanceScope {
    PerfSimulation,
    PerfDraw,
    PerfPresentation,
    PerfPresentWait,
    PerfReplay,
    PerfDrawScene,
    PerfDrawRaster,
    PerfDrawFrontRaster,
    PerfDrawOverlays,
    PerfScopeCount,
};

#ifdef __cplusplus
inline constexpr const char* PerformanceScopeNames[] = {
    "simulation", "draw", "presentation", "present_wait", "replay", "draw_scene",
    "draw_raster", "draw_front_raster", "draw_overlays", "frame_interval"};
static_assert(sizeof(PerformanceScopeNames) / sizeof(*PerformanceScopeNames) == PerfScopeCount + 1);
#endif

/* Cumulative drawing-backend counters sampled once per presented frame.
 * The capture stores per-frame deltas for the measured window only. */
struct PerformanceDrawingCounters {
    unsigned long long submits, dispatches, waits, wait_ns;
    unsigned long long checkpoints, checkpoint_copy_bytes, validation_waits;
    unsigned long long flagged_invalid_frames, status_stalls;
    unsigned long long asset_upload_bytes, command_upload_bytes;
    unsigned long long upload_bytes, readback_bytes, full_readbacks, full_readback_bytes;
    unsigned long long buffers, buffer_bytes, batches, commands, ordered_sprites;
    unsigned long long ordered_sprite_layers, ordered_sprite_passes;
    unsigned long long arena_evictions, arena_overflows, arena_bytes_uploaded;
    unsigned long long tile_allocations, tile_entries, bridge_target_flushes, bridge_target_runs;
    unsigned long long terrain_tile_entries, prepared_row_words, prepared_row_allocations;
    /* tile_entries split by record kind, in KfxWgpuDrawCounters::tile_entries_by_kind order. */
    unsigned long long tile_entries_by_kind[18];
    /* Per-pass GPU execution time; zero unless KFX_WGPU_GPU_TIMING=1. */
    unsigned long long gpu_raster_ns, gpu_terrain_prepare_ns;
    unsigned long long gpu_shadow_mask_ns, gpu_target_trig_ns, gpu_ordered_sprite_ns;
    unsigned long long gpu_minimap_ns, gpu_lens_ns, gpu_present_ns;
    unsigned long long gpu_timed_passes, gpu_untimed_passes;
    /* Union of the frame's timed pass intervals; overlapping windows count once. Pass
     * windows include their own stalls, so neither this nor their sum is exclusive —
     * only a KFX_WGPU_GPU_TIMING=2 run is. */
    unsigned long long gpu_pass_union_ns;
    unsigned long long target_trig_geometry_bytes;
    unsigned long long target_trig_table_bytes;
    unsigned long long other_asset_upload_bytes;
    unsigned long long target_trig_table_hits;
    unsigned long long target_trig_table_misses;
    unsigned long long target_trig_asset_buffers;
    unsigned long long shadow_pairs;
    unsigned long long preparer_buffers;
    unsigned long long preparer_buffer_bytes;
    unsigned long long arena_misses_new_id;
    unsigned long long arena_misses_forget;
    unsigned long long arena_misses_size_class;
    unsigned long long arena_misses_generation;
    unsigned long long arena_misses_eviction;
    unsigned long long arena_miss_new_id_bytes;
    unsigned long long arena_miss_forget_bytes;
    unsigned long long arena_miss_size_class_bytes;
    unsigned long long arena_miss_generation_bytes;
    unsigned long long arena_miss_eviction_bytes;
    unsigned long long arena_explicit_forgets;
    struct KfxArenaKindCounters arena_by_kind[KFX_ARENA_KIND_COUNT];
    unsigned long long arena_trig_texture_source_bytes;
    unsigned long long replay_pack_ns;
    unsigned long long replay_upload_ns;
    unsigned long long replay_bind_ns;
    unsigned long long replay_encode_ns;
    unsigned long long replay_tile_index_ns;
    unsigned long long replay_other_ns;
    unsigned long long replay_bind_groups;
    unsigned long long replay_buffers;
    unsigned long long replay_passes;
    unsigned long long replay_staged_bytes;
    /* Trailing gauges are stored as observed, not differenced. */
    unsigned long long host_staged_asset_bytes, arena_bytes_resident, arena_scratch_bytes_peak;
    unsigned long long arena_capacity_bytes;
    unsigned long long arena_live_bytes;
    unsigned long long arena_retired_bytes;
    unsigned long long arena_growth_peak_bytes;

};
void performance_drawing_backend(const char* backend);
void performance_drawing_frame(const struct PerformanceDrawingCounters* cumulative);

struct PerformancePresenterCounters {
    unsigned long long acquire_ns, acquire_block_ns, reconfigure_count, present_record_ns, submit_ns;
    unsigned long long replay_ns, allocations, allocated_bytes;
};
void performance_presenter_frame(const struct PerformancePresenterCounters* counters);

int performance_requested(void);
int performance_active(void);
void performance_failed(const char* reason);
void performance_prepare_turn(void);
void performance_begin(enum PerformanceScope scope);
void performance_end(enum PerformanceScope scope);
void performance_renderer_info(const char* renderer, const char* driver, int width, int height,
    int output_width, int output_height, int vsync);
void performance_renderer_details(const char* details);

#ifdef __cplusplus
}
#endif
