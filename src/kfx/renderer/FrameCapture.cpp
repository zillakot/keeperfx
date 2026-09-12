#include "pre_inc.h"
#include "kfx/renderer/FrameCapture.h"
#include "keeperfx.hpp"
#include "creature_states.h"
#include "frontend.h"
#include "game_legacy.h"
#include "vidfade.h"
#include <SDL3/SDL.h>
#include <SDL3_image/SDL_image.h>
#include <cstdlib>
#include <cstring>
#include <cerrno>
#include <filesystem>
#include <fstream>
#include <cstdio>
#include <string>
#include "post_inc.h"

static void write_u32(std::ostream& output, uint32_t value)
{
    const char bytes[] = {static_cast<char>(value), static_cast<char>(value >> 8),
        static_cast<char>(value >> 16), static_cast<char>(value >> 24)};
    output.write(bytes, sizeof(bytes));
}

struct CaptureOptions {
    const char* directory = nullptr;
    const char* scene = "dungeon";
    uint32_t turn = 20;
    uint32_t count = 1;
    uint32_t interval = 1;
    bool exit_after = false;
    bool valid = true;
};

static bool read_number(const char* name, uint32_t minimum, uint32_t maximum, uint32_t& value)
{
    const char* text = std::getenv(name);
    if (text == nullptr) {
        return true;
    }
    char* end = nullptr;
    errno = 0;
    const unsigned long parsed = std::strtoul(text, &end, 10);
    if (*text < '0' || *text > '9' || errno == ERANGE || *end != '\0' ||
        parsed < minimum || parsed > maximum) {
        ERRORLOG("Invalid %s", name);
        std::fprintf(stderr, "Invalid %s\n", name);
        return false;
    }
    value = static_cast<uint32_t>(parsed);
    return true;
}

static CaptureOptions read_options()
{
    CaptureOptions options;
    options.directory = std::getenv("KFX_FRAME_CAPTURE");
    if (options.directory == nullptr || *options.directory == '\0') {
        return options;
    }
    const char* scene = std::getenv("KFX_FRAME_CAPTURE_SCENE");
    if (scene != nullptr) {
        if (std::strcmp(scene, "dungeon") != 0 && std::strcmp(scene, "menu") != 0 &&
            std::strcmp(scene, "possession") != 0) {
            ERRORLOG("Invalid KFX_FRAME_CAPTURE_SCENE");
            std::fprintf(stderr, "Invalid KFX_FRAME_CAPTURE_SCENE\n");
            options.valid = false;
        } else {
            options.scene = scene;
        }
    }
    options.valid = read_number("KFX_FRAME_CAPTURE_TURN", 0, UINT32_MAX, options.turn) && options.valid;
    options.valid = read_number("KFX_FRAME_CAPTURE_COUNT", 1, 32, options.count) && options.valid;
    options.valid = read_number("KFX_FRAME_CAPTURE_INTERVAL", 1, UINT32_MAX, options.interval) && options.valid;
    const char* exit_after = std::getenv("KFX_FRAME_CAPTURE_EXIT");
    if (exit_after != nullptr && std::strcmp(exit_after, "0") != 0 && std::strcmp(exit_after, "1") != 0) {
        ERRORLOG("Invalid KFX_FRAME_CAPTURE_EXIT");
        std::fprintf(stderr, "Invalid KFX_FRAME_CAPTURE_EXIT\n");
        options.valid = false;
    }
    options.exit_after = exit_after != nullptr && std::strcmp(exit_after, "1") == 0;
    return options;
}

static std::string frame_directory(uint32_t index)
{
    char name[32];
    std::snprintf(name, sizeof(name), "frame-%04u", index);
    return name;
}

static bool write_sequence(const CaptureOptions& options)
{
    std::ofstream manifest(std::filesystem::path(options.directory) / "sequence.json");
    manifest << "{\"format\":\"KFXSEQ01\",\"frames\":[";
    for (uint32_t i = 0; i < options.count; ++i) {
        if (i != 0) {
            manifest << ',';
        }
        const std::string name = frame_directory(i);
        manifest << "{\"frame\":\"" << name << "/frame.kfx\",\"reference\":\""
            << name << "/reference.png\"}";
    }
    manifest << "]}\n";
    manifest.close();
    return manifest.good();
}

static bool capture_frame(SDL_Surface* indexed, SDL_Surface* rgba, const std::filesystem::path& path,
    const char* scene, uint32_t index, uint64_t ordinal)
{
    SDL_Palette* palette = SDL_GetSurfacePalette(indexed);
    if (indexed->format != SDL_PIXELFORMAT_INDEX8 || palette == nullptr || palette->ncolors != 256 ||
        indexed->w <= 0 || indexed->h <= 0 || indexed->pitch < indexed->w) {
        return false;
    }
    std::error_code error;
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
        << ",\"game_turn\":" << get_gameturn() << ",\"scene\":\"" << scene
        << "\",\"frame_index\":" << index << ",\"present_ordinal\":" << ordinal;
    if (std::strcmp(scene, "menu") == 0) {
        metadata << ",\"view\":\"main_menu\",\"frontend_state\":" << frontend_menu_state;
    } else {
        const PlayerInfo* player = get_my_player();
        metadata << ",\"view\":\"" << (player->view_mode == PVM_CreatureView ? "creature" : "dungeon_top")
            << "\",\"view_type\":" << static_cast<int>(player->view_type)
            << ",\"view_mode\":" << static_cast<int>(player->view_mode)
            << ",\"controlled_thing_index\":" << player->controlled_thing_idx;
    }
    metadata << "}\n";
    metadata.close();
    return metadata.good();
}

void CaptureFrameIfRequested(SDL_Surface* indexed, SDL_Surface* rgba)
{
    static const CaptureOptions options = read_options();
    static bool finished = false;
    static uint32_t captured = 0;
    static uint64_t ordinal = 0;
    static uint64_t matching = 0;
    static bool possession_started = false;
    static uint32_t possession_turn = 0;
    if (options.directory == nullptr || *options.directory == '\0' || finished) {
        return;
    }
    if (!options.valid) {
        finished = true;
        exit_keeper = 1;
        return;
    }
    ++ordinal;
    if (std::strcmp(options.scene, "menu") == 0) {
        if (frontend_menu_state != FeSt_MAIN_MENU || fade_palette_in != 0 || exit_keeper) {
            return;
        }
    } else {
        PlayerInfo* player = get_my_player();
        if (frontend_menu_state != FeSt_INITIAL || !player_exists(player) ||
            (game.game_kind != GKind_LocalGame && game.game_kind != GKind_MultiGame) ||
            get_gameturn() < options.turn || quit_game || exit_keeper) {
            return;
        }
        if (std::strcmp(options.scene, "possession") == 0) {
            if (game.game_kind != GKind_LocalGame || (game.system_flags & GSF_NetworkActive) != 0) {
                ERRORLOG("Possession frame capture requires a local game");
                std::fprintf(stderr, "Possession frame capture requires a local game\n");
                finished = true;
                exit_keeper = 1;
                return;
            }
            if (!possession_started) {
                if (player->instance_num != 0 || player->view_type != PVT_DungeonTop) {
                    return;
                }
                for (int i = 1; i < THINGS_COUNT; ++i) {
                    Thing* thing = thing_get(i);
                    if (thing_exists(thing) && thing_is_creature(thing) && thing->owner == player->id_number &&
                        thing->health > 0 && !thing_is_in_limbo(thing) &&
                        !creature_is_kept_in_custody_by_enemy_or_dying(thing) &&
                        thing_can_be_controlled_as_controller(thing)) {
                        possession_started = control_creature_as_controller(player, thing);
                        possession_turn = get_gameturn();
                        break;
                    }
                }
                // This surface was rendered before possession changed the view.
                return;
            }
            if (get_gameturn() <= possession_turn || player->view_type != PVT_CreatureContrl ||
                player->view_mode != PVM_CreatureView) {
                return;
            }
        } else if (player->view_type != PVT_DungeonTop) {
            return;
        }
    }
    if (matching++ % options.interval != 0) {
        return;
    }
    bool success = true;
    std::filesystem::path path(options.directory);
    if (options.count > 1) {
        if (captured == 0) {
            std::error_code error;
            success = std::filesystem::create_directory(path, error);
        }
        path /= frame_directory(captured);
    }
    success = success && capture_frame(indexed, rgba, path, options.scene, captured, ordinal);
    if (success) {
        ++captured;
        if (captured == options.count && options.count > 1) {
            success = write_sequence(options);
        }
    }
    if (!success) {
        ERRORLOG("Frame capture failed in %s; use a new directory with an existing parent", options.directory);
        std::fprintf(stderr, "Frame capture failed in %s; use a new directory with an existing parent\n", options.directory);
        finished = true;
        if (options.exit_after) {
            exit_keeper = 1;
        }
    } else if (captured == options.count) {
        SYNCLOG("Captured %u frame(s) in %s", captured, options.directory);
        finished = true;
        if (options.exit_after) {
            exit_keeper = 1;
        }
    }
}
