#include "WgpuLens.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include <algorithm>
#include <cstring>
#include <limits>
#include <vector>

namespace {
struct Effect {
    uint32_t kind;
    uint8_t* dst;
    long dp;
    const uint8_t* src;
    long sp, width, height;
    const uint8_t* asset;
    const uint8_t* fade = nullptr;
    int aw = 0, ah = 0, alpha = 0;
    uint8_t px = 0, py = 0, sx = 0, sy = 0;
    unsigned fade_rows = 33;
};

void Software(const Effect& e)
{
    const unsigned scale_x = e.kind == 0 ? 0 : ((e.kind == 2 ? e.aw : 640) << 16) / e.width;
    const unsigned scale_y = e.kind == 0 ? 0 : ((e.kind == 2 ? e.ah : 480) << 16) / e.height;
    const int alpha = std::clamp(e.alpha, 0, 256);
    for (long y = 0; y < e.height; ++y) {
        for (long x = 0; x < e.width; ++x) {
            uint8_t value;
            if (e.kind == 0) {
                KfxLensLookup point;
                std::memcpy(&point, e.asset + (y * e.width + x) * sizeof(point), sizeof(point));
                value = e.src[point.y * e.sp + point.x];
            } else if (e.kind == 1) {
                const int vx = (x * scale_x) >> 16;
                const int vy = (y * scale_y) >> 16;
                const int a = e.asset[(((e.py + vy) & 255) << 8) + ((e.px + vx) & 255)];
                const int b = e.asset[(((e.sy + 65536 - vx) & 255) << 8) + ((e.sx + 65536 - vy) & 255)];
                value = e.fade[(std::min((a + b) >> 3, 32) << 8) + e.src[y * e.sp + x]];
            } else {
                const int ox = std::min<int>((x * scale_x) >> 16, e.aw - 1);
                const int oy = std::min<int>((y * scale_y) >> 16, e.ah - 1);
                const uint8_t pixel = e.asset[oy * e.aw + ox];
                const uint8_t source = e.src[y * e.sp + x];
                value = pixel == 255 ? source : (pixel * alpha + source * (256 - alpha)) >> 8;
            }
            e.dst[y * e.dp + x] = value;
        }
    }
}

void Oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    const auto& original = *static_cast<Effect*>(context);
    auto e = original;
    const auto sb = reinterpret_cast<uintptr_t>(e.src);
    const auto db = reinterpret_cast<uintptr_t>(e.dst);
    const size_t sl = (e.height - 1) * e.sp + e.width;
    const size_t dl = (e.height - 1) * e.dp + e.width;
    const bool alias = sb < db + dl && db < sb + sl;
    std::vector<uint8_t> storage;
    if (alias) {
        const auto begin = std::min(sb, db);
        storage.resize(std::max(sb + sl, db + dl) - begin);
        std::memcpy(storage.data() + sb - begin, e.src, sl);
        std::memcpy(storage.data() + db - begin, e.dst, dl);
        e.src = storage.data() + sb - begin;
        e.dst = storage.data() + db - begin;
    } else {
        storage.resize(dl);
        std::memcpy(storage.data(), e.dst, dl);
        e.dst = storage.data();
    }
    Software(e);
    for (long y = 0; y < e.height; ++y)
        std::memcpy(pixels + y * pitch, e.dst + y * e.dp, e.width);
}

bool Gpu(Effect& e)
{
    if (!kfx_wgpu_native_enabled()) return false;
    kfx_wgpu_terrain_boundary(0);
    if (!e.dst || !e.src || !e.asset || e.width <= 0 || e.height <= 0 ||
        e.width > 8192 || e.height > 8192 || e.sp < e.width || e.dp < e.width ||
        e.sp > 1048576 || e.dp > 1048576) return false;
    const size_t sl = (e.height - 1) * e.sp + e.width;
    const size_t dl = (e.height - 1) * e.dp + e.width;
    if (sl > 32 * 1024 * 1024 || dl > 32 * 1024 * 1024) return false;
    size_t al;
    if (e.kind == 0) {
        al = e.width * e.height * sizeof(KfxLensLookup);
        for (long i = 0; i < e.width * e.height; ++i) {
            KfxLensLookup p;
            std::memcpy(&p, e.asset + i * sizeof(p), sizeof(p));
            if (p.x < 0 || p.y < 0 || p.x >= e.width || p.y >= e.height) return false;
        }
    } else if (e.kind == 1) {
        if (!e.fade || e.fade_rows < 33) return false;
        al = 65536;
    } else {
        if (e.aw <= 0 || e.ah <= 0 || e.aw > 8192 || e.ah > 8192) return false;
        al = static_cast<size_t>(e.aw) * e.ah;
    }
    if (64 + sl + al + (e.kind == 1 ? 33 * 256 : 0) > 16 * 1024 * 1024) return false;
    const auto sb = reinterpret_cast<uintptr_t>(e.src);
    const auto db = reinterpret_cast<uintptr_t>(e.dst);
    const auto overlaps = [db, dl](const uint8_t* bytes, size_t length) {
        const auto begin = reinterpret_cast<uintptr_t>(bytes);
        return begin < db + dl && db < begin + length;
    };
    if (overlaps(e.asset, al) || (e.kind == 1 && overlaps(e.fade, 33 * 256))) return false;
    const bool alias = sb < db + dl && db < sb + sl;
    const int64_t delta = alias ? (sb >= db ? static_cast<int64_t>(sb - db) : -static_cast<int64_t>(db - sb)) : 0;
    const uint32_t header[] = {e.kind, static_cast<uint32_t>(e.width), static_cast<uint32_t>(e.height),
        static_cast<uint32_t>(e.sp), static_cast<uint32_t>(e.dp), static_cast<uint32_t>(delta), alias,
        static_cast<uint32_t>(((e.kind == 2 ? e.aw : 640) << 16) / e.width),
        static_cast<uint32_t>(((e.kind == 2 ? e.ah : 480) << 16) / e.height),
        static_cast<uint32_t>(std::clamp(e.alpha, 0, 256)),
        static_cast<uint32_t>(e.px | (e.py << 8) | (e.sx << 16) | (e.sy << 24)),
        64, static_cast<uint32_t>(64 + sl), static_cast<uint32_t>(64 + sl + al),
        static_cast<uint32_t>(e.aw), static_cast<uint32_t>(e.ah)};
    try {
        std::vector<uint8_t> packed(64 + sl + al + (e.kind == 1 ? 33 * 256 : 0));
        for (size_t i = 0; i < 16; ++i)
            for (size_t j = 0; j < 4; ++j) packed[i * 4 + j] = header[i] >> (j * 8);
        std::memcpy(packed.data() + 64, e.src, sl);
        std::memcpy(packed.data() + 64 + sl, e.asset, al);
        if (e.kind == 0) {
            for (long i = 0; i < e.width * e.height; ++i) {
                KfxLensLookup p;
                std::memcpy(&p, e.asset + i * sizeof(p), sizeof(p));
                auto* out = packed.data() + 64 + sl + i * 4;
                out[0] = p.x; out[1] = static_cast<uint16_t>(p.x) >> 8;
                out[2] = p.y; out[3] = static_cast<uint16_t>(p.y) >> 8;
            }
        }
        if (e.kind == 1) std::memcpy(packed.data() + 64 + sl + al, e.fade, 33 * 256);
        KfxGpolyTarget target = {e.dst, static_cast<uint32_t>(e.width), static_cast<uint32_t>(e.height), static_cast<uint32_t>(e.dp)};
        KfxWgpuDrawCommand command = {};
        command.abi_version = 1;
        command.kind = 10;
        command.width = command.clip_width = e.width;
        command.height = command.clip_height = e.height;
        command.transparent = KFX_WGPU_DRAW_OPAQUE;
        KfxWgpuNativeResource resource = {packed.data(), packed.size(), 1, 1, 1};
        return kfx_wgpu_native_draw(&target, &command, &resource, nullptr, Oracle, &e) == 1;
    } catch (...) { return false; }
}

void Render(Effect& e) { if (!Gpu(e)) Software(e); }
}

void KfxLensRemap(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h, const void* map)
{
    Effect e = {0, dst, dp, src, sp, w, h, static_cast<const uint8_t*>(map)};
    Render(e);
}
void KfxLensMist(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h,
    const uint8_t* texture, const uint8_t* fade, uint8_t px, uint8_t py, uint8_t sx, uint8_t sy, unsigned fade_rows)
{
    Effect e = {1, dst, dp, src, sp, w, h, texture, fade};
    e.px = px; e.py = py; e.sx = sx; e.sy = sy; e.fade_rows = fade_rows;
    Render(e);
}
void KfxLensOverlay(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h,
    const uint8_t* overlay, int ow, int oh, short alpha)
{
    Effect e = {2, dst, dp, src, sp, w, h, overlay, nullptr, ow, oh, alpha};
    Render(e);
}
