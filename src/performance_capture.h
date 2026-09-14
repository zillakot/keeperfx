#pragma once

#ifdef __cplusplus
extern "C" {
#endif

enum PerformanceScope {
    PerfSimulation,
    PerfDraw,
    PerfPresentation,
    PerfPresentWait,
    PerfDrawScene,
    PerfDrawRaster,
    PerfDrawFrontRaster,
    PerfDrawOverlays,
    PerfScopeCount,
};

/* Cumulative drawing-backend counters sampled once per presented frame.
 * The capture stores per-frame deltas for the measured window only. */
struct PerformanceDrawingCounters {
    unsigned long long submits, dispatches, waits, wait_ns;
    unsigned long long checkpoints, checkpoint_copy_bytes, validation_waits;
    unsigned long long flagged_invalid_frames, status_stalls;
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
    /* First pass begin to last pass end in the frame; pass windows include stalls and
     * may overlap, so only this is an exclusive GPU window. */
    unsigned long long gpu_frame_ns;
    /* Trailing gauges are stored as observed, not differenced. */
    unsigned long long host_staged_asset_bytes, arena_bytes_resident;
};
void performance_drawing_backend(const char* backend);
void performance_drawing_frame(const struct PerformanceDrawingCounters* cumulative);

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
