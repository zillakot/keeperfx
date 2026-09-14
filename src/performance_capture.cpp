#include "pre_inc.h"
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <sys/resource.h>
#endif
#include "performance_capture.h"
#include "keeperfx.hpp"
#include "creature_states.h"
#include "game_legacy.h"
#include "config_keeperfx.h"
#include "packets.h"
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

#ifdef KFX_RUST_PRESENTER
extern "C" void kfx_wgpu_allocation_counts(uint64_t* allocations, uint64_t* requested_bytes);
#endif

namespace {
using Clock = std::chrono::steady_clock;
struct Sample { int scope; unsigned long turn; uint64_t ns; };
struct Scope { Clock::time_point start; unsigned long turn; bool active = false; uint64_t total_ns = 0; };
struct Resources {
    Clock::time_point wall;
    uint64_t user_ns = 0, system_ns = 0, allocations = 0, allocated_bytes = 0;
    bool cpu_available = false;
};

Resources resources()
{
    Resources r;
    r.wall = Clock::now();
#ifdef _WIN32
    FILETIME created, exited, kernel, user;
    r.cpu_available = GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user) != 0;
    if (r.cpu_available) {
        r.user_ns = ((static_cast<uint64_t>(user.dwHighDateTime) << 32) | user.dwLowDateTime) * 100;
        r.system_ns = ((static_cast<uint64_t>(kernel.dwHighDateTime) << 32) | kernel.dwLowDateTime) * 100;
    }
#else
    struct rusage usage;
    r.cpu_available = getrusage(RUSAGE_SELF, &usage) == 0;
    if (r.cpu_available) {
        r.user_ns = static_cast<uint64_t>(usage.ru_utime.tv_sec) * 1000000000 + usage.ru_utime.tv_usec * 1000;
        r.system_ns = static_cast<uint64_t>(usage.ru_stime.tv_sec) * 1000000000 + usage.ru_stime.tv_usec * 1000;
    }
#endif
#ifdef KFX_RUST_PRESENTER
    kfx_wgpu_allocation_counts(&r.allocations, &r.allocated_bytes);
#endif
    return r;
}

std::string json_quote(const std::string& value)
{
    std::string result = "\"";
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') result += std::string("\\") + static_cast<char>(c);
        else if (c < 0x20) {
            char escaped[7];
            std::snprintf(escaped, sizeof(escaped), "\\u%04x", c);
            result += escaped;
        } else result += static_cast<char>(c);
    }
    return result + '"';
}

constexpr int DrawingCounterCount = sizeof(PerformanceDrawingCounters) / sizeof(unsigned long long);
constexpr int DrawingGaugeCount = 9;
/* Names must stay in PerformanceDrawingCounters member order; gauges come last. */
const char* const drawing_counter_names[DrawingCounterCount] = {
    "submits", "dispatches", "waits", "wait_ns", "checkpoints", "checkpoint_copy_bytes",
    "validation_waits", "flagged_invalid_frames", "status_stalls",
    "asset_upload_bytes", "command_upload_bytes", "upload_bytes", "readback_bytes", "full_readbacks", "full_readback_bytes",
    "buffers", "buffer_bytes", "batches", "commands", "ordered_sprites",
    "ordered_sprite_layers", "ordered_sprite_passes",
    "arena_evictions", "arena_overflows", "arena_bytes_uploaded",
    "tile_allocations", "tile_entries", "bridge_target_flushes", "bridge_target_runs",
    "terrain_tile_entries", "prepared_row_words", "prepared_row_allocations",
    "tile_entries_clear", "tile_entries_rect", "tile_entries_image", "tile_entries_gpoly_span",
    "tile_entries_circle_filled", "tile_entries_circle_outline", "tile_entries_sprite", "tile_entries_raw_image",
    "tile_entries_tiled_image", "tile_entries_trig", "tile_entries_lens", "tile_entries_shadow",
    "tile_entries_reserved12", "tile_entries_movie", "tile_entries_map_view", "tile_entries_bitmap",
    "tile_entries_transition", "tile_entries_terrain_tri",
    "gpu_raster_ns", "gpu_terrain_prepare_ns",
    "gpu_shadow_mask_ns", "gpu_target_trig_ns", "gpu_ordered_sprite_ns",
    "gpu_minimap_ns", "gpu_lens_ns", "gpu_present_ns",
    "gpu_timed_passes", "gpu_untimed_passes", "gpu_pass_union_ns",
    "target_trig_geometry_bytes",
    "target_trig_table_bytes",
    "other_asset_upload_bytes",
    "target_trig_table_hits",
    "target_trig_table_misses",
    "target_trig_asset_buffers",
    "shadow_pairs",
    "preparer_buffers",
    "preparer_buffer_bytes",
    "arena_misses_new_id",
    "arena_misses_forget",
    "arena_misses_size_class",
    "arena_misses_generation",
    "arena_misses_eviction",
    "arena_miss_new_id_bytes",
    "arena_miss_forget_bytes",
    "arena_miss_size_class_bytes",
    "arena_miss_generation_bytes",
    "arena_miss_eviction_bytes",
    "arena_explicit_forgets",
#define KFX_ARENA_FIELDS(kind) "arena_" #kind "_bytes", "arena_" #kind "_misses", "arena_" #kind "_hits", "arena_" #kind "_source_bytes", "arena_" #kind "_distinct_lengths", "arena_" #kind "_length_overflows",
    KFX_ARENA_KINDS(KFX_ARENA_FIELDS)
#undef KFX_ARENA_FIELDS
    "arena_trig_texture_source_bytes",
    "arena_minimap_prefix_bytes",
    "arena_minimap_dictionary_bytes",
    "arena_minimap_cells_bytes",
    "arena_minimap_styles_bytes",
    "minimap_dictionary_hits",
    "minimap_dictionary_misses",
    "minimap_cells_hits",
    "minimap_cells_misses",
    "minimap_styles_hits",
    "minimap_styles_misses",
    "host_staged_asset_bytes", "arena_bytes_resident", "arena_scratch_bytes_peak",
    "arena_capacity_bytes",
    "arena_live_bytes",
    "arena_retired_bytes",
    "arena_growth_peak_bytes", "minimap_cache_class_bytes", "minimap_cache_cpu_bytes"};
static_assert(sizeof(PerformanceDrawingCounters) == DrawingCounterCount * sizeof(unsigned long long),
    "drawing counters must be a packed array of unsigned long long");

struct Profile {
    const char* output = std::getenv("KFX_PERF_OUTPUT");
    unsigned long warmup = 40, turns = 200, start_turn = 0;
    bool initialized = false, active = false, finished = false, possession = false, draw_breakdown = false;
    bool possession_started = false;
    unsigned long possession_turn = 0;
    FILE* file = nullptr;
    std::vector<Sample> samples;
    std::array<Scope, PerfScopeCount> scopes;
    Clock::time_point last_present;
    std::string start_state, renderer, driver, renderer_details;
    std::string drawing_backend;
    bool drawing_seen = false;
    PerformanceDrawingCounters drawing_previous = {};
    std::vector<std::array<unsigned long long, DrawingCounterCount>> drawing_frames;
    std::vector<PerformancePresenterCounters> presenter_frames;
    Resources start_resources;
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
    for (const Scope& scope : p.scopes) {
        if (scope.active) { fail(p, "unfinished timing scope"); return; }
    }
    p.active = false;
    const Resources end_resources = resources();
    const bool cpu_available = p.start_resources.cpu_available && end_resources.cpu_available;
    const uint64_t wall_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(end_resources.wall - p.start_resources.wall).count();

    std::fprintf(p.file, "kind,turn,wall_ns\n");
    for (const Sample& s : p.samples)
        std::fprintf(p.file, "%s,%lu,%llu\n", PerformanceScopeNames[s.scope], s.turn, static_cast<unsigned long long>(s.ns));
    bool failed = std::ferror(p.file) != 0;
    failed = std::fclose(p.file) != 0 || failed;
    p.file = nullptr;
    if (failed) { fail(p, "writing samples"); return; }
    std::string path = std::string(p.output) + ".json";
    FILE* info = std::fopen(path.c_str(), "wx");
    if (!info) { fail(p, "metadata already exists or cannot be created"); return; }
    std::fprintf(info,
        "{\"format\":\"KFXPERF01\",\"complete\":true,\"draw_breakdown\":%s,\"start\":%s,\"end\":%s,"
        "\"scene\":\"%s\",\"view\":\"%s\",\"renderer\":%s,\"video_driver\":%s,\"renderer_details\":%s,"
        "\"width\":%d,\"height\":%d,\"output_width\":%d,\"output_height\":%d,"
        "\"vsync_actual\":%d,\"turns_per_second\":%ld,\"fps_limit\":%d,\"interpolation\":%s,"
        "\"resources\":{\"wall_ns\":%llu,\"process_cpu\":{\"available\":%s,\"source\":\"%s\",\"user_ns\":%llu,\"system_ns\":%llu},"
        "\"rust_allocations\":{\"available\":%s,\"calls\":%llu,\"requested_bytes\":%llu}}",
        p.draw_breakdown ? "true" : "false", p.start_state.c_str(), state().c_str(), p.possession ? "possession" : "dungeon",
        p.possession ? "creature" : "dungeon_top", json_quote(p.renderer).c_str(), json_quote(p.driver).c_str(), json_quote(p.renderer_details).c_str(),
        p.width, p.height, p.output_width, p.output_height, p.vsync,
        static_cast<long>(turns_per_second), fps_limit_current, is_feature_on(Ft_DeltaTime) ? "true" : "false",
        static_cast<unsigned long long>(wall_ns), cpu_available ? "true" : "false",
#ifdef _WIN32
        "GetProcessTimes",
#else
        "getrusage(RUSAGE_SELF)",
#endif
        static_cast<unsigned long long>(cpu_available ? end_resources.user_ns - p.start_resources.user_ns : 0),
        static_cast<unsigned long long>(cpu_available ? end_resources.system_ns - p.start_resources.system_ns : 0),
#ifdef KFX_RUST_PRESENTER
        "true",
#else
        "false",
#endif
        static_cast<unsigned long long>(end_resources.allocations - p.start_resources.allocations),
        static_cast<unsigned long long>(end_resources.allocated_bytes - p.start_resources.allocated_bytes));
    std::fprintf(info, ",\"drawing\":{\"available\":%s,\"backend\":%s,\"frames\":%zu,\"counters\":[",
        p.drawing_seen ? "true" : "false",
        json_quote(p.drawing_backend.empty() ? "unknown" : p.drawing_backend).c_str(),
        p.drawing_frames.size());
    for (int i = 0; i < DrawingCounterCount; ++i)
        std::fprintf(info, "%s%s", i ? "," : "", json_quote(drawing_counter_names[i]).c_str());
    std::fprintf(info, "],\"gauges\":[");
    for (int i = DrawingCounterCount - DrawingGaugeCount; i < DrawingCounterCount; ++i)
        std::fprintf(info, "%s%s", i > DrawingCounterCount - DrawingGaugeCount ? "," : "",
            json_quote(drawing_counter_names[i]).c_str());
    std::fprintf(info, "],\"per_frame\":[");
    for (size_t frame = 0; frame < p.drawing_frames.size(); ++frame) {
        std::fprintf(info, "%s[", frame ? "," : "");
        for (int i = 0; i < DrawingCounterCount; ++i)
            std::fprintf(info, "%s%llu", i ? "," : "", p.drawing_frames[frame][i]);
        std::fprintf(info, "]");
    }
    std::fprintf(info, "]},\"replay_scope\":true,\"presenter\":{\"per_frame\":[");
    for (size_t frame = 0; frame < p.presenter_frames.size(); ++frame) {
        const auto& c = p.presenter_frames[frame];
        std::fprintf(info, "%s[%llu,%llu,%llu,%llu,%llu,%llu,%llu,%llu]", frame ? "," : "",
            c.acquire_ns, c.acquire_block_ns, c.reconfigure_count, c.present_record_ns, c.submit_ns,
            c.replay_ns, c.allocations, c.allocated_bytes);
    }
    std::fprintf(info, "]}}\n");
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
    if (quit_game || exit_keeper || get_local_packet()->action == PckA_ForceApplicationClose) {
        fail(p, "game closed before capture completed"); return;
    }
    if (!p.initialized) {
        p.initialized = true;
        const char* scene = std::getenv("KFX_PERF_SCENE");
        if ((scene && std::strcmp(scene, "dungeon") && std::strcmp(scene, "possession")) ||
            !number("KFX_PERF_TURN", 1, 600, p.warmup) || !number("KFX_PERF_TURNS", 20, 1200, p.turns) ||
            std::getenv("KFX_FRAME_CAPTURE")) {
            fail(p, "invalid options or simultaneous frame capture"); return;
        }
        const char* breakdown = std::getenv("KFX_PERF_DRAW_BREAKDOWN");
        if (breakdown && std::strcmp(breakdown, "0") && std::strcmp(breakdown, "1")) {
            fail(p, "invalid draw breakdown option"); return;
        }
        p.draw_breakdown = breakdown && !std::strcmp(breakdown, "1");
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
    p.start_resources = resources();
    p.drawing_frames.reserve(p.turns * 4);
    p.presenter_frames.reserve(p.turns * 4);
    p.active = true;
}

void performance_begin(enum PerformanceScope scope)
{
    Profile& p = profile();
    if (!p.active) return;
    if (scope < 0 || scope >= PerfScopeCount) { fail(p, "invalid timing scope"); return; }
    if (scope >= PerfDrawScene) {
        if (!p.draw_breakdown || !p.scopes[PerfDraw].active) return;
        for (int child = PerfDrawScene; child < PerfScopeCount; ++child) {
            if (p.scopes[child].active) { fail(p, "overlapping draw timing scopes"); return; }
        }
    }
    Scope& s = p.scopes[scope];
    if (s.active || ((scope == PerfPresentWait || scope == PerfReplay) && !p.scopes[PerfPresentation].active)
        || (scope == PerfReplay && p.scopes[PerfPresentWait].active)
        || (scope == PerfPresentWait && p.scopes[PerfReplay].active)) {
        fail(p, "invalid timing scope nesting"); return;
    }
    if (scope == PerfPresentation) p.scopes[PerfReplay].total_ns = 0;
    if (scope == PerfDraw) {
        for (int child = PerfDrawScene; child < PerfScopeCount; ++child) p.scopes[child].total_ns = 0;
    }
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
    if (!p.active) return;
    if (scope < 0 || scope >= PerfScopeCount) { fail(p, "invalid timing scope"); return; }
    if (scope >= PerfDrawScene && (!p.draw_breakdown || !p.scopes[PerfDraw].active)) return;
    Scope& s = p.scopes[scope];
    if (!s.active) { fail(p, "unmatched timing scope end"); return; }
    const auto end = Clock::now();
    uint64_t ns = std::chrono::duration_cast<std::chrono::nanoseconds>(end - s.start).count();
    s.active = false;
    if (scope >= PerfDrawScene || scope == PerfReplay) {
        s.total_ns += ns;
        return;
    }
    if (scope == PerfPresentation && p.scopes[PerfPresentWait].active) {
        fail(p, "unfinished present_wait scope"); return;
    }
    if (scope == PerfPresentation) {
        if (p.scopes[PerfReplay].active || p.scopes[PerfReplay].total_ns > ns) {
            fail(p, "invalid replay scope"); return;
        }
        ns -= p.scopes[PerfReplay].total_ns;
    }
    const size_t count = scope == PerfDraw && p.draw_breakdown ? 1 + PerfScopeCount - PerfDrawScene
        : scope == PerfPresentation ? 2 : 1;
    if (p.samples.size() + count > 100000) { fail(p, "sample limit reached"); return; }
    if (scope == PerfDraw && p.draw_breakdown) {
        uint64_t accounted = 0;
        for (int child = PerfDrawScene; child < PerfScopeCount; ++child) {
            if (p.scopes[child].active) { fail(p, "unfinished draw timing scope"); return; }
            accounted += p.scopes[child].total_ns;
        }
        if (accounted > ns) { fail(p, "draw timing scopes exceed parent"); return; }
        for (int child = PerfDrawScene; child < PerfScopeCount; ++child)
            p.samples.push_back({child, s.turn, p.scopes[child].total_ns});
    }
    if (scope == PerfPresentation) p.samples.push_back({PerfReplay, s.turn, p.scopes[PerfReplay].total_ns});
    p.samples.push_back({scope, s.turn, ns});
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

void performance_drawing_backend(const char* backend)
{
    Profile& p = profile();
    if (!p.active) return;
    const std::string value = backend ? backend : "unknown";
    if (p.drawing_backend.empty()) { p.drawing_backend = value; return; }
    for (size_t start = 0; start <= p.drawing_backend.size();) {
        const size_t end = p.drawing_backend.find('+', start);
        const size_t stop = end == std::string::npos ? p.drawing_backend.size() : end;
        if (p.drawing_backend.compare(start, stop - start, value) == 0) return;
        if (end == std::string::npos) break;
        start = end + 1;
    }
    p.drawing_backend += '+' + value;
}

void performance_drawing_frame(const struct PerformanceDrawingCounters* cumulative)
{
    Profile& p = profile();
    if (!p.active || !cumulative) return;
    const unsigned long long* current = reinterpret_cast<const unsigned long long*>(cumulative);
    const unsigned long long* previous = reinterpret_cast<const unsigned long long*>(&p.drawing_previous);
    if (p.drawing_seen) {
        if (p.drawing_frames.size() >= 100000) { fail(p, "drawing sample limit reached"); return; }
        std::array<unsigned long long, DrawingCounterCount> delta;
        for (int i = 0; i < DrawingCounterCount - DrawingGaugeCount; ++i) {
            if (current[i] < previous[i]) { fail(p, "drawing counter went backwards"); return; }
            delta[i] = current[i] - previous[i];
        }
        for (int i = DrawingCounterCount - DrawingGaugeCount; i < DrawingCounterCount; ++i)
            delta[i] = current[i];
        p.drawing_frames.push_back(delta);
    }
    p.drawing_previous = *cumulative;
    p.drawing_seen = true;
}

void performance_renderer_details(const char* details)
{
    Profile& p = profile();
    if (!p.active) return;
    const char* value = details ? details : "";
    if (!p.renderer_details.empty() && p.renderer_details != value) {
        fail(p, "renderer details changed during capture"); return;
    }
    p.renderer_details = value;
}

int performance_requested(void) { return profile().output && *profile().output; }

int performance_active(void) { return profile().active; }

void performance_failed(const char* reason)
{
    if (profile().active) fail(profile(), reason);
}

void performance_presenter_frame(const struct PerformancePresenterCounters* counters)
{
    Profile& p = profile();
    if (!p.active || !counters) return;
    if (p.presenter_frames.size() >= 100000) { fail(p, "presenter sample limit reached"); return; }
    p.presenter_frames.push_back(*counters);
}
