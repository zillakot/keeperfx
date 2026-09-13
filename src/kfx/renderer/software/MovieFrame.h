#pragma once
#include <stdint.h>

struct KfxMovieFrame { uint8_t *pixels; int width, height, pitch; };
struct KfxMovieTarget { uint8_t *pixels; int pitch, height, view_width, view_height; };
void kfx_movie_copy(const KfxMovieFrame &frame, const KfxMovieTarget &target, int flags);
void kfx_movie_copy_scaled(const KfxMovieFrame &frame, const KfxMovieTarget &target, int flags);
