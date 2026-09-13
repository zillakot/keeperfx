#include "kfx/renderer/WgpuTerrainBridge.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstring>
#include <map>
#include <vector>

#ifndef KFX_BRIDGE_REAL_GPU
struct FakeResource { std::vector<uint8_t> bytes; uint32_t width, height, pitch; };
struct FakeContext {
    uint64_t next = 1;
    std::map<uint64_t, FakeResource> resources, targets;
};
static bool fail_readback = false;
extern "C" int32_t kfx_wgpu_draw_counters(void*, KfxWgpuDrawCounters* counters, char*, size_t)
{ *counters = {}; return 1; }
extern "C" void* kfx_wgpu_draw_create(char*, size_t) { return new FakeContext; }
extern "C" void kfx_wgpu_draw_destroy(void* handle) { delete static_cast<FakeContext*>(handle); }
extern "C" uint64_t kfx_wgpu_draw_target_create(void* handle, uint32_t width, uint32_t height, char*, size_t)
{
    auto& context = *static_cast<FakeContext*>(handle);
    const auto id = context.next++;
    context.targets[id] = {std::vector<uint8_t>(width * height), width, height, width};
    return id;
}
extern "C" int32_t kfx_wgpu_draw_target_release(void* handle, uint64_t id, char*, size_t)
{ return static_cast<FakeContext*>(handle)->targets.erase(id) == 1 ? 1 : -1; }
extern "C" uint64_t kfx_wgpu_draw_resource_create(void* handle, const uint8_t* bytes, size_t length,
    uint32_t width, uint32_t height, uint32_t pitch, char*, size_t)
{
    auto& context = *static_cast<FakeContext*>(handle);
    const auto id = context.next++;
    context.resources[id] = {std::vector<uint8_t>(bytes, bytes + length), width, height, pitch};
    return id;
}
extern "C" int32_t kfx_wgpu_draw_resource_release(void* handle, uint64_t id, char*, size_t)
{ return static_cast<FakeContext*>(handle)->resources.erase(id) == 1 ? 1 : -1; }
extern "C" int32_t kfx_wgpu_draw_submit(void* handle, uint64_t id, const KfxWgpuDrawCommand* commands,
    size_t count, char*, size_t)
{
    auto& context = *static_cast<FakeContext*>(handle);
    auto& target = context.targets.at(id);
    for (size_t i = 0; i < count; ++i) {
        const auto& c = commands[i];
        const auto& source = context.resources.at(c.source);
        if (c.kind == KFX_WGPU_DRAW_IMAGE) {
            for (uint32_t y = 0; y < c.height; ++y)
                std::memcpy(target.bytes.data() + y * target.pitch, source.bytes.data() + y * source.pitch, c.width);
        } else {
            assert(c.kind == KFX_WGPU_DRAW_GPOLY_SPAN);
            const auto& table = context.resources.at(c.table);
            uint64_t value = (uint64_t(c.start_high) << 32) | c.start_low;
            const uint64_t step = (uint64_t(c.step_high) << 32) | c.step_low;
            for (uint32_t x = 0; x < c.width; ++x, value += step) {
                const uint32_t high = value >> 32;
                const uint32_t uv = ((high << 8) | (high >> 24)) & 0x1f1f;
                target.bytes[c.y * target.pitch + c.x + x] = table.bytes[(value & 0xff00) | source.bytes[uv]];
            }
        }
    }
    return 1;
}
extern "C" int32_t kfx_wgpu_draw_readback(void* handle, uint64_t id, uint8_t* bytes,
    size_t length, uint32_t pitch, char* error, size_t capacity)
{
    if (fail_readback) {
        std::fill(bytes, bytes + length, 0xee);
        std::snprintf(error, capacity, "injected partial readback failure");
        return -1;
    }
    const auto& target = static_cast<FakeContext*>(handle)->targets.at(id);
    for (uint32_t y = 0; y < target.height; ++y)
        std::memcpy(bytes + y * pitch, target.bytes.data() + y * target.pitch, target.width);
    return 1;
}
#endif

static void oracle(std::vector<uint8_t>& target, uint32_t pitch, const KfxGpolySpan& span,
    const std::vector<uint8_t>& texture, const std::vector<uint8_t>& fade)
{
    uint64_t value = (uint64_t(span.start_high) << 32) | span.start_low;
    const uint64_t step = (uint64_t(span.step_high) << 32) | span.step_low;
    for (uint32_t i = 0; i < span.count; ++i, value += step) {
        uint32_t high = value >> 32;
        target[span.y * pitch + span.x + i] = fade[(value & 0xff00) | texture[((high << 8) | (high >> 24)) & 0x1f1f]];
    }
}

static void copy_oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    const auto* source = static_cast<const KfxWgpuNativeResource*>(context);
    for (uint32_t row = 0; row < source->height; ++row)
        std::memcpy(pixels + row * pitch, source->bytes + row * source->pitch, source->width);
}

int main()
{
    std::vector<uint8_t> texture(7968), fade(16384);
    for (size_t i = 0; i < texture.size(); ++i) texture[i] = (i * 17 + 3) & 255;
    for (size_t i = 0; i < fade.size(); ++i) fade[i] = ((i >> 8) * 7 + i) & 255;
    std::vector<uint8_t> pixels(24 * 10, 0x6a), expected = pixels;
    KfxGpolyTarget target = {pixels.data(), 20, 10, 24};
    const KfxGpolySpan a = {2, 3, 12, 0x3010, 0x03000801, 0x22, 0x01000100};
    const KfxGpolySpan b = {6, 3, 10, 0x3e10, 0x01000a02, 0, 0x01000001};
    {
        WgpuTerrainBridge bridge(0, false, true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 0);
        kfx_wgpu_terrain_boundary(1);
        const int accepted = kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data());
        if (accepted != 1) std::fprintf(stderr, "GPU bridge rejected: %s\n", bridge.GetError());
        assert(accepted == 1);
        oracle(expected, target.pitch, a, texture, fade);
        for (auto& value : texture) value ^= 0xff;
        for (auto& value : fade) value ^= 0x5a;
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &b, texture.data(), fade.data()) == 1);
        oracle(expected, target.pitch, b, texture, fade);
        assert(pixels != expected);
        kfx_wgpu_terrain_boundary(0);
        assert(pixels == expected);
        assert(bridge.GetCounters().gpu_batches == 1 && bridge.GetCounters().gpu_spans == 2);
        pixels[3 * 24 + 5] = expected[3 * 24 + 5] = 99;
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &b, texture.data(), fade.data()) == 1);
        oracle(expected, target.pitch, b, texture, fade);
        kfx_wgpu_terrain_boundary(0);
        assert(pixels == expected);
        assert(bridge.GetCounters().gpu_batches == 2 && bridge.GetCounters().failures == 0);
    }
    {
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        for (unsigned i = 0; i < 70; ++i) {
            texture[0] = static_cast<uint8_t>(i);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
            oracle(expected, target.pitch, a, texture, fade);
        }
        std::vector<uint8_t> resized(32 * 12, 71), resized_expected = resized;
        KfxGpolyTarget other = {resized.data(), 25, 12, 32};
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &other, &a, texture.data(), fade.data()) == 1);
        assert(pixels == expected);
        oracle(resized_expected, other.pitch, a, texture, fade);
        kfx_wgpu_terrain_boundary(0);
        assert(resized == resized_expected);
        assert(bridge.GetCounters().target_creations == 2 && bridge.GetCounters().failures == 0);
    }
    {
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        std::vector<uint8_t> image(200, 19);
        KfxWgpuNativeResource source = {image.data(), image.size(), 20, 10, 20};
        KfxWgpuDrawCommand command = {};
        command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        command.kind = KFX_WGPU_DRAW_IMAGE;
        command.width = command.clip_width = command.source_width = 20;
        command.height = command.clip_height = command.source_height = 10;
        command.transparent = KFX_WGPU_DRAW_OPAQUE;
        assert(kfx_wgpu_native_draw(&target, &command, &source, nullptr, copy_oracle, &source) == 1);
        copy_oracle(expected.data(), target.pitch, &source);
        assert(pixels == expected);
        assert(bridge.GetCounters().native_commands == 1 && bridge.GetCounters().gpu_spans == 1);
        assert(bridge.GetCounters().verification_cpu_commands == 1);
    }
    assert(kfx_gpoly_sink == nullptr);
    {
        WgpuTerrainBridge bridge(1, false);
        for (unsigned i = 0; i < 2; ++i) {
            kfx_wgpu_terrain_boundary(1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
            oracle(expected, target.pitch, a, texture, fade);
            kfx_wgpu_terrain_boundary(0);
            assert(pixels == expected);
        }
        assert(bridge.Failed());
        assert(bridge.GetCounters().gpu_batches == 1 && bridge.GetCounters().cpu_replayed_spans == 1);
    }
    {
        WgpuTerrainBridge bridge(0, true);
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 0);
        assert(pixels == expected && bridge.Failed());
    }
    {
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        oracle(expected, target.pitch, a, texture, fade);
        KfxGpolySpan invalid = a;
        invalid.start_low = 0x4000;
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &invalid, texture.data(), fade.data()) == 0);
        assert(pixels == expected && bridge.Failed());
        assert(bridge.GetCounters().cpu_replayed_spans == 1);
    }
#ifndef KFX_BRIDGE_REAL_GPU
    {
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        oracle(expected, target.pitch, a, texture, fade);
        fail_readback = true;
        kfx_wgpu_terrain_boundary(0);
        assert(pixels == expected && bridge.Failed());
        assert(bridge.GetCounters().gpu_spans == 0 && bridge.GetCounters().cpu_replayed_spans == 1);
        fail_readback = false;
    }
#endif
    std::puts("Terrain bridge ordering, resource ownership, CPU interleave and failure reconstruction passed");
}
