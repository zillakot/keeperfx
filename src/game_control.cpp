#include "pre_inc.h"
#include "game_control.h"
#include "bflib_video.h"
#include "bflib_inputctrl.h"
#include "bflib_mouse.h"
#include "frontend.h"
#include "front_input.h"
#include "kjm_input.h"
#include "game_legacy.h"
#include "player_data.h"
#include "kfx/renderer/RendererManager.h"
#include <algorithm>
#include <array>
#include <cstring>
#include <cstdlib>
#include "post_inc.h"

namespace {
struct Step { SDL_Event event; unsigned delay; };
std::array<Step, 64> steps;
size_t step_count = 0, step_index = 0;
unsigned delay = 0;
uint64_t ticks = 0, deadline = 0, command_id = 0, completed = 0;
std::array<SDL_Event, 8> held;
size_t held_count = 0;
bool cancel_pending = false;
const char *last_error = "";

bool integer(VALUE *request, const char *name, int low, int high, int &out)
{
    VALUE *v = value_dict_get(request, name);
    if (value_type(v) != VALUE_INT32) return false;
    out = value_int32(v);
    return out >= low && out <= high;
}
void append(SDL_Event event, unsigned frames)
{
    steps[step_count++] = {event, frames};
}
SDL_Event motion(int x, int y)
{
    SDL_Event event = {};
    event.type = SDL_EVENT_MOUSE_MOTION;
    event.motion.x = x;
    event.motion.y = y;
    return event;
}
SDL_Event button(int x, int y, int which, bool down)
{
    SDL_Event event = {};
    event.type = down ? SDL_EVENT_MOUSE_BUTTON_DOWN : SDL_EVENT_MOUSE_BUTTON_UP;
    event.button.button = which;
    event.button.down = down;
    event.button.x = x;
    event.button.y = y;
    return event;
}
void release(void (*dispatch)(const SDL_Event *, bool))
{
    while (held_count) {
        SDL_Event event = held[--held_count];
        if (event.type == SDL_EVENT_KEY_DOWN) {
            event.type = SDL_EVENT_KEY_UP;
            event.key.down = false;
            event.key.mod = SDL_KMOD_NONE;
        } else {
            event.type = SDL_EVENT_MOUSE_BUTTON_UP;
            event.button.down = false;
        }
        dispatch(&event, true);
    }
}
void state(VALUE *out)
{
    value_init_dict(out);
    int width = 0, height = 0, pixel_width = 0, pixel_height = 0;
    SDL_GetWindowSize(lbWindow, &width, &height);
    SDL_GetWindowSizeInPixels(lbWindow, &pixel_width, &pixel_height);
    value_init_string(value_dict_add(out, "session"), getenv("KFX_DEV_CONTROL_SESSION"));
    value_init_string(value_dict_add(out, "error"), last_error);
    value_init_uint64(value_dict_add(out, "tick"), ticks);
    value_init_uint64(value_dict_add(out, "command"), command_id);
    value_init_uint64(value_dict_add(out, "completed"), completed);
    value_init_bool(value_dict_add(out, "busy"), step_index < step_count || cancel_pending);
    value_init_int32(value_dict_add(out, "frontend"), frontend_menu_state);
    value_init_uint32(value_dict_add(out, "turn"), get_gameturn());
    value_init_int32(value_dict_add(out, "view"), get_my_player()->view_type);
    value_init_bool(value_dict_add(out, "paused"), (game.operation_flags & GOF_Paused) != 0);
    value_init_bool(value_dict_add(out, "focused"), (SDL_GetWindowFlags(lbWindow) & SDL_WINDOW_INPUT_FOCUS) != 0);
    value_init_bool(value_dict_add(out, "grabbed"), SDL_GetWindowRelativeMouseMode(lbWindow) || SDL_GetWindowMouseGrab(lbWindow));
    value_init_bool(value_dict_add(out, "fullscreen"), (SDL_GetWindowFlags(lbWindow) & SDL_WINDOW_FULLSCREEN) != 0);
    value_init_bool(value_dict_add(out, "minimized"), (SDL_GetWindowFlags(lbWindow) & SDL_WINDOW_MINIMIZED) != 0);
    value_init_int32(value_dict_add(out, "width"), width);
    value_init_int32(value_dict_add(out, "height"), height);
    value_init_int32(value_dict_add(out, "pixel_width"), pixel_width);
    value_init_int32(value_dict_add(out, "pixel_height"), pixel_height);
    value_init_int32(value_dict_add(out, "game_width"), LbScreenWidth());
    value_init_int32(value_dict_add(out, "game_height"), LbScreenHeight());
    value_init_int32(value_dict_add(out, "mouse_x"), GetMouseX());
    value_init_int32(value_dict_add(out, "mouse_y"), GetMouseY());
    value_init_string(value_dict_add(out, "presenter"), RendererGetPresenterName());
}
}

const char *game_control_token(void)
{
    static const char *token = []() -> const char * {
        const char *s = getenv("KFX_DEV_CONTROL_TOKEN");
        if (!s || strlen(s) != 64) return nullptr;
        for (const char *p = s; *p; ++p)
            if (!((*p >= '0' && *p <= '9') || (*p >= 'a' && *p <= 'f'))) return nullptr;
        const char *id = getenv("KFX_DEV_CONTROL_SESSION");
        if (!id || strlen(id) != 32) return nullptr;
        for (const char *p = id; *p; ++p)
            if (!((*p >= '0' && *p <= '9') || (*p >= 'a' && *p <= 'f'))) return nullptr;
        return s;
    }();
    return token;
}
bool game_control_enabled(void) { return game_control_token() != nullptr; }
void game_control_disconnect(void) { cancel_pending = true; }

const char *game_control_request(VALUE *request, VALUE *response)
{
    if (!game_control_enabled()) return "CONTROL_DISABLED";
    const char *op = value_string(value_dict_get(request, "op"));
    if (!op) return "MISSING_OP";
    if (!lbWindow) return "NO_WINDOW";
    if (!strcmp(op, "state")) { state(response); return nullptr; }
    if (!strcmp(op, "cancel")) { cancel_pending = true; state(response); return nullptr; }
    if (step_index < step_count || cancel_pending) return "CONTROL_BUSY";
    int frames = 3;
    if (value_dict_get(request, "frames") && !integer(request, "frames", 2, 120, frames)) return "INVALID_FRAMES";
    step_count = step_index = delay = 0;
    if (!strcmp(op, "move") || !strcmp(op, "click") || !strcmp(op, "drag")) {
        int w = 0, h = 0, x, y, b = SDL_BUTTON_LEFT;
        SDL_GetWindowSize(lbWindow, &w, &h);
        if (!integer(request, "x", 0, w - 1, x) || !integer(request, "y", 0, h - 1, y)) return "INVALID_POSITION";
        if (value_dict_get(request, "button") && !integer(request, "button", 1, 3, b)) return "INVALID_BUTTON";
        int end_x = x, end_y = y;
        if (!strcmp(op, "drag") && (!integer(request, "to_x", 0, w - 1, end_x) || !integer(request, "to_y", 0, h - 1, end_y))) return "INVALID_POSITION";
        append(motion(x, y), 2);
        if (strcmp(op, "move")) {
            append(button(x, y, b, true), frames);
            if (!strcmp(op, "drag")) {
                for (int i = 1; i <= 8; ++i)
                    append(motion(x + (end_x-x)*i/8, y + (end_y-y)*i/8), 2);
            }
            append(button(end_x, end_y, b, false), 2);
        }
    } else if (!strcmp(op, "key") || !strcmp(op, "chord") || !strcmp(op, "cycle-mode")) {
        VALUE *keys = value_dict_get(request, "keys");
        SDL_Keycode codes[4] = {};
        size_t n = 0;
        if (!strcmp(op, "cycle-mode")) { codes[0] = SDLK_LALT; codes[1] = SDLK_R; n = 2; }
        else if (!strcmp(op, "key")) {
            const char *name = value_string(value_dict_get(request, "key"));
            if (!name) return "INVALID_KEY";
            codes[0] = SDL_GetKeyFromName(name); n = 1;
        } else {
            if (value_type(keys) != VALUE_ARRAY || (n = value_array_size(keys)) < 1 || n > 4) return "INVALID_KEYS";
            for (size_t i = 0; i < n; ++i) {
                const char *name = value_string(value_array_get(keys, i));
                if (!name) return "INVALID_KEY";
                codes[i] = SDL_GetKeyFromName(name);
            }
        }
        SDL_Keymod mods = SDL_KMOD_NONE;
        for (size_t i = 0; i < n; ++i) {
            if (!codes[i]) return "INVALID_KEY";
            for (size_t j = 0; j < i; ++j) if (codes[j] == codes[i]) return "DUPLICATE_KEY";
            if (codes[i] == SDLK_LALT || codes[i] == SDLK_RALT) mods |= SDL_KMOD_ALT;
            if (codes[i] == SDLK_LCTRL || codes[i] == SDLK_RCTRL) mods |= SDL_KMOD_CTRL;
            if (codes[i] == SDLK_LSHIFT || codes[i] == SDLK_RSHIFT) mods |= SDL_KMOD_SHIFT;
        }
        for (size_t i = 0; i < n; ++i) {
            SDL_Event event = {};
            event.type = SDL_EVENT_KEY_DOWN;
            event.key.key = codes[i]; event.key.down = true; event.key.mod = mods;
            append(event, i + 1 == n ? frames : 0);
        }
        for (size_t i = n; i > 0; --i) {
            SDL_Event event = {};
            event.type = SDL_EVENT_KEY_UP;
            event.key.key = codes[i-1]; event.key.mod = SDL_KMOD_NONE;
            append(event, i == 1 ? 2 : 0);
        }
    } else if (!strcmp(op, "wait")) {
        append({}, frames);
    } else if (!strcmp(op, "resize")) {
        int w, h;
        if (!integer(request, "width", 320, 4096, w) || !integer(request, "height", 200, 2160, h)) return "INVALID_SIZE";
        if (SDL_GetWindowFlags(lbWindow) & SDL_WINDOW_FULLSCREEN) return "RESIZE_REQUIRES_WINDOWED";
        if (!SDL_SetWindowSize(lbWindow, w, h)) return "WINDOW_OPERATION_FAILED";
        append({}, 3);
    } else if (!strcmp(op, "minimize") || !strcmp(op, "restore") || !strcmp(op, "focus")) {
        bool ok = !strcmp(op, "minimize") ? SDL_MinimizeWindow(lbWindow) :
            !strcmp(op, "restore") ? SDL_RestoreWindow(lbWindow) : SDL_RaiseWindow(lbWindow);
        if (!ok) return "WINDOW_OPERATION_FAILED";
        append({}, 3);
    } else if (!strcmp(op, "snapshot")) {
        char path[80];
        snprintf(path, sizeof(path), "scrshots/control-%llu.png", (unsigned long long)(command_id + 1));
        if (!RendererScheduleScreenshot(path, 1)) return "SNAPSHOT_FAILED";
        append({}, 3);
    } else if (!strcmp(op, "quit")) {
        SDL_Event event = {}; event.type = SDL_EVENT_QUIT; append(event, 0);
    } else return "UNKNOWN_CONTROL_OP";
    ++command_id;
    last_error = "";
    deadline = SDL_GetTicks() + 15000;
    state(response);
    return nullptr;
}

void game_control_tick(void (*dispatch)(const SDL_Event *, bool))
{
    if (!game_control_enabled()) return;
    ++ticks;
    if (cancel_pending || (step_index < step_count && SDL_GetTicks() > deadline)) {
        if (step_index < step_count)
            last_error = cancel_pending ? "CANCELLED" : "CONTROL_TIMEOUT";
        release(dispatch);
        step_index = step_count;
        completed = command_id;
        cancel_pending = false;
        return;
    }
    if (delay) { --delay; return; }
    while (step_index < step_count) {
        const Step &step = steps[step_index++];
        SDL_Event event = step.event;
        if (event.type == SDL_EVENT_KEY_DOWN || event.type == SDL_EVENT_MOUSE_BUTTON_DOWN)
            held[held_count++] = event;
        if (event.type == SDL_EVENT_KEY_UP || event.type == SDL_EVENT_MOUSE_BUTTON_UP) {
            for (size_t i = 0; i < held_count; ++i) {
                if ((event.type == SDL_EVENT_KEY_UP && held[i].type == SDL_EVENT_KEY_DOWN && event.key.key == held[i].key.key) ||
                    (event.type == SDL_EVENT_MOUSE_BUTTON_UP && held[i].type == SDL_EVENT_MOUSE_BUTTON_DOWN && event.button.button == held[i].button.button)) {
                    held[i] = held[--held_count]; break;
                }
            }
        }
        if (event.type) dispatch(&event, true);
        delay = step.delay;
        if (delay) return;
    }
    completed = command_id;
}
