#include "kfx/lense/WgpuLens.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <fstream>
#include <vector>

static void native_oracle(int kind, uint8_t* dst, int dp, const uint8_t* src, int sp,
    int w, int h, const std::vector<KfxLensLookup>& map, const std::vector<uint8_t>& texture,
    const std::vector<uint8_t>& fade, int alpha, unsigned phase)
{
    const unsigned scale_x = ((kind == 2 ? 7 : 640) << 16) / w;
    const unsigned scale_y = ((kind == 2 ? 5 : 480) << 16) / h;
    for (long y = 0; y < h; ++y) {
        for (long x = 0; x < w; ++x) {
            if (kind == 0) {
                auto p = map[y * w + x];
                dst[y * dp + x] = src[p.y * sp + p.x];
            } else if (kind == 1) {
                int vx = (x * scale_x) >> 16, vy = (y * scale_y) >> 16;
                int a = texture[(((static_cast<uint8_t>(phase + 17) + vy) & 255) << 8) + ((static_cast<uint8_t>(phase) + vx) & 255)];
                int b = texture[(((static_cast<uint8_t>(phase + 251) + 65536 - vx) & 255) << 8) + ((static_cast<uint8_t>(phase + 128) + 65536 - vy) & 255)];
                int n = (a + b) >> 3;
                if (n > 32) n = 32;
                dst[y * dp + x] = fade[(n << 8) + src[y * sp + x]];
            } else {
                int ox = std::min<int>((x * scale_x) >> 16, 6);
                int oy = std::min<int>((y * scale_y) >> 16, 4);
                uint8_t pixel = texture[oy * 7 + ox];
                int a = std::clamp(alpha, 0, 256);
                dst[y * dp + x] = pixel == 255 ? src[y * sp + x] : (pixel * a + src[y * sp + x] * (256 - a)) >> 8;
            }
        }
    }
}

#ifndef KFX_RUST_PRESENTER
static bool capture = false;
static std::vector<uint8_t> captured_source, captured_initial;
extern "C" int kfx_wgpu_native_enabled(void) { return capture; }
extern "C" void kfx_wgpu_terrain_boundary(int) {}
extern "C" int kfx_wgpu_native_draw(const KfxGpolyTarget* target, const KfxWgpuDrawCommand*,
    const KfxWgpuNativeResource* source, const KfxWgpuNativeResource*, KfxWgpuNativeOracle, void*)
{
    captured_source.assign(source->bytes, source->bytes + source->length);
    captured_initial.clear();
    for (uint32_t y=0; y<target->height; ++y)
        captured_initial.insert(captured_initial.end(), target->pixels+y*target->pitch, target->pixels+y*target->pitch+target->width);
    return 0;
}
#endif

int main(int argc, char** argv)
{
    std::vector<uint8_t> texture(65536), fade(33 * 256);
    for (size_t i = 0; i < texture.size(); ++i) texture[i] = i % 7 == 0 ? 255 : (i * 73 + i / 256 * 39) & 255;
    for (size_t i = 0; i < fade.size(); ++i) fade[i] = (i * 37 + i / 256 * 71) & 255;
    unsigned cases = 0;
#ifndef KFX_RUST_PRESENTER
    std::ofstream output(argc > 1 ? argv[1] : "lenses.bin", std::ios::binary);
    auto word = [&](uint32_t value) { for (int i=0; i<4; ++i) output.put(value >> (i*8)); };
    word(0x534e454c); word(54);
#else
    (void)argc; (void)argv;
#endif
    for (int w : {1, 13, 37}) for (int kind = 0; kind < 3; ++kind) for (int mode = 0; mode < 6; ++mode) {
        int h = w == 1 ? 1 : 9;
        int sp = w + 5, dp = w + (mode == 5 ? 3 : 5);
        int source = 64, destination = mode == 0 ? 4096 : 64 + (mode == 1 ? 0 : mode == 2 ? 2 : mode == 3 ? -2 : sp);
        int alpha = std::vector<int>{-20, 0, 1, 128, 256, 310}[mode];
        unsigned phase = mode * 51;
        std::vector<KfxLensLookup> map(w * h);
        for (int y = 0; y < h; ++y) for (int x = 0; x < w; ++x)
            map[y * w + x] = {static_cast<int16_t>((x + w - 1) % w), static_cast<int16_t>((y + h - 1) % h)};
        std::vector<uint8_t> expected(8192), disabled;
        for (size_t i = 0; i < expected.size(); ++i) expected[i] = (i * 13 + i / 29) & 255;
        disabled = expected;
        auto actual = expected;
        native_oracle(kind, expected.data() + destination, dp, expected.data() + source, sp,
            w, h, map, texture, fade, alpha, phase);
        auto render = [&](std::vector<uint8_t>& pixels) {
            auto* dst = pixels.data() + destination;
            const auto* src = pixels.data() + source;
            if (kind == 0) KfxLensRemap(dst, dp, src, sp, w, h, map.data());
            if (kind == 1) KfxLensMist(dst, dp, src, sp, w, h, texture.data(), fade.data(), phase, phase+17, phase+128, phase+251);
            if (kind == 2) KfxLensOverlay(dst, dp, src, sp, w, h, texture.data(), 7, 5, alpha);
        };
        assert(!kfx_wgpu_native_enabled());
        render(disabled);
        assert(disabled == expected);
#ifdef KFX_RUST_PRESENTER
        {
            WgpuTerrainBridge bridge(0, false, true);
            render(actual);
            if (bridge.Failed()) std::fprintf(stderr, "%s\n", bridge.GetError());
            assert(!bridge.Failed());
            assert(bridge.GetCounters().native_commands == 1);
            assert(bridge.GetCounters().verified_batches == 1);
            assert(bridge.GetGpuCounters().readback_bytes == static_cast<uint64_t>(w * h * 4));
            assert(actual == expected);
        }
#endif
#ifndef KFX_RUST_PRESENTER
        capture = true;
        render(actual);
        capture = false;
        assert(actual == expected && !captured_source.empty());
        word(w); word(h); word(captured_source.size());
        output.write(reinterpret_cast<const char*>(captured_initial.data()), captured_initial.size());
        output.write(reinterpret_cast<const char*>(captured_source.data()), captured_source.size());
        for (int y=0; y<h; ++y) output.write(reinterpret_cast<const char*>(expected.data()+destination+y*dp), w);
#endif
        ++cases;
    }
#ifdef KFX_RUST_PRESENTER
    {
        std::vector<uint8_t> pixels(64, 99), low_texture(65536, 0);
        WgpuTerrainBridge bridge(0, false, true);
        KfxLensMist(pixels.data()+32, 1, pixels.data()+16, 1, 1, 1, low_texture.data(), fade.data(), 0, 0, 0, 0, 32);
        assert(pixels[32] == fade[99]);
        KfxLensOverlay(pixels.data()+32, 1, pixels.data()+16, 1, 1, 1, pixels.data()+32, 1, 1, 128);
        assert(bridge.GetCounters().native_commands == 0 && !bridge.Failed());
    }
    for (bool fail : {false, true}) {
        std::vector<uint8_t> pixels(64, 77);
        KfxLensLookup lookup = {-1, 0};
        WgpuTerrainBridge bridge(0, fail, true);
        KfxLensRemap(pixels.data() + 32, 1, pixels.data() + 16, 1, 1, 1, &lookup);
        assert(pixels[32] == 77 && bridge.GetCounters().native_commands == 0);
        lookup = {0, 0};
        KfxLensRemap(pixels.data() + 32, 1, pixels.data() + 16, 1, 1, 1, &lookup);
        assert(bridge.Failed() == fail);
        assert(bridge.GetCounters().native_commands == (fail ? 0u : 1u));
    }
    {
        constexpr int w = 13, h = 9, pitch = 18;
        std::vector<uint8_t> actual(pitch * h), expected;
        for (size_t i = 0; i < actual.size(); ++i) actual[i] = i * 39;
        expected = actual;
        std::vector<KfxLensLookup> map(w*h);
        for (int y=0; y<h; ++y) for (int x=0; x<w; ++x)
            map[y*w+x] = {static_cast<int16_t>((x+7)%w), static_cast<int16_t>((y+3)%h)};
        for (int kind=0; kind<3; ++kind)
            native_oracle(kind, expected.data(), pitch, expected.data(), pitch,
                w, h, map, texture, fade, 128, 255);
        WgpuTerrainBridge bridge(0, false, true);
        KfxLensRemap(actual.data(), pitch, actual.data(), pitch, w, h, map.data());
        KfxLensMist(actual.data(), pitch, actual.data(), pitch, w, h, texture.data(), fade.data(), 255, 16, 127, 250);
        KfxLensOverlay(actual.data(), pitch, actual.data(), pitch, w, h, texture.data(), 7, 5, 128);
        assert(!bridge.Failed() && actual == expected);
        assert(bridge.GetCounters().native_commands == 3);
    }
#endif
    std::printf("%u native lens cases: signed alpha, phase wrap, palette index, transparent index, remap, padding, separate/in-place/partial alias passed\n", cases);
    return 0;
}
