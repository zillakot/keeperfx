#include "pre_inc.h"
#include "performance_capture.h"
#include "keeperfx.hpp"
#include "creature_states.h"
#include "game_legacy.h"
#include "config_keeperfx.h"
#include <SDL3/SDL.h>
#include <array>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cerrno>
#include <sstream>
#include <string>
#include <vector>
#include "post_inc.h"

namespace {
using Clock = std::chrono::steady_clock;
struct Sample { int scope; unsigned long turn; uint64_t ns; };
struct Scope { Clock::time_point start; unsigned long turn; bool active = false; };
struct Profile {
    const char* output = std::getenv("KFX_PERF_OUTPUT");
    unsigned long warmup = 40, turns = 200, start_turn = 0;
    bool initialized = false, active = false, finished = false, possession = false;
    bool possession_started = false;
    unsigned long possession_turn = 0;
    FILE* file = nullptr;
    std::vector<Sample> samples;
    std::array<Scope, PerfScopeCount> scopes;
    Clock::time_point last_present;
    std::string start_state, renderer, driver;
    int width = 0, height = 0, output_width = 0, output_height = 0, vsync = -2;
};
Profile& profile() { static Profile p; return p; }

bool number(const char* name, unsigned long low, unsigned long high, unsigned long& value)
{
    const char* text = std::getenv(name);
    if (!text) return true;
    char* end;
    errno = 0;
    unsigned long parsed = std::strtoul(text, &end, 10);
    if (*text < '0' || *text > '9' || errno || *end || parsed < low || parsed > high) return false;
    value = parsed;
    return true;
}

std::string state()
{
    unsigned int things = 0, creatures = 0;
    for (int i = 1; i < THINGS_COUNT; ++i) {
        Thing* thing = thing_get(i);
        if (!thing_exists(thing)) continue;
        ++things;
        if (thing_is_creature(thing)) ++creatures;
    }
    const PlayerInfo* player = get_my_player();
    const Camera* camera = get_player_active_camera(player);
    std::ostringstream out;
    out << "{\"turn\":" << get_gameturn() << ",\"action_seed\":" << game.action_random_seed
        << ",\"ai_seed\":" << game.ai_random_seed << ",\"player_seed\":" << game.player_random_seed
        << ",\"unsync_seed\":" << game.unsync_random_seed << ",\"sound_seed\":" << game.sound_random_seed
        << ",\"creatures\":" << creatures << ",\"things\":" << things
        << ",\"controlled_thing\":" << player->controlled_thing_idx;
    if (camera) out << ",\"camera\":{\"x\":" << camera->mappos.x.val << ",\"y\":" << camera->mappos.y.val
        << ",\"z\":" << camera->mappos.z.val << ",\"zoom\":" << camera->zoom
        << ",\"angle_x\":" << camera->rotation_angle_x << ",\"angle_y\":" << camera->rotation_angle_y
        << ",\"angle_z\":" << camera->rotation_angle_z << '}';
    out << '}';
    return out.str();
}

void fail(Profile& p, const char* reason)
{
    std::fprintf(stderr, "Performance capture failed: %s\n", reason);
    if (p.file) { std::fclose(p.file); p.file = nullptr; }
    p.active = false;
    p.finished = true;
    exit_keeper = 1;
}

void finish(Profile& p)
{
    p.active = false;
    const char* names[] = {"simulation", "draw", "presentation", "present_wait", "frame_interval"};
    std::fprintf(p.file, "kind,turn,wall_ns\n");
    for (const Sample& s : p.samples)
        std::fprintf(p.file, "%s,%lu,%llu\n", names[s.scope], s.turn, static_cast<unsigned long long>(s.ns));
    bool failed = std::ferror(p.file) != 0;
    failed = std::fclose(p.file) != 0 || failed;
    p.file = nullptr;
    if (failed) { fail(p, "writing samples"); return; }
    std::string path = std::string(p.output) + ".json";
    FILE* info = std::fopen(path.c_str(), "wx");
    if (!info) { fail(p, "metadata already exists or cannot be created"); return; }
    std::fprintf(info,
        "{\"format\":\"KFXPERF01\",\"complete\":true,\"start\":%s,\"end\":%s,"
        "\"scene\":\"%s\",\"view\":\"%s\",\"renderer\":\"%s\",\"video_driver\":\"%s\","
        "\"width\":%d,\"height\":%d,\"output_width\":%d,\"output_height\":%d,"
        "\"vsync_actual\":%d,\"turns_per_second\":%ld,\"fps_limit\":%d,\"interpolation\":%s}\n",
        p.start_state.c_str(), state().c_str(), p.possession ? "possession" : "dungeon",
        p.possession ? "creature" : "dungeon_top", p.renderer.c_str(), p.driver.c_str(),
        p.width, p.height, p.output_width, p.output_height, p.vsync,
        static_cast<long>(turns_per_second), fps_limit_current, is_feature_on(Ft_DeltaTime) ? "true" : "false");
    failed = std::ferror(info) != 0;
    failed = std::fclose(info) != 0 || failed;
    if (failed) { std::remove(path.c_str()); fail(p, "writing metadata"); return; }
    p.finished = true;
    exit_keeper = 1;
}
}

void performance_prepare_turn(void)
{
    Profile& p = profile();
    if (!p.output || !*p.output || p.finished) return;
    if (!p.initialized) {
        p.initialized = true;
        const char* scene = std::getenv("KFX_PERF_SCENE");
        if ((scene && std::strcmp(scene, "dungeon") && std::strcmp(scene, "possession")) ||
            !number("KFX_PERF_TURN", 1, 600, p.warmup) || !number("KFX_PERF_TURNS", 20, 1200, p.turns) ||
            std::getenv("KFX_FRAME_CAPTURE")) {
            fail(p, "invalid options or simultaneous frame capture"); return;
        }
        p.possession = scene && !std::strcmp(scene, "possession");
        p.file = std::fopen(p.output, "wx");
        if (!p.file) { fail(p, "output already exists or cannot be created"); return; }
        p.samples.reserve(100000);
    }
    PlayerInfo* player = get_my_player();
    if (game.game_kind != GKind_LocalGame || (game.system_flags & GSF_NetworkActive) || !player_exists(player)) {
        fail(p, "local gameplay required"); return;
    }
    if (p.active) {
        if ((p.possession && player->view_mode != PVM_CreatureView) ||
            (!p.possession && player->view_type != PVT_DungeonTop)) {
            fail(p, "view changed during capture"); return;
        }
        if (get_gameturn() >= p.start_turn + p.turns) finish(p);
        return;
    }
    if (get_gameturn() < p.warmup) return;
    if (p.possession) {
        if (!p.possession_started) {
            if (player->instance_num || player->view_type != PVT_DungeonTop) return;
            for (int i = 1; i < THINGS_COUNT; ++i) {
                Thing* thing = thing_get(i);
                if (thing_exists(thing) && thing_is_creature(thing) && thing->owner == player->id_number &&
                    thing->health > 0 && !thing_is_in_limbo(thing) &&
                    !creature_is_kept_in_custody_by_enemy_or_dying(thing) && thing_can_be_controlled_as_controller(thing)) {
                    p.possession_started = control_creature_as_controller(player, thing);
                    p.possession_turn = get_gameturn();
                    break;
                }
            }
            return;
        }
        if (get_gameturn() <= p.possession_turn || player->view_type != PVT_CreatureContrl ||
            player->view_mode != PVM_CreatureView) return;
    } else if (player->view_type != PVT_DungeonTop) return;
    p.start_state = state();
    p.start_turn = get_gameturn();
    p.active = true;
}

void performance_begin(enum PerformanceScope scope)
{
    Profile& p = profile();
    if (!p.active) return;
    Scope& s = p.scopes[scope];
    s.start = Clock::now();
    s.turn = get_gameturn();
    s.active = true;
    if (scope == PerfPresentation) {
        if (p.last_present != Clock::time_point{}) {
            if (p.samples.size() >= 100000) { fail(p, "sample limit reached"); return; }
            p.samples.push_back({PerfScopeCount, s.turn,
                static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(s.start - p.last_present).count())});
        }
        p.last_present = s.start;
    }
}

void performance_end(enum PerformanceScope scope)
{
    Profile& p = profile();
    if (!p.active || !p.scopes[scope].active) return;
    auto end = Clock::now();
    Scope& s = p.scopes[scope];
    s.active = false;
    if (p.samples.size() >= 100000) { fail(p, "sample limit reached"); return; }
    p.samples.push_back({scope, s.turn,
        static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(end - s.start).count())});
}

void performance_renderer_info(const char* renderer, const char* driver, int width, int height,
    int output_width, int output_height, int vsync)
{
    Profile& p = profile();
    if (!p.active) return;
    if (p.width && (p.width != width || p.height != height || p.output_width != output_width ||
        p.output_height != output_height || p.vsync != vsync || p.renderer != (renderer ? renderer : "unknown") ||
        p.driver != (driver ? driver : "unknown"))) {
        fail(p, "presentation configuration changed during capture"); return;
    }
    p.renderer = renderer ? renderer : "unknown";
    p.driver = driver ? driver : "unknown";
    p.width = width; p.height = height;
    p.output_width = output_width; p.output_height = output_height; p.vsync = vsync;
}

int performance_requested(void) { return profile().output && *profile().output; }

int performance_active(void) { return profile().active; }

void performance_failed(const char* reason)
{
    if (profile().active) fail(profile(), reason);
}
