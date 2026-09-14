#include "kfx/renderer/WgpuTerrainBridge.h"
#include <cstdio>
extern "C" int shadow_cases(FILE*, int);
extern "C" uint64_t shadow_hash, shadow_scratch_hash;
extern "C" unsigned shadow_perturb_cases;
static WgpuTerrainBridge* active;
// RendererSoftware answers an invalidated frame with a full CPU redraw; model that here.
extern "C" void shadow_native_recover(void) { if (active) active->FullRedraw(); }
static int run(WgpuTerrainBridge& bridge, int expected_accept)
{
    active = &bridge;
    const int result = shadow_cases(nullptr, expected_accept);
    active = nullptr;
    return result;
}
int main() {
    uint64_t expected, expected_scratch;
    {
        WgpuTerrainBridge bridge(0, false, true);
        if (run(bridge, 1)) {
            std::fprintf(stderr, "shadow bridge: %s\n", bridge.GetError());return 1;
        }
        expected = shadow_hash;
        expected_scratch = shadow_scratch_hash;
        const auto &c = bridge.GetCounters();
        if (bridge.Failed() || c.gpu_shadow_commands != 192 || c.verified_batches != 192 ||
            c.shadow_scratch_upload_bytes || c.shadow_scratch_copy_bytes ||
            c.shadow_prior_divergence ||
            c.shadow_scratch_readback_bytes != 192 * 65536 * 4) {
            std::fprintf(stderr, "shadow bridge: %s\n", bridge.GetError());return 2;
        }
    }
    {
        // Production keeps the mask chain on the GPU, so the CPU scratch is not mirrored back.
        WgpuTerrainBridge bridge(0, false, false);
        const auto &c = bridge.GetCounters();
        if (run(bridge, 1) || shadow_hash != expected || bridge.Failed() ||
            c.gpu_shadow_commands != 192 || c.verified_batches || c.verification_cpu_commands ||
            c.shadow_scratch_upload_bytes || c.shadow_scratch_readback_bytes ||
            c.shadow_scratch_copy_bytes || c.shadow_prior_divergence) {
            std::fprintf(stderr, "production shadow bridge: %s\n", bridge.GetError());return 5;
        }
    }
    {
        WgpuTerrainBridge bridge(0, true, true);
        if (run(bridge, 0) || shadow_hash != expected ||
            shadow_scratch_hash != expected_scratch || !bridge.Failed() ||
            bridge.GetCounters().gpu_shadow_commands) return 3;
    }
    {
        WgpuTerrainBridge bridge(1, false, true);
        if (run(bridge, 2) || shadow_hash != expected ||
            shadow_scratch_hash != expected_scratch || !bridge.Failed() ||
            bridge.GetCounters().gpu_shadow_commands != 1) return 4;
    }
    {
        // Two consecutive shadows whose CPU prior the resident chain cannot know: each is
        // counted and skipped, and verification resumes from the GPU prior straight after.
        WgpuTerrainBridge bridge(0, false, true);
        shadow_perturb_cases = 1;
        const int result = run(bridge, 1);
        shadow_perturb_cases = 0;
        const auto &c = bridge.GetCounters();
        if (result || bridge.Failed() || c.shadow_prior_divergence != 2 ||
            c.verified_batches != 190 || c.gpu_shadow_commands != 192 ||
            c.shadow_scratch_copy_bytes != 2 * 65536) {
            std::fprintf(stderr, "diverged shadow prior: %s\n", bridge.GetError());return 6;
        }
    }
    std::fprintf(stderr, "192 verified and 192 production native shadows, two counted prior divergences, initialization and later batch fallback exact\n");
    return 0;
}
