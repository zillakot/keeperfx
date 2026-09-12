#include "pre_inc.h"
#include "kfx/renderer/FrameCapture.h"
#include "keeperfx.hpp"
#include <SDL3/SDL.h>
#include <SDL3_image/SDL_image.h>
#include <cstdlib>
#include <cstring>
#include <cerrno>
#include <filesystem>
#include <fstream>
#include "post_inc.h"

static void write_u32(std::ostream& output, uint32_t value)
{
    const char bytes[] = {static_cast<char>(value), static_cast<char>(value >> 8),
        static_cast<char>(value >> 16), static_cast<char>(value >> 24)};
    output.write(bytes, sizeof(bytes));
}

static bool capture_frame(SDL_Surface* indexed, SDL_Surface* rgba, const char* directory)
{
    SDL_Palette* palette = SDL_GetSurfacePalette(indexed);
    if (indexed->format != SDL_PIXELFORMAT_INDEX8 || palette == nullptr || palette->ncolors != 256 ||
        indexed->w <= 0 || indexed->h <= 0 || indexed->pitch < indexed->w) {
        return false;
    }
    std::error_code error;
    const std::filesystem::path path(directory);
    if (!std::filesystem::create_directory(path, error)) {
        return false;
    }
    std::ofstream output(path / "frame.kfx", std::ios::binary);
    output.write("KFXFRM01", 8);
    write_u32(output, indexed->w);
    write_u32(output, indexed->h);
    for (int i = 0; i < 256; ++i) {
        const SDL_Color& color = palette->colors[i];
        const char bytes[] = {static_cast<char>(color.r), static_cast<char>(color.g),
            static_cast<char>(color.b), static_cast<char>(color.a)};
        output.write(bytes, sizeof(bytes));
    }
    if (!SDL_LockSurface(indexed)) {
        return false;
    }
    for (int y = 0; y < indexed->h; ++y) {
        output.write(static_cast<const char*>(indexed->pixels) + y * indexed->pitch, indexed->w);
    }
    SDL_UnlockSurface(indexed);
    output.close();
    if (!output || !IMG_SavePNG(rgba, (path / "reference.png").string().c_str())) {
        return false;
    }
    std::ofstream metadata(path / "capture.json");
    metadata << "{\"width\":" << indexed->w << ",\"height\":" << indexed->h
        << ",\"game_turn\":" << get_gameturn() << "}\n";
    metadata.close();
    return metadata.good();
}

void CaptureFrameIfRequested(SDL_Surface* indexed, SDL_Surface* rgba)
{
    static const char* directory = std::getenv("KFX_FRAME_CAPTURE");
    static bool attempted = false;
    if (directory == nullptr || *directory == '\0' || attempted) {
        return;
    }
    const char* turn_text = std::getenv("KFX_FRAME_CAPTURE_TURN");
    char* end = nullptr;
    errno = 0;
    unsigned long turn = turn_text == nullptr ? 20 : std::strtoul(turn_text, &end, 10);
    if (turn_text != nullptr && (*turn_text < '0' || *turn_text > '9' || errno == ERANGE ||
        end == turn_text || *end != '\0' || turn > UINT32_MAX)) {
        attempted = true;
        ERRORLOG("Invalid KFX_FRAME_CAPTURE_TURN");
        return;
    }
    if (get_gameturn() < turn) {
        return;
    }
    attempted = true;
    if (capture_frame(indexed, rgba, directory)) {
        SYNCLOG("Captured frame in %s", directory);
    } else {
        ERRORLOG("Frame capture failed in %s; use a new directory with an existing parent", directory);
    }
    const char* exit_after = std::getenv("KFX_FRAME_CAPTURE_EXIT");
    if (exit_after != nullptr && std::strcmp(exit_after, "1") == 0) {
        exit_keeper = 1;
    }
}
