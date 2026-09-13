#ifndef RENDERER_RENDERERSOFTWARE_H
#define RENDERER_RENDERERSOFTWARE_H

#include "kfx/renderer/IRenderer.h"
#include "kfx/renderer/backends/SoftwareUIRenderer.h"
#include "kfx/renderer/backends/SoftwareTextRenderer.h"

struct SDL_Window;
struct SDL_Renderer;
struct SDL_Texture;
struct SDL_Surface;

// Software backend. it's small for now, it's gonna grow the more I bring things into it.
class RendererSoftware : public IRenderer {
public:
    const char* GetPresenterName() const override {
#ifdef KFX_RUST_PRESENTER
        if (m_rust != nullptr) return "wgpu";
#endif
        return m_renderer != nullptr ? "sdl" : "uninitialized";
    }
    bool Init() override;
    void Shutdown() override;
    const char* GetName() const override { return "software"; }
    void SetDisplayPalette(const unsigned char* pal6) override;
    void ClearScreen(unsigned char colour) override;
    void PresentFrame() override;
    unsigned char* LockFramebuffer(int* out_pitch) override;
    void UnlockFramebuffer() override;
    bool ScheduleScreenshot(const char* path, int fmt) override;


    IUIRenderer*   GetUIRenderer()   override { return &m_ui_renderer; }
    ITextRenderer* GetTextRenderer() override { return &m_text_renderer; }

private:
    bool ensure_present_target();
    void destroy_present_target();
#ifdef KFX_RUST_PRESENTER
    bool try_rust_presenter();
    bool present_rust_frame();
    void destroy_rust_presenter();
    void* m_rust = nullptr;
    void* m_metal_view = nullptr;
    SDL_Window* m_rust_window = nullptr;
    bool m_rust_attempted = false;
    unsigned long m_rust_frames = 0;
    char m_rust_details[1024] = {};
#endif

    SDL_Renderer* m_renderer = nullptr;
    SDL_Texture*  m_texture  = nullptr;
    int           m_tex_w    = 0;
    int           m_tex_h    = 0;
    int           m_vsync    = -1; // SDL_SetRenderVSync value; -1 = unset

    SoftwareUIRenderer   m_ui_renderer;
    SoftwareTextRenderer m_text_renderer;
};

#endif // RENDERER_RENDERERSOFTWARE_H
