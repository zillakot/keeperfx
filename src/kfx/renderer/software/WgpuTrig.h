#pragma once
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "front_simple.h"
#include "engine_textures.h"

static int wgpu_trig_oracle_active;

static void wgpu_trig_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    struct PolyPoint *vertices = context;
    unsigned char *saved_poly = poly_screen, *saved_vec = vec_screen;
    unsigned long saved_pitch = vec_screen_width;
    poly_screen = pixels - pitch;
    vec_screen = pixels;
    vec_screen_width = pitch;
    wgpu_trig_oracle_active = 1;
    trig(&vertices[0], &vertices[1], &vertices[2]);
    wgpu_trig_oracle_active = 0;
    poly_screen = saved_poly;
    vec_screen = saved_vec;
    vec_screen_width = saved_pitch;
}

static int wgpu_trig(struct PolyPoint *a, struct PolyPoint *b, struct PolyPoint *c)
{
    if (sizeof(long) != 8 || wgpu_trig_oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    const int textured = vec_mode == 2 || vec_mode == 3 || vec_mode == 7 || vec_mode == 8 ||
        vec_mode == 11 || vec_mode == 12 || vec_mode == 13 || vec_mode == 18 || vec_mode == 19 ||
        vec_mode == 22 || vec_mode == 23 || vec_mode == 5 || vec_mode == 6 || vec_mode == 9 ||
        vec_mode == 20 || vec_mode == 21 || vec_mode == 24 || vec_mode == 25 || vec_mode == 26;
    if (textured && vec_map == big_scratch) return 0;
    const int shaded = vec_mode == 1 || vec_mode == 4 || vec_mode == 16 || vec_mode == 17 ||
        vec_mode == 5 || vec_mode == 6 || vec_mode == 20 || vec_mode == 21 ||
        vec_mode == 24 || vec_mode == 25 || vec_mode == 26;
    if (!(textured || shaded || vec_mode == 0 || vec_mode == 14 || vec_mode == 15)) return 0;
    if (vec_window_width <= 0 || vec_window_height <= 0 || vec_window_width > 8192 ||
        vec_window_height > 8192 || vec_screen_width < (unsigned long)vec_window_width ||
        vec_screen_width > UINT32_MAX || poly_screen == NULL || (textured && vec_map == NULL)) return 0;
    if ((vec_mode == 7 || vec_mode == 8 || vec_mode == 11) && vec_colour >= 64) return 0;
    struct PolyPoint vertices[] = {*a, *b, *c};
    for (unsigned i = 0; i < 3; i++) {
        if (vertices[i].X < -32767 || vertices[i].X > 32767 ||
            vertices[i].Y < -32767 || vertices[i].Y > 32767) return 0;
        if (textured && (vertices[i].U < -0x04000000 || vertices[i].U > 0x04000000 ||
            vertices[i].V < -0x04000000 || vertices[i].V > 0x04000000)) return 0;
        if (shaded && (vertices[i].S < -0x04000000 || vertices[i].S > 0x04000000)) return 0;
    }
    for (unsigned i = 0; i < 3; i++) for (unsigned j = i + 1; j < 3; j++) {
        if (labs(vertices[i].X - vertices[j].X) > 32767 ||
            labs(vertices[i].Y - vertices[j].Y) > 32767) return 0;
    }
    size_t texture_length = 0;
    if (textured) {
        const uintptr_t address = (uintptr_t)vec_map, base = (uintptr_t)block_mem;
        if (address < base || address - base >= sizeof(block_mem)) return 0;
        texture_length = sizeof(block_mem) - (address - base);
        if (texture_length > 65536) texture_length = 65536;
    }
    if (!kfx_wgpu_native_read_barrier(vec_map, texture_length) ||
        !kfx_wgpu_native_read_barrier(pixmap.fade_tables, 16384) ||
        !kfx_wgpu_native_read_barrier(pixmap.ghost, 65536)) return 0;
    uint8_t *source = malloc(60 + texture_length);
    if (source == NULL) return 0;
    for (unsigned i = 0; i < 3; i++) {
        const uint32_t fields[] = {vertices[i].X, vertices[i].Y,
            textured ? vertices[i].U : 0, textured ? vertices[i].V : 0, shaded ? vertices[i].S : 0};
        for (unsigned j = 0; j < 5; j++) for (unsigned k = 0; k < 4; k++)
            source[i * 20 + j * 4 + k] = fields[j] >> (k * 8);
    }
    if (textured) memcpy(source + 60, vec_map, texture_length);
    const struct KfxWgpuNativeResource source_resource = {source, 60 + texture_length, 1, 1, 1,
        NULL, 0};
    const struct KfxWgpuNativeResource table_resource = {pixmap.fade_tables, 16384, 256, 320, 256,
        pixmap.ghost, 65536};
    const struct KfxGpolyTarget target = {poly_screen + vec_screen_width,
        vec_window_width, vec_window_height, vec_screen_width};
    struct KfxWgpuDrawCommand command = {0};
    command.abi_version = 1;
    command.kind = KFX_WGPU_DRAW_TRIG;
    command.colour = vec_colour;
    command.width = command.clip_width = target.width;
    command.height = command.clip_height = target.height;
    command.source_x = vec_mode;
    command.source_y = texture_length;
    command.source_width = 64;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    int result = kfx_wgpu_native_draw(&target, &command, &source_resource, &table_resource,
        wgpu_trig_oracle, vertices);
    free(source);
    return result == 1;
}
