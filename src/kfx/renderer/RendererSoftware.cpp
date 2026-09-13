#include "pre_inc.h"
#include "kfx/renderer/RendererSoftware.h"
#include "kfx/renderer/FrameCapture.h"
#include "performance_capture.h"
#include "bflib_video.h"       // PALETTE_COLORS, lbWindow, SDL, vsync_enabled
#include "bflib_vidsurface.h"  // lbDrawSurface (goes away when the framebuffer migrates)
#include "bflib_mouse.h"       // LbMouseOnBeginSwap/EndSwap (software cursor around present)
#include <SDL3_image/SDL_image.h> // IMG_SavePNG (screenshots)
#ifdef KFX_RUST_PRESENTER
#include "kfx/renderer/RustPresenter.h"
#include <SDL3/SDL_metal.h>
#endif
#include "post_inc.h"

bool RendererSoftware::Init()
{
    const char* backend = SDL_getenv("KFX_PRESENT_BACKEND");
    if (backend != nullptr && strcmp(backend, "sdl") != 0 && strcmp(backend, "wgpu") != 0)
        WARNLOG("Unknown KFX_PRESENT_BACKEND '%s'; using SDL", backend);
#ifndef KFX_RUST_PRESENTER
    if (backend != nullptr && strcmp(backend, "wgpu") == 0)
        WARNLOG("Rust presentation is not built on this platform; using SDL");
#endif
    return true;
}

void RendererSoftware::Shutdown()
{
    destroy_present_target();
}

void RendererSoftware::SetDisplayPalette(const unsigned char* rgb8)
{
    if (lbDrawSurface == NULL)
        return;
    SDL_Color colors[PALETTE_COLORS];
    for (int i = 0; i < PALETTE_COLORS; i++)
    {
        colors[i].r = rgb8[3 * i + 0];
        colors[i].g = rgb8[3 * i + 1];
        colors[i].b = rgb8[3 * i + 2];
        colors[i].a = SDL_ALPHA_OPAQUE;
    }
    SDL_Palette* surfpal = SDL_GetSurfacePalette(lbDrawSurface);
    if (surfpal != NULL)
        SDL_SetPaletteColors(surfpal, colors, 0, PALETTE_COLORS);
}

void RendererSoftware::ClearScreen(unsigned char colour)
{
    if (lbDrawSurface == NULL)
        return;
    if (!SDL_FillSurfaceRect(lbDrawSurface, NULL, colour))
        ERRORLOG("Error while clearing screen: %s", SDL_GetError());
}

bool RendererSoftware::ensure_present_target()
{
#ifdef KFX_RUST_PRESENTER
    if (m_rust_window != nullptr && m_rust_window != lbWindow)
        destroy_rust_presenter();
    if (m_rust != nullptr || try_rust_presenter())
        return true;
#endif
    if (m_renderer != nullptr && SDL_GetRenderWindow(m_renderer) != lbWindow)
        destroy_present_target();
    if (m_renderer == nullptr)
    {
        m_renderer = SDL_CreateRenderer(lbWindow, NULL);
        if (m_renderer == nullptr)
        {
            ERRORLOG("SDL_CreateRenderer failed: %s", SDL_GetError());
            return false;
        }
        // Name the backend SDL picked for us, so a bug report tells which graphics path the
        // game was presenting through, and which driver libraries that pulls into the process.
        const char * backend = SDL_GetRendererName(m_renderer);
        SYNCLOG("Presenting through SDL renderer: %s", (backend != nullptr) ? backend : "unknown");
    }

    const int want_vsync = vsync_enabled ? 1 : 0;
    if (m_vsync != want_vsync)
    {
        SDL_SetRenderVSync(m_renderer, want_vsync);
        m_vsync = want_vsync;
    }

    if (m_texture == nullptr || m_tex_w != lbDrawSurface->w || m_tex_h != lbDrawSurface->h)
    {
        if (m_texture != nullptr) { SDL_DestroyTexture(m_texture); m_texture = nullptr; }
        m_texture = SDL_CreateTexture(m_renderer, SDL_PIXELFORMAT_RGBA32,
                                      SDL_TEXTUREACCESS_STREAMING, lbDrawSurface->w, lbDrawSurface->h);
        if (m_texture == nullptr)
        {
            ERRORLOG("SDL_CreateTexture failed: %s", SDL_GetError());
            return false;
        }
        SDL_SetTextureScaleMode(m_texture, SDL_SCALEMODE_NEAREST); // crisp pixels
        m_tex_w = lbDrawSurface->w;
        m_tex_h = lbDrawSurface->h;
    }
    return true;
}

void RendererSoftware::destroy_present_target()
{
#ifdef KFX_RUST_PRESENTER
    destroy_rust_presenter();
#endif
    if (m_texture != nullptr) { SDL_DestroyTexture(m_texture); m_texture = nullptr; }
    if (m_renderer != nullptr) { SDL_DestroyRenderer(m_renderer); m_renderer = nullptr; }
    m_tex_w = 0;
    m_tex_h = 0;
    m_vsync = -1;
}

unsigned char* RendererSoftware::LockFramebuffer(int* out_pitch)
{
    if (lbDrawSurface == NULL || !SDL_LockSurface(lbDrawSurface))
        return nullptr;
    if (out_pitch != nullptr)
        *out_pitch = lbDrawSurface->pitch;
    return static_cast<unsigned char*>(lbDrawSurface->pixels);
}

void RendererSoftware::UnlockFramebuffer()
{
    if (lbDrawSurface != NULL)
        SDL_UnlockSurface(lbDrawSurface);
}

bool RendererSoftware::ScheduleScreenshot(const char* path, int fmt)
{
    if (lbDrawSurface == NULL)
        return false;
    bool ok;
    switch (fmt)
    {
        case 1:  ok = IMG_SavePNG(lbDrawSurface, path); break;
        case 2:  ok = SDL_SaveBMP(lbDrawSurface, path); break;
        default: return false;
    }
    if (!ok)
        ERRORLOG("Screenshot save failed (%s): %s", path, SDL_GetError());
    return ok;
}

void RendererSoftware::PresentFrame()
{
    if (lbDrawSurface == NULL || !ensure_present_target()) {
        performance_failed("presentation target unavailable");
        return;
    }
#ifdef KFX_RUST_PRESENTER
    if (m_rust != nullptr) {
        if (present_rust_frame()) return;
        if (!ensure_present_target()) return;
    }
#endif
    if (performance_active()) {
        int output_width = 0, output_height = 0, actual_vsync = -2;
        SDL_GetRenderOutputSize(m_renderer, &output_width, &output_height);
        SDL_GetRenderVSync(m_renderer, &actual_vsync);
        performance_renderer_info(SDL_GetRendererName(m_renderer), SDL_GetCurrentVideoDriver(),
            lbDrawSurface->w, lbDrawSurface->h, output_width, output_height, actual_vsync);
    }
    performance_begin(PerfPresentation);
    SDL_Surface* texture_surface;
    if (!SDL_LockTextureToSurface(m_texture, NULL, &texture_surface))
    {
        ERRORLOG("Present texture lock failed: %s", SDL_GetError());
        performance_failed("texture lock failed");
        return;
    }
    LbMouseOnBeginSwap();
    // INDEX8 (palette) -> RGBA and present
    if (!SDL_BlitSurface(lbDrawSurface, NULL, texture_surface, NULL))
    {
        ERRORLOG("Present blit failed: %s", SDL_GetError());
        performance_failed("palette blit failed");
        SDL_UnlockTexture(m_texture);
        LbMouseOnEndSwap();
        return;
    }
    CaptureFrameIfRequested(lbDrawSurface, texture_surface);
    SDL_UnlockTexture(m_texture);
    if (!SDL_RenderClear(m_renderer)) performance_failed("SDL_RenderClear failed");
    if (!SDL_RenderTexture(m_renderer, m_texture, NULL, NULL)) performance_failed("SDL_RenderTexture failed");
    performance_begin(PerfPresentWait);
    const bool presented = SDL_RenderPresent(m_renderer);
    performance_end(PerfPresentWait);
    LbMouseOnEndSwap();
    performance_end(PerfPresentation);
    if (!presented) performance_failed("SDL_RenderPresent failed");
}

#ifdef KFX_RUST_PRESENTER
void RendererSoftware::destroy_rust_presenter()
{
    if (m_rust != nullptr) {
        kfx_wgpu_details(m_rust, m_rust_details, sizeof(m_rust_details));
        SYNCLOG("Rust presenter shutdown after %lu frames: %s", m_rust_frames, m_rust_details);
        kfx_wgpu_destroy(m_rust);
        m_rust = nullptr;
    }
    if (m_metal_view != nullptr) {
        SDL_Metal_DestroyView(m_metal_view);
        m_metal_view = nullptr;
    }
    m_rust_window = nullptr;
    m_vsync = -1;
}

bool RendererSoftware::try_rust_presenter()
{
    if (m_rust_attempted) return false;
    m_rust_attempted = true;
    const char* backend = SDL_getenv("KFX_PRESENT_BACKEND");
    if (backend == nullptr || strcmp(backend, "wgpu") != 0) return false;
    char error[1024] = {};
    if (SDL_getenv("KFX_WGPU_FAIL_INIT") != nullptr) {
        snprintf(error, sizeof(error), "injected initialization failure");
    } else {
        m_metal_view = SDL_Metal_CreateView(lbWindow);
        int width = 0, height = 0;
        SDL_GetWindowSizeInPixels(lbWindow, &width, &height);
        if (m_metal_view != nullptr && width > 0 && height > 0)
            m_rust = kfx_wgpu_create(SDL_Metal_GetLayer(m_metal_view), width, height,
                vsync_enabled ? 1 : 0, error, sizeof(error));
        else
            snprintf(error, sizeof(error), "%s", SDL_GetError());
    }
    if (m_rust == nullptr) {
        WARNLOG("Rust presentation initialization failed: %s; falling back to SDL", error);
        destroy_rust_presenter();
        return false;
    }
    m_rust_window = lbWindow;
    kfx_wgpu_details(m_rust, m_rust_details, sizeof(m_rust_details));
    SYNCLOG("Presenting through Rust wgpu-metal: %s", m_rust_details);
    return true;
}

bool RendererSoftware::present_rust_frame()
{
    int width = 0, height = 0;
    SDL_GetWindowSizeInPixels(lbWindow, &width, &height);
    if (width <= 0 || height <= 0 || (SDL_GetWindowFlags(lbWindow) & SDL_WINDOW_MINIMIZED)) {
        performance_failed("Rust presentation skipped while minimized");
        return true;
    }
    if (performance_active()) {
        performance_renderer_info("wgpu-metal", SDL_GetCurrentVideoDriver(),
            lbDrawSurface->w, lbDrawSurface->h, width, height, vsync_enabled ? 1 : 0);
        performance_renderer_details(m_rust_details);
    }
    char error[1024] = {};
    performance_begin(PerfPresentation);
    LbMouseOnBeginSwap();
    SDL_Palette* palette = SDL_GetSurfacePalette(lbDrawSurface);
    int result = -1;
    if (palette != nullptr && palette->ncolors == 256) {
        result = kfx_wgpu_submit(m_rust, static_cast<const uint8_t*>(lbDrawSurface->pixels),
            static_cast<size_t>(lbDrawSurface->pitch) * lbDrawSurface->h,
            lbDrawSurface->w, lbDrawSurface->h, lbDrawSurface->pitch,
            reinterpret_cast<const uint8_t*>(palette->colors), sizeof(SDL_Color) * 256,
            width, height, vsync_enabled ? 1 : 0, error, sizeof(error));
    } else {
        snprintf(error, sizeof(error), "invalid indexed display palette");
    }
    if (result == 1 && SDL_getenv("KFX_FRAME_CAPTURE") != nullptr) {
        SDL_Surface* rgba = SDL_ConvertSurface(lbDrawSurface, SDL_PIXELFORMAT_RGBA32);
        if (rgba != nullptr) {
            CaptureFrameIfRequested(lbDrawSurface, rgba);
            SDL_DestroySurface(rgba);
        }
    }
    performance_begin(PerfPresentWait);
    if (result == 1) {
        result = kfx_wgpu_present(m_rust, error, sizeof(error));
        ++m_rust_frames;
        const char* fail_after = SDL_getenv("KFX_WGPU_FAIL_AFTER");
        if (fail_after != nullptr && m_rust_frames >= strtoul(fail_after, nullptr, 10)) {
            result = -1;
            snprintf(error, sizeof(error), "injected presentation failure");
        }
    }
    performance_end(PerfPresentWait);
    LbMouseOnEndSwap();
    performance_end(PerfPresentation);
    if (m_vsync != (vsync_enabled ? 1 : 0)) {
        m_vsync = vsync_enabled ? 1 : 0;
        kfx_wgpu_details(m_rust, m_rust_details, sizeof(m_rust_details));
    }
    if (result == 0) {
        performance_failed("Rust surface acquisition skipped");
        return true;
    }
    if (result < 0) {
        WARNLOG("Rust presentation failed: %s; falling back to SDL", error);
        performance_failed("Rust presentation failed; fallback invalidates measurement");
        destroy_rust_presenter();
        return false;
    }
    return true;
}
#endif
