#include "performance_capture.h"
#include "kfx/renderer/WgpuDraw.h"
#include <cassert>
#include <cstddef>
#include <string>
#include "counters.inc"

int main()
{
    const auto start = offsetof(PerformanceDrawingCounters, arena_by_kind) / sizeof(unsigned long long);
    const char* metrics[] = {"bytes", "misses", "hits", "source_bytes", "distinct_lengths", "length_overflows"};
    for (unsigned kind = 0; kind < KFX_ARENA_KIND_COUNT; ++kind)
        for (unsigned metric = 0; metric < 6; ++metric)
            assert(drawing_counter_names[start + kind * 6 + metric] ==
                std::string("arena_") + KfxArenaKindNames[kind] + "_" + metrics[metric]);
    assert(std::string(drawing_counter_names[start + KFX_ARENA_KIND_COUNT * 6]) == "arena_trig_texture_source_bytes");
    const auto gauges = offsetof(PerformanceDrawingCounters, host_staged_asset_bytes) / sizeof(unsigned long long);
    assert(gauges == DrawingCounterCount - DrawingGaugeCount);
    for (const auto name : drawing_counter_names) assert(name != nullptr);
    static_assert(offsetof(KfxWgpuDrawCounters, arena_by_kind) == 77 * sizeof(uint64_t));
    static_assert(sizeof(KfxWgpuDrawCounters) == (77 + 19 * 6 + 1) * sizeof(uint64_t));
}
