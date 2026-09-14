#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#include <cassert>
#include <cstdio>
#include <cstring>
#include <vector>
extern "C" void vertex_draw(const KfxGpolyTarget*, const KfxWgpuTriangle*, const uint8_t*, const uint8_t*, int);
extern "C" int vertex_cpu_setup_ran(void);

static void hud_oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    const auto* source = static_cast<const uint8_t*>(context);
    std::memcpy(pixels, source, 2);
    std::memcpy(pixels + pitch, source + 2, 2);
}

int main()
{
    std::vector<uint8_t> texture(7968), fade(16384);
    for (size_t i = 0; i < texture.size(); ++i) texture[i] = i*17+i/256*3;
    for (size_t i = 0; i < fade.size(); ++i) fade[i] = i*7+i/256*19;
    KfxWgpuTriangle triangle = {1, 0, 0, 0, {{-13,-5,0,0,31*65536}, {75,8,31*65536,0,47*65536}, {16,70,0,31*65536,18*65536}}};
    for (int fault = 0; fault < 3; ++fault) {
        std::vector<uint8_t> actual(83*61+32, 167), expected = actual;
        KfxGpolyTarget target = {actual.data()+16,79,61,83};
        KfxGpolyTarget oracle = {expected.data()+16,79,61,83};
        WgpuTerrainBridge bridge(fault == 1 ? 1 : 0, fault == 2, true);
        for (int batch = 0; batch < 3; ++batch) {
            bridge.Boundary(true);
            for (int i = 0; i < 3; ++i) {
                vertex_draw(&oracle, &triangle, texture.data(), fade.data(), 1);
                vertex_draw(&target, &triangle, texture.data(), fade.data(), 0);
                if (batch == 0 && !bridge.Failed()) assert(!vertex_cpu_setup_ran());
                for (auto& byte : texture) byte += 13;
                for (auto& byte : fade) byte += 17;
                triangle.vertices[0].x += 2;
            }
            bridge.Boundary(false);
            assert(actual == expected);
            actual[16+27*83+37] ^= 127; expected[16+27*83+37] ^= 127;
        }
        const auto& stats = bridge.GetCounters();
        if (fault == 0) { assert(stats.gpu_triangles == 9 && stats.verified_triangles == 9 && stats.gpu_spans == 0); }
        if (fault == 1) { assert(stats.gpu_triangles == 3 && stats.replayed_triangles > 0 && stats.cpu_triangles > 0 && stats.gpu_triangles + stats.replayed_triangles + stats.cpu_triangles == 9); }
        if (fault == 2) { assert(stats.cpu_triangles == 9 && stats.gpu_triangles == 0); }
    }
    {
        std::vector<uint8_t> actual(83*61+32, 167);
        const auto initial = actual;
        KfxGpolyTarget target = {actual.data()+16,79,61,83};
        WgpuTerrainBridge bridge(0, false, true);
        bridge.Boundary(true);
        vertex_draw(&target, &triangle, texture.data(), fade.data(), 0);
        auto invalid = triangle;
        for (auto& vertex : invalid.vertices) vertex.shade = 64*65536;
        vertex_draw(&target, &invalid, texture.data(), fade.data(), 0);
        bridge.Boundary(false);
        // Verification cannot reject what the kernels flag and skip: the CPU oracle and the
        // GPU disagree by construction, so the batch is counted and the bridge keeps drawing.
        assert(!bridge.Failed());
        assert(actual != initial);
        assert(bridge.GetCounters().rejected_triangles == 0);
        assert(bridge.GetCounters().replayed_triangles == 0);
        assert(bridge.GetCounters().gpu_triangles == 2);
        assert(bridge.GetCounters().verification_flagged_shades == 1);
        assert(bridge.GetCounters().verified_triangles == 0);
        uint32_t flags = 0;
        char error[1024] = {};
        assert(kfx_wgpu_draw_frame_status(bridge.Context(), &flags, error, sizeof(error)) == 1);
        assert(flags != 0);
    }
    for (bool verify : {false, true}) {
        std::vector<uint8_t> actual(83*61+32, 167), expected = actual;
        const auto initial = actual;
        KfxGpolyTarget target = {actual.data()+16,79,61,83};
        KfxGpolyTarget oracle = {expected.data()+16,79,61,83};
        WgpuTerrainBridge bridge(0, false, verify, true);
        bridge.Boundary(true);
        for (int i = 0; i < 300; ++i) {
            auto command = triangle;
            command.vertices[0].x = (i * 17) % 80 - 23;
            command.vertices[1].y = (i * 13) % 75 - 10;
            vertex_draw(&oracle, &command, texture.data(), fade.data(), 1);
            vertex_draw(&target, &command, texture.data(), fade.data(), 0);
            if (!verify) assert(!vertex_cpu_setup_ran());
        }
        bridge.Flush();
        const auto before = bridge.GetCounters();
        assert(actual == initial && actual != expected);
        assert(before.gpu_triangles == 300 && before.resident_batches == 1);
        assert(before.bridge_initial_index_bytes == 83 * 60 + 79);
        assert(before.bridge_readbacks == (verify ? 1 : 0));
        assert(before.native_copy_bytes == 0);
        bridge.Boundary(false);
        assert(actual == expected && bridge.FrameValid() && !bridge.Failed());
        assert(bridge.GetCounters().barrier_readbacks == 1);
        std::printf("Resident native vertices: verify=%d, triangles=300, batches=1, initial_indices=%llu, final_materializations=1, diagnostic_readbacks=%llu\n",
            verify, static_cast<unsigned long long>(before.bridge_initial_index_bytes),
            static_cast<unsigned long long>(before.verification_readbacks));
    }
    {
        std::vector<uint8_t> actual(83*61+32, 167), expected = actual;
        KfxGpolyTarget target = {actual.data()+16,79,61,83};
        KfxGpolyTarget oracle = {expected.data()+16,79,61,83};
        WgpuTerrainBridge bridge(0, false, false, true);
        bridge.Boundary(true);
        vertex_draw(&oracle, &triangle, texture.data(), fade.data(), 1);
        vertex_draw(&target, &triangle, texture.data(), fade.data(), 0);
        bridge.Flush();
        auto outside_gpu_domain = triangle;
        for (auto& vertex : outside_gpu_domain.vertices) vertex.x = 40000;
        vertex_draw(&oracle, &outside_gpu_domain, texture.data(), fade.data(), 1);
        vertex_draw(&target, &outside_gpu_domain, texture.data(), fade.data(), 0);
        assert(actual == expected && !bridge.Failed());
        assert(bridge.GetCounters().barrier_readbacks == 1 && bridge.GetCounters().cpu_triangles == 1);
    }
    for (bool verify : {false, true}) {
        std::vector<uint8_t> actual(87 * 67 + 32, 167), expected = actual;
        const auto initial = actual;
        KfxGpolyTarget frame = {actual.data() + 16, 83, 67, 87};
        KfxGpolyTarget view = {frame.pixels + 2 * 87 + 3, 79, 61, 87};
        KfxGpolyTarget oracle = {expected.data() + 16 + 2 * 87 + 3, 79, 61, 87};
        WgpuTerrainBridge bridge(0, false, verify, true);
        assert(bridge.BeginFrame(frame));
        for (int batch = 0; batch < 3; ++batch) {
            bridge.Boundary(true);
            for (int i = 0; i < 100; ++i) {
                auto command = triangle;
                command.vertices[0].x = (i * 17) % 80 - 23;
                command.vertices[1].y = (i * 13) % 75 - 10;
                vertex_draw(&oracle, &command, texture.data(), fade.data(), 1);
                vertex_draw(&view, &command, texture.data(), fade.data(), 0);
                if (!verify) assert(!vertex_cpu_setup_ran());
            }
            bridge.Boundary(false);
            uint8_t source_pixels[4] = {static_cast<uint8_t>(batch), 11, 21, 31};
            KfxWgpuNativeResource source = {source_pixels, 4, 2, 2, 2, nullptr, 0};
            KfxWgpuDrawCommand hud = {};
            hud.abi_version = 1;
            hud.kind = KFX_WGPU_DRAW_IMAGE;
            hud.width = hud.clip_width = hud.source_width = 2;
            hud.height = hud.clip_height = hud.source_height = 2;
            hud.transparent = KFX_WGPU_DRAW_OPAQUE;
            assert(bridge.SubmitNative(frame, hud, &source, nullptr, hud_oracle, source_pixels) == 1);
            hud_oracle(expected.data() + 16, 87, source_pixels);
        }
        // Batched commands are counted at their flush, and this snapshot is about GPU-side
        // accumulation, so close the run first. Flush submits without materializing.
        bridge.Flush();
        const auto before = bridge.GetCounters();
        assert(actual == initial && actual != expected);
        assert(before.target_creations == 1 && before.target_alias_barriers == 0);
        assert(before.gpu_triangles == 300 && before.native_commands == 3);
        assert(before.bridge_initial_index_bytes == 87 * 66 + 83);
        assert(before.bridge_readbacks == (verify ? 6 : 0) && before.native_copy_bytes == 0);
        KfxWgpuFrameCounters queued = {};
        char error[1024] = {};
        assert(kfx_wgpu_draw_frame_counters(bridge.Context(), &queued, error, sizeof(error)) == 1);
        // A flush is a replay into the frame's open encoder, not a submission boundary.
        assert(queued.checkpoints == 0);
        assert(queued.validation_waits == 0);
        assert(bridge.EndFrame(true));
        assert(kfx_wgpu_draw_frame_counters(bridge.Context(), &queued, error, sizeof(error)) == 1);
        assert(queued.checkpoints == 0);
        assert(queued.validation_waits == 0);
        assert(actual == expected && !bridge.Failed());
        assert(bridge.GetCounters().barrier_readbacks == 1);
        assert(queued.checkpoint_copy_bytes == 0);
        std::printf("Full frame native vertices+HUD: verify=%d, triangles=300, HUD=3, root_targets=1, view_barriers=0, final_materializations=1, GPU_checkpoints=%llu, validation_waits=%llu, checkpoint_copy_bytes=%llu\n", verify,
            static_cast<unsigned long long>(queued.checkpoints), static_cast<unsigned long long>(queued.validation_waits),
            static_cast<unsigned long long>(queued.checkpoint_copy_bytes));
    }
    {
        std::vector<uint8_t> actual(87 * 67 + 32, 167);
        KfxGpolyTarget frame = {actual.data() + 16, 83, 67, 87};
        WgpuTerrainBridge bridge(0, false, false, true);
        auto invalid = triangle;
        for (auto& vertex : invalid.vertices) vertex.shade = 64 * 65536;
        assert(bridge.BeginFrame(frame));
        bridge.Boundary(true);
        vertex_draw(&frame, &triangle, texture.data(), fade.data(), 0);
        vertex_draw(&frame, &invalid, texture.data(), fade.data(), 0);
        bridge.Boundary(false);
        assert(bridge.EndFrame(true));
        // Flag and recover: the batch is accepted, the invalid triangle writes nothing.
        assert(!bridge.Failed() && bridge.FrameValid());
        assert(bridge.GetCounters().rejected_triangles == 0);
        assert(bridge.GetCounters().gpu_triangles == 2);
        assert(bridge.GetCounters().invalid_frames == 0);
        uint64_t invalid_frames = 0;
        int observed = 0;
        for (; observed < 2 && invalid_frames == 0; ++observed) {
            assert(bridge.BeginFrame(frame));
            assert(bridge.EndFrame(true));
            invalid_frames = bridge.GetCounters().invalid_frames;
        }
        assert(invalid_frames == 1 && observed <= 2);
        assert(!bridge.Failed() && bridge.FrameValid());
        KfxWgpuFrameCounters queued = {};
        char error[1024] = {};
        assert(kfx_wgpu_draw_frame_counters(bridge.Context(), &queued, error, sizeof(error)) == 1);
        assert(queued.validation_waits == 0 && queued.checkpoint_copy_bytes == 0);
        std::printf("Flagged frame recovery: observed after %d frames, invalid_frames=%llu, status_reads=%llu, status_stalls=%llu\n",
            observed, static_cast<unsigned long long>(invalid_frames),
            static_cast<unsigned long long>(queued.status_reads),
            static_cast<unsigned long long>(queued.status_stalls));
    }
    std::puts("PASS: native original vertices bypass CPU setup; Metal exact clipped overlaps/resource mutations, CPU interleaving, original-input batch recovery and initialization fallback");
}
