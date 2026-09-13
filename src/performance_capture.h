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
    unsigned long long upload_bytes, readback_bytes, full_readbacks, full_readback_bytes;
    unsigned long long buffers, buffer_bytes, batches, commands, ordered_sprites;
    unsigned long long gpu_span_ns, gpu_spans;
    /* Trailing gauges are stored as observed, not differenced. */
    unsigned long long arena_bytes_resident;
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
