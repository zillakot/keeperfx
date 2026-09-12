#pragma once

#ifdef __cplusplus
extern "C" {
#endif

enum PerformanceScope {
    PerfSimulation,
    PerfDraw,
    PerfPresentation,
    PerfPresentWait,
    PerfScopeCount,
};

int performance_requested(void);
int performance_active(void);
void performance_failed(const char* reason);
void performance_prepare_turn(void);
void performance_begin(enum PerformanceScope scope);
void performance_end(enum PerformanceScope scope);
void performance_renderer_info(const char* renderer, const char* driver, int width, int height,
    int output_width, int output_height, int vsync);

#ifdef __cplusplus
}
#endif
