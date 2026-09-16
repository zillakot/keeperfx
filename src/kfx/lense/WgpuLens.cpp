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
    KfxLensIdentity name = {0, 0};
};

/* The last lens map converted for the drawing context. A resident key needs no bytes,
   but the ABI still takes a pointer, so the conversion and its bounds check are kept
   here instead of being redone every frame. */
struct MapCache {
    uint64_t id = 0, generation = 0;
    long width = 0, height = 0;
    std::vector<uint8_t> bytes;
};
MapCache map_cache;
uint64_t lens_generation = 1;

void SoftwareRemap(const Effect& e)
{
    const uint8_t* const src = e.src;
    uint8_t* const dst = e.dst;
    const long sp = e.sp, dp = e.dp, width = e.width, height = e.height;
    const uint8_t* entry = e.asset;
    for (long y = 0; y < height; ++y) {
        uint8_t* const drow = dst + y * dp;
        for (long x = 0; x < width; ++x, entry += sizeof(KfxLensLookup)) {
            KfxLensLookup point;
            std::memcpy(&point, entry, sizeof(point));
            drow[x] = src[point.y * sp + point.x];
        }
    }
}

void SoftwareMist(const Effect& e)
{
    const uint8_t* const src = e.src;
    uint8_t* const dst = e.dst;
    const uint8_t* const texture = e.asset;
    const uint8_t* const fade = e.fade;
    const long sp = e.sp, dp = e.dp, width = e.width, height = e.height;
    const unsigned scale_x = (640 << 16) / width;
    const unsigned scale_y = (480 << 16) / height;
    const int px = e.px, py = e.py, sx = e.sx, sy = e.sy;
    for (long y = 0; y < height; ++y) {
        const int vy = (y * scale_y) >> 16;
        const int primary_row = ((py + vy) & 255) << 8;
        const int secondary_column = (sx + 65536 - vy) & 255;
        const uint8_t* const srow = src + y * sp;
        uint8_t* const drow = dst + y * dp;
        for (long x = 0; x < width; ++x) {
            const int vx = (x * scale_x) >> 16;
            const int a = texture[primary_row + ((px + vx) & 255)];
            const int b = texture[(((sy + 65536 - vx) & 255) << 8) + secondary_column];
            drow[x] = fade[(std::min((a + b) >> 3, 32) << 8) + srow[x]];
        }
    }
}

void SoftwareOverlay(const Effect& e)
{
    const uint8_t* const src = e.src;
    uint8_t* const dst = e.dst;
    const uint8_t* const overlay = e.asset;
    const long sp = e.sp, dp = e.dp, width = e.width, height = e.height;
    const int aw = e.aw, ah = e.ah;
    const unsigned scale_x = (aw << 16) / width;
    const unsigned scale_y = (ah << 16) / height;
    const int alpha = std::clamp(e.alpha, 0, 256);
    const int inverse = 256 - alpha;
    for (long y = 0; y < height; ++y) {
        const int oy = std::min<int>((y * scale_y) >> 16, ah - 1);
        const uint8_t* const orow = overlay + static_cast<long>(oy) * aw;
        const uint8_t* const srow = src + y * sp;
        uint8_t* const drow = dst + y * dp;
        for (long x = 0; x < width; ++x) {
            const int ox = std::min<int>((x * scale_x) >> 16, aw - 1);
            const uint8_t pixel = orow[ox];
            const uint8_t source = srow[x];
            drow[x] = pixel == 255 ? source : (pixel * alpha + source * inverse) >> 8;
        }
    }
}

void Software(const Effect& e)
{
    if (e.width <= 0 || e.height <= 0) return;
    if (e.kind == 0) SoftwareRemap(e);
    else if (e.kind == 1) SoftwareMist(e);
    else SoftwareOverlay(e);
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
    for (long y = 0; y < e.height; ++y)
        std::memcpy(e.dst + y * e.dp, pixels + y * pitch, e.width);
    Software(e);
    for (long y = 0; y < e.height; ++y)
        std::memcpy(pixels + y * pitch, e.dst + y * e.dp, e.width);
}

bool Gpu(Effect& e)
{
    if (!kfx_wgpu_native_enabled()) return false;
    kfx_wgpu_native_flush();
    if (!e.dst || !e.src || !e.asset || e.width <= 0 || e.height <= 0 ||
        e.width > 8192 || e.height > 8192 || e.sp < e.width || e.dp < e.width ||
        e.sp > 1048576 || e.dp > 1048576) return false;
    const size_t sl = (e.height - 1) * e.sp + e.width;
    const size_t dl = (e.height - 1) * e.dp + e.width;
    if (sl > 32 * 1024 * 1024 || dl > 32 * 1024 * 1024) return false;
    size_t al;
    const bool cached = e.name.id != 0 && map_cache.id == e.name.id &&
        map_cache.generation == e.name.generation && map_cache.width == e.width &&
        map_cache.height == e.height;
    if (e.kind == 0) {
        al = e.width * e.height * sizeof(KfxLensLookup);
        if (!kfx_wgpu_native_read_barrier(e.asset, al)) return false;
        if (!cached) for (long i = 0; i < e.width * e.height; ++i) {
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
    const size_t fl = e.kind == 1 ? 33 * 256 : 0;
    /* One table is named only when every table of the effect is: a half-split source
       would need a second packed layout for no gain. */
    const bool named = e.name.id != 0 &&
        (e.kind != 1 || kfx_render_asset_stable(e.fade, fl) != 0);
    // The packed limit is kept for the named form too: a decline falls back to the CPU,
    // an arena overflow fails the bridge.
    if (64 + sl + al + fl > 16 * 1024 * 1024) return false;
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
        64, named ? 0 : static_cast<uint32_t>(64 + sl),
        named ? 0 : static_cast<uint32_t>(64 + sl + al),
        static_cast<uint32_t>(e.aw), static_cast<uint32_t>(e.ah)};
    if (!kfx_wgpu_native_read_barrier(e.src, sl) ||
        !kfx_wgpu_native_read_barrier(e.asset, al) ||
        !kfx_wgpu_native_read_barrier(e.fade, e.kind == 1 ? 33 * 256 : 0)) return false;
    try {
        const uint8_t* map = e.asset;
        if (e.kind == 0) {
            if (!cached) {
                map_cache.bytes.assign(al, 0);
                for (long i = 0; i < e.width * e.height; ++i) {
                    KfxLensLookup p;
                    std::memcpy(&p, e.asset + i * sizeof(p), sizeof(p));
                    auto* out = map_cache.bytes.data() + i * 4;
                    out[0] = p.x; out[1] = static_cast<uint16_t>(p.x) >> 8;
                    out[2] = p.y; out[3] = static_cast<uint16_t>(p.y) >> 8;
                }
                map_cache.id = e.name.id;
                map_cache.generation = e.name.generation;
                map_cache.width = e.width;
                map_cache.height = e.height;
            }
            map = map_cache.bytes.data();
        }
        std::vector<uint8_t> packed(64 + sl + (named ? 0 : al + fl));
        for (size_t i = 0; i < 16; ++i)
            for (size_t j = 0; j < 4; ++j) packed[i * 4 + j] = header[i] >> (j * 8);
        std::memcpy(packed.data() + 64, e.src, sl);
        if (!named) {
            std::memcpy(packed.data() + 64 + sl, map, al);
            if (fl != 0) std::memcpy(packed.data() + 64 + sl + al, e.fade, fl);
        }
        KfxGpolyTarget target = {e.dst, static_cast<uint32_t>(e.width), static_cast<uint32_t>(e.height), static_cast<uint32_t>(e.dp)};
        KfxWgpuDrawCommand command = {};
        command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
        command.kind = 10;
        command.width = command.clip_width = e.width;
        command.height = command.clip_height = e.height;
        command.transparent = KFX_WGPU_DRAW_OPAQUE;
        KfxWgpuNativeResource resource = {packed.data(), packed.size(), 1, 1, 1, nullptr, 0, 0};
        if (!named)
            return kfx_wgpu_native_draw(&target, &command, &resource, nullptr, Oracle, &e) == 1;
        KfxWgpuNativePart parts[2] = {};
        parts[0].resource = {map, al, 1, 1, 1, nullptr, 0, 0};
        parts[0].name = {KFX_WGPU_DRAW_KEY_LENS_MAP, e.name.id,
            static_cast<uint64_t>(e.width) << 32 | static_cast<uint32_t>(e.height),
            e.name.generation};
        unsigned count = 1;
        if (fl != 0) {
            parts[1].resource = {e.fade, fl, 1, 1, 1, nullptr, 0, 0};
            parts[1].name = {KFX_WGPU_DRAW_KEY_LENS_FADE, 0,
                static_cast<uint64_t>(reinterpret_cast<uintptr_t>(e.fade)),
                kfx_render_asset_generation};
            count = 2;
        }
        return kfx_wgpu_native_draw_parts(&target, &command, &resource, parts, count, nullptr,
            Oracle, &e) == 1;
    } catch (...) { return false; }
}

void Render(Effect& e) { if (!Gpu(e) && kfx_wgpu_native_cpu_barrier()) Software(e); }
}

uint64_t KfxLensGeneration() { return lens_generation; }

void KfxLensTablesChanged()
{
    ++lens_generation;
    map_cache = MapCache();
}

void KfxLensRemap(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h,
    const void* map, KfxLensIdentity name)
{
    Effect e = {0, dst, dp, src, sp, w, h, static_cast<const uint8_t*>(map)};
    e.name = name;
    Render(e);
}
void KfxLensMist(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h,
    const uint8_t* texture, const uint8_t* fade, uint8_t px, uint8_t py, uint8_t sx, uint8_t sy,
    unsigned fade_rows, KfxLensIdentity name)
{
    Effect e = {1, dst, dp, src, sp, w, h, texture, fade};
    e.px = px; e.py = py; e.sx = sx; e.sy = sy; e.fade_rows = fade_rows; e.name = name;
    Render(e);
}
void KfxLensOverlay(uint8_t* dst, long dp, const uint8_t* src, long sp, long w, long h,
    const uint8_t* overlay, int ow, int oh, short alpha, KfxLensIdentity name)
{
    Effect e = {2, dst, dp, src, sp, w, h, overlay, nullptr, ow, oh, alpha};
    e.name = name;
    Render(e);
}
