#include "kfx/renderer/WgpuTerrainBridge.h"
#include <cassert>
#include <cstdio>
#include <vector>
extern "C" void vertex_draw(const KfxGpolyTarget*, const KfxWgpuTriangle*, const uint8_t*, const uint8_t*, int);
extern "C" int vertex_cpu_setup_ran(void);

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
        assert(bridge.Failed());
        assert(actual == initial);
        assert(bridge.GetCounters().rejected_triangles == 2);
        assert(bridge.GetCounters().replayed_triangles == 0);
        assert(bridge.GetCounters().gpu_triangles == 0);
    }
    std::puts("PASS: native original vertices bypass CPU setup; Metal exact clipped overlaps/resource mutations, CPU interleaving, original-input batch recovery and initialization fallback");
}
