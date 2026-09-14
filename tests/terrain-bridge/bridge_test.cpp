#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/WgpuShadow.h"
#include "kfx/renderer/WgpuMinimap.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstring>
#include <map>
#include <stdexcept>
#include <vector>

#ifndef KFX_BRIDGE_REAL_GPU
struct FakeResource { std::vector<uint8_t> bytes; uint32_t width, height, pitch; };
struct FakeContext {
    uint64_t next = 1;
    std::map<uint64_t, FakeResource> resources, targets;
    struct View { uint64_t root; uint32_t x, y, width, height; };
    std::map<uint64_t, View> views;
};
extern "C" int32_t kfx_wgpu_draw_frame_begin(void*, uint64_t, char*, size_t) { return 1; }
extern "C" int32_t kfx_wgpu_draw_frame_flush(void*, char*, size_t) { return 1; }
extern "C" int32_t kfx_wgpu_draw_frame_end(void*, char*, size_t) { return 1; }
extern "C" int32_t kfx_wgpu_draw_frame_abort(void*, char*, size_t) { return 1; }
static uint32_t fake_frame_flags = 0;
extern "C" int32_t kfx_wgpu_draw_frame_status(void*, uint32_t* flags, char*, size_t)
{ *flags = fake_frame_flags; fake_frame_flags = 0; return 1; }
extern "C" uint64_t kfx_wgpu_draw_target_view(void* handle, uint64_t root, uint32_t x, uint32_t y,
    uint32_t width, uint32_t height, char*, size_t)
{
    auto& context = *static_cast<FakeContext*>(handle);
    const uint64_t id = context.next++;
    context.views[id] = {root, x, y, width, height};
    return id;
}
extern "C" void* kfx_wgpu_draw_context(void* presenter, char*, size_t) { return presenter; }
static std::vector<std::vector<uint32_t>> submit_log;
// The mock ABI rejects what the Rust packer rejects, through the bridge's own predicate.
static bool packable(uint32_t kind) { return WgpuTerrainBridge::PacksInBatch(kind); }
extern "C" int32_t kfx_wgpu_draw_submit_shadow(void*, uint64_t, const KfxWgpuDrawCommand*, char*, size_t) { return -1; }
extern "C" int32_t kfx_wgpu_draw_shadow_scratch_reset(void*, char*, size_t) { return 1; }
extern "C" int32_t kfx_wgpu_draw_shadow_scratch_read(void*, uint8_t*, size_t, char*, size_t) { return -1; }
extern "C" uint64_t kfx_wgpu_draw_target_snapshot(void*, uint64_t, uint32_t, uint32_t, uint32_t, uint32_t, uint32_t, char*, size_t) { return 0; }
extern "C" int32_t kfx_wgpu_draw_target_snapshot_release(void*, uint64_t, char*, size_t) { return -1; }
static bool mock_target_images = false;
extern "C" int32_t kfx_wgpu_draw_submit_target_images(void* handle, uint64_t id,
    const KfxWgpuDrawCommand* commands, size_t count, char*, size_t)
{
    if (!mock_target_images) return -1;
    if (count != 1) return -1;
    submit_log.push_back({commands[0].kind});
    auto& target = static_cast<FakeContext*>(handle)->targets.at(id);
    target.bytes[commands[0].y * target.pitch + commands[0].x] = commands[0].colour;
    return 1;
}
static bool fail_readback = false;
static bool mock_triangles = false, mismatch_triangles = false;
extern "C" int32_t kfx_wgpu_draw_submit_triangles(void* handle, uint64_t target, const KfxWgpuTriangle*, size_t, char*, size_t)
{
    if (!mock_triangles) return -1;
    static_cast<FakeContext*>(handle)->targets.at(target).bytes[0] = mismatch_triangles ? 122 : 123;
    return 1;
}
static int triangle_oracle(const KfxGpolyTarget* target, const KfxWgpuTriangle*, const uint8_t*, const uint8_t*)
{
    target->pixels[0] = 123;
    return 1;
}
extern "C" int32_t kfx_wgpu_draw_counters(void*, KfxWgpuDrawCounters* counters, char*, size_t)
{ *counters = {}; return 1; }
extern "C" void* kfx_wgpu_draw_create(char*, size_t) { return new FakeContext; }
extern "C" void kfx_wgpu_draw_destroy(void* handle) { delete static_cast<FakeContext*>(handle); }
static bool fail_target_create = false, throw_target_create = false, fail_resource_create = false;
extern "C" uint64_t kfx_wgpu_draw_target_create(void* handle, uint32_t width, uint32_t height, char* error, size_t capacity)
{
    if (throw_target_create) throw std::runtime_error("injected target creation exception");
    if (fail_target_create) {
        std::snprintf(error, capacity, "injected target creation failure");
        return 0;
    }
    auto& context = *static_cast<FakeContext*>(handle);
    const auto id = context.next++;
    context.targets[id] = {std::vector<uint8_t>(width * height), width, height, width};
    return id;
}
extern "C" int32_t kfx_wgpu_draw_target_release(void* handle, uint64_t id, char*, size_t)
{ auto& context = *static_cast<FakeContext*>(handle); return context.targets.erase(id) + context.views.erase(id) == 1 ? 1 : -1; }
extern "C" void kfx_wgpu_draw_resource_mark_cursor(void*, uint64_t) {}
extern "C" uint64_t kfx_wgpu_draw_resource_create(void* handle, const uint8_t* bytes, size_t length,
    uint32_t width, uint32_t height, uint32_t pitch, char* error, size_t capacity)
{
    if (fail_resource_create) {
        std::snprintf(error, capacity, "injected resource creation failure");
        return 0;
    }
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
    const auto view = context.views.find(id);
    auto& target = context.targets.at(view == context.views.end() ? id : view->second.root);
    const size_t offset = view == context.views.end() ? 0 : view->second.y * target.pitch + view->second.x;
    std::vector<uint32_t> kinds;
    for (size_t i = 0; i < count; ++i) kinds.push_back(commands[i].kind);
    if (count != 0) submit_log.push_back(kinds);
    // The Rust packer whitelist rejects any unpackable kind outside a single-command batch.
    for (size_t i = 0; i < count; ++i)
        if (!packable(commands[i].kind) && count != 1) return -1;
    for (size_t i = 0; i < count; ++i) {
        const auto& c = commands[i];
        if (c.kind == KFX_WGPU_DRAW_RECT || c.kind == KFX_WGPU_DRAW_LENS_EFFECT ||
            c.kind == KFX_WGPU_DRAW_MINIMAP) {
            for (uint32_t y = 0; y < c.height; ++y)
                std::fill_n(target.bytes.data() + offset + (c.y + y) * target.pitch + c.x, c.width, c.colour);
            continue;
        }
        if (c.kind == KFX_WGPU_DRAW_SPRITE) {
            const auto& sprite = context.resources.at(c.source);
            target.bytes[offset + c.y * target.pitch + c.x] = sprite.bytes[0];
            continue;
        }
        if (c.kind == KFX_WGPU_DRAW_CLEAR) {
            for (uint32_t y = 0; y < c.height; ++y)
                std::fill_n(target.bytes.data() + offset + y * target.pitch, c.width, c.colour);
            continue;
        }
        const auto& source = context.resources.at(c.source);
        if (c.kind == KFX_WGPU_DRAW_IMAGE) {
            for (uint32_t y = 0; y < c.height; ++y)
                std::memcpy(target.bytes.data() + offset + y * target.pitch, source.bytes.data() + y * source.pitch, c.width);
        } else {
            assert(c.kind == KFX_WGPU_DRAW_GPOLY_SPAN);
            const auto& table = context.resources.at(c.table);
            uint64_t value = (uint64_t(c.start_high) << 32) | c.start_low;
            const uint64_t step = (uint64_t(c.step_high) << 32) | c.step_low;
            for (uint32_t x = 0; x < c.width; ++x, value += step) {
                const uint32_t high = value >> 32;
                const uint32_t uv = ((high << 8) | (high >> 24)) & 0x1f1f;
                target.bytes[offset + c.y * target.pitch + c.x + x] = table.bytes[(value & 0xff00) | source.bytes[uv]];
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

struct NestedOracle {
    WgpuTerrainBridge* bridge;
    KfxWgpuNativeResource* source;
    unsigned calls = 0;
};

static void nested_primitive_oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    auto& nested = *static_cast<NestedOracle*>(context);
    assert(nested.bridge->IsOracleActive());
    assert(!kfx_wgpu_native_enabled());
    KfxGpolyTarget target = {pixels, nested.source->width, nested.source->height, pitch};
    KfxWgpuDrawCommand primitive = {};
    primitive.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    primitive.kind = KFX_WGPU_DRAW_RECT;
    primitive.width = primitive.height = 1;
    primitive.clip_width = target.width;
    primitive.clip_height = target.height;
    assert(kfx_wgpu_native_draw(&target, &primitive, nullptr, nullptr, nullptr, nullptr) == 0);
    assert(nested.bridge->SubmitNative(target, primitive, nullptr, nullptr, nullptr, nullptr) == 0);
    assert(kfx_wgpu_native_cpu_barrier());
    kfx_wgpu_terrain_boundary(0);
    kfx_wgpu_native_flush();
    assert(nested.bridge->IsOracleActive());
    copy_oracle(pixels, pitch, nested.source);
    ++nested.calls;
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
        // Rewriting asset bytes behind a stable pointer requires a generation bump.
        kfx_render_assets_changed();
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
        // Recovery purges the interned caches: m_frame_invalid clears only in FullRedraw, which
        // releases every cached handle first, so arena residency never outlives a discarded frame.
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        const uint64_t first = bridge.GetCounters().resource_snapshot_bytes;
        assert(first > fade.size());
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &b, texture.data(), fade.data()) == 1);
        assert(bridge.GetCounters().resource_snapshot_bytes == first);
        kfx_wgpu_terrain_boundary(0);
        bridge.FullRedraw();
        assert(bridge.FrameValid());
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        assert(bridge.GetCounters().resource_snapshot_bytes == 2 * first);
        kfx_wgpu_terrain_boundary(0);
        assert(bridge.GetCounters().failures == 0);
    }
    {
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        for (unsigned i = 0; i < 70; ++i) {
            texture[0] = static_cast<uint8_t>(i);
            kfx_render_assets_changed();
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
        KfxWgpuNativeResource source = {image.data(), image.size(), 20, 10, 20, nullptr, 0, 0};
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
    for (bool resident : {false, true}) {
        WgpuTerrainBridge bridge(0, false, true, resident);
        std::vector<uint8_t> image(200, 37);
        KfxWgpuNativeResource source = {image.data(), image.size(), 20, 10, 20, nullptr, 0, 0};
        KfxWgpuDrawCommand command = {};
        command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        command.kind = KFX_WGPU_DRAW_IMAGE;
        command.width = command.clip_width = command.source_width = 20;
        command.height = command.clip_height = command.source_height = 10;
        command.transparent = KFX_WGPU_DRAW_OPAQUE;
        NestedOracle nested = {&bridge, &source};
        if (resident) kfx_wgpu_terrain_boundary(1);
        assert(kfx_wgpu_native_draw(&target, &command, &source, nullptr, nested_primitive_oracle, &nested) == 1);
        assert(kfx_wgpu_native_cpu_barrier());
        copy_oracle(expected.data(), target.pitch, &source);
        assert(pixels == expected && nested.calls == 1);
        assert(!bridge.IsOracleActive());
        assert(bridge.GetCounters().native_commands == 1);
        assert(bridge.GetCounters().verified_batches == 1);
        assert(bridge.GetCounters().failures == 0);
    }
    for (bool verify : {false, true}) {
        std::vector<uint8_t> resident_pixels(24 * 10, 0x6a), independent = resident_pixels;
        KfxGpolyTarget resident = {resident_pixels.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, verify, true);
        bridge.BeginResident();
        bridge.Boundary(true);
        for (unsigned i = 0; i < 5; ++i) {
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
            oracle(independent, resident.pitch, a, texture, fade);
            bridge.Flush();
        }
        assert(resident_pixels != independent);
        uint64_t replay_ns = UINT64_MAX;
        assert(bridge.ResidentTarget(resident, &replay_ns) != 0);
        assert(replay_ns != UINT64_MAX);
        const auto before = bridge.GetCounters();
        assert(before.bridge_initial_index_bytes == 236);
        assert(before.resident_batches == 5 && before.native_copy_bytes == 0);
        assert(before.bridge_readbacks == (verify ? 5 : 0));
        assert(before.verification_readbacks == (verify ? 5 : 0));
        std::vector<uint8_t> source_pixels(200, 17);
        KfxWgpuNativeResource source = {source_pixels.data(), source_pixels.size(), 20, 10, 20, nullptr, 0, 0};
        KfxWgpuDrawCommand image = {};
        image.abi_version = 1;
        image.kind = KFX_WGPU_DRAW_IMAGE;
        image.width = image.clip_width = image.source_width = 20;
        image.height = image.clip_height = image.source_height = 10;
        image.transparent = KFX_WGPU_DRAW_OPAQUE;
        assert(bridge.SubmitNative(resident, image, &source, nullptr, copy_oracle, &source) == 1);
        copy_oracle(independent.data(), resident.pitch, &source);
        std::fill(source_pixels.begin(), source_pixels.end(), 99);
        assert(bridge.GetCounters().bridge_initial_index_bytes == 236);
        assert(bridge.CpuBarrier());
        assert(resident_pixels == independent);
        assert(bridge.ResidentTarget(resident) == 0);
        assert(bridge.GetCounters().barrier_readbacks == 1);
        resident_pixels[0] = independent[0] = 88;
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        oracle(independent, resident.pitch, a, texture, fade);
        bridge.Flush();
        KfxGpolyTarget alias = {resident_pixels.data() + 1, 20, 10, 24};
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &alias, &b, texture.data(), fade.data()) == 1);
        std::vector<uint8_t> alias_expected(independent.begin() + 1, independent.end());
        oracle(alias_expected, alias.pitch, b, texture, fade);
        std::copy(alias_expected.begin(), alias_expected.end(), independent.begin() + 1);
        assert(bridge.CpuBarrier());
        assert(resident_pixels == independent);
        assert(bridge.GetCounters().target_alias_barriers == 1);
        assert(bridge.FrameValid() && !bridge.Failed());
    }
    for (bool verify : {false, true}) {
        std::vector<uint8_t> frame_pixels(24 * 10, 0x6a), independent = frame_pixels;
        KfxGpolyTarget frame = {frame_pixels.data(), 20, 10, 24};
        KfxGpolyTarget view = {frame_pixels.data() + 2 * 24 + 3, 12, 6, 24};
        WgpuTerrainBridge bridge(0, false, verify, true);
        assert(bridge.BeginFrame(frame));
        std::vector<uint8_t> source_pixels(12 * 6, 17);
        KfxWgpuNativeResource source = {source_pixels.data(), source_pixels.size(), 12, 6, 12, nullptr, 0, 0};
        KfxWgpuDrawCommand image = {};
        image.abi_version = 1;
        image.kind = KFX_WGPU_DRAW_IMAGE;
        image.width = image.clip_width = image.source_width = 12;
        image.height = image.clip_height = image.source_height = 6;
        image.transparent = KFX_WGPU_DRAW_OPAQUE;
        for (int i = 0; i < 5; ++i) {
            bridge.Boundary(false);
            source_pixels[0] = 17 + i;
            assert(bridge.SubmitNative(view, image, &source, nullptr, copy_oracle, &source) == 1);
            copy_oracle(independent.data() + 2 * 24 + 3, 24, &source);
            bridge.Boundary(true);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &frame, &a, texture.data(), fade.data()) == 1);
            oracle(independent, 24, a, texture, fade);
            bridge.Flush();
        }
        std::fill(source_pixels.begin(), source_pixels.end(), 99);
        assert(bridge.GetCounters().target_creations == 1);
        assert(bridge.GetCounters().target_alias_barriers == 0);
        assert(bridge.GetCounters().bridge_initial_index_bytes == 236);
        assert(bridge.GetCounters().native_copy_bytes == 0);
        assert(bridge.GetCounters().bridge_readbacks == (verify ? 10 : 0));
        assert(bridge.EndFrame(true));
        assert(frame_pixels == independent);
        assert(bridge.GetCounters().barrier_readbacks == 1);
        assert(bridge.GetCounters().native_copy_bytes == 200);
        assert(bridge.BeginFrame(frame, true));
        std::vector<uint8_t> clear_pixels(200, 144);
        KfxWgpuNativeResource clear_reference = {clear_pixels.data(), clear_pixels.size(), 20, 10, 20, nullptr, 0, 0};
        KfxWgpuDrawCommand clear = {};
        clear.abi_version = 1;
        clear.kind = KFX_WGPU_DRAW_CLEAR;
        clear.colour = 144;
        clear.width = clear.clip_width = 20;
        clear.height = clear.clip_height = 10;
        clear.transparent = KFX_WGPU_DRAW_OPAQUE;
        assert(bridge.SubmitNative(frame, clear, nullptr, nullptr, copy_oracle, &clear_reference) == 1);
        copy_oracle(independent.data(), 24, &clear_reference);
        assert(bridge.GetCounters().bridge_initial_index_bytes == 236);
        assert(bridge.GetCounters().target_creations == 1);
        const auto before_read = bridge.GetCounters().barrier_readbacks;
        assert(bridge.ReadBarrier(clear_pixels.data(), clear_pixels.size()));
        assert(bridge.GetCounters().barrier_readbacks == before_read);
        assert(bridge.ReadBarrier(view.pixels, 12));
        assert(frame_pixels == independent && bridge.ResidentTarget(frame) != 0);
        assert(bridge.GetCounters().barrier_readbacks == before_read + 1);
        assert(bridge.SubmitNative(frame, clear, nullptr, nullptr, copy_oracle, &clear_reference) == 1);
        assert(bridge.GetCounters().bridge_initial_index_bytes == 236);
        assert(bridge.EndFrame(true));
        assert(frame_pixels == independent);
        // A kernel flag from an earlier frame is not a failure: it counts the frame and
        // takes the full redraw, and the bridge keeps drawing.
        const auto before_invalid = bridge.GetCounters().invalid_frames;
        fake_frame_flags = 1;
        assert(bridge.BeginFrame(frame, true));
        assert(bridge.GetCounters().invalid_frames == before_invalid + 1);
        assert(bridge.FrameValid() && !bridge.Failed());
        assert(bridge.SubmitNative(frame, clear, nullptr, nullptr, copy_oracle, &clear_reference) == 1);
        assert(bridge.EndFrame(true));
        assert(bridge.GetCounters().invalid_frames == before_invalid + 1);
    }
    {
        std::vector<uint8_t> resident_pixels(240, 0x6a), checkpoint = resident_pixels;
        KfxGpolyTarget resident = {resident_pixels.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(1, false, false, true);
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        bridge.Flush();
        assert(bridge.FrameValid() && resident_pixels == checkpoint);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &b, texture.data(), fade.data()) == 1);
        bridge.Flush();
        assert(bridge.Failed() && !bridge.FrameValid());
        assert(!bridge.CpuBarrier() && bridge.ResidentTarget(resident) == 0);
        assert(resident_pixels == checkpoint && bridge.GetCounters().invalid_frames == 1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        assert(resident_pixels == checkpoint && bridge.GetCounters().cpu_gpoly_spans == 0);
        std::fill(resident_pixels.begin(), resident_pixels.end(), 0);
        bridge.FullRedraw();
        assert(bridge.FrameValid() && bridge.Failed());
        const auto clear_checkpoint = resident_pixels;
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 0);
        oracle(resident_pixels, resident.pitch, a, texture, fade);
        assert(bridge.CpuBarrier() && bridge.FrameValid() && resident_pixels != clear_checkpoint);
    }
    for (bool at_barrier : {false, true}) {
        std::vector<uint8_t> resident_pixels(240, 0x6a);
        KfxGpolyTarget resident = {resident_pixels.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, true, true);
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        bridge.Flush();
        resident_pixels[0] ^= 1;
        if (at_barrier) {
            assert(!bridge.CpuBarrier());
        } else {
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &b, texture.data(), fade.data()) == 1);
            bridge.Flush();
        }
        assert(bridge.Failed() && !bridge.FrameValid());
        assert(bridge.GetCounters().missing_cpu_barriers == 1);
    }
    {
        std::vector<uint8_t> resident_pixels(240, 0x6a), independent = resident_pixels;
        KfxGpolyTarget resident = {resident_pixels.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, true, true);
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        oracle(independent, resident.pitch, a, texture, fade);
        bridge.Flush();
        KfxWgpuNativeResource alias_source = {resident_pixels.data(), 236, 20, 10, 24, nullptr, 0, 0};
        KfxWgpuDrawCommand image = {};
        image.abi_version = 1;
        image.kind = KFX_WGPU_DRAW_IMAGE;
        image.width = image.clip_width = image.source_width = 20;
        image.height = image.clip_height = image.source_height = 10;
        image.transparent = KFX_WGPU_DRAW_OPAQUE;
        assert(bridge.SubmitNative(resident, image, &alias_source, nullptr, copy_oracle, &alias_source) == 1);
        assert(bridge.CpuBarrier());
        assert(resident_pixels == independent);
        assert(bridge.GetCounters().target_alias_barriers == 1);
        assert(bridge.GetCounters().barrier_readbacks == 2);
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
        auto* context = kfx_wgpu_draw_create(nullptr, 0);
        WgpuTerrainBridge bridge(0, false, false, true);
        assert(bridge.AttachPresenter(context));
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        oracle(expected, target.pitch, a, texture, fade);
        bridge.Flush();
        bridge.DetachPresenter();
        assert(pixels == expected && bridge.Context() == nullptr);
        assert(static_cast<FakeContext*>(context)->targets.empty());
        assert(static_cast<FakeContext*>(context)->resources.empty());
        kfx_wgpu_draw_destroy(context);
    }
    {
        std::vector<uint8_t> resident_pixels(240, 0x6a), checkpoint = resident_pixels;
        KfxGpolyTarget resident = {resident_pixels.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, false, true);
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &resident, &a, texture.data(), fade.data()) == 1);
        bridge.Flush();
        fail_readback = true;
        assert(!bridge.CpuBarrier());
        fail_readback = false;
        assert(bridge.Failed() && !bridge.FrameValid() && resident_pixels == checkpoint);
    }
    for (const bool mismatch : {false, true}) {
        std::vector<uint8_t> triangle_pixels(20 * 10, 106);
        KfxGpolyTarget triangle_target = {triangle_pixels.data(), 20, 10, 20};
        KfxWgpuTriangle triangle = {};
        triangle.abi_version = 1;
        WgpuTerrainBridge bridge(0, false, true);
        mock_triangles = true;
        mismatch_triangles = mismatch;
        kfx_wgpu_terrain_boundary(1);
        assert(kfx_gpoly_triangle_sink(kfx_gpoly_triangle_context, &triangle_target,
            &triangle, texture.data(), fade.data(), triangle_oracle) == 1);
        kfx_wgpu_terrain_boundary(0);
        assert(bridge.Failed() == mismatch);
        assert(bridge.GetCounters().verified_triangles == (mismatch ? 0 : 1));
        assert(bridge.GetCounters().verified_batches == (mismatch ? 0 : 1));
        assert(bridge.GetCounters().gpu_triangles == (mismatch ? 0 : 1));
        assert(bridge.GetCounters().replayed_triangles == (mismatch ? 1 : 0));
        assert(triangle_pixels[0] == 123);
        assert(std::all_of(triangle_pixels.begin() + 1, triangle_pixels.end(),
            [](uint8_t pixel) { return pixel == 106; }));
        mock_triangles = false;
    }
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
    // A failure while a batched run is pending must replay the run or invalidate the frame;
    // dropping it while FrameValid() stays true would present a hole with no redraw request.
    {
        // Spans only: the CPU rasterizer owns them, so recovery repaints and the frame survives.
        std::vector<uint8_t> run(24 * 10, 0x6a), run_expected = run;
        KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, false, true);
        assert(bridge.BeginFrame(run_target));
        bridge.Boundary(true);
        for (unsigned i = 0; i < 3; ++i) {
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &run_target, &a, texture.data(), fade.data()) == 1);
            oracle(run_expected, run_target.pitch, a, texture, fade);
        }
        assert(bridge.GetCounters().gpu_batches == 0);
        std::vector<uint8_t> blend(256, 3);
        KfxWgpuNativeResource blend_table = {blend.data(), blend.size(), 256, 1, 256, nullptr, 0, 0};
        KfxWgpuDrawCommand rect = {};
        rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        rect.kind = KFX_WGPU_DRAW_RECT;
        rect.width = rect.clip_width = 4;
        rect.height = rect.clip_height = 2;
        rect.transparent = KFX_WGPU_DRAW_OPAQUE;
        fail_resource_create = true;
        assert(bridge.SubmitNative(run_target, rect, nullptr, &blend_table, nullptr, nullptr) == 0);
        fail_resource_create = false;
        assert(bridge.Failed());
        assert(bridge.FrameValid());
        assert(run == run_expected);
        assert(bridge.GetCounters().cpu_replayed_spans == 3);
        assert(bridge.GetCounters().rejected_commands == 0 && bridge.GetCounters().rejected_spans == 0);
    }
    {
        // A run recorded against two views of the frame root has no single CPU replay buffer:
        // the rasterizer would place the larger view's spans outside the smaller one, so the
        // run is discarded for a full redraw instead of replayed.
        std::vector<uint8_t> run(24 * 10, 0x6a);
        const std::vector<uint8_t> untouched = run;
        KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
        KfxGpolyTarget corner = {run.data(), 8, 4, 24};
        const KfxGpolySpan small = {1, 1, 4, a.start_low, a.start_high, a.step_low, a.step_high};
        WgpuTerrainBridge bridge(0, false, false, true);
        assert(bridge.BeginFrame(run_target));
        bridge.Boundary(true);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &run_target, &a, texture.data(), fade.data()) == 1);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &corner, &small, texture.data(), fade.data()) == 1);
        assert(bridge.GetCounters().bridge_target_runs == 1);
        assert(bridge.GetCounters().gpu_batches == 0 && bridge.FrameValid());
        fail_target_create = true;
        bridge.Flush();
        fail_target_create = false;
        assert(bridge.Failed());
        assert(!bridge.FrameValid());
        assert(run == untouched);
        assert(bridge.GetCounters().cpu_replayed_spans == 0);
        assert(bridge.GetCounters().rejected_spans == 2);
        bridge.FullRedraw();
        assert(bridge.FrameValid());
    }
    for (const bool by_exception : {false, true}) {
        // Mixed run: the generic command has no CPU oracle here, so the frame must go invalid.
        std::vector<uint8_t> run(24 * 10, 0x6a);
        const std::vector<uint8_t> untouched = run;
        KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, false, true);
        assert(bridge.BeginFrame(run_target));
        KfxWgpuDrawCommand rect = {};
        rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        rect.kind = KFX_WGPU_DRAW_RECT;
        rect.x = 1; rect.y = 1;
        rect.width = rect.clip_width = 4;
        rect.height = rect.clip_height = 2;
        rect.colour = 55;
        rect.transparent = KFX_WGPU_DRAW_OPAQUE;
        assert(bridge.SubmitNative(run_target, rect, nullptr, nullptr, nullptr, nullptr) == 1);
        bridge.Boundary(true);
        for (unsigned i = 0; i < 2; ++i)
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &run_target, &a, texture.data(), fade.data()) == 1);
        assert(bridge.GetCounters().gpu_batches == 0 && bridge.FrameValid());
        // The first flush builds the target, so it fails before the frame is marked dirty.
        fail_target_create = !by_exception;
        throw_target_create = by_exception;
        bridge.Flush();
        fail_target_create = throw_target_create = false;
        assert(bridge.Failed());
        assert(!bridge.FrameValid());
        assert(run == untouched);
        assert(bridge.GetCounters().rejected_commands == 1);
        assert(bridge.GetCounters().rejected_spans == 2);
        assert(bridge.GetCounters().invalid_frames == 1);
        bridge.FullRedraw();
        assert(bridge.FrameValid());
    }
    {
        // The same guarantee through SubmitNative's own exception handler.
        std::vector<uint8_t> run(24 * 10, 0x6a);
        const std::vector<uint8_t> untouched = run;
        KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
        WgpuTerrainBridge bridge(0, false, false, true);
        KfxWgpuDrawCommand rect = {};
        rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        rect.kind = KFX_WGPU_DRAW_RECT;
        rect.width = rect.clip_width = 4;
        rect.height = rect.clip_height = 2;
        rect.colour = 55;
        rect.transparent = KFX_WGPU_DRAW_OPAQUE;
        throw_target_create = true;
        assert(bridge.SubmitNative(run_target, rect, nullptr, nullptr, nullptr, nullptr) == 0);
        throw_target_create = false;
        assert(bridge.Failed() && !bridge.FrameValid());
        assert(run == untouched);
        assert(bridge.GetCounters().rejected_commands == 1);
        bridge.FullRedraw();
        assert(bridge.FrameValid());
    }
    // Seeded interleave: batching must match per-command flushing and isolate solo kinds.
    std::vector<uint8_t> interleave_expected;
    WgpuTerrainBridge::Counters interleave_flushed = {};
    for (const bool flush_each : {true, false}) {
        std::vector<uint8_t> frame_pixels(24 * 10, 0x6a);
        KfxGpolyTarget frame = {frame_pixels.data(), 20, 10, 24};
        std::vector<uint8_t> sprite_bytes(64, 0), image_bytes(200, 0);
        KfxWgpuNativeResource sprite_source = {sprite_bytes.data(), sprite_bytes.size(), 8, 8, 8, nullptr, 0, 0};
        KfxWgpuNativeResource image_source = {image_bytes.data(), image_bytes.size(), 20, 10, 20, nullptr, 0, 0};
        submit_log.clear();
        mock_triangles = true;
        mismatch_triangles = false;
        mock_target_images = true;
        unsigned commands_issued = 0;
        WgpuTerrainBridge bridge(0, false, false, true);
        assert(bridge.BeginFrame(frame));
        uint32_t seed = 0x5eed1234u;
        auto next = [&seed]() { seed = seed * 1664525u + 1013904223u; return seed >> 16; };
        for (unsigned step = 0; step < 180; ++step) {
            const unsigned pick = next() % 12u;
            const uint8_t colour = static_cast<uint8_t>(next() & 0x7fu);
            if (pick < 3) {
                if (step % 5u == 0) bridge.Boundary(false);
                bridge.Boundary(true);
                if (pick < 2) {
                    assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &frame, &a, texture.data(), fade.data()) == 1);
                } else {
                    KfxWgpuTriangle triangle = {};
                    triangle.abi_version = 1;
                    assert(kfx_gpoly_triangle_sink(kfx_gpoly_triangle_context, &frame,
                        &triangle, texture.data(), fade.data(), triangle_oracle) == 1);
                }
                if (flush_each) bridge.Flush();
                continue;
            }
            KfxWgpuDrawCommand c = {};
            c.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            c.clip_width = 20;
            c.clip_height = 10;
            c.transparent = KFX_WGPU_DRAW_OPAQUE;
            c.colour = colour;
            c.x = static_cast<int32_t>(next() % 16u);
            c.y = static_cast<int32_t>(next() % 8u);
            c.width = 1 + next() % 4u;
            c.height = 1 + next() % 2u;
            const KfxWgpuNativeResource* source = nullptr;
            switch (pick) {
            case 3:
                c.kind = KFX_WGPU_DRAW_IMAGE;
                c.x = c.y = 0;
                c.width = c.source_width = 20;
                c.height = c.source_height = 10;
                std::fill(image_bytes.begin(), image_bytes.end(), colour);
                source = &image_source;
                break;
            case 4:
            case 5: c.kind = KFX_WGPU_DRAW_RECT; break;
            case 6:
                c.kind = KFX_WGPU_DRAW_CLEAR;
                c.x = c.y = 0;
                c.width = 20;
                c.height = 10;
                break;
            case 7:
            case 8:
            case 9:
                c.kind = KFX_WGPU_DRAW_SPRITE;
                c.source_x = pick == 9 ? 8u : 0u;
                c.source_width = c.source_height = 8;
                sprite_bytes[0] = colour;
                source = &sprite_source;
                break;
            case 10: c.kind = KFX_WGPU_DRAW_LENS_EFFECT; break;
            default:
                if (step & 1u) {
                    c.kind = KFX_WGPU_DRAW_MINIMAP;
                } else {
                    c.kind = KFX_WGPU_DRAW_TRANSITION;
                    c.width = c.height = 1;
                }
                break;
            }
            assert(bridge.SubmitNative(frame, c, source, nullptr, nullptr, nullptr) == 1);
            ++commands_issued;
            if (flush_each) bridge.Flush();
        }
        assert(bridge.EndFrame(true));
        const auto counts = bridge.GetCounters();
        assert(counts.failures == 0 && bridge.FrameValid());
        for (const auto& batch : submit_log)
            for (const uint32_t kind : batch)
                assert(packable(kind) || batch.size() == 1);
        mock_triangles = false;
        mock_target_images = false;
        if (flush_each) {
            interleave_expected = frame_pixels;
            interleave_flushed = counts;
            continue;
        }
        assert(frame_pixels == interleave_expected);
        assert(counts.native_commands == interleave_flushed.native_commands);
        assert(counts.gpu_spans == interleave_flushed.gpu_spans);
        assert(counts.gpu_pixels == interleave_flushed.gpu_pixels);
        assert(counts.gpu_triangles == interleave_flushed.gpu_triangles);
        assert(counts.gpu_sprite_commands == interleave_flushed.gpu_sprite_commands);
        assert(counts.gpu_ordered_sprites == interleave_flushed.gpu_ordered_sprites);
        assert(counts.bridge_solo_batches == interleave_flushed.bridge_solo_batches);
        assert(counts.gpu_batches < interleave_flushed.gpu_batches);
        assert(counts.gpu_batches < commands_issued);
    }
    {
        // Registered pages are keyed by pointer: two pages drawn in one frame must
        // not collide, and each must render its own bytes.
        std::vector<uint8_t> first(KFX_GPOLY_TEXTURE_BYTES), second(KFX_GPOLY_TEXTURE_BYTES);
        for (size_t i = 0; i < first.size(); ++i) {
            first[i] = (i * 31 + 5) & 255;
            second[i] = first[i] ^ 0x3c;
        }
        kfx_render_asset_range(first.data(), first.size());
        kfx_render_asset_range(second.data(), second.size());
        kfx_render_asset_range(fade.data(), fade.size());
        kfx_render_assets_changed();
        const KfxGpolySpan top = {2, 3, 12, 0x3010, 0x03000801, 0x22, 0x01000100};
        const KfxGpolySpan bottom = {2, 5, 12, 0x3010, 0x03000801, 0x22, 0x01000100};
        std::vector<uint8_t> keyed(24 * 10, 0x6a), keyed_expected = keyed;
        KfxGpolyTarget keyed_target = {keyed.data(), 20, 10, 24};
        {
            WgpuTerrainBridge bridge(0, false, false);
            kfx_wgpu_terrain_boundary(1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &top, first.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, top, first, fade);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &bottom, second.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, bottom, second, fade);
            kfx_wgpu_terrain_boundary(0);
            assert(keyed == keyed_expected);
            bool differ = false;
            for (uint32_t i = 0; i < top.count; ++i)
                differ = differ || keyed[3 * 24 + 2 + i] != keyed[5 * 24 + 2 + i];
            assert(differ);
            // Rewriting registered bytes without a bump serves the cached upload;
            // the bump is what makes the new bytes visible.
            const std::vector<uint8_t> stale = keyed;
            for (auto& value : second) value ^= 0x5a;
            kfx_wgpu_terrain_boundary(1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &bottom, second.data(), fade.data()) == 1);
            kfx_wgpu_terrain_boundary(0);
            assert(keyed == stale);
            kfx_render_assets_changed();
            kfx_wgpu_terrain_boundary(1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &bottom, second.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, bottom, second, fade);
            kfx_wgpu_terrain_boundary(0);
            assert(keyed == keyed_expected);
        }
        // An unregistered pointer must fall back to content comparison.
        assert(kfx_render_asset_stable(first.data(), first.size()));
        assert(!kfx_render_asset_stable(texture.data(), texture.size()));
    }
#endif
    std::puts("Terrain bridge ordering, batching, resource ownership, CPU interleave and failure reconstruction passed");
}
