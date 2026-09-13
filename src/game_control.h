#pragma once
#include <stdbool.h>
#include <value.h>

#ifdef __cplusplus
extern "C" {
#endif
bool game_control_enabled(void);
const char *game_control_token(void);
const char *game_control_request(VALUE *request, VALUE *response);
void game_control_disconnect(void);
#ifdef __cplusplus
}
#include <SDL3/SDL.h>
void game_control_tick(void (*dispatch)(const SDL_Event *, bool));
#endif
