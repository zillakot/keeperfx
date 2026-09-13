#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/software/WgpuRawImage.h"
#include "kfx/renderer/software/MovieFrame.h"
#include "kfx/lense/WgpuLens.h"
#include <cassert>
#include <cstdio>
#include <cstring>
#include <vector>

static void image_oracle(uint8_t* output, uint32_t pitch, void* context)
{
    const auto* source = static_cast<const uint8_t*>(context);
    for (unsigned y = 0; y < 4; ++y) std::memcpy(output + y * pitch, source + y * 4, 4);
}

int main()
{
    constexpr unsigned pitch = 32, height = 16;
    std::vector<uint8_t> screen(pitch * height, 9);
    WgpuTerrainBridge bridge(0, false, false, true);
    KfxGpolyTarget frame = {screen.data(), pitch, height, pitch};
    assert(bridge.BeginFrame(frame, true));
    assert(kfx_wgpu_raw_clear(screen.data(), pitch, pitch, height, 144));
    uint8_t source[16]; std::memset(source, 32, sizeof(source));
    for (unsigned i = 0; i < 10; ++i)
        assert(kfx_wgpu_raw_image(screen.data(), pitch, height, 4, 4, 0, 0,
            source, 4, 4, image_oracle, source));
    auto counts = bridge.GetCounters();
    assert(counts.native_copy_bytes == 0 && counts.bridge_initial_index_bytes == 0);
    assert(kfx_wgpu_native_read_barrier(source, sizeof(source)));
    assert(bridge.GetCounters().native_copy_bytes == 0);
    assert(kfx_wgpu_native_read_barrier(screen.data(), 1));
    assert(screen[0] == 32 && screen[8] == 0);
    const auto uploads = bridge.GetCounters().bridge_initial_index_bytes;
    assert(kfx_wgpu_raw_clear(screen.data(), pitch, pitch, height, 71));
    // Source and destination overlap: the actual movie helper must decline before its CPU loop.
    KfxMovieFrame movie = {screen.data() + pitch * 7, 4, 4, pitch};
    KfxMovieTarget output = {screen.data() + pitch * 8, pitch, 4, 4, 4};
    kfx_movie_copy(movie, output, 0);
    assert(!bridge.Failed());
    for (unsigned y = 0; y < height; ++y)
        for (unsigned x = 0; x < pitch; ++x) assert(screen[y * pitch + x] == 71);
    assert(bridge.GetCounters().bridge_initial_index_bytes == uploads);
    assert(kfx_wgpu_raw_clear(screen.data(), pitch, pitch, height, 12));
    uint8_t overlay = 55;
    KfxLensOverlay(screen.data(), pitch, screen.data(), pitch, pitch, height, &overlay, 1, 1, 128);
    assert(bridge.EndFrame(true));
    for (auto pixel : screen) assert(pixel == 33);
    assert(bridge.GetCounters().bridge_initial_index_bytes > uploads);
    std::puts("queued real raw helpers: zero initial uploads/copies; read-only overlap retained GPU authority; actual movie CPU fallback synchronized before loop");
}
