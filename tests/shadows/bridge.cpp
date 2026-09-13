#include "kfx/renderer/WgpuTerrainBridge.h"
#include <cstdio>
extern "C" int shadow_cases(FILE*, int);
extern "C" uint64_t shadow_hash;
int main() {
    uint64_t expected;
    {
        WgpuTerrainBridge bridge(0, false, true);
        if (shadow_cases(nullptr, 1)) {
            std::fprintf(stderr, "shadow bridge: %s\n", bridge.GetError());return 1;
        }
        expected = shadow_hash;
        const auto &c = bridge.GetCounters();
        if (bridge.Failed() || c.gpu_shadow_commands != 192 || c.verified_batches != 192 ||
            c.shadow_scratch_copy_bytes != 192 * 65536 || c.shadow_scratch_readback_bytes != 192 * 65536 * 4) {
            std::fprintf(stderr, "shadow bridge: %s\n", bridge.GetError());return 2;
        }
    }
    {
        WgpuTerrainBridge bridge(0, false, false);
        if (shadow_cases(nullptr, 1) || shadow_hash != expected || bridge.Failed() ||
            bridge.GetCounters().gpu_shadow_commands != 192 ||
            bridge.GetCounters().verified_batches || bridge.GetCounters().verification_cpu_commands) {
            std::fprintf(stderr, "production shadow bridge: %s\n", bridge.GetError());return 5;
        }
    }
    {
        WgpuTerrainBridge bridge(0, true, true);
        if (shadow_cases(nullptr, 0) || shadow_hash != expected || !bridge.Failed() || bridge.GetCounters().gpu_shadow_commands) return 3;
    }
    {
        WgpuTerrainBridge bridge(1, false, true);
        if (shadow_cases(nullptr, 2) || shadow_hash != expected || !bridge.Failed() || bridge.GetCounters().gpu_shadow_commands != 1) return 4;
    }
    std::fprintf(stderr, "192 verified and 192 production native shadows, initialization and later batch fallback exact\n");
    return 0;
}
