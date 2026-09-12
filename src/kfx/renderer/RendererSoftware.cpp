#include "pre_inc.h"
#include "kfx/renderer/RendererSoftware.h"
#include "kfx/renderer/FrameCapture.h"
#include "performance_capture.h"
#include "bflib_video.h"       // PALETTE_COLORS, lbWindow, SDL, vsync_enabled
#include "bflib_vidsurface.h"  // lbDrawSurface (goes away when the framebuffer migrates)
#include "bflib_mouse.h"       // LbMouseOnBeginSwap/EndSwap (software cursor around present)
#include <SDL3_image/SDL_image.h> // IMG_SavePNG (screenshots)
#include "post_inc.h"

bool RendererSoftware::Init()
{
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
