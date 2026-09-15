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
    uint8_t *storage = NULL, *draw_pixels = pixels;
    size_t size = (size_t)pitch * SwTargetScreenHeight();
    if (((uintptr_t)screen & 3) != ((uintptr_t)pixels & 3)) {
        storage = malloc(size + 3);
        if (!storage) return;
        draw_pixels = storage + (((uintptr_t)screen - (uintptr_t)storage) & 3);
        memcpy(draw_pixels, pixels, size);
    }
    lbDisplay.WScreen = draw_pixels;
    lbDisplay.GraphicsWindowPtr = draw_pixels + SwTargetWindowY() * pitch + SwTargetWindowX();
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
    if (storage) {
        memcpy(pixels, draw_pixels, size);
        free(storage);
    }
}

static void put_u32(uint8_t *bytes, uint32_t value)
{
    for (unsigned i = 0; i < 4; i++) bytes[i] = value >> (8 * i);
}

/* Expanded artwork, held by the name the emitter gave it. The expansion is a function
 * of the RLE, the decoded size and nothing else, so a hit skips the walk; rle_length is
 * kept because the walk is also what measured how much of the source the sprite reads,
 * which the read barrier and the target-alias test both still need. */
struct SpriteArtwork {
    const void *identity;
    uint64_t generation;
    unsigned width, height;
    size_t rle_length;
    uint8_t *bytes;
};

enum { SPRITE_CACHE_SLOTS = 1024, SPRITE_CACHE_PROBE = 8,
    SPRITE_CACHE_BYTE_LIMIT = 8 * 1024 * 1024 };
static struct SpriteArtwork sprite_cache[SPRITE_CACHE_SLOTS];
static size_t sprite_cache_bytes;

static size_t sprite_cache_slot(const void *identity, unsigned w, unsigned h)
{
    uint64_t value = (uint64_t)(uintptr_t)identity;
    value ^= value >> 33;
    value *= 0xff51afd7ed558ccdULL;
    value += (uint64_t)w * 0x9e3779b1u + (uint64_t)h * 0x85ebca6bu;
    value ^= value >> 29;
    return (size_t)(value & (SPRITE_CACHE_SLOTS - 1));
}

static struct SpriteArtwork *sprite_cache_find(const void *identity, unsigned w, unsigned h,
    uint64_t generation)
{
    size_t base = sprite_cache_slot(identity, w, h);
    for (unsigned i = 0; i < SPRITE_CACHE_PROBE; i++) {
        struct SpriteArtwork *entry = &sprite_cache[(base + i) & (SPRITE_CACHE_SLOTS - 1)];
        if (entry->bytes && entry->identity == identity && entry->generation == generation &&
            entry->width == w && entry->height == h) return entry;
    }
    return NULL;
}

static void sprite_cache_evict(struct SpriteArtwork *entry)
{
    if (!entry->bytes) return;
    sprite_cache_bytes -= (size_t)entry->width * entry->height * 2;
    free(entry->bytes);
    entry->bytes = NULL;
}

/* Takes ownership of bytes on success; the caller keeps it on failure. */
static int sprite_cache_store(const void *identity, unsigned w, unsigned h, uint64_t generation,
    size_t rle_length, uint8_t *bytes)
{
    size_t base = sprite_cache_slot(identity, w, h), chosen = base;
    for (unsigned i = 0; i < SPRITE_CACHE_PROBE; i++) {
        size_t slot = (base + i) & (SPRITE_CACHE_SLOTS - 1);
        if (!sprite_cache[slot].bytes) { chosen = slot; break; }
        if (sprite_cache[slot].identity == identity && sprite_cache[slot].width == w &&
            sprite_cache[slot].height == h) { chosen = slot; break; }
    }
    sprite_cache_evict(&sprite_cache[chosen]);
    sprite_cache[chosen] = (struct SpriteArtwork){identity, generation, w, h, rle_length, bytes};
    sprite_cache_bytes += (size_t)w * h * 2;
    for (size_t i = 0; sprite_cache_bytes > SPRITE_CACHE_BYTE_LIMIT && i < SPRITE_CACHE_SLOTS; i++)
        if (i != chosen) sprite_cache_evict(&sprite_cache[i]);
    return 1;
}

void kfx_wgpu_sprite_cache_stats(size_t *entries, size_t *bytes)
{
    if (entries) {
        *entries = 0;
        for (size_t i = 0; i < SPRITE_CACHE_SLOTS; i++) *entries += sprite_cache[i].bytes != NULL;
    }
    if (bytes) *bytes = sprite_cache_bytes;
}

static const TbPixel *sprite_identity_remap(void)
{
    static TbPixel table[256];
    static int ready;
    if (!ready) {
        for (unsigned i = 0; i < 256; i++) table[i] = i;
        ready = 1;
    }
    return table;
}

int kfx_wgpu_sprite(long posx, long posy, const struct TbSourceBuffer *source,
    const struct TbSprite *sprite, const TbPixel *remap, TbPixel colour, unsigned mode)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    if (!kfx_wgpu_native_read_barrier(sprite, sprite ? sizeof(*sprite) : 0) ||
        !kfx_wgpu_native_read_barrier(source, source ? sizeof(*source) : 0)) return 0;
    struct TbSourceBuffer converted;
    if (sprite) {
        converted = (struct TbSourceBuffer){sprite->Data, sprite->SWidth, sprite->SHeight,
            sprite->SWidth, sprite->Data};
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
    size_t range_length = (size_t)(w + h) * 8;
    if (axis + range_length + 256 > 16 * 1024 * 1024) return 0;
    if (!kfx_wgpu_native_read_barrier(remap, remap ? 256 : 0) ||
        !kfx_wgpu_native_read_barrier(table, blend ? 65536 : 0) ||
        (mode < 4 && (!kfx_wgpu_native_read_barrier(xsteps_array + posx * 2, w * 8) ||
        !kfx_wgpu_native_read_barrier(ysteps_array + posy * 2, h * 8)))) return 0;
    uint8_t *ranges = calloc(range_length, 1);
    if (!ranges) return 0;
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
            size_t offset = (a ? w * 8 : 0) + i * 8;
            put_u32(ranges + offset, start + (a ? SwTargetWindowY() : SwTargetWindowX()));
            put_u32(ranges + offset + 4, count);
        }
    }
    if (!valid) { free(ranges); return 0; }
    const void *identity = sprite ? (const void *)sprite->Data : source->identity;
    uint64_t generation = kfx_render_sprite_generation;
    struct SpriteArtwork *cached = identity ? sprite_cache_find(identity, w, h, generation) : NULL;
    uint8_t *built = NULL;
    const uint8_t *artwork;
    size_t rle_length;
    if (cached) {
        rle_length = cached->rle_length;
        artwork = cached->bytes;
        if (!kfx_wgpu_native_read_barrier(source->data, rle_length)) { free(ranges); return 0; }
    } else {
        built = calloc(axis, 1);
        if (!built) { free(ranges); return 0; }
        const uint8_t *rle = source->data;
        for (unsigned y = 0; valid && y < h; y++) {
            unsigned x = 0;
            for (;;) {
                if (!kfx_wgpu_native_read_barrier(rle, 1)) { valid = 0; break; }
                int run = (int8_t)*rle++;
                if (!run) break;
                unsigned n = run < 0 ? -run : run;
                if (n > w - x) { valid = 0; break; }
                if (run > 0 && !kfx_wgpu_native_read_barrier(rle, n)) { valid = 0; break; }
                if (run > 0) for (unsigned i = 0; i < n; i++) {
                    built[2 * (y * w + x + i)] = *rle++;
                    built[2 * (y * w + x + i) + 1] = i + 1 == n ? 2 : 1;
                }
                x += n;
            }
        }
        if (!valid) { free(built); free(ranges); return 0; }
        rle_length = (size_t)(rle - (const uint8_t *)source->data);
        artwork = built;
    }
    uintptr_t begin = (uintptr_t)SwTargetWScreen();
    uintptr_t end = begin + (size_t)pitch * height;
    const TbPixel *remap_bytes = remap ? remap : sprite_identity_remap();
    if (((uintptr_t)source->data < end && begin < (uintptr_t)source->data + rle_length) ||
        ((uintptr_t)remap_bytes < end && begin < (uintptr_t)remap_bytes + 256) ||
        (blend && (uintptr_t)table < end && begin < (uintptr_t)table + 65536)) {
        free(built);
        free(ranges);
        return 0;
    }
    if (built && identity && sprite_cache_store(identity, w, h, generation, rle_length, built))
        built = NULL;
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
    command.source_y = ordered ? (uintptr_t)SwTargetWScreen() & 3 : 0;
    command.source_width = w;
    command.source_height = h;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    struct KfxWgpuNativeResource resource = {artwork, axis, 1, 1, 1, NULL, 0, 0};
    struct KfxWgpuNativeResource range_resource = {ranges, range_length, 1, 1, 1, NULL, 0, 0};
    struct KfxWgpuNativeResource remap_resource = {remap_bytes, 256, 1, 1, 1, NULL, 0, 0};
    struct KfxWgpuNativeResource lookup = {table, 65536, 256, 256, 256, NULL, 0, 0};
    struct KfxWgpuSpriteAssets assets = {&resource, &range_resource, &remap_resource,
        blend ? &lookup : NULL, identity, generation};
    struct KfxGpolyTarget target = {SwTargetWScreen(), pitch, height, pitch};
    struct SpriteOracle oracle = {posx, posy, source, sprite, remap, colour, mode};
    int accepted = kfx_wgpu_native_draw_sprite(&target, &command, &assets, sprite_oracle,
        &oracle);
    free(built);
    free(ranges);
    return accepted;
}
