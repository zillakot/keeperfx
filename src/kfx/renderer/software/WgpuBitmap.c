#include "kfx/renderer/software/WgpuBitmap.h"
#include "bflib_sprite.h"
#include <stdlib.h>
#include <string.h>

static int separate(const void *a, size_t alen, const void *b, size_t blen)
{
    uintptr_t x = (uintptr_t)a, y = (uintptr_t)b;
    return x < y ? alen <= y - x : blen <= x - y;
}

static void word(uint8_t *dst, uint32_t value)
{
    for (int i = 0; i < 4; i++) dst[i] = value >> (8 * i);
}

static uint32_t read_word(const uint8_t *src)
{
    return (uint32_t)src[0] | (uint32_t)src[1] << 8 | (uint32_t)src[2] << 16 | (uint32_t)src[3] << 24;
}

static int valid_target(const struct KfxGpolyTarget *t)
{
    return t->pixels && t->width > 0 && t->width <= t->pitch && t->pitch <= 8192 &&
        t->height > 0 && t->height <= 8192;
}

int kfx_wgpu_bitmap_huge(uint8_t *dst, int pitch, int height, const int32_t *xs,
    const int32_t *ys, const struct TbHugeSprite *s, size_t length,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    struct KfxGpolyTarget target = {dst, pitch, height, pitch};
    if (!kfx_wgpu_native_read_barrier(s, sizeof(*s))) return 0;
    if (!valid_target(&target) || !s ||
        !separate(s, sizeof(*s), dst, (size_t)pitch * height)) return 0;
    if (!s->Data || !s->Lines || !xs || !ys || !oracle ||
        !s->SWidth || s->SWidth > 8192 || !s->SHeight || s->SHeight > 8192 ||
        s->SWidth * s->SHeight > 1024 * 1024) return 0;
    /* Length zero is the legacy helper's trusted asset contract. */
    size_t bound = length ? length : s->SHeight * (s->SWidth * 9 + 8);
    size_t capacity = (size_t)pitch * height;
    if (!separate(s->Data, bound, dst, capacity) ||
        !separate(s->Lines, s->SHeight * sizeof(*s->Lines), dst, capacity) ||
        !separate(xs, (s->SWidth + 1) * sizeof(*xs) * 2, dst, capacity) ||
        !separate(ys, s->SHeight * sizeof(*ys) * 2, dst, capacity)) return 0;
    if (!kfx_wgpu_native_read_barrier(s->Data, bound) ||
        !kfx_wgpu_native_read_barrier(s->Lines, s->SHeight * sizeof(*s->Lines)) ||
        !kfx_wgpu_native_read_barrier(xs, (s->SWidth + 1) * sizeof(*xs) * 2) ||
        !kfx_wgpu_native_read_barrier(ys, s->SHeight * sizeof(*ys) * 2)) return 0;
    size_t records_base = s->SHeight * 16;
    uint8_t *asset = malloc(records_base + s->SWidth * s->SHeight * 12);
    if (!asset) return 0;
    size_t used = records_base;
    int output_y = 0;
    for (size_t row = 0; row < s->SHeight; row++) {
        int copies = ys[row * 2 + 1];
        if (copies < 0 || ys[row * 2] < 0 || ys[row * 2] > height) goto decline;
        if ((int64_t)ys[row * 2] + copies > height) copies = height - ys[row * 2];
        if (ys[row * 2 + 1] != 0 && copies < 1) copies = 1;
        if (output_y + copies > height) goto decline;
        word(asset + row * 16, output_y);
        word(asset + row * 16 + 4, copies);
        word(asset + row * 16 + 8, used);
        size_t first = used;
        if (!copies) { word(asset + row * 16 + 12, 0); continue; }
        if (s->Lines[row] < 0) goto decline;
        size_t at = s->Lines[row], sx = 0;
        int out_x = 0, previous_end = 0;
        while (out_x < pitch) {
            if (at > bound || bound - at < 4) goto decline;
            uint32_t solid = read_word(s->Data + at); at += 4;
            if (solid > s->SWidth - sx || at > bound || solid > bound - at) goto decline;
            for (uint32_t p = 0; p < solid; p++, sx++, at++) {
                int n = xs[sx * 2 + 1];
                if (xs[sx * 2] < 0 || n < 0) goto decline;
                if ((int64_t)xs[sx * 2] + n > pitch) n = pitch - xs[sx * 2];
                if (n <= 0) continue;
                if (out_x < previous_end || out_x < 0 || out_x + n > pitch) goto decline;
                word(asset + used, out_x); word(asset + used + 4, n);
                word(asset + used + 8, s->Data[at]); used += 12;
                out_x += n; previous_end = out_x;
            }
            if (at > bound || bound - at < 4) goto decline;
            uint32_t skip = read_word(s->Data + at); at += 4;
            if (skip > s->SWidth - sx || (!solid && !skip)) goto decline;
            int current = xs[sx * 2];
            sx += skip;
            if (sx >= s->SWidth) break;
            int64_t next = (int64_t)out_x - current + xs[sx * 2];
            if (next < previous_end || next > pitch) goto decline;
            out_x = next;
        }
        word(asset + row * 16 + 12, (used - first) / 12);
        output_y += copies;
    }
    struct KfxWgpuDrawCommand command = {0};
    command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command.kind = KFX_WGPU_DRAW_BITMAP;
    command.width = command.clip_width = pitch;
    command.height = command.clip_height = height;
    command.source_height = s->SHeight;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    struct KfxWgpuNativeResource source = {asset, used, 1, 1, 1, NULL, 0};
    int accepted = kfx_wgpu_native_draw(&target, &command, &source, NULL, oracle, context);
    free(asset);
    return accepted;
decline:
    free(asset);
    return 0;
}

int kfx_wgpu_bitmap_font(const struct KfxGpolyTarget *target, int wx, int wy,
    int ww, int wh, int x, int y, const uint8_t *bits, int sw, int sh,
    int dw, int dh, int scaled, int foreground, int background, int shadow,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    if (!valid_target(target) || !bits || !oracle || wx < 0 || wy < 0 || ww < 1 || wh < 1 ||
        (int64_t)wx + ww > target->width || (int64_t)wy + wh > target->height ||
        sw < 1 || sw > 256 || sh < 1 || sh > 256 || dw < 0 || dh < 0 || (int64_t)dw * dh > 8192 ||
        dw > 8192 || dh > 8192 || x < -16384 || x > 16384 || y < -16384 || y > 16384) return 0;
    if (dw == 0 || dh == 0) return 1;
    size_t bytes = ((sw + 7) / 8) * sh;
    if (!separate(bits, bytes, target->pixels, (size_t)target->pitch * target->height)) return 0;
    if (!kfx_wgpu_native_read_barrier(bits, bytes)) return 0;
    uint8_t *asset = malloc(bytes + 12);
    if (!asset) return 0;
    word(asset, (foreground & 0xff00) ? 256 : foreground);
    word(asset + 4, (background & 0xff00) ? 256 : background);
    word(asset + 8, shadow < 0 ? 256 : shadow);
    memcpy(asset + 12, bits, bytes);
    struct KfxWgpuDrawCommand c = {0};
    c.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    c.kind = KFX_WGPU_DRAW_BITMAP;
    c.width = target->width; c.height = target->height;
    c.clip_x = wx; c.clip_y = wy; c.clip_width = ww; c.clip_height = wh;
    c.source_x = scaled ? 2 : 1;
    c.source_y = (y + 1 < 0 || y + 1 + dh > wh);
    c.source_width = sw; c.source_height = sh;
    c.start_low = wx + x; c.start_high = wy + y;
    c.step_low = dw; c.step_high = dh;
    c.transparent = KFX_WGPU_DRAW_OPAQUE;
    struct KfxWgpuNativeResource source = {asset, bytes + 12, 1, 1, 1, NULL, 0};
    int accepted = kfx_wgpu_native_draw(target, &c, &source, NULL, oracle, context);
    free(asset);
    return accepted;
}
