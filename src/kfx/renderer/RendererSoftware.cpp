#include "pre_inc.h"
#include "kfx/renderer/software/WgpuRawImage.h"
#include "kfx/renderer/RendererSoftware.h"
#include "kfx/renderer/FrameCapture.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "performance_capture.h"
#include "bflib_video.h"       // PALETTE_COLORS, lbWindow, SDL, vsync_enabled
#include "bflib_vidsurface.h"  // lbDrawSurface (goes away when the framebuffer migrates)
#include "bflib_mouse.h"       // LbMouseOnBeginSwap/EndSwap (software cursor around present)
#include <SDL3_image/SDL_image.h> // IMG_SavePNG (screenshots)
#ifdef KFX_RUST_PRESENTER
#include "kfx/renderer/RustPresenter.h"
#include "kfx/renderer/KfxWgpuFrame.h"
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
    const char* drawing = SDL_getenv("KFX_DRAW_BACKEND");
    if (drawing != nullptr && strcmp(drawing, "software") != 0 && strcmp(drawing, "wgpu") != 0)
        WARNLOG("Unknown KFX_DRAW_BACKEND '%s'; using software drawing", drawing);
    if (drawing != nullptr && strcmp(drawing, "wgpu") == 0) {
#ifdef KFX_RUST_PRESENTER
        const char* fail_after = SDL_getenv("KFX_WGPU_DRAW_FAIL_AFTER");
        m_drawing = new WgpuTerrainBridge(fail_after != nullptr ? strtoull(fail_after, nullptr, 10) : 0,
            SDL_getenv("KFX_WGPU_DRAW_FAIL_INIT") != nullptr, SDL_getenv("KFX_WGPU_DRAW_VERIFY") != nullptr, true);
        SYNCLOG("Drawing selected: wgpu frame commands with explicit CPU compatibility leases");
#else
        WARNLOG("Rust drawing is not built on this platform; using software drawing");
#endif
    }
    return true;
}

void RendererSoftware::Shutdown()
{
#ifdef KFX_RUST_PRESENTER
    if (m_drawing != nullptr) {
        m_drawing->EndFrame(true);
        report_drawing();
        const auto& counts = m_drawing->GetCounters();
        SYNCLOG("GPU terrain shutdown: %llu batches, %llu spans, %llu readbacks, %llu CPU replayed spans, %llu failures",
            static_cast<unsigned long long>(counts.gpu_batches), static_cast<unsigned long long>(counts.gpu_spans),
            static_cast<unsigned long long>(counts.bridge_readbacks), static_cast<unsigned long long>(counts.cpu_replayed_spans),
            static_cast<unsigned long long>(counts.failures));
        delete m_drawing;
        m_drawing = nullptr;
    }
#endif
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
#ifdef KFX_RUST_PRESENTER
    if (m_drawing != nullptr && lbWindow != nullptr)
        ensure_present_target();
#endif
    SDL_Rect clip;
    const bool full = SDL_GetSurfaceClipRect(lbDrawSurface, &clip) && clip.x == 0 && clip.y == 0 &&
        clip.w == lbDrawSurface->w && clip.h == lbDrawSurface->h;
#ifdef KFX_RUST_PRESENTER
    if (full && m_drawing != nullptr)
        m_drawing->BeginFrame({static_cast<uint8_t*>(lbDrawSurface->pixels),
            static_cast<uint32_t>(lbDrawSurface->w), static_cast<uint32_t>(lbDrawSurface->h),
            static_cast<uint32_t>(lbDrawSurface->pitch)}, true);
#endif
    if (full && kfx_wgpu_raw_clear(static_cast<uint8_t*>(lbDrawSurface->pixels), lbDrawSurface->pitch,
        lbDrawSurface->w, lbDrawSurface->h, colour)) return;
    if (!full && !kfx_wgpu_native_cpu_barrier()) return;
    if (!SDL_FillSurfaceRect(lbDrawSurface, NULL, colour))
        ERRORLOG("Error while clearing screen: %s", SDL_GetError());
#ifdef KFX_RUST_PRESENTER
    else if (full && m_drawing != nullptr) m_drawing->FullRedraw();
#endif
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
#ifdef KFX_RUST_PRESENTER
    const KfxGpolyTarget target = lbDrawSurface ? KfxGpolyTarget{static_cast<uint8_t*>(lbDrawSurface->pixels),
        static_cast<uint32_t>(lbDrawSurface->w), static_cast<uint32_t>(lbDrawSurface->h),
        static_cast<uint32_t>(lbDrawSurface->pitch)} : KfxGpolyTarget{};
    if ((!m_drawing || !m_drawing->OwnsFrame(target)) && !kfx_wgpu_native_cpu_barrier()) return nullptr;
    if (m_drawing != nullptr && lbWindow != nullptr)
        ensure_present_target();
#endif
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
    if (lbDrawSurface == NULL) return false;
    if (!kfx_wgpu_native_read_barrier(lbDrawSurface->pixels,
            static_cast<size_t>(lbDrawSurface->pitch) * lbDrawSurface->h)) return false;
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
#ifdef KFX_RUST_PRESENTER
    if (m_drawing != nullptr) {
        m_drawing->Flush();
        ++m_drawing_frames;
        report_drawing();
        if (!m_drawing->FrameValid()) {
            performance_failed("resident GPU frame invalid; awaiting full screen redraw");
            return;
        }
    } else performance_drawing_backend("software");
#else
    performance_drawing_backend("software");
#endif
    if (lbDrawSurface == NULL || !ensure_present_target()) {
        performance_failed("presentation target unavailable");
        return;
    }
#ifdef KFX_RUST_PRESENTER
    if (m_rust != nullptr) {
        if (present_rust_frame()) return;
        if (!ensure_present_target()) return;
    }
    if (m_drawing != nullptr && !m_drawing->EndFrame(true)) return;
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
void RendererSoftware::report_drawing()
{
    const auto& counts = m_drawing->GetCounters();
    const auto gpu = m_drawing->GetGpuCounters();
    KfxWgpuFrameCounters frame = {};
    char error[1024] = {};
    if (m_drawing->Context())
        kfx_wgpu_draw_frame_counters(m_drawing->Context(), &frame, error, sizeof(error));
    if (m_drawing->Failed() && !m_drawing_failure_reported) {
        WARNLOG("GPU terrain drawing failed: %s", m_drawing->GetError());
        m_drawing_failure_reported = true;
    }
    performance_drawing_backend(m_drawing->Failed() ? "wgpu-fallback" : "wgpu");
    const PerformanceDrawingCounters sample = {
        gpu.submits, gpu.dispatches, gpu.waits, gpu.wait_ns,
        frame.checkpoints, frame.checkpoint_copy_bytes, frame.validation_waits,
        frame.invalid_frames, frame.status_stalls,
        gpu.asset_upload_bytes + gpu.command_upload_bytes, gpu.readback_bytes,
        counts.bridge_readbacks, counts.gpu_readback_bytes,
        gpu.buffers, gpu.buffer_bytes, gpu.batches, gpu.commands, counts.gpu_ordered_sprites,
        gpu.arena_evictions, gpu.arena_overflows, gpu.arena_bytes_uploaded,
        gpu.tile_allocations, gpu.tile_entries,
        counts.bridge_target_flushes, counts.bridge_target_runs,
        gpu.prepared_row_words, gpu.prepared_row_allocations,
        gpu.pass_ns[0], gpu.pass_ns[1], gpu.pass_ns[2], gpu.pass_ns[3], gpu.pass_ns[4],
        gpu.pass_ns[5], gpu.pass_ns[6], gpu.pass_ns[7], gpu.pass_ns[8],
        gpu.timed_passes, gpu.untimed_passes,
        gpu.host_staged_asset_bytes, gpu.arena_bytes_resident};
    performance_drawing_frame(&sample);
    const char* path = SDL_getenv("KFX_WGPU_DRAW_STATS");
    if (path != nullptr) {
        FILE* output = fopen(path, "w");
        if (output != nullptr) {
            fprintf(output, "{\"backend\":\"wgpu-native-frame\",\"frames\":%lu,"
                "\"gpu_batches\":%llu,\"gpu_spans\":%llu,\"gpu_pixels\":%llu,"
                "\"cpu_gpoly_spans\":%llu,\"cpu_replayed_spans\":%llu,"
                "\"bridge_readbacks\":%llu,\"gpu_readback_bytes\":%llu,\"native_copy_bytes\":%llu,"
                "\"bridge_initial_index_bytes\":%llu,\"resource_snapshot_bytes\":%llu,"
                "\"target_creations\":%llu,\"failures\":%llu,\"verified_batches\":%llu,\"verification_cpu_spans\":%llu,\"verification_flagged_shades\":%llu,\"gpu_api_batches\":%llu,\"gpu_api_commands\":%llu,\"gpu_asset_upload_bytes\":%llu,\"gpu_command_upload_bytes\":%llu,\"gpu_api_readback_bytes\":%llu,\"native_commands\":%llu,\"verification_cpu_commands\":%llu,\"gpu_triangles\":%llu,\"cpu_triangles\":%llu,\"replayed_triangles\":%llu,\"verified_triangles\":%llu,\"rejected_triangles\":%llu,\"gpu_sprite_commands\":%llu,\"gpu_shadow_commands\":%llu,\"shadow_scratch_upload_bytes\":%llu,\"shadow_scratch_readback_bytes\":%llu,\"shadow_scratch_copy_bytes\":%llu,\"shadow_prior_divergence\":%llu,\"resident_sequences\":%llu,\"resident_batches\":%llu,\"cpu_barriers\":%llu,\"target_alias_barriers\":%llu,\"barrier_readbacks\":%llu,\"verification_readbacks\":%llu,\"invalid_frames\":%llu,\"missing_cpu_barriers\":%llu,\"transition_checkpoint_bytes\":%llu,\"transition_snapshot_copy_bytes\":%llu,\"transition_commands\":%llu,\"frame_queued_commands\":%llu,\"frame_checkpoints\":%llu,\"frame_validation_waits\":%llu,\"frame_validation_bytes\":%llu,\"frame_gpu_checkpoint_copy_bytes\":%llu,\"frame_rejected_checkpoints\":%llu,\"frame_flagged_invalid\":%llu,\"frame_status_reads\":%llu,\"frame_status_stalls\":%llu,"
                "\"gpu_submits\":%llu,\"gpu_dispatches\":%llu,\"gpu_waits\":%llu,\"gpu_wait_ns\":%llu,"
                "\"gpu_buffers\":%llu,\"gpu_buffer_bytes\":%llu,"
                "\"gpu_ordered_sprites\":%llu,\"gpu_host_staged_asset_bytes\":%llu,"
                "\"arena_evictions\":%llu,\"arena_overflows\":%llu,"
                "\"arena_bytes_uploaded\":%llu,\"arena_bytes_resident\":%llu,"
                "\"bridge_solo_batches\":%llu,\"bridge_target_flushes\":%llu,"
                "\"bridge_target_runs\":%llu,\"tile_allocations\":%llu,\"tile_entries\":%llu,"
                "\"prepared_row_words\":%llu,\"prepared_row_allocations\":%llu,"
                "\"gpu_raster_ns\":%llu,"
                "\"gpu_terrain_prepare_ns\":%llu,\"gpu_terrain_validate_ns\":%llu,"
                "\"gpu_shadow_mask_ns\":%llu,\"gpu_target_trig_ns\":%llu,"
                "\"gpu_ordered_sprite_ns\":%llu,\"gpu_minimap_ns\":%llu,"
                "\"gpu_lens_ns\":%llu,\"gpu_present_ns\":%llu,"
                "\"gpu_timed_passes\":%llu,\"gpu_untimed_passes\":%llu,"
                "\"rejected_commands\":%llu,\"rejected_spans\":%llu}\n",
                m_drawing_frames, static_cast<unsigned long long>(counts.gpu_batches),
                static_cast<unsigned long long>(counts.gpu_spans), static_cast<unsigned long long>(counts.gpu_pixels),
                static_cast<unsigned long long>(counts.cpu_gpoly_spans), static_cast<unsigned long long>(counts.cpu_replayed_spans),
                static_cast<unsigned long long>(counts.bridge_readbacks), static_cast<unsigned long long>(counts.gpu_readback_bytes),
                static_cast<unsigned long long>(counts.native_copy_bytes), static_cast<unsigned long long>(counts.bridge_initial_index_bytes),
                static_cast<unsigned long long>(counts.resource_snapshot_bytes), static_cast<unsigned long long>(counts.target_creations),
                static_cast<unsigned long long>(counts.failures), static_cast<unsigned long long>(counts.verified_batches),
                static_cast<unsigned long long>(counts.verification_cpu_spans),
                static_cast<unsigned long long>(counts.verification_flagged_shades),
                static_cast<unsigned long long>(gpu.batches),
                static_cast<unsigned long long>(gpu.commands), static_cast<unsigned long long>(gpu.asset_upload_bytes),
                static_cast<unsigned long long>(gpu.command_upload_bytes), static_cast<unsigned long long>(gpu.readback_bytes),
                static_cast<unsigned long long>(counts.native_commands), static_cast<unsigned long long>(counts.verification_cpu_commands),
                static_cast<unsigned long long>(counts.gpu_triangles), static_cast<unsigned long long>(counts.cpu_triangles),
                static_cast<unsigned long long>(counts.replayed_triangles), static_cast<unsigned long long>(counts.verified_triangles), static_cast<unsigned long long>(counts.rejected_triangles), static_cast<unsigned long long>(counts.gpu_sprite_commands),
                static_cast<unsigned long long>(counts.gpu_shadow_commands), static_cast<unsigned long long>(counts.shadow_scratch_upload_bytes),
                static_cast<unsigned long long>(counts.shadow_scratch_readback_bytes), static_cast<unsigned long long>(counts.shadow_scratch_copy_bytes),
                static_cast<unsigned long long>(counts.shadow_prior_divergence),
                static_cast<unsigned long long>(counts.resident_sequences), static_cast<unsigned long long>(counts.resident_batches),
                static_cast<unsigned long long>(counts.cpu_barriers), static_cast<unsigned long long>(counts.target_alias_barriers),
                static_cast<unsigned long long>(counts.barrier_readbacks), static_cast<unsigned long long>(counts.verification_readbacks),
                static_cast<unsigned long long>(counts.invalid_frames), static_cast<unsigned long long>(counts.missing_cpu_barriers),
                static_cast<unsigned long long>(counts.transition_checkpoint_bytes),
                static_cast<unsigned long long>(counts.transition_snapshot_copy_bytes),
                static_cast<unsigned long long>(counts.transition_commands),
                static_cast<unsigned long long>(frame.queued_commands),
                static_cast<unsigned long long>(frame.checkpoints),
                static_cast<unsigned long long>(frame.validation_waits),
                static_cast<unsigned long long>(frame.validation_bytes),
                static_cast<unsigned long long>(frame.checkpoint_copy_bytes),
                static_cast<unsigned long long>(frame.rejected_checkpoints),
                static_cast<unsigned long long>(frame.invalid_frames),
                static_cast<unsigned long long>(frame.status_reads),
                static_cast<unsigned long long>(frame.status_stalls),
                static_cast<unsigned long long>(gpu.submits),
                static_cast<unsigned long long>(gpu.dispatches),
                static_cast<unsigned long long>(gpu.waits),
                static_cast<unsigned long long>(gpu.wait_ns),
                static_cast<unsigned long long>(gpu.buffers),
                static_cast<unsigned long long>(gpu.buffer_bytes),
                static_cast<unsigned long long>(counts.gpu_ordered_sprites),
                static_cast<unsigned long long>(gpu.host_staged_asset_bytes),
                static_cast<unsigned long long>(gpu.arena_evictions),
                static_cast<unsigned long long>(gpu.arena_overflows),
                static_cast<unsigned long long>(gpu.arena_bytes_uploaded),
                static_cast<unsigned long long>(gpu.arena_bytes_resident),
                static_cast<unsigned long long>(counts.bridge_solo_batches),
                static_cast<unsigned long long>(counts.bridge_target_flushes),
                static_cast<unsigned long long>(counts.bridge_target_runs),
                static_cast<unsigned long long>(gpu.tile_allocations),
                static_cast<unsigned long long>(gpu.tile_entries),
                static_cast<unsigned long long>(gpu.prepared_row_words),
                static_cast<unsigned long long>(gpu.prepared_row_allocations),
                static_cast<unsigned long long>(gpu.pass_ns[0]),
                static_cast<unsigned long long>(gpu.pass_ns[1]),
                static_cast<unsigned long long>(gpu.pass_ns[2]),
                static_cast<unsigned long long>(gpu.pass_ns[3]),
                static_cast<unsigned long long>(gpu.pass_ns[4]),
                static_cast<unsigned long long>(gpu.pass_ns[5]),
                static_cast<unsigned long long>(gpu.pass_ns[6]),
                static_cast<unsigned long long>(gpu.pass_ns[7]),
                static_cast<unsigned long long>(gpu.pass_ns[8]),
                static_cast<unsigned long long>(gpu.timed_passes),
                static_cast<unsigned long long>(gpu.untimed_passes),
                static_cast<unsigned long long>(counts.rejected_commands),
                static_cast<unsigned long long>(counts.rejected_spans));
            fclose(output);
        }
    }
}

void RendererSoftware::destroy_rust_presenter()
{
    if (m_rust != nullptr) {
        if (m_drawing != nullptr) m_drawing->DetachPresenter();
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
    if (m_drawing != nullptr)
        m_drawing->AttachPresenter(m_rust);
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
        const KfxGpolyTarget native = {static_cast<uint8_t*>(lbDrawSurface->pixels),
            static_cast<uint32_t>(lbDrawSurface->w), static_cast<uint32_t>(lbDrawSurface->h),
            static_cast<uint32_t>(lbDrawSurface->pitch)};
        const uint64_t target = m_drawing && m_drawing->UsesPresenter() ? m_drawing->ResidentTarget(native) : 0;
        if (target) {
            result = kfx_wgpu_draw_prepare_present(m_rust, target,
                reinterpret_cast<const uint8_t*>(palette->colors), sizeof(SDL_Color) * 256,
                width, height, vsync_enabled ? 1 : 0, error, sizeof(error));
        } else if (kfx_wgpu_native_cpu_barrier()) {
            result = kfx_wgpu_submit(m_rust, static_cast<const uint8_t*>(lbDrawSurface->pixels),
                static_cast<size_t>(lbDrawSurface->pitch) * lbDrawSurface->h,
                lbDrawSurface->w, lbDrawSurface->h, lbDrawSurface->pitch,
                reinterpret_cast<const uint8_t*>(palette->colors), sizeof(SDL_Color) * 256,
                width, height, vsync_enabled ? 1 : 0, error, sizeof(error));
        }
    } else {
        snprintf(error, sizeof(error), "invalid indexed display palette");
    }
    if (result == 1 && SDL_getenv("KFX_FRAME_CAPTURE") != nullptr &&
        kfx_wgpu_native_read_barrier(lbDrawSurface->pixels, static_cast<size_t>(lbDrawSurface->pitch) * lbDrawSurface->h)) {
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
        const bool recoverable = kfx_wgpu_native_cpu_barrier();
        destroy_rust_presenter();
        return !recoverable;
    }
    return true;
}
#endif
