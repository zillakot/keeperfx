#include "kfx/renderer/WgpuCursor.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "bflib_sprite.h"
#include "bflib_vidraw.h"
#include "bflib_vidsurface.h"
#include "bflib_planar.h"
#include "bflib_mspointer.hpp"
#include "kfx/renderer/RendererManager.h"
#include <SDL3/SDL.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <memory>

extern volatile TbBool lbPointerAdvancedDraw;
volatile TbBool lbInteruptMouse;
static int mouse_scale = 1;
extern "C" long scale_ui_value_lofi(long value) { return value * mouse_scale; }
extern "C" TbResult RendererLockFramebuffer() { return Lb_SUCCESS; }
extern "C" TbResult RendererUnlockFramebuffer() { return Lb_SUCCESS; }
extern "C" void LbSetRect(TbRect* r, long x, long y, long w, long h) { *r = {x,y,w,h}; }
extern "C" GameTurn get_gameturn() { return 0; }
extern "C" int LbErrorLog(const char*, ...) { return 0; }
extern "C" int LbSyncLog(const char*, ...) { return 0; }
static void* drawing;
static char error[1024];
static bool enabled = true, shared = false, delayed_context = false, context_ready = true;
static uint64_t shared_target;
static unsigned direct_cases;
extern "C" void cursor_native(uint8_t*, int, int, const TbSprite*, int32_t*, int32_t*);
static void check(bool result, const char* message)
{
    if (!result) { std::fprintf(stderr, "%s: %s\n", message, error); std::exit(1); }
}
extern "C" int kfx_wgpu_native_enabled() { return enabled; }
extern "C" int kfx_wgpu_native_read_barrier(const void* bytes, size_t length) { (void)bytes; (void)length; return 1; }
extern "C" void kfx_wgpu_native_flush(void) { kfx_wgpu_terrain_boundary(0); }
extern "C" int kfx_wgpu_native_cpu_barrier(void) { return 1; }
extern "C" void kfx_wgpu_native_context_cleanup(void (*)(void*)) {}
extern "C" void* kfx_wgpu_native_context(void) { return shared && enabled && context_ready ? drawing : nullptr; }
extern "C" uint64_t kfx_wgpu_native_target(const KfxGpolyTarget*) { return shared_target; }
extern "C" void kfx_wgpu_native_invalidate_frame(void) { check(false, "unexpected invalidation"); }
extern "C" void kfx_wgpu_terrain_boundary(int allow) { check(!allow, "unexpected terrain enabled"); }
static uint64_t upload(const KfxGpolyTarget& t)
{
    uint64_t target = kfx_wgpu_draw_target_create(drawing, t.width, t.height, error, sizeof(error));
    check(target, "target");
    uint64_t asset = kfx_wgpu_draw_resource_create(drawing, t.pixels, size_t(t.pitch) * t.height,
        t.width, t.height, t.pitch, error, sizeof(error));
    check(asset, "upload resource");
    KfxWgpuDrawCommand c = {};
    c.abi_version = 1; c.kind = KFX_WGPU_DRAW_IMAGE;
    c.width = c.clip_width = c.source_width = t.width;
    c.height = c.clip_height = c.source_height = t.height;
    c.source = asset; c.transparent = 256;
    check(kfx_wgpu_draw_submit(drawing, target, &c, 1, error, sizeof(error)) == 1, "upload");
    kfx_wgpu_draw_resource_release(drawing, asset, error, sizeof(error));
    return target;
}
extern "C" int kfx_wgpu_native_draw(const KfxGpolyTarget* t, const KfxWgpuDrawCommand* command,
    const KfxWgpuNativeResource* resource, const KfxWgpuNativeResource*, KfxWgpuNativeOracle oracle, void* context)
{
    uint64_t target = shared_target ? shared_target : upload(*t);
    uint64_t asset = kfx_wgpu_draw_resource_create(drawing, resource->bytes, resource->length,
        1, 1, 1, error, sizeof(error));
    check(asset, "sprite resource");
    auto c = *command; c.source = asset;
    check(kfx_wgpu_draw_submit(drawing, target, &c, 1, error, sizeof(error)) == 1, "direct draw");
    kfx_wgpu_draw_resource_release(drawing, asset, error, sizeof(error));
    std::vector<uint8_t> expected(t->pixels, t->pixels + size_t(t->pitch) * t->height);
    oracle(expected.data(), t->pitch, context);
    check(kfx_wgpu_draw_readback(drawing, target, t->pixels, expected.size(), t->pitch, error, sizeof(error)) == 1, "direct read");
    check(!std::memcmp(t->pixels, expected.data(), expected.size()), "direct CPU oracle differs");
    if (!shared_target) kfx_wgpu_draw_target_release(drawing, target, error, sizeof(error));
    ++direct_cases;
    return 1;
}
static void steps(int32_t* xs, int32_t* ys, int x, int y, int scale, int w, int h)
{
    LbSpriteSetScalingWidthClippedArray(xs, x, 7, 7 * scale, w);
    LbSpriteSetScalingHeightClippedArray(ys, y, 5, 5 * scale, h);
}
static void paint(SDL_Surface* surface, unsigned seed)
{
    auto* pixels = static_cast<uint8_t*>(surface->pixels);
    for (int i = 0; i < surface->pitch * surface->h; ++i) pixels[i] = (i * 19 + i / surface->pitch * 13 + seed) & 255;
}
static std::vector<uint8_t> bytes(SDL_Surface* s)
{
    auto* p = static_cast<uint8_t*>(s->pixels);
    if (shared_target && s == lbDrawSurface)
        check(kfx_wgpu_draw_readback(drawing, shared_target, p, size_t(s->pitch) * s->h,
            s->pitch, error, sizeof(error)) == 1, "diagnostic shared frame read");
    return {p, p + size_t(s->pitch) * s->h};
}
static void same(SDL_Surface* surface, const std::vector<uint8_t>& expected, const char* message)
{
    check(bytes(surface) == expected, message);
}
static void blit(SDL_Surface* source, SDL_Surface* dest, int sx, int sy, int x, int y, int w, int h, bool keyed)
{
    SDL_SetSurfaceColorKey(source, keyed, 255);
    SDL_Rect sr = {sx, sy, w, h}, dr = {x, y, w, h};
    check(SDL_BlitSurface(source, &sr, dest, &dr), "SDL oracle blit");
}
static std::vector<std::vector<uint8_t>> lifecycle(bool gpu, bool advanced, int scale,
    const TbSprite* sprite, SDL_Palette* palette, bool offscreen)
{
    enabled = gpu;
    context_ready = !delayed_context;
    mouse_scale = scale;
    lbPointerAdvancedDraw = advanced;
    lbInteruptMouse = true;
    auto* screen = SDL_CreateSurface(37, 31, SDL_PIXELFORMAT_INDEX8);
    SDL_SetSurfacePalette(screen, palette);
    lbDrawSurface = screen;
    lbDisplay.WScreen = static_cast<uint8_t*>(screen->pixels);
    lbDisplay.GraphicsScreenWidth = screen->pitch;
    lbDisplay.MouseWindowWidth = lbDisplay.PhysicalScreenWidth = screen->w;
    lbDisplay.MouseWindowHeight = lbDisplay.PhysicalScreenHeight = screen->h;
    paint(screen, 0);
    if (shared && gpu) {
        const KfxGpolyTarget t = {static_cast<uint8_t*>(screen->pixels),
            uint32_t(screen->w), uint32_t(screen->h), uint32_t(screen->pitch)};
        shared_target = upload(t);
        check(kfx_wgpu_draw_frame_begin(drawing, shared_target, error, sizeof(error)) == 1, "shared frame begin");
    }
    TbPoint position = offscreen ? TbPoint{80,80} : TbPoint{3,4};
    TbPoint offset = {1,2};
    LbI_PointerHandler pointer;
    pointer.Initialise(sprite, &position, &offset);
    context_ready = true;
    const auto initialized = kfx_wgpu_cursor_counters();
    std::vector<std::vector<uint8_t>> frames;
    const int positions[][2] = {{0,0},{19,13},{36,30},{3,2},{-20,5},{80,80},{6,8}};
    for (const auto& p : positions) {
        position = {p[0],p[1]};
        pointer.OnMove(); frames.push_back(bytes(screen));
        pointer.OnBeginSwap(); frames.push_back(bytes(screen));
        pointer.OnEndSwap(); frames.push_back(bytes(screen));
        pointer.SetHotspot(20,-4); frames.push_back(bytes(screen));
        pointer.SetHotspot(1,2); frames.push_back(bytes(screen));
    }
    lbInteruptMouse = false;
    position = {15,16}; pointer.OnMove(); frames.push_back(bytes(screen));
    pointer.OnBeginSwap(); frames.push_back(bytes(screen));
    pointer.OnEndSwap(); frames.push_back(bytes(screen));
    lbInteruptMouse = true;
    pointer.Release(); frames.push_back(bytes(screen));
    pointer.Release(); frames.push_back(bytes(screen));
    if (shared_target) {
        check(kfx_wgpu_cursor_counters().bridge_initial_index_bytes == initialized.bridge_initial_index_bytes,
            "late cursor borrowing uploaded native frame");
        check(kfx_wgpu_draw_frame_end(drawing, error, sizeof(error)) == 1, "shared frame end");
        kfx_wgpu_draw_target_release(drawing, shared_target, error, sizeof(error));
        shared_target = 0;
    }
    SDL_DestroySurface(screen);
    return frames;
}
static void refill(SDL_Surface* surface, unsigned seed)
{
    paint(surface, seed);
    if (!shared_target) return;
    uint64_t asset = kfx_wgpu_draw_resource_create(drawing, static_cast<uint8_t*>(surface->pixels),
        size_t(surface->pitch) * surface->h, surface->w, surface->h, surface->pitch, error, sizeof(error));
    check(asset, "tail repaint resource");
    KfxWgpuDrawCommand c = {};
    c.abi_version = 1; c.kind = KFX_WGPU_DRAW_IMAGE;
    c.width = c.clip_width = c.source_width = surface->w;
    c.height = c.clip_height = c.source_height = surface->h;
    c.source = asset; c.transparent = 256;
    check(kfx_wgpu_draw_submit(drawing, shared_target, &c, 1, error, sizeof(error)) == 1, "tail repaint");
    kfx_wgpu_draw_resource_release(drawing, asset, error, sizeof(error));
}
static SDL_Surface* adopt(SDL_Surface* surface, SDL_Palette* palette)
{
    SDL_SetSurfacePalette(surface, palette);
    lbDrawSurface = surface;
    lbDisplay.WScreen = static_cast<uint8_t*>(surface->pixels);
    lbDisplay.GraphicsScreenWidth = surface->pitch;
    lbDisplay.MouseWindowWidth = lbDisplay.PhysicalScreenWidth = surface->w;
    lbDisplay.MouseWindowHeight = lbDisplay.PhysicalScreenHeight = surface->h;
    if (!enabled) return surface;
    const KfxGpolyTarget t = {static_cast<uint8_t*>(surface->pixels),
        uint32_t(surface->w), uint32_t(surface->h), uint32_t(surface->pitch)};
    shared_target = upload(t);
    check(kfx_wgpu_draw_frame_begin(drawing, shared_target, error, sizeof(error)) == 1, "tail frame begin");
    return surface;
}
static void retire()
{
    if (!shared_target) return;
    check(kfx_wgpu_draw_frame_end(drawing, error, sizeof(error)) == 1, "tail frame end");
    kfx_wgpu_draw_target_release(drawing, shared_target, error, sizeof(error));
    shared_target = 0;
}
// A presented frame flushes once and the swap records behind it, so the backup,
// composition and restore add no queued-frame checkpoint of their own.
static std::vector<std::vector<uint8_t>> tail_trace(bool gpu, int scale, bool interrupted,
    bool hidden, bool resize, const TbSprite* sprite, SDL_Palette* palette)
{
    enabled = gpu;
    context_ready = true;
    mouse_scale = scale;
    lbPointerAdvancedDraw = true;
    lbInteruptMouse = interrupted;
    auto* screen = adopt(SDL_CreateSurface(37, 31, SDL_PIXELFORMAT_INDEX8), palette);
    paint(screen, 0);
    TbPoint position = {5, 6}, offset = {1, 2};
    LbI_PointerHandler pointer;
    pointer.Initialise(sprite, &position, &offset);
    std::vector<std::vector<uint8_t>> frames;
    const int moves[][2] = {{5,6},{18,12},{2,25},{34,3},{-3,9},{5,6}};
    unsigned seed = 11;
    for (unsigned i = 0; i < 6; ++i) {
        if (resize && i == 3) {
            retire();
            SDL_DestroySurface(screen);
            screen = adopt(SDL_CreateSurface(43, 35, SDL_PIXELFORMAT_INDEX8), palette);
            paint(screen, 7);
        }
        position = {moves[i][0], moves[i][1]};
        if (interrupted) pointer.OnMove();
        refill(screen, seed++);
        KfxWgpuFrameCounters before = {}, after = {};
        if (shared_target) {
            check(kfx_wgpu_draw_frame_flush(drawing, error, sizeof(error)) == 1, "tail frame flush");
            check(kfx_wgpu_draw_frame_counters(drawing, &before, error, sizeof(error)) == 1, "tail counters");
        }
        if (!hidden) { pointer.OnBeginSwap(); pointer.OnEndSwap(); }
        if (shared_target) {
            check(kfx_wgpu_draw_frame_counters(drawing, &after, error, sizeof(error)) == 1, "tail counters");
            check(after.checkpoints == before.checkpoints, "swap tail forced a queued frame checkpoint");
        }
        frames.push_back(bytes(screen));
    }
    pointer.Release();
    frames.push_back(bytes(screen));
    retire();
    SDL_DestroySurface(screen);
    return frames;
}
int main()
{
    drawing = kfx_wgpu_draw_create(error, sizeof(error));
    check(drawing, "Metal/Vulkan drawing context");
    uint8_t data[] = {2,0,255,254,3,17,93,0,0, 255,4,127,0,4,8,254,0,
        7,43,29,17,0,251,180,3,0, 253,1,222,253,0, 1,0,251,1,255,0};
    TbSprite sprite = {data, 7, 5};
    const int positions[][2] = {{0,0},{3,5},{-4,-3},{30,21},{-40,4},{4,-40},{45,40},{-6,25},{33,-4}};
    int32_t xs[16] = {}, ys[12] = {};
    for (int scale = 1; scale <= 3; ++scale) for (auto& p : positions) for (unsigned alignment = 0; alignment < 4; ++alignment) {
        std::vector<uint8_t> guarded(41 * 31 + 8, 203);
        auto* pixels = guarded.data() + 4 + alignment;
        for (size_t i = 0; i < 41 * 31; ++i) pixels[i] = i * 19;
        steps(xs, ys, p[0], p[1], scale, 37, 31);
        KfxGpolyTarget target = {pixels, 37, 31, 41};
        check(kfx_wgpu_cursor_direct(target, &sprite, xs, ys), "direct cursor declined");
        for (unsigned i = 0; i < 4 + alignment; ++i) check(guarded[i] == 203, "direct prefix guard");
        for (size_t i = 4 + alignment + 41 * 31; i < guarded.size(); ++i) check(guarded[i] == 203, "direct suffix guard");
    }
    SDL_Palette* palette = SDL_CreatePalette(256);
    SDL_Color colours[256];
    for (int i = 0; i < 256; ++i) colours[i] = {uint8_t(i), uint8_t(i), uint8_t(i), 255};
    SDL_SetPaletteColors(palette, colours, 0, 256);
    for (int scale = 1; scale <= 3; ++scale) for (bool advanced : {false,true}) for (bool offscreen : {false,true}) {
        auto native = lifecycle(false, advanced, scale, &sprite, palette, offscreen);
        const auto cpu_before = kfx_wgpu_cursor_counters();
        auto gpu = lifecycle(true, advanced, scale, &sprite, palette, offscreen);
        const auto cpu_after = kfx_wgpu_cursor_counters();
        check(cpu_before.cpu_sprite_draws == cpu_after.cpu_sprite_draws &&
            cpu_before.cpu_backups == cpu_after.cpu_backups &&
            cpu_before.cpu_compositions == cpu_after.cpu_compositions,
            "GPU lifecycle used software cursor drawing");
        if (native != gpu) {
            for (size_t i = 0; i < native.size(); ++i) if (native[i] != gpu[i]) {
                for (size_t j = 0; j < native[i].size(); ++j) if (native[i][j] != gpu[i][j]) {
                    std::fprintf(stderr, "lifecycle scale=%d advanced=%d frame=%zu index=%zu CPU=%u GPU=%u\n", scale, advanced, i, j, native[i][j], gpu[i][j]); break;
                }
                break;
            }
        }
        check(native == gpu, "actual pointer lifecycle differs");
    }
    shared = true;
    for (int scale = 1; scale <= 3; ++scale) for (bool advanced : {false,true}) for (bool offscreen : {false,true}) {
        auto native = lifecycle(false, advanced, scale, &sprite, palette, offscreen);
        const auto before_shared = kfx_wgpu_cursor_counters();
        auto gpu = lifecycle(true, advanced, scale, &sprite, palette, offscreen);
        const auto after_shared = kfx_wgpu_cursor_counters();
        check(native == gpu, "shared frame actual pointer lifecycle differs");
        check(before_shared.bridge_initial_index_bytes == after_shared.bridge_initial_index_bytes &&
            before_shared.cpu_sprite_draws == after_shared.cpu_sprite_draws &&
            before_shared.cpu_backups == after_shared.cpu_backups &&
            before_shared.cpu_compositions == after_shared.cpu_compositions, "shared frame cursor used CPU frame transfer or fallback");
    }
    for (int scale = 1; scale <= 3; ++scale)
        for (bool interrupted : {false,true}) for (bool hidden : {false,true}) for (bool resize : {false,true}) {
            auto reference = tail_trace(false, scale, interrupted, hidden, resize, &sprite, palette);
            auto tail = tail_trace(true, scale, interrupted, hidden, resize, &sprite, palette);
            check(reference == tail, "swap tail pixels differ from the software cursor");
        }
    delayed_context = true;
    for (int scale = 1; scale <= 3; ++scale) for (bool offscreen : {false,true}) {
        auto native = lifecycle(false, true, scale, &sprite, palette, offscreen);
        auto gpu = lifecycle(true, true, scale, &sprite, palette, offscreen);
        check(native == gpu, "late context pointer promotion differs");
    }
    delayed_context = false;
    shared = false;
    for (int scale = 1; scale <= 3; ++scale) {
        auto* screen = SDL_CreateSurface(37, 31, SDL_PIXELFORMAT_INDEX8);
        auto* expected = SDL_CreateSurface(37, 31, SDL_PIXELFORMAT_INDEX8);
        auto* cursor = SDL_CreateSurface(8 * scale, 6 * scale, SDL_PIXELFORMAT_INDEX8);
        auto* background = SDL_CreateSurface(8 * scale, 6 * scale, SDL_PIXELFORMAT_INDEX8);
        for (auto* s : {screen, expected, cursor, background}) check(SDL_SetSurfacePalette(s, palette), "palette");
        lbDrawSurface = screen;
        SSurface front = {cursor, 0, cursor->pitch}, back = {background, 0, background->pitch};
        steps(xs, ys, 0, 0, scale, 37, 31);
        std::vector<uint8_t> native(size_t(cursor->pitch) * cursor->h, 255);
        cursor_native(native.data(), cursor->pitch, 31, &sprite, xs, ys);
        auto gpu = std::make_unique<WgpuCursor>();
        check(gpu->Initialise(front, &sprite, xs, ys), "cursor initialize");
        same(cursor, native, "cursor GPU raster differs from native");
        for (unsigned i = 0; i < 27; ++i) {
            auto& p = positions[i % 9];
            paint(screen, i); std::memcpy(expected->pixels, screen->pixels, size_t(screen->pitch) * screen->h);
            const auto initial = bytes(screen);
            TbRect rect = {0, 0, 7 * scale, 5 * scale};
            int x = p[0], y = p[1];
            if (x < 0) { rect.left -= x; x = 0; }
            else if (x + rect.right > 37) rect.right = 37 - x;
            if (y < 0) { rect.top -= y; y = 0; }
            else if (y + rect.bottom > 31) rect.bottom = 31 - y;
            check(gpu->Backup(back, x, y, rect), "backup");
            same(screen, initial, "backup changed screen");
            check(gpu->Compose(x, y, rect, false), "compose");
            blit(cursor, expected, rect.left, rect.top, x, y, rect.right - rect.left, rect.bottom - rect.top, true);
            same(screen, bytes(expected), "SDL cursor composition differs");
            check(gpu->Compose(x, y, rect, false), "repeat draw");
            same(screen, bytes(expected), "repeat changed composition");
            check(gpu->Compose(x, y, rect, true), "restore");
            same(screen, initial, "old background restoration differs");
            check(gpu->Compose(x, y, rect, true), "repeat restore");
            same(screen, initial, "repeat restoration differs");
        }
        lbDrawSurface = nullptr;
        check(!gpu->Compose(0, 0, TbRect{0,0,7 * scale,5 * scale}, false), "absent surface accepted");
        auto* resized = SDL_CreateSurface(43, 35, SDL_PIXELFORMAT_INDEX8);
        SDL_SetSurfacePalette(resized, palette);
        lbDrawSurface = resized;
        paint(resized, 143);
        auto resize_initial = bytes(resized);
        TbRect resize_rect = {0,0,7 * scale,5 * scale};
        check(gpu->Backup(back,22,20,resize_rect) && gpu->Compose(22,20,resize_rect,false) &&
            gpu->Compose(22,20,resize_rect,true), "resized surface lifecycle");
        same(resized, resize_initial, "resized background differs");
        SDL_DestroySurface(resized);
        lbDrawSurface = screen;
        paint(screen, 91);
        const auto initial = bytes(screen);
        TbRect rect = {0, 0, 7 * scale, 5 * scale};
        check(gpu->Backup(back, 3, 4, rect) && gpu->Compose(3, 4, rect, false), "failure setup");
        check(!gpu->ComposeTarget(UINT64_MAX, 37, 31, 3, 4, rect, true), "invalid target accepted");
        check(!gpu->Compose(3, 4, rect, true), "failed cursor accepted");
        blit(background, screen, 0, 0, 3, 4, rect.right, rect.bottom, false);
        same(screen, initial, "software checkpoint recovery differs");
        gpu.reset();
        for (auto* s : {screen, expected, cursor, background}) SDL_DestroySurface(s);
    }
    // A borrowed target queues draw/copy/restore without a native checkpoint or upload.
    const auto before = kfx_wgpu_cursor_counters();
    {
        std::vector<uint8_t> pixels(37 * 31, 71);
        KfxGpolyTarget t = {pixels.data(), 37, 31, 37};
        uint64_t target = upload(t);
        WgpuCursor gpu(drawing);
        steps(xs, ys, 0, 0, 2, 37, 31);
        check(gpu.InitialiseTarget(16, 12, &sprite, xs, ys), "borrowed cursor");
        const auto artwork = std::vector<uint8_t>(data, data + sizeof(data));
        std::memset(data, 199, sizeof(data));
        TbRect rect = {0,0,14,10};
        check(gpu.BackupTarget(target,37,31,4,5,rect), "borrowed backup");
        check(gpu.ComposeTarget(target,37,31,4,5,rect,false), "borrowed draw");
        check(kfx_wgpu_draw_readback(drawing,target,pixels.data(),pixels.size(),37,error,sizeof(error)) == 1, "borrowed drawn read");
        check(pixels[(5 + ys[0]) * 37 + 4 + xs[0]] == 0 &&
            pixels[(5 + ys[0]) * 37 + 4 + xs[2]] == 71,
            "immutable cursor artwork or index255 transparency differs");
        check(gpu.ComposeTarget(target,37,31,4,5,rect,true), "borrowed restore");
        check(kfx_wgpu_draw_readback(drawing,target,pixels.data(),pixels.size(),37,error,sizeof(error)) == 1, "borrowed read");
        for (auto p : pixels) check(p == 71, "borrowed restoration");
        kfx_wgpu_cursor_detach_context(drawing);
        check(!gpu.ComposeTarget(target,37,31,4,5,rect,false), "detached cursor reused snapshots");
        kfx_wgpu_draw_target_release(drawing,target,error,sizeof(error));
        std::memcpy(data, artwork.data(), sizeof(data));
    }
    const auto after = kfx_wgpu_cursor_counters();
    check(before.bridge_initial_index_bytes == after.bridge_initial_index_bytes &&
        before.checkpoint_index_bytes == after.checkpoint_index_bytes, "borrowed target used native bridge");
    check(after.copies.snapshots && after.copies.snapshot_copy_bytes && after.copies.sampling_copy_bytes &&
        after.gpu.readback_bytes && after.failures == 3, "missing counters");
    SDL_DestroyPalette(palette);
    kfx_wgpu_draw_destroy(drawing);
    std::printf("%u actual native direct cursor cases; 81 advanced compositions/restores; 30 pointer lifecycle traces including 18 shared frames; 24 checkpoint-free swap tails; failure checkpoints and borrowed targets exact\n", direct_cases);
}
