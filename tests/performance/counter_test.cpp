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
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_pack_ns) / sizeof(unsigned long long)]) == "replay_pack_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_pack_ns) == (192) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_upload_ns) / sizeof(unsigned long long)]) == "replay_upload_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_upload_ns) == (193) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_bind_ns) / sizeof(unsigned long long)]) == "replay_bind_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_bind_ns) == (194) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_encode_ns) / sizeof(unsigned long long)]) == "replay_encode_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_encode_ns) == (195) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_tile_index_ns) / sizeof(unsigned long long)]) == "replay_tile_index_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_tile_index_ns) == (196) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_other_ns) / sizeof(unsigned long long)]) == "replay_other_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_other_ns) == (197) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_bind_groups) / sizeof(unsigned long long)]) == "replay_bind_groups");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_bind_groups) == (198) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_buffers) / sizeof(unsigned long long)]) == "replay_buffers");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_buffers) == (199) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_passes) / sizeof(unsigned long long)]) == "replay_passes");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_passes) == (200) * sizeof(uint64_t));
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_staged_bytes) / sizeof(unsigned long long)]) == "replay_staged_bytes");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_staged_bytes) == (201) * sizeof(uint64_t));
    for (const auto name : drawing_counter_names) assert(name != nullptr);
    static_assert(offsetof(KfxWgpuDrawCounters, arena_by_kind) == 77 * sizeof(uint64_t));
    static_assert(sizeof(KfxWgpuDrawCounters) == (77 + 19 * 6 + 1 + 10) * sizeof(uint64_t));
}
