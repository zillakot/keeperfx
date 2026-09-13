#pragma once
#include "kfx/renderer/WgpuShadow.h"
#include "bflib_render.h"
#include "bflib_video.h"
#include "bflib_vidraw.h"
#include "vidmode.h"
#include <stdlib.h>
#include <string.h>

struct KfxShadowSprite {
    const uint8_t *data;
    int clear_width, clear_height, width, height, x, y, flip;
};

static int kfx_wgpu_shadow_sprite(const struct KfxShadowSprite *sprite,
    const struct PolyPoint vertices[4], uint8_t *scratch,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!kfx_wgpu_native_enabled() || sizeof(long) != 8) return 0;
    kfx_wgpu_native_flush();
    if (!sprite->data || !scratch || !poly_screen || sprite->width <= 0 || sprite->width > 256 ||
        sprite->height <= 0 || sprite->height > 256 || sprite->clear_width <= 0 ||
        sprite->clear_width > 256 || sprite->clear_height <= 0 || sprite->clear_height > 256 ||
        sprite->x < 0 || sprite->x > 256 || sprite->y < 0 || sprite->y + sprite->height > 256 ||
        256 * (sprite->y + sprite->height - 1) + sprite->x + (sprite->flip ? 0 : sprite->width - 1) >= 65536 ||
        (sprite->flip ? sprite->x + 1 < sprite->width : sprite->x + sprite->width > 256) ||
        vec_colour >= 64 || vec_screen_width > UINT32_MAX ||
        vec_window_width <= 0 || vec_window_height <= 0 || vec_window_width > 8192 ||
        vec_window_height > 8192 || vec_screen_width < (unsigned long)vec_window_width) return 0;
    const uint8_t *end = sprite->data;
    for (unsigned y = 0; y < sprite->height; y++) {
        unsigned x = 0;
        for (;;) {
            if (!kfx_wgpu_native_read_barrier(end, 1)) return 0;
            int run = (int8_t)*end++;
            if (!run) break;
            unsigned n = run < 0 ? -run : run;
            if (n > sprite->width - x) return 0;
            x += n;
            if (run > 0) end += n;
        }
    }
    if ((uintptr_t)sprite->data < (uintptr_t)scratch + 65536 && (uintptr_t)scratch < (uintptr_t)end) return 0;
    size_t rle_length = end - sprite->data;
    if (!kfx_wgpu_native_read_barrier(sprite->data, rle_length) ||
        !kfx_wgpu_native_read_barrier(scratch, 65536) ||
        !kfx_wgpu_native_read_barrier(pixmap.fade_tables, 16384) ||
        !kfx_wgpu_native_read_barrier(pixmap.ghost, 65536)) return 0;
    size_t length = 65688 + rle_length;
    uint8_t *asset = malloc(length), *tables = malloc(81920);
    if (!asset || !tables) { free(asset); free(tables); return 0; }
    const uint32_t descriptor[] = {sprite->clear_width, sprite->clear_height, sprite->width,
        sprite->height, sprite->x, sprite->y, sprite->flip, rle_length};
    for (unsigned i = 0; i < 8; i++) for (unsigned j = 0; j < 4; j++) asset[i * 4 + j] = descriptor[i] >> (8 * j);
    const unsigned order[] = {0, 1, 2, 0, 2, 3};
    int valid = 1;
    for (unsigned i = 0; i < 6; i++) {
        const struct PolyPoint *v = &vertices[order[i]];
        if (v->X < -32767 || v->X > 32767 || v->Y < -32767 || v->Y > 32767 ||
            v->U < -0x04000000 || v->U > 0x04000000 || v->V < -0x04000000 || v->V > 0x04000000) valid = 0;
        const uint32_t fields[] = {v->X, v->Y, v->U, v->V, 0};
        for (unsigned j = 0; j < 5; j++) for (unsigned k = 0; k < 4; k++)
            asset[32 + i * 20 + j * 4 + k] = fields[j] >> (8 * k);
    }
    if (!valid) { free(asset); free(tables); return 0; }
    memcpy(asset + 152, scratch, 65536);
    memcpy(asset + 65688, sprite->data, rle_length);
    memcpy(tables, pixmap.fade_tables, 16384);
    memcpy(tables + 16384, pixmap.ghost, 65536);
    const struct KfxWgpuNativeResource source = {asset, length, 1, 1, 1};
    const struct KfxWgpuNativeResource table = {tables, 81920, 256, 320, 256};
    const struct KfxGpolyTarget target = {poly_screen + vec_screen_width,
        vec_window_width, vec_window_height, vec_screen_width};
    struct KfxWgpuDrawCommand command = {0};
    command.abi_version = 1; command.kind = KFX_WGPU_DRAW_SHADOW; command.colour = vec_colour;
    command.width = command.clip_width = target.width;
    command.height = command.clip_height = target.height;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    int accepted = kfx_wgpu_native_shadow(&target, &command, &source, &table, scratch, oracle, context);
    free(asset); free(tables);
    return accepted == 1;
}
