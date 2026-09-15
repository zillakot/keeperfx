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
    static_assert(offsetof(KfxWgpuDrawCounters, replay_pack_ns) == 194 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_pack_ns) == 0 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[0]) == "replay_pack_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_upload_ns) / sizeof(unsigned long long)]) == "replay_upload_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_upload_ns) == 195 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_upload_ns) == 1 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[1]) == "replay_upload_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_bind_ns) / sizeof(unsigned long long)]) == "replay_bind_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_bind_ns) == 196 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_bind_ns) == 2 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[2]) == "replay_bind_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_encode_ns) / sizeof(unsigned long long)]) == "replay_encode_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_encode_ns) == 197 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_encode_ns) == 3 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[3]) == "replay_encode_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_tile_index_ns) / sizeof(unsigned long long)]) == "replay_tile_index_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_tile_index_ns) == 198 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_tile_index_ns) == 4 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[4]) == "replay_tile_index_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_other_ns) / sizeof(unsigned long long)]) == "replay_other_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_other_ns) == 199 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_other_ns) == 5 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[5]) == "replay_other_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_submit_wait_ns) / sizeof(unsigned long long)]) == "replay_submit_wait_ns");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_submit_wait_ns) == 200 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_submit_wait_ns) == 6 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[6]) == "replay_submit_wait_ns");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_bind_groups) / sizeof(unsigned long long)]) == "replay_bind_groups");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_bind_groups) == 201 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_bind_groups) == 7 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[7]) == "replay_bind_groups");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_buffers) / sizeof(unsigned long long)]) == "replay_buffers");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_buffers) == 202 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_buffers) == 8 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[8]) == "replay_buffers");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_passes) / sizeof(unsigned long long)]) == "replay_passes");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_passes) == 203 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_passes) == 9 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[9]) == "replay_passes");
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, replay_staged_bytes) / sizeof(unsigned long long)]) == "replay_staged_bytes");
    static_assert(offsetof(KfxWgpuDrawCounters, replay_staged_bytes) == 204 * sizeof(uint64_t));
    static_assert(offsetof(PerformanceReplayCounters, replay_staged_bytes) == 10 * sizeof(unsigned long long));
    assert(std::string(replay_counter_names[10]) == "replay_staged_bytes");
    static_assert(offsetof(PerformancePresenterCounters, replay) == 8 * sizeof(unsigned long long));
    static_assert(sizeof(PerformancePresenterCounters) == (19 + KFX_UPLOAD_COUNTER_COUNT) * sizeof(unsigned long long));
#define KFX_UPLOAD_FIELD(field) \
    assert(std::string(replay_counter_names[offsetof(PerformanceReplayCounters, field) / sizeof(unsigned long long)]) == #field); \
    assert(std::string(drawing_counter_names[offsetof(PerformanceDrawingCounters, field) / sizeof(unsigned long long)]) == #field); \
    static_assert(offsetof(KfxWgpuDrawCounters, field) - offsetof(KfxWgpuDrawCounters, replay_pack_ns) == offsetof(PerformanceReplayCounters, field));
    KFX_UPLOAD_ALL_FIELDS
#undef KFX_UPLOAD_FIELD
    for (const auto name : drawing_counter_names) assert(name != nullptr);
    static_assert(offsetof(KfxWgpuDrawCounters, arena_by_kind) == 79 * sizeof(uint64_t));
    static_assert(sizeof(KfxWgpuDrawCounters) == (79 + 19 * 6 + 1 + 11 + KFX_UPLOAD_COUNTER_COUNT) * sizeof(uint64_t));
}
