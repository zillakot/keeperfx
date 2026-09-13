#include "kfx/renderer/software/WgpuSprite.h"
#include "kfx/renderer/software/SwDrawTarget.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/RendererManager.h"
#include "bflib_render.h"
#include <stdlib.h>
#include <string.h>

struct SpriteOracle {
    long x, y;
    const struct TbSourceBuffer *source;
    const struct TbSprite *sprite;
    const TbPixel *remap;
    TbPixel colour;
    unsigned mode;
};
static int oracle_active;

static void sprite_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    const struct SpriteOracle *o = context;
    TbPixel *screen = lbDisplay.WScreen, *window = lbDisplay.GraphicsWindowPtr;
    unsigned long old_pitch = lbDisplay.GraphicsScreenWidth;
    lbDisplay.WScreen = pixels;
    lbDisplay.GraphicsWindowPtr = pixels + SwTargetWindowY() * pitch + SwTargetWindowX();
    lbDisplay.GraphicsScreenWidth = pitch;
    oracle_active = 1;
    switch (o->mode) {
    case 0: LbSpriteDrawUsingScalingData(o->x, o->y, o->source); break;
    case 1: LbSpriteDrawRemapUsingScalingData(o->x, o->y, o->source, o->remap); break;
    case 2: LbSpriteDrawOneColourUsingScalingData(o->x, o->y, o->sprite, o->colour); break;
    case 3: DrawAlphaSpriteUsingScalingData(o->x, o->y, o->source); break;
    case 4: LbSpriteDrawImmediate(o->x, o->y, o->sprite); break;
    case 5: LbSpriteDrawOneColourImmediate(o->x, o->y, o->sprite, o->colour); break;
    }
    oracle_active = 0;
    lbDisplay.WScreen = screen;
    lbDisplay.GraphicsWindowPtr = window;
    lbDisplay.GraphicsScreenWidth = old_pitch;
}

static void put_u32(uint8_t *bytes, uint32_t value)
{
    for (unsigned i = 0; i < 4; i++) bytes[i] = value >> (8 * i);
}

int kfx_wgpu_sprite(long posx, long posy, const struct TbSourceBuffer *source,
    const struct TbSprite *sprite, const TbPixel *remap, TbPixel colour, unsigned mode)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_terrain_boundary(0);
    struct TbSourceBuffer converted;
    if (sprite) {
        converted = (struct TbSourceBuffer){sprite->Data, sprite->SWidth, sprite->SHeight, sprite->SWidth};
        source = &converted;
    }
    int pitch = SwTargetScanline(), height = SwTargetScreenHeight();
    if (!source || !source->data || !source->width || !source->height ||
        source->width > SPRITE_SCALING_XSTEPS || source->height > SPRITE_SCALING_YSTEPS ||
        pitch <= 0 || height <= 0 || !SwTargetWScreen() || SwTargetWindowX() < 0 ||
        SwTargetWindowY() < 0 || SwTargetWindowWidth() <= 0 || SwTargetWindowHeight() <= 0 ||
        SwTargetWindowX() + SwTargetWindowWidth() > pitch ||
        SwTargetWindowY() + SwTargetWindowHeight() > height ||
        SwTargetGraphicsWindowPtr() != SwTargetWScreen() +
            (ptrdiff_t)SwTargetWindowY() * pitch + SwTargetWindowX()) return 0;
    unsigned flags = RendererGetDrawFlags();
    unsigned flip = ((flags & Lb_SPRITE_FLIP_HORIZ) ? 1 : 0) |
        ((flags & Lb_SPRITE_FLIP_VERTIC) ? 2 : 0);
    unsigned blend = (flags & Lb_SPRITE_TRANSPAR4) ? 1 : ((flags & Lb_SPRITE_TRANSPAR8) ? 2 : 0);
    const uint8_t *table = mode >= 4 ? lbDisplay.GlassMap : render_ghost;
    if (mode == 0 && (flags & Lb_SPRITE_REMAP)) { remap = lbSpriteReMapPtr; blend = 0; }
    if (mode == 3) { blend = 1; table = render_alpha; }
    if ((blend && !table) || (mode == 1 && !remap)) return 0;
    unsigned w = source->width, h = source->height;
    if (mode < 4 && (posx < 0 || posy < 0 || posx + w > SPRITE_SCALING_XSTEPS ||
        posy + h > SPRITE_SCALING_YSTEPS)) return 0;
    size_t axis = (size_t)w * h * 2;
    size_t length = axis + (w + h) * 8 + 256;
    if (length > 16 * 1024 * 1024) return 0;
    uint8_t *asset = calloc(length, 1);
    if (!asset) return 0;
    int valid = 1;
    unsigned solid_rl = mode < 4 && scale_up && !blend && (flip & 1);
    unsigned ordered = 0;
    for (unsigned a = 0; a < 2; a++) {
        unsigned n = a ? h : w;
        int limit = a ? SwTargetWindowHeight() : SwTargetWindowWidth();
        long pos = a ? posy : posx;
        const int32_t *steps = (a ? ysteps_array : xsteps_array) + (mode < 4 ? 2 * pos : 0);
        int previous = -1;
        for (unsigned i = 0; i < n; i++) {
            long start = mode >= 4 ? pos + i : steps[2 * i];
            long count = mode >= 4 ? 1 : steps[2 * i + 1];
            if (mode >= 4) {
                if (start < 0) { start = 0; count = 0; }
                if (start >= limit) { start = limit; count = 0; }
            }
            if (start < 0 || count < 0 || start + count > limit ||
                (previous >= 0 && start != previous) || (mode < 4 && !scale_up && count > 1)) valid = 0;
            previous = start + count;
            if (a && solid_rl && count > 1) ordered = 1;
            size_t offset = axis + (a ? w * 8 : 0) + i * 8;
            put_u32(asset + offset, start + (a ? SwTargetWindowY() : SwTargetWindowX()));
            put_u32(asset + offset + 4, count);
        }
    }
    const uint8_t *rle = source->data;
    for (unsigned y = 0; valid && y < h; y++) {
        unsigned x = 0;
        for (;;) {
            int run = (int8_t)*rle++;
            if (!run) break;
            unsigned n = run < 0 ? -run : run;
            if (n > w - x) { valid = 0; break; }
            if (run > 0) for (unsigned i = 0; i < n; i++) {
                asset[2 * (y * w + x + i)] = *rle++;
                asset[2 * (y * w + x + i) + 1] = ordered && i + 1 == n ? 2 : 1;
            }
            x += n;
        }
    }
    if (!valid) { free(asset); return 0; }
    for (unsigned i = 0; i < 256; i++) asset[length - 256 + i] = remap ? remap[i] : i;
    struct KfxWgpuDrawCommand command = {0};
    command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command.kind = KFX_WGPU_DRAW_SPRITE;
    command.blend = blend;
    command.colour = colour;
    command.width = pitch;
    command.height = height;
    command.clip_x = SwTargetWindowX();
    command.clip_y = SwTargetWindowY();
    command.clip_width = SwTargetWindowWidth();
    command.clip_height = SwTargetWindowHeight();
    command.source_x = flip | ((mode == 2 || mode == 5) ? 4 : 0) | (ordered ? 8 : 0);
    /* The legacy remap and one-colour down/Trans2RL kernels discard the source byte. */
    if ((mode == 1 || mode == 2) && !scale_up && (flip & 1) && blend == 2) {
        command.source_x |= 4;
        command.colour = 0;
    }
    command.source_width = w;
    command.source_height = h;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    struct KfxWgpuNativeResource resource = {asset, length, 1, 1, 1};
    struct KfxWgpuNativeResource lookup = {table, 65536, 256, 256, 256};
    struct KfxGpolyTarget target = {SwTargetWScreen(), pitch, height, pitch};
    struct SpriteOracle oracle = {posx, posy, source, sprite, remap, colour, mode};
    int accepted = kfx_wgpu_native_draw(&target, &command, &resource,
        blend ? &lookup : NULL, sprite_oracle, &oracle);
    free(asset);
    return accepted;
}
