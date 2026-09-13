#include "kfx/renderer/WgpuCursor.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "bflib_sprite.h"
#include "bflib_vidraw.h"
#include "bflib_vidsurface.h"
#include "bflib_planar.h"
#include <SDL3/SDL.h>
#include <algorithm>
#include <cstring>
#include <vector>

static KfxWgpuCursorCounters totals = {};
KfxWgpuCursorCounters kfx_wgpu_cursor_counters() { return totals; }
void kfx_wgpu_cursor_software(KfxWgpuCursorSoftware operation)
{
    if (operation == CursorSoftwareSprite) ++totals.cpu_sprite_draws;
    else if (operation == CursorSoftwareBackup) ++totals.cpu_backups;
    else ++totals.cpu_compositions;
}

#ifdef KFX_RUST_PRESENTER
namespace {
struct SurfaceLock {
    SDL_Surface* surface;
    bool locked;
    explicit SurfaceLock(SDL_Surface* value) : surface(value), locked(value && SDL_LockSurface(value)) {}
    ~SurfaceLock() { if (locked) SDL_UnlockSurface(surface); }
};
void put(std::vector<uint8_t>& data, size_t at, uint32_t value)
{
    for (unsigned i = 0; i < 4; ++i) data[at + i] = value >> (i * 8);
}
bool sprite_asset(const TbSprite* sprite, const int32_t* xs, const int32_t* ys,
    std::vector<uint8_t>& asset)
{
    if (!sprite || !sprite->Data || !sprite->SWidth || !sprite->SHeight || !xs || !ys ||
        sprite->SHeight > MAX_SUPPORTED_SCREEN_HEIGHT / 10) return false;
    const unsigned w = sprite->SWidth, h = sprite->SHeight;
    const size_t axis = size_t(w) * h * 2;
    asset.assign(axis + 8 * (w + h) + 256, 0);
    for (unsigned a = 0; a < 2; ++a) {
        const int32_t* steps = a ? ys : xs;
        int previous = -1;
        for (unsigned i = 0; i < (a ? h : w); ++i) {
            const int64_t start = steps[i * 2], n = steps[i * 2 + 1];
            if (start < 0 || n < 0 || start + n > 16384 ||
                (previous >= 0 && start != previous)) return false;
            previous = start + n;
            const size_t at = axis + 8 * ((a ? w : 0) + i);
            put(asset, at, start);
            put(asset, at + 4, n);
        }
    }
    const uint8_t* rle = sprite->Data;
    for (unsigned y = 0; y < h; ++y) {
        unsigned x = 0;
        for (;;) {
            const int run = int8_t(*rle++);
            if (!run) break;
            const unsigned n = run < 0 ? -run : run;
            if (n > w - x) return false;
            if (run > 0) for (unsigned i = 0; i < n; ++i) {
                asset[2 * (y * w + x + i)] = *rle++;
                asset[2 * (y * w + x + i) + 1] = 1;
            }
            x += n;
        }
    }
    for (unsigned i = 0; i < 256; ++i) asset[asset.size() - 256 + i] = i;
    return true;
}
KfxWgpuDrawCommand command(uint32_t kind, uint32_t width, uint32_t height)
{
    KfxWgpuDrawCommand c = {};
    c.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    c.kind = kind;
    c.width = c.clip_width = width;
    c.height = c.clip_height = height;
    c.transparent = KFX_WGPU_DRAW_OPAQUE;
    return c;
}
struct Oracle { const TbSprite* sprite; const int32_t* xs; const int32_t* ys; uint32_t height; };
void oracle(uint8_t* pixels, uint32_t pitch, void* context)
{
    const auto& o = *static_cast<Oracle*>(context);
    const TbSourceBuffer source = {o.sprite->Data, o.sprite->SWidth, o.sprite->SHeight, o.sprite->SWidth};
    LbSpriteDrawUsingScalingUpDataSolidLR(pixels + o.xs[0] + pitch * o.ys[0], pitch,
        o.height, const_cast<int32_t*>(o.xs), const_cast<int32_t*>(o.ys), &source);
}
struct Region { int sx, sy, dx, dy, w, h; };
bool clip(Region& r, int sw, int sh, int dw, int dh)
{
    if (r.sx < 0) { r.dx -= r.sx; r.w += r.sx; r.sx = 0; }
    if (r.sy < 0) { r.dy -= r.sy; r.h += r.sy; r.sy = 0; }
    if (r.dx < 0) { r.sx -= r.dx; r.w += r.dx; r.dx = 0; }
    if (r.dy < 0) { r.sy -= r.dy; r.h += r.dy; r.dy = 0; }
    r.w = std::min({r.w, sw - r.sx, dw - r.dx});
    r.h = std::min({r.h, sh - r.sy, dh - r.dy});
    return r.w > 0 && r.h > 0;
}
}

int kfx_wgpu_cursor_direct(const KfxGpolyTarget& target, const TbSprite* sprite,
    const int32_t* xs, const int32_t* ys)
{
    if (!kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_terrain_boundary(0);
    std::vector<uint8_t> asset;
    if (!sprite_asset(sprite, xs, ys, asset)) return 0;
    auto c = command(KFX_WGPU_DRAW_SPRITE, target.width, target.height);
    c.source_width = sprite->SWidth;
    c.source_height = sprite->SHeight;
    const KfxWgpuNativeResource source = {asset.data(), asset.size(), 1, 1, 1};
    Oracle o = {sprite, xs, ys, target.height};
    const int result = kfx_wgpu_native_draw(&target, &c, &source, nullptr, oracle, &o);
    totals.sprite_draws += result != 0;
    return result;
}

struct WgpuCursor::State {
    void* context;
    bool owned, failed = false;
    uint64_t sprite = 0, raster = 0, background = 0, backup = 0, screen = 0;
    uint32_t width = 0, height = 0, screen_width = 0, screen_height = 0;
    char error[1024] = {};
    KfxWgpuDrawCounters reported = {};
    KfxWgpuTargetResourceCounters reported_copies = {};
    explicit State(void* borrowed) : context(borrowed), owned(!borrowed) {}
    ~State() {
        collect();
        if (!context) return;
        for (auto id : {sprite, backup}) if (id)
            kfx_wgpu_draw_target_snapshot_release(context, id, error, sizeof(error));
        for (auto id : {raster, background, screen}) if (id)
            kfx_wgpu_draw_target_release(context, id, error, sizeof(error));
        if (owned) kfx_wgpu_draw_destroy(context);
    }
    bool good(bool ok) {
        if (!ok && !failed) { ++totals.failures; failed = true; }
        collect();
        return ok;
    }
    void collect() {
        if (!context || !owned) return;
        KfxWgpuDrawCounters c = {};
        if (kfx_wgpu_draw_counters(context, &c, error, sizeof(error)) == 1) {
#define COLLECT(field) totals.gpu.field += c.field - reported.field
            COLLECT(batches); COLLECT(commands); COLLECT(asset_upload_bytes);
            COLLECT(command_upload_bytes); COLLECT(readback_bytes);
#undef COLLECT
            reported = c;
        }
        KfxWgpuTargetResourceCounters s = {};
        if (kfx_wgpu_draw_target_resource_counters(context, &s, error, sizeof(error)) == 1) {
            totals.copies.snapshots += s.snapshots - reported_copies.snapshots;
            totals.copies.snapshot_copy_bytes += s.snapshot_copy_bytes - reported_copies.snapshot_copy_bytes;
            totals.copies.sampling_copy_bytes += s.sampling_copy_bytes - reported_copies.sampling_copy_bytes;
            reported_copies = s;
        }
    }
    bool upload(SDL_Surface* source) {
        if (failed || !source || source->format != SDL_PIXELFORMAT_INDEX8) return false;
        if (!screen || screen_width != unsigned(source->w) || screen_height != unsigned(source->h)) {
            if (screen) kfx_wgpu_draw_target_release(context, screen, error, sizeof(error));
            screen_width = source->w;
            screen_height = source->h;
            screen = kfx_wgpu_draw_target_create(context, screen_width, screen_height, error, sizeof(error));
            if (!screen) return good(false);
        }
        const uint64_t resource = kfx_wgpu_draw_resource_create(context,
            static_cast<uint8_t*>(source->pixels), size_t(source->pitch) * source->h,
            source->w, source->h, source->pitch, error, sizeof(error));
        if (!resource) return good(false);
        auto c = command(KFX_WGPU_DRAW_IMAGE, source->w, source->h);
        c.source = resource;
        c.source_width = source->w;
        c.source_height = source->h;
        const bool ok = kfx_wgpu_draw_submit(context, screen, &c, 1, error, sizeof(error)) == 1;
        kfx_wgpu_draw_resource_release(context, resource, error, sizeof(error));
        totals.bridge_initial_index_bytes += size_t(source->pitch) * source->h;
        return good(ok);
    }
    bool read(uint64_t target, SDL_Surface* surface, bool checkpoint) {
        std::vector<uint8_t> pixels(size_t(surface->pitch) * surface->h);
        std::memcpy(pixels.data(), surface->pixels, pixels.size());
        if (!good(kfx_wgpu_draw_readback(context, target, pixels.data(), pixels.size(),
                surface->pitch, error, sizeof(error)) == 1)) return false;
        std::memcpy(surface->pixels, pixels.data(), pixels.size());
        totals.native_commit_bytes += pixels.size();
        if (checkpoint) totals.checkpoint_index_bytes += pixels.size();
        return true;
    }
};

WgpuCursor::WgpuCursor(void* borrowed) : state(new State(borrowed)) {}
WgpuCursor::~WgpuCursor() { delete state; }

bool WgpuCursor::InitialiseTarget(uint32_t width, uint32_t height, const TbSprite* spr,
    const int32_t* xs, const int32_t* ys)
{
    auto& s = *state;
    if (s.failed || s.sprite || !width || !height) return false;
    std::vector<uint8_t> asset;
    if (!sprite_asset(spr, xs, ys, asset)) return false;
    if (!s.context) s.context = kfx_wgpu_draw_create(s.error, sizeof(s.error));
    if (!s.context) return s.good(false);
    s.width = width; s.height = height;
    s.background = kfx_wgpu_draw_target_create(s.context, width, height, s.error, sizeof(s.error));
    if (!s.background) return s.good(false);
    s.backup = kfx_wgpu_draw_target_snapshot(s.context, s.background, 0, 0,
        width, height, width, s.error, sizeof(s.error));
    if (!s.backup) return s.good(false);
    s.raster = kfx_wgpu_draw_target_create(s.context, width, height, s.error, sizeof(s.error));
    if (!s.raster) return s.good(false);
    uint64_t resource = kfx_wgpu_draw_resource_create(s.context, asset.data(), asset.size(), 1, 1, 1, s.error, sizeof(s.error));
    if (!resource) return s.good(false);
    auto clear = command(KFX_WGPU_DRAW_CLEAR, width, height);
    clear.colour = 255;
    auto draw = command(KFX_WGPU_DRAW_SPRITE, width, height);
    draw.source = resource;
    draw.source_width = spr->SWidth;
    draw.source_height = spr->SHeight;
    const KfxWgpuDrawCommand commands[] = {clear, draw};
    bool ok = kfx_wgpu_draw_submit(s.context, s.raster, commands, 2, s.error, sizeof(s.error)) == 1;
    kfx_wgpu_draw_resource_release(s.context, resource, s.error, sizeof(s.error));
    if (ok) s.sprite = kfx_wgpu_draw_target_snapshot(s.context, s.raster, 0, 0, width, height, width, s.error, sizeof(s.error));
    if (ok && s.sprite) ++totals.sprite_draws;
    return s.good(ok && s.sprite);
}

bool WgpuCursor::Initialise(SSurface& surface, const TbSprite* spr, const int32_t* xs, const int32_t* ys)
{
    if (!kfx_wgpu_native_enabled() || !surface.surf_data || surface.surf_data->format != SDL_PIXELFORMAT_INDEX8) return false;
    kfx_wgpu_terrain_boundary(0);
    SurfaceLock lock(surface.surf_data);
    if (!lock.locked) return false;
    return InitialiseTarget(surface.surf_data->pitch, surface.surf_data->h, spr, xs, ys) &&
        state->read(state->raster, surface.surf_data, true);
}

bool WgpuCursor::BackupTarget(uint64_t target, uint32_t width, uint32_t height,
    int x, int y, const TbRect& rect)
{
    auto& s = *state;
    if (s.failed || !s.sprite) return false;
    Region r = {x, y, int(rect.left), int(rect.top), int(rect.right - rect.left), int(rect.bottom - rect.top)};
    if (!clip(r, width, height, s.width, s.height)) return true;
    const uint64_t part = kfx_wgpu_draw_target_snapshot(s.context, target, r.sx, r.sy, r.w, r.h, r.w, s.error, sizeof(s.error));
    if (!part) return s.good(false);
    auto c = command(KFX_WGPU_DRAW_IMAGE, r.w, r.h);
    c.x = r.dx; c.y = r.dy;
    c.clip_width = s.width; c.clip_height = s.height;
    c.source = part; c.source_width = r.w; c.source_height = r.h;
    bool ok = kfx_wgpu_draw_submit_target_images(s.context, s.background, &c, 1, s.error, sizeof(s.error)) == 1;
    kfx_wgpu_draw_target_snapshot_release(s.context, part, s.error, sizeof(s.error));
    uint64_t next = ok ? kfx_wgpu_draw_target_snapshot(s.context, s.background, 0, 0,
        s.width, s.height, s.width, s.error, sizeof(s.error)) : 0;
    if (!next) return s.good(false);
    if (s.backup) kfx_wgpu_draw_target_snapshot_release(s.context, s.backup, s.error, sizeof(s.error));
    s.backup = next;
    ++totals.backups;
    return s.good(true);
}

bool WgpuCursor::ComposeTarget(uint64_t target, uint32_t width, uint32_t height,
    int x, int y, const TbRect& rect, bool restore)
{
    auto& s = *state;
    if (s.failed || !s.sprite) return false;
    Region r = {int(rect.left), int(rect.top), x, y, int(rect.right - rect.left), int(rect.bottom - rect.top)};
    if (!clip(r, s.width, s.height, width, height)) return true;
    if (restore && !s.backup) return false;
    auto c = command(KFX_WGPU_DRAW_IMAGE, r.w, r.h);
    c.x = r.dx; c.y = r.dy;
    c.clip_width = width; c.clip_height = height;
    c.source = restore ? s.backup : s.sprite;
    c.source_x = r.sx; c.source_y = r.sy;
    c.source_width = r.w; c.source_height = r.h;
    c.transparent = restore ? KFX_WGPU_DRAW_OPAQUE : 255;
    const bool ok = kfx_wgpu_draw_submit_target_images(s.context, target, &c, 1, s.error, sizeof(s.error)) == 1;
    if (ok) ++totals.compositions;
    return s.good(ok);
}

bool WgpuCursor::Backup(SSurface& checkpoint, int x, int y, const TbRect& rect)
{
    if (!checkpoint.surf_data || !lbDrawSurface || state->failed) return false;
    kfx_wgpu_terrain_boundary(0);
    SurfaceLock screen(lbDrawSurface), backup(checkpoint.surf_data);
    if (!screen.locked || !backup.locked) return false;
    return state->upload(lbDrawSurface) && BackupTarget(state->screen, lbDrawSurface->w,
        lbDrawSurface->h, x, y, rect) && state->read(state->background, checkpoint.surf_data, true);
}

bool WgpuCursor::Compose(int x, int y, const TbRect& rect, bool restore)
{
    if (!lbDrawSurface || state->failed) return false;
    kfx_wgpu_terrain_boundary(0);
    SurfaceLock screen(lbDrawSurface);
    if (!screen.locked) return false;
    return state->upload(lbDrawSurface) && ComposeTarget(state->screen, lbDrawSurface->w,
        lbDrawSurface->h, x, y, rect, restore) && state->read(state->screen, lbDrawSurface, false);
}
#else
struct WgpuCursor::State {};
WgpuCursor::WgpuCursor(void*) : state(nullptr) {}
WgpuCursor::~WgpuCursor() { delete state; }
int kfx_wgpu_cursor_direct(const KfxGpolyTarget&, const TbSprite*, const int32_t*, const int32_t*) { return 0; }
bool WgpuCursor::Initialise(SSurface&, const TbSprite*, const int32_t*, const int32_t*) { return false; }
bool WgpuCursor::Backup(SSurface&, int, int, const TbRect&) { return false; }
bool WgpuCursor::Compose(int, int, const TbRect&, bool) { return false; }
bool WgpuCursor::InitialiseTarget(uint32_t, uint32_t, const TbSprite*, const int32_t*, const int32_t*) { return false; }
bool WgpuCursor::BackupTarget(uint64_t, uint32_t, uint32_t, int, int, const TbRect&) { return false; }
bool WgpuCursor::ComposeTarget(uint64_t, uint32_t, uint32_t, int, int, const TbRect&, bool) { return false; }
#endif
