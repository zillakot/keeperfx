#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/WgpuShadow.h"
#include "kfx/renderer/WgpuMinimap.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstring>
#include <map>
#include <tuple>
#include <stdexcept>
#include <vector>

#ifndef KFX_BRIDGE_REAL_GPU
struct FakeResource { std::vector<uint8_t> bytes; uint32_t width, height, pitch; };
struct FakeContext {
    uint64_t next = 1;
    std::map<uint64_t, FakeResource> resources, targets;
    struct View { uint64_t root; uint32_t x, y, width, height; };
    std::map<uint64_t, View> views;
    // The keyed half of the resource ABI: one resident handle per key and generation.
    std::map<std::tuple<uint32_t, uint64_t, uint64_t>, std::pair<uint64_t, uint64_t>> keyed;
};
static uint64_t keyed_creates = 0, live_resources = 0;
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
// Table handles the mock ABI was asked to draw with, in submission order.
static std::vector<std::pair<uint32_t, uint64_t>> table_log;
extern "C" int32_t kfx_wgpu_draw_submit_shadow(void* handle, uint64_t, const KfxWgpuDrawCommand* command,
    char*, size_t)
{
    (void)static_cast<FakeContext*>(handle)->resources.at(command->table);
    table_log.emplace_back(command->kind, command->table);
    return 1;
}
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
extern "C" void kfx_wgpu_draw_destroy(void* handle)
{
    auto* context = static_cast<FakeContext*>(handle);
    live_resources -= context->resources.size();
    delete context;
}
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
    ++live_resources;
    return id;
}
extern "C" uint64_t kfx_wgpu_draw_resource_create_keyed(void* handle, uint32_t kind,
    uint64_t key_hi, uint64_t key_lo, uint64_t generation, const uint8_t* bytes, size_t length,
    uint32_t width, uint32_t height, uint32_t pitch, uint64_t* previous, char* error,
    size_t capacity)
{
    auto& context = *static_cast<FakeContext*>(handle);
    const auto key = std::make_tuple(kind, key_hi, key_lo);
    const auto resident = context.keyed.find(key);
    *previous = resident == context.keyed.end() ? 0 : resident->second.first;
    if (resident != context.keyed.end() && resident->second.second == generation) {
        // A key names one extent per generation; another extent is refused, not served.
        const auto& kept = context.resources.at(resident->second.first);
        if (kept.bytes.size() != length || kept.width != width || kept.height != height ||
            kept.pitch != pitch) {
            std::snprintf(error, capacity, "keyed resource changed shape without a generation bump");
            *previous = 0;
            return 0;
        }
        return resident->second.first;
    }
    ++keyed_creates;
    const uint64_t id = kfx_wgpu_draw_resource_create(handle, bytes, length, width, height,
        pitch, error, capacity);
    if (id == 0) return 0;
    context.keyed[key] = {id, generation};
    return id;
}
static bool fail_keyed_purge = false;
extern "C" int32_t kfx_wgpu_draw_resources_purge_keyed(void* handle, char* error, size_t capacity)
{
    auto& context = *static_cast<FakeContext*>(handle);
    int32_t result = 1;
    // Releases every handle even when one refuses, so none is left resident unnamed.
    for (const auto& entry : context.keyed)
        if (fail_keyed_purge ||
            kfx_wgpu_draw_resource_release(handle, entry.second.first, error, capacity) != 1) {
            std::snprintf(error, capacity, "injected keyed purge failure");
            result = -1;
        }
    context.keyed.clear();
    return result;
}
extern "C" int32_t kfx_wgpu_draw_resource_release(void* handle, uint64_t id, char*, size_t)
{
    if (static_cast<FakeContext*>(handle)->resources.erase(id) != 1) return -1;
    --live_resources;
    return 1;
}
extern "C" int32_t kfx_wgpu_draw_submit(void* handle, uint64_t id, const KfxWgpuDrawCommand* commands,
    size_t count, char*, size_t)
{
    auto& context = *static_cast<FakeContext*>(handle);
    const auto view = context.views.find(id);
    auto& target = context.targets.at(view == context.views.end() ? id : view->second.root);
    const size_t offset = view == context.views.end() ? 0 : view->second.y * target.pitch + view->second.x;
    std::vector<uint32_t> kinds;
    for (size_t i = 0; i < count; ++i) kinds.push_back(commands[i].kind);
    for (size_t i = 0; i < count; ++i) table_log.emplace_back(commands[i].kind, commands[i].table);
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
        if (c.kind == KFX_WGPU_DRAW_BITMAP) {
            // The artwork's first byte and the per-call geometry's, side by side, so a
            // command served the wrong part shows up as a pixel.
            const uint64_t artwork = (uint64_t(c.start_high) << 32) | c.start_low;
            target.bytes[offset + c.y * target.pitch + c.x] = context.resources.at(artwork).bytes[0];
            target.bytes[offset + c.y * target.pitch + c.x + 1] =
                context.resources.at(c.source).bytes[0];
            continue;
        }
        if (c.kind == KFX_WGPU_DRAW_TRIG) {
            (void)context.resources.at(c.source);
            (void)context.resources.at(c.table);
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

/* What the drawing context does with a sprite: write the artwork's first byte at the
   command's origin. Verification compares this against the readback, so a sprite served
   the wrong artwork or the wrong position fails the batch. */
struct SpriteVerify { uint8_t value; uint32_t x, y; };
static void sprite_verify_oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    const auto* expected = static_cast<const SpriteVerify*>(context);
    pixels[expected->y * pitch + expected->x] = expected->value;
}

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
    // Terrain assets are interned by the pointer they are registered under, so the
    // fixture registers its own pages the way the engine registers block_mem and pixmap.
    kfx_render_asset_range(texture.data(), texture.size());
    kfx_render_asset_range(fade.data(), fade.size());
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
        // Recovery purges the keyed resources: m_frame_invalid clears only in FullRedraw, which
        // releases every keyed handle first, so arena residency never outlives a discarded frame.
        WgpuTerrainBridge bridge(0, false, true);
        kfx_wgpu_terrain_boundary(1);
        const uint64_t created = keyed_creates;
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
        const uint64_t first = bridge.GetCounters().resource_snapshot_bytes;
        assert(first > fade.size());
        assert(keyed_creates == created + 2);
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &b, texture.data(), fade.data()) == 1);
        // A second span over the same page and fade table snapshots and creates nothing.
        assert(bridge.GetCounters().resource_snapshot_bytes == first);
        assert(keyed_creates == created + 2);
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
        const uint64_t live = live_resources;
        for (unsigned i = 0; i < 70; ++i) {
            texture[0] = static_cast<uint8_t>(i);
            kfx_render_assets_changed();
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &target, &a, texture.data(), fade.data()) == 1);
            oracle(expected, target.pitch, a, texture, fade);
        }
        // Every bump takes a new handle so the recorded spans keep the bytes they named;
        // the superseded ones are released once the run they belong to is submitted.
        assert(live_resources == live + 140);
        std::vector<uint8_t> resized(32 * 12, 71), resized_expected = resized;
        KfxGpolyTarget other = {resized.data(), 25, 12, 32};
        assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &other, &a, texture.data(), fade.data()) == 1);
        assert(pixels == expected);
        oracle(resized_expected, other.pitch, a, texture, fade);
        kfx_wgpu_terrain_boundary(0);
        assert(resized == resized_expected);
        assert(live_resources == live + 2);
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
        image.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
        image.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
        clear.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
        image.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
        triangle.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
            if (i == 2) {
                // A bump mid-run supersedes the page handle; the run must still replay from
                // the bytes each span was issued against, so the superseded snapshot lives
                // until the run it belongs to is submitted or discarded.
                for (auto& value : texture) value ^= 0x35;
                kfx_render_assets_changed();
            }
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
                    triangle.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
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
            const uint64_t created = keyed_creates;
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &top, first.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, top, first, fade);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &bottom, second.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, bottom, second, fade);
            // Two pages and one fade table: alternating pages resolve through the key, not
            // the one-slot memo, so the run creates three resources and no more.
            assert(keyed_creates == created + 3);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &top, first.data(), fade.data()) == 1);
            oracle(keyed_expected, keyed_target.pitch, top, first, fade);
            assert(keyed_creates == created + 3);
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
        {
            // Lookup tables with a registered pointer take one keyed handle across commands;
            // a table with the same bytes and no registration stays on the content path and
            // the two never cross.
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            KfxWgpuDrawCommand rect = {};
            rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            rect.kind = KFX_WGPU_DRAW_RECT;
            rect.blend = KFX_WGPU_DRAW_SOURCE_DESTINATION;
            rect.colour = 7;
            rect.width = rect.clip_width = 4;
            rect.height = rect.clip_height = 2;
            KfxWgpuNativeResource keyed_table = {fade.data(), fade.size(), 256, 64, 256, nullptr, 0, 0};
            // A second identity: the memo must hold both, or alternating commands would
            // rebuild the concatenation the context already holds.
            KfxWgpuNativeResource paired = {first.data(), first.size(), 256, 64, 256,
                second.data(), second.size(), 0};
            const uint64_t created = keyed_creates, live = live_resources;
            for (unsigned i = 0; i < 4; ++i) {
                assert(bridge.SubmitNative(run_target, rect, nullptr, &keyed_table, nullptr, nullptr) == 1);
                assert(bridge.SubmitNative(run_target, rect, nullptr, &paired, nullptr, nullptr) == 1);
            }
            assert(keyed_creates == created + 2 && live_resources == live + 2);
            std::vector<uint8_t> unnamed = fade;
            KfxWgpuNativeResource content_table = {unnamed.data(), unnamed.size(), 256, 64, 256, nullptr, 0, 0};
            for (unsigned i = 0; i < 2; ++i)
                assert(bridge.SubmitNative(run_target, rect, nullptr, &content_table, nullptr, nullptr) == 1);
            assert(keyed_creates == created + 2 && live_resources == live + 3);
            assert(bridge.GetCounters().failures == 0);
        }
        {
            // A named image source takes one resident handle across commands, a bump
            // renames it without disturbing the run that named the old bytes, an unnamed
            // source stays per-call, and the name with another shape is refused.
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            std::vector<uint8_t> image(200, 19);
            KfxWgpuDrawCommand picture = {};
            picture.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            picture.kind = KFX_WGPU_DRAW_IMAGE;
            picture.width = picture.clip_width = picture.source_width = 20;
            picture.height = picture.clip_height = picture.source_height = 10;
            picture.transparent = KFX_WGPU_DRAW_OPAQUE;
            KfxWgpuNativeResource picture_source = {image.data(), image.size(), 20, 10, 20,
                nullptr, 0, 0};
            KfxWgpuNativeKey name = {KFX_WGPU_DRAW_KEY_RAW_IMAGE, 0, 0x5eed, 1};
            const uint64_t created = keyed_creates, live = live_resources;
            for (unsigned i = 0; i < 4; ++i)
                assert(bridge.SubmitNative(run_target, picture, &picture_source, nullptr,
                    nullptr, nullptr, nullptr, &name) == 1);
            assert(keyed_creates == created + 1 && live_resources == live + 1);
            name.generation = 2;
            assert(bridge.SubmitNative(run_target, picture, &picture_source, nullptr, nullptr,
                nullptr, nullptr, &name) == 1);
            assert(keyed_creates == created + 2 && live_resources == live + 1);
            assert(bridge.SubmitNative(run_target, picture, &picture_source, nullptr, nullptr,
                nullptr) == 1);
            assert(keyed_creates == created + 2 && live_resources == live + 1);
            KfxWgpuNativeResource reshaped = picture_source;
            reshaped.length = image.size() / 2;
            reshaped.height = 5;
            assert(bridge.SubmitNative(run_target, picture, &reshaped, nullptr, nullptr,
                nullptr, nullptr, &name) == 0);
            assert(bridge.Failed() && std::strstr(bridge.GetError(), "shape") != nullptr);
        }
        {
            // A named part beside a per-call source: the artwork takes one resident handle
            // across commands and across positions, a bump renames it, and another extent
            // under a live name is refused rather than served.
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            std::vector<uint8_t> artwork(64, 0x41), geometry(32, 0x30);
            KfxWgpuDrawCommand huge = {};
            huge.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            huge.kind = KFX_WGPU_DRAW_BITMAP;
            huge.width = huge.clip_width = 20;
            huge.height = huge.clip_height = 10;
            huge.transparent = KFX_WGPU_DRAW_OPAQUE;
            KfxWgpuNativeResource source = {geometry.data(), geometry.size(), 1, 1, 1,
                nullptr, 0, 0};
            KfxWgpuNativePart part = {{artwork.data(), artwork.size(), 1, 1, 1, nullptr, 0, 0},
                {KFX_WGPU_DRAW_KEY_HUGE_SPRITE, 0x11, 0x22, 1}};
            const uint64_t created = keyed_creates, live = live_resources;
            const int positions[] = {0, 5, 10, 15};
            for (unsigned i = 0; i < 4; ++i) {
                huge.x = positions[i];
                geometry[0] = 0x51 + i;
                assert(bridge.SubmitNative(run_target, huge, &source, nullptr, nullptr, nullptr,
                    nullptr, nullptr, &part, 1) == 1);
            }
            // One identity, four scroll positions: one keyed handle, four per-call sources,
            // and every position drew its own geometry beside the shared artwork.
            assert(keyed_creates == created + 1 && live_resources == live + 1);
            for (unsigned i = 0; i < 4; ++i)
                assert(run[positions[i]] == 0x41 && run[positions[i] + 1] == 0x51 + i);
            part.name.generation = 2;
            assert(bridge.SubmitNative(run_target, huge, &source, nullptr, nullptr, nullptr,
                nullptr, nullptr, &part, 1) == 1);
            assert(keyed_creates == created + 2 && live_resources == live + 1);
            KfxWgpuNativePart reshaped = part;
            reshaped.resource.length = artwork.size() / 2;
            assert(bridge.SubmitNative(run_target, huge, &source, nullptr, nullptr, nullptr,
                nullptr, nullptr, &reshaped, 1) == 0);
            assert(bridge.Failed() && std::strstr(bridge.GetError(), "shape") != nullptr);
        }
        {
            // Free-then-reuse: the address a name vouched for is released and a different
            // buffer lands on it. The forget takes the name away and the bump renames what
            // is registered next, so the second buffer can never resolve the first's handle.
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            KfxWgpuDrawCommand picture = {};
            picture.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            picture.kind = KFX_WGPU_DRAW_IMAGE;
            picture.width = picture.clip_width = picture.source_width = 20;
            picture.height = picture.clip_height = picture.source_height = 10;
            picture.transparent = KFX_WGPU_DRAW_OPAQUE;
            std::vector<uint8_t> storage(200, 19);
            KfxWgpuNativeResource asset = {storage.data(), storage.size(), 20, 10, 20,
                nullptr, 0, 0};
            kfx_render_asset_range(storage.data(), storage.size());
            assert(kfx_render_asset_stable(storage.data(), storage.size()));
            KfxWgpuNativeKey name = {KFX_WGPU_DRAW_KEY_RAW_IMAGE, 0,
                static_cast<uint64_t>(reinterpret_cast<uintptr_t>(storage.data())),
                kfx_render_asset_generation};
            const uint64_t created = keyed_creates;
            assert(bridge.SubmitNative(run_target, picture, &asset, nullptr, nullptr,
                nullptr, nullptr, &name) == 1);
            assert(bridge.SubmitNative(run_target, picture, &asset, nullptr, nullptr,
                nullptr, nullptr, &name) == 1);
            assert(keyed_creates == created + 1);
            // The release: LbDataFree forgets the range and bumps before free().
            const uint64_t before = kfx_render_asset_generation;
            kfx_render_asset_range_forget(storage.data());
            kfx_render_assets_changed();
            assert(!kfx_render_asset_stable(storage.data(), storage.size()));
            assert(kfx_render_asset_generation == before + 1);
            // The reuse: another asset at the same address, registered again.
            std::fill(storage.begin(), storage.end(), 0x5b);
            kfx_render_asset_range(storage.data(), storage.size());
            KfxWgpuNativeKey reused = {KFX_WGPU_DRAW_KEY_RAW_IMAGE, 0, name.lo,
                kfx_render_asset_generation};
            assert(reused.lo == name.lo && reused.generation != name.generation);
            assert(bridge.SubmitNative(run_target, picture, &asset, nullptr, nullptr,
                nullptr, nullptr, &reused) == 1);
            assert(keyed_creates == created + 2);
            kfx_render_asset_range_forget(storage.data());
            assert(bridge.GetCounters().failures == 0);
        }
        {
            // A name the table cannot hold is refused, and the refusal is counted rather
            // than silent: one more registration than there are slots must drop the last.
            std::vector<uint8_t> slots(KFX_RENDER_ASSET_RANGES + 1, 0);
            const uint64_t dropped = kfx_render_asset_range_drops;
            for (int i = 0; i <= KFX_RENDER_ASSET_RANGES; ++i)
                kfx_render_asset_range(&slots[i], 1);
            assert(kfx_render_asset_range_drops > dropped);
            assert(!kfx_render_asset_stable(&slots[KFX_RENDER_ASSET_RANGES], 1));
            for (int i = 0; i <= KFX_RENDER_ASSET_RANGES; ++i)
                kfx_render_asset_range_forget(&slots[i]);
        }
        {
            /* Two named buffers instead of one concatenation: the pair is the key, so the
               shadow, triangle, transition and map-row tables are resolved, not rebuilt. */
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            KfxWgpuDrawCommand rect = {};
            rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            rect.kind = KFX_WGPU_DRAW_RECT;
            rect.blend = KFX_WGPU_DRAW_SOURCE_DESTINATION;
            rect.width = rect.clip_width = 4;
            rect.height = rect.clip_height = 2;
            std::vector<uint8_t> head(256 * 64, 0x31), ghost(256 * 256, 0x57);
            kfx_render_asset_range(head.data(), head.size());
            kfx_render_asset_range(ghost.data(), ghost.size());
            KfxWgpuNativeResource pair = {head.data(), head.size(), 256, 320, 256,
                ghost.data(), ghost.size(), 0};
            const uint64_t created = keyed_creates;
            const uint64_t snapshot = bridge.GetCounters().resource_snapshot_bytes;
            for (unsigned i = 0; i < 6; ++i)
                assert(bridge.SubmitNative(run_target, rect, nullptr, &pair, nullptr,
                    nullptr) == 1);
            assert(keyed_creates == created + 1);
            assert(bridge.GetCounters().resource_snapshot_bytes ==
                snapshot + head.size() + ghost.size());
            // The generation is what makes a rewrite behind the pair visible.
            kfx_render_assets_changed();
            assert(bridge.SubmitNative(run_target, rect, nullptr, &pair, nullptr, nullptr) == 1);
            assert(keyed_creates == created + 2);
            // A key names one extent: the same pair with another shape is refused.
            KfxWgpuNativeResource reshaped = {head.data(), head.size(), 256, 160, 256,
                ghost.data(), ghost.size(), 0};
            assert(bridge.SubmitNative(run_target, rect, nullptr, &reshaped, nullptr,
                nullptr) == 0);
            assert(bridge.Failed() && std::strstr(bridge.GetError(), "shape") != nullptr);
            kfx_render_asset_range_forget(head.data());
            kfx_render_asset_range_forget(ghost.data());
        }
        {
            /* One key across two emitters: a creature shadow and a general triangle declare
               the same fade-plus-ghost pair through kfx_wgpu_fade_ghost_table, so a run that
               draws both holds one handle for it rather than two shapes of one key. */
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            std::vector<uint8_t> fade_rows(16384, 0x13), ghost_rows(65536, 0x71);
            kfx_render_asset_range(fade_rows.data(), fade_rows.size());
            kfx_render_asset_range(ghost_rows.data(), ghost_rows.size());
            const KfxWgpuNativeResource pair =
                kfx_wgpu_fade_ghost_table(fade_rows.data(), ghost_rows.data());
            std::vector<uint8_t> geometry(60 + 8192, 0x24), artwork(152 + 16, 0x35);
            KfxWgpuNativeResource triangle_source = {geometry.data(), geometry.size(), 1, 1, 1,
                nullptr, 0, 0};
            KfxWgpuNativeResource shadow_source = {artwork.data(), artwork.size(), 1, 1, 1,
                nullptr, 0, 0};
            KfxWgpuDrawCommand triangle = {};
            triangle.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            triangle.kind = KFX_WGPU_DRAW_TRIG;
            triangle.width = triangle.clip_width = 20;
            triangle.height = triangle.clip_height = 10;
            triangle.source_y = 8192;
            triangle.source_width = 64;
            triangle.transparent = KFX_WGPU_DRAW_OPAQUE;
            KfxWgpuDrawCommand shadow = triangle;
            shadow.kind = KFX_WGPU_DRAW_SHADOW;
            shadow.source_y = 0;
            shadow.source_width = 0;
            const uint64_t created = keyed_creates;
            table_log.clear();
            assert(bridge.SubmitNative(run_target, triangle, &triangle_source, &pair, nullptr,
                nullptr) == 1);
            assert(bridge.SubmitShadow(run_target, shadow, &shadow_source, &pair, nullptr,
                nullptr, nullptr) == 1);
            assert(keyed_creates == created + 1);
            uint64_t triangle_table = 0, shadow_table = 0;
            for (const auto& row : table_log) {
                if (row.first == KFX_WGPU_DRAW_TRIG) triangle_table = row.second;
                if (row.first == KFX_WGPU_DRAW_SHADOW) shadow_table = row.second;
            }
            assert(triangle_table != 0 && triangle_table == shadow_table);
            assert(bridge.GetCounters().failures == 0);
            kfx_render_asset_range_forget(fade_rows.data());
            kfx_render_asset_range_forget(ghost_rows.data());
        }
        {
            // A key names one extent: the same pointer with another shape is refused, not
            // served with the first shape's resource.
            std::vector<uint8_t> run(24 * 10, 0x6a);
            KfxGpolyTarget run_target = {run.data(), 20, 10, 24};
            WgpuTerrainBridge bridge(0, false, false);
            KfxWgpuDrawCommand rect = {};
            rect.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            rect.kind = KFX_WGPU_DRAW_RECT;
            rect.blend = KFX_WGPU_DRAW_SOURCE_DESTINATION;
            rect.width = rect.clip_width = 4;
            rect.height = rect.clip_height = 2;
            KfxWgpuNativeResource whole = {fade.data(), fade.size(), 256, 64, 256, nullptr, 0, 0};
            KfxWgpuNativeResource half = {fade.data(), fade.size() / 2, 256, 32, 256, nullptr, 0, 0};
            assert(bridge.SubmitNative(run_target, rect, nullptr, &whole, nullptr, nullptr) == 1);
            assert(bridge.SubmitNative(run_target, rect, nullptr, &half, nullptr, nullptr) == 0);
            assert(bridge.Failed() && std::strstr(bridge.GetError(), "shape") != nullptr);
        }
        {
            // A refused purge leaves residency the caller was told is gone, so the frame
            // stays invalid and the CPU keeps the target until a redraw that can release it.
            WgpuTerrainBridge bridge(0, false, false);
            kfx_wgpu_terrain_boundary(1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &top, first.data(), fade.data()) == 1);
            kfx_wgpu_terrain_boundary(0);
            const uint64_t invalid = bridge.GetCounters().invalid_frames;
            fail_keyed_purge = true;
            bridge.FullRedraw();
            fail_keyed_purge = false;
            // The redraw that failed is one invalid frame like any other; what it leaves
            // behind is the residency, which no later redraw can reach.
            assert(!bridge.FrameValid() && bridge.GetCounters().resource_purge_failures == 1);
            assert(bridge.GetCounters().invalid_frames == invalid + 1);
            bridge.FullRedraw();
            assert(bridge.FrameValid() && bridge.GetCounters().resource_purge_failures == 1);
            assert(bridge.GetCounters().invalid_frames == invalid + 1);
        }
        // An unregistered pointer carries no name, so its bytes take a per-call resource.
        assert(kfx_render_asset_stable(first.data(), first.size()));
        std::vector<uint8_t> unregistered(KFX_GPOLY_TEXTURE_BYTES, 0x21);
        assert(!kfx_render_asset_stable(unregistered.data(), unregistered.size()));
        {
            // Registrations nest: the land map and the front-end background share storage.
            // A range that ends inside the queried bytes must not hide one that covers them.
            std::vector<uint8_t> nested(4096, 0);
            kfx_render_asset_range(nested.data(), nested.size());
            kfx_render_asset_range(nested.data() + 1024, 16);
            assert(kfx_render_asset_stable(nested.data() + 1024, 2048));
            kfx_render_asset_range_forget(nested.data() + 1024);
            kfx_render_asset_range_forget(nested.data());
            assert(!kfx_render_asset_stable(nested.data(), 16));
        }
        {
            WgpuTerrainBridge bridge(0, false, false);
            kfx_wgpu_terrain_boundary(1);
            const uint64_t created = keyed_creates, live = live_resources;
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &top, unregistered.data(), fade.data()) == 1);
            assert(kfx_gpoly_sink(kfx_gpoly_sink_context, &keyed_target, &bottom, unregistered.data(), fade.data()) == 1);
            // One keyed fade table, one per-call page per span, all released with the run.
            assert(keyed_creates == created + 1 && live_resources == live + 3);
            kfx_wgpu_terrain_boundary(0);
            assert(live_resources == live + 1);
            assert(bridge.GetCounters().failures == 0);
        }
    }
    {
        /* Sprite artwork is named by the emitter, so the same name serves one handle
           until the generation moves. The fake submit writes the artwork's first byte,
           which is what tells a stale upload from a fresh one. */
        std::vector<uint8_t> sprite_pixels(24 * 10, 0x11);
        KfxGpolyTarget sprite_target = {sprite_pixels.data(), 20, 10, 24};
        std::vector<uint8_t> artwork(2 * 4 * 3), ranges(8 * (4 + 3)), remap(256);
        for (size_t i = 0; i < remap.size(); ++i) remap[i] = i;
        for (size_t i = 0; i < 4 * 3u; ++i) { artwork[2 * i] = 0x40 + i; artwork[2 * i + 1] = 1; }
        KfxWgpuNativeResource art = {artwork.data(), artwork.size(), 1, 1, 1, nullptr, 0, 0};
        KfxWgpuNativeResource rng = {ranges.data(), ranges.size(), 1, 1, 1, nullptr, 0, 0};
        KfxWgpuNativeResource map = {remap.data(), remap.size(), 1, 1, 1, nullptr, 0, 0};
        KfxWgpuDrawCommand command = {};
        command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        command.kind = KFX_WGPU_DRAW_SPRITE;
        command.x = 2;
        command.y = 3;
        command.width = command.clip_width = 20;
        command.height = command.clip_height = 10;
        command.source_width = 4;
        command.source_height = 3;
        command.transparent = KFX_WGPU_DRAW_OPAQUE;
        // One name reused across a whole scene takes one handle and one upload.
        {
            WgpuTerrainBridge bridge(0, false, false);
            const uint64_t created = keyed_creates;
            const uint64_t snapshot = bridge.GetCounters().resource_snapshot_bytes;
            KfxWgpuSpriteAssets assets = {&art, &rng, &map, nullptr, artwork.data(), 1, 0};
            for (unsigned i = 0; i < 5; ++i)
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            assert(keyed_creates == created + 1);
            assert(bridge.GetCounters().resource_snapshot_bytes ==
                snapshot + artwork.size() + 5 * ranges.size() + remap.size());
            assert(sprite_pixels[3 * 24 + 2] == artwork[0]);
            // Rewriting the bytes behind a live name without a bump serves the upload the
            // name already has; the bump is what makes the new artwork visible.
            for (auto& value : artwork) value ^= 0x5a;
            assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            assert(sprite_pixels[3 * 24 + 2] == (artwork[0] ^ 0x5a) && keyed_creates == created + 1);
            assets.generation = 2;
            assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            assert(sprite_pixels[3 * 24 + 2] == artwork[0] && keyed_creates == created + 2);
        }
        // An address reused by different artwork of the same size: only the generation
        // separates them, and the superseded handle is released with the run.
        {
            WgpuTerrainBridge bridge(0, false, false);
            const uint64_t created = keyed_creates, live = live_resources;
            std::vector<uint8_t> reborn = artwork;
            for (auto& value : reborn) value ^= 0x27;
            KfxWgpuSpriteAssets assets = {&art, &rng, &map, nullptr, artwork.data(), 7, 0};
            assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            assert(sprite_pixels[3 * 24 + 2] == artwork[0]);
            art.bytes = reborn.data();
            assets.generation = 8;
            assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            assert(sprite_pixels[3 * 24 + 2] == reborn[0]);
            // The live artwork and the interned remap; the superseded artwork is gone.
            assert(keyed_creates == created + 2 && live_resources == live + 2);
            art.bytes = artwork.data();
        }
        /* One name, two positions in one frame: the ranges carry the position and stay
           per call, so verification against the CPU oracle must hold for both. */
        {
            WgpuTerrainBridge bridge(0, false, true);
            KfxWgpuSpriteAssets assets = {&art, &rng, &map, nullptr, artwork.data(), 11, 0};
            const std::pair<uint32_t, uint32_t> places[] = {{2, 3}, {9, 6}};
            for (const auto& place : places) {
                command.x = place.first;
                command.y = place.second;
                SpriteVerify expected = {artwork[0], place.first, place.second};
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    sprite_verify_oracle, &expected) == 1);
            }
            command.x = 2;
            command.y = 3;
            assert(sprite_pixels[3 * 24 + 2] == artwork[0]);
            assert(sprite_pixels[6 * 24 + 9] == artwork[0]);
            assert(bridge.GetCounters().gpu_sprite_commands == 2);
            assert(bridge.GetCounters().verification_cpu_commands == 2);
            assert(bridge.GetCounters().failures == 0);
        }
        /* A remap row is enumerated as (kind, row) for the record words and keyed by the
           address its registered range makes stable; two rows stay two handles. */
        {
            std::vector<uint8_t> rows(256 * 2);
            for (size_t i = 0; i < rows.size(); ++i) rows[i] = i;
            kfx_render_remap_rows(KFX_REMAP_WHITE, rows.data(), 2);
            assert(kfx_render_remap_id(rows.data()) == (KFX_REMAP_WHITE << 16));
            assert(kfx_render_remap_id(rows.data() + 256) == ((KFX_REMAP_WHITE << 16) | 1));
            assert(kfx_render_remap_id(rows.data() + 1) == KFX_REMAP_NONE);
            assert(kfx_render_remap_id(rows.data() + 512) == KFX_REMAP_NONE);
            // Registering changes what an id names, so it bumps without the caller doing it.
            const uint64_t generation = kfx_render_asset_generation;
            kfx_render_remap_rows(KFX_REMAP_RED, rows.data(), 1);
            assert(kfx_render_asset_generation == generation + 1);
            kfx_render_remap_rows(KFX_REMAP_RED, nullptr, 0);
            // Arguments outside the table are dropped, not stored under a neighbouring kind.
            kfx_render_remap_rows(KFX_REMAP_NONE, rows.data(), 1);
            kfx_render_remap_rows(KFX_REMAP_KIND_COUNT, rows.data(), 1);
            kfx_render_remap_rows(KFX_REMAP_GHOST, rows.data(), 0x10001);
            assert(kfx_render_remap_id(rows.data()) == (KFX_REMAP_WHITE << 16));
            WgpuTerrainBridge bridge(0, false, false);
            kfx_render_asset_range(rows.data(), rows.size());
            const uint64_t created = keyed_creates;
            KfxWgpuNativeResource first = {rows.data(), 256, 1, 1, 1, nullptr, 0, 0};
            KfxWgpuNativeResource second = {rows.data() + 256, 256, 1, 1, 1, nullptr, 0, 0};
            KfxWgpuSpriteAssets assets = {&art, &rng, &first, nullptr, artwork.data(), 21,
                kfx_render_remap_id(rows.data())};
            for (unsigned i = 0; i < 3; ++i)
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    nullptr, nullptr) == 1);
            assets.remap = &second;
            assets.remap_id = kfx_render_remap_id(rows.data() + 256);
            assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                nullptr, nullptr) == 1);
            // The artwork and the two rows; three draws of one row are one create.
            assert(keyed_creates == created + 3);
            assert(bridge.GetCounters().failures == 0);
            kfx_render_asset_range_forget(rows.data());
            kfx_render_remap_rows(KFX_REMAP_WHITE, nullptr, 0);
        }
        // No name, no residency: the artwork takes a per-call resource like the ranges.
        {
            WgpuTerrainBridge bridge(0, false, false);
            const uint64_t created = keyed_creates, live = live_resources;
            KfxWgpuSpriteAssets assets = {&art, &rng, &map, nullptr, nullptr, 0, 0};
            for (unsigned i = 0; i < 3; ++i)
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets, nullptr, nullptr) == 1);
            // Only the remap is interned, by content: it carries no registered pointer.
            assert(keyed_creates == created && live_resources == live + 1);
            assert(bridge.GetCounters().failures == 0);
        }
        /* The cursor is a sprite whose artwork carries the cursor flag. It keys in a
           namespace of its own, because the pointer path expands one address into
           different coverage bytes than the sprite path does. */
        {
            std::vector<uint8_t> cursor_artwork = artwork;
            cursor_artwork[0] ^= 0x33;
            for (size_t i = 0; i < 4 * 3u; ++i) cursor_artwork[2 * i + 1] = 1;
            KfxWgpuNativeResource cursor_art = {cursor_artwork.data(), cursor_artwork.size(),
                1, 1, 1, nullptr, 0, 1};
            // One pointer sprite across a scene is one upload; only the ranges are per call.
            {
                WgpuTerrainBridge bridge(0, false, false);
                const uint64_t created = keyed_creates;
                const uint64_t snapshot = bridge.GetCounters().resource_snapshot_bytes;
                KfxWgpuSpriteAssets assets = {&cursor_art, &rng, &map, nullptr,
                    cursor_artwork.data(), 31, KFX_REMAP_NONE};
                for (unsigned i = 0; i < 5; ++i)
                    assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                        nullptr, nullptr) == 1);
                assert(keyed_creates == created + 1);
                assert(bridge.GetCounters().resource_snapshot_bytes ==
                    snapshot + cursor_artwork.size() + 5 * ranges.size() + remap.size());
                assert(sprite_pixels[3 * 24 + 2] == cursor_artwork[0]);
                // A pointer pack reloaded behind the same address: the bump re-uploads.
                for (auto& value : cursor_artwork) value ^= 0x5a;
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    nullptr, nullptr) == 1);
                assert(sprite_pixels[3 * 24 + 2] == (cursor_artwork[0] ^ 0x5a) &&
                    keyed_creates == created + 1);
                assets.generation = 32;
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    nullptr, nullptr) == 1);
                assert(sprite_pixels[3 * 24 + 2] == cursor_artwork[0] &&
                    keyed_creates == created + 2);
                for (auto& value : cursor_artwork) value ^= 0x5a;
            }
            // One address on both paths stays two uploads: neither expansion serves the other.
            {
                WgpuTerrainBridge bridge(0, false, false);
                const uint64_t created = keyed_creates;
                KfxWgpuSpriteAssets cursor = {&cursor_art, &rng, &map, nullptr,
                    cursor_artwork.data(), 41, KFX_REMAP_NONE};
                KfxWgpuSpriteAssets sprite = {&art, &rng, &map, nullptr,
                    cursor_artwork.data(), 41, 0};
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &cursor,
                    nullptr, nullptr) == 1);
                assert(sprite_pixels[3 * 24 + 2] == cursor_artwork[0]);
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &sprite,
                    nullptr, nullptr) == 1);
                assert(sprite_pixels[3 * 24 + 2] == artwork[0]);
                assert(keyed_creates == created + 2);
                assert(bridge.GetCounters().failures == 0);
            }
            // A key names one extent: a second extent under one generation is refused.
            {
                WgpuTerrainBridge bridge(0, false, false);
                std::vector<uint8_t> shorter(cursor_artwork.begin(), cursor_artwork.end() - 2);
                KfxWgpuNativeResource narrow = cursor_art;
                narrow.bytes = shorter.data();
                narrow.length = shorter.size();
                KfxWgpuSpriteAssets assets = {&cursor_art, &rng, &map, nullptr,
                    cursor_artwork.data(), 51, KFX_REMAP_NONE};
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    nullptr, nullptr) == 1);
                assets.artwork = &narrow;
                assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                    nullptr, nullptr) == 0);
                assert(bridge.GetCounters().failures == 1);
            }
            /* One name, two positions in one frame: the ranges carry where the pointer
               sits, so verification must hold for both pixel sets. */
            {
                WgpuTerrainBridge bridge(0, false, true);
                KfxWgpuSpriteAssets assets = {&cursor_art, &rng, &map, nullptr,
                    cursor_artwork.data(), 61, KFX_REMAP_NONE};
                const std::pair<uint32_t, uint32_t> places[] = {{2, 3}, {9, 6}};
                for (const auto& place : places) {
                    command.x = place.first;
                    command.y = place.second;
                    SpriteVerify expected = {cursor_artwork[0], place.first, place.second};
                    assert(kfx_wgpu_native_draw_sprite(&sprite_target, &command, &assets,
                        sprite_verify_oracle, &expected) == 1);
                }
                command.x = 2;
                command.y = 3;
                assert(sprite_pixels[3 * 24 + 2] == cursor_artwork[0]);
                assert(sprite_pixels[6 * 24 + 9] == cursor_artwork[0]);
                assert(bridge.GetCounters().gpu_sprite_commands == 2);
                assert(bridge.GetCounters().verification_cpu_commands == 2);
                assert(bridge.GetCounters().failures == 0);
            }
        }
    }
#endif
    std::puts("Terrain bridge ordering, batching, resource ownership, CPU interleave and failure reconstruction passed");
}
