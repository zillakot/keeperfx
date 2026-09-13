#include "kfx/renderer/software/WgpuMapView.h"
#include <stdlib.h>
#include <string.h>

static int oracle_active;
struct MapOracle { KfxWgpuNativeOracle function; void *context; };
static void map_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    struct MapOracle *o = context;
    oracle_active = 1;
    o->function(pixels, pitch, o->context);
    oracle_active = 0;
}
static int ready(uint8_t *dst, int pitch, int width, int height)
{
    return !oracle_active && kfx_wgpu_native_enabled() && dst && width > 0 &&
        width <= pitch && pitch <= 8192 && height > 0 && height <= 8192;
}
static int submit(uint8_t *dst, int pitch, int width, int height,
    struct KfxWgpuDrawCommand *c, const uint8_t *bytes, int length,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!oracle) return 0;
    struct KfxGpolyTarget target = {dst, width, height, pitch};
    struct KfxWgpuNativeResource source = {bytes, length, 1, 1, 1};
    c->abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    c->kind = KFX_WGPU_DRAW_MAP_VIEW;
    c->clip_width = width;
    c->clip_height = height;
    c->transparent = KFX_WGPU_DRAW_OPAQUE;
    struct MapOracle o = {oracle, context};
    return kfx_wgpu_native_draw(&target, c, &source, NULL, map_oracle, &o);
}
int kfx_wgpu_map_row(uint8_t *dst, int pitch, int height, int x, int y, int block_size,
    const int *styles, int count, const uint8_t *ghost, const uint8_t *abyss,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!ready(dst, pitch, pitch, height) || !styles || !ghost || !abyss ||
        count < 1 || count > 2048 || block_size < 1 || block_size > 2048 ||
        x < 0 || y < 0 || (int64_t)x + count * block_size > pitch ||
        (int64_t)y + block_size > height) return 0;
    if (!kfx_wgpu_native_read_barrier(styles, count * sizeof(*styles)) ||
        !kfx_wgpu_native_read_barrier(ghost, 65536) ||
        !kfx_wgpu_native_read_barrier(abyss, 256)) return 0;
    uint8_t bytes[2048 * 2 + 1280];
    for (int i = 0; i < count; i++) {
        if (styles[i] < 0 || styles[i] > 262) return 0;
        bytes[2*i] = styles[i]; bytes[2*i+1] = styles[i] >> 8;
    }
    uint8_t *tables = bytes + count * 2;
    memcpy(tables, ghost + 0x1a00, 256);
    memcpy(tables + 256, ghost + 0x8c00, 256);
    memcpy(tables + 512, ghost, 256);
    memcpy(tables + 768, ghost + 0x1000, 256);
    memcpy(tables + 1024, abyss, 256);
    struct KfxWgpuDrawCommand c = {0};
    c.x=x; c.y=y; c.width=count*block_size; c.height=block_size;
    c.source_width=count; c.source_height=block_size;
    return submit(dst,pitch,pitch,height,&c,bytes,count*2+1280,oracle,context);
}
int kfx_wgpu_map_texture(uint8_t *dst, int pitch, int width, int height, int x, int y,
    int dw, int dh, int flags, const uint8_t *texture, const uint8_t *fade,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!ready(dst,pitch,width,height) || !texture || dw < 1 || dh < 1 ||
        dw > 640 || dh > 480 || x < -8192 || y < -8192 || x > 8192 || y > 8192 ||
        flags < 0 || flags > 0x70 || (flags & 15)) return 0;
    if (!kfx_wgpu_native_read_barrier(texture, 31*256+32) ||
        !kfx_wgpu_native_read_barrier(fade, fade ? 256 : 0)) return 0;
    uint8_t bytes[32*256+256];
    memset(bytes,0,32*256);
    memcpy(bytes,texture,31*256+32);
    for (int i=0;i<256;i++) bytes[32*256+i]=fade?fade[i]:i;
    struct KfxWgpuDrawCommand c = {0};
    c.x=x; c.y=y; c.width=dw; c.height=dh; c.source_x=1; c.source_y=flags;
    return submit(dst,pitch,width,height,&c,bytes,sizeof(bytes),oracle,context);
}
int kfx_wgpu_map_zoom(uint8_t *dst, int pitch, int width, int height, int x, int y,
    int map_x, int map_y, int delta, const uint8_t *source, int sw, int sh,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!ready(dst,pitch,pitch,height) || !source || width < 2 || width > pitch ||
        height < 2 || x < 1 || x >= width || y < 1 || y >= height || delta < 0 ||
        delta > 4096 || sw < 1 || sh < 1 || sw > 8192 || sh > 8192 ||
        (int64_t)sw*sh > 16*1024*1024 || map_x < ((int64_t)(x+1)*delta>>8) ||
        map_y < ((int64_t)y*delta>>8) ||
        (int64_t)map_x+1+((int64_t)(width-x)*delta>>8)>=sw ||
        (int64_t)map_y+1+((int64_t)(height-y)*delta>>8)>=sh) return 0;
    struct KfxWgpuDrawCommand c = {0};
    c.width=width; c.height=height; c.source_x=2; c.source_y=delta;
    c.source_width=sw; c.source_height=sh;
    c.start_low=x; c.start_high=y; c.step_low=map_x; c.step_high=map_y;
    return submit(dst,pitch,pitch,height,&c,source,sw*sh,oracle,context);
}

int kfx_wgpu_map_marker(uint8_t *dst, int pitch, int height, int x, int y,
    const int32_t *pattern, int count, int spread, int cross, uint8_t colour,
    KfxWgpuNativeOracle oracle, void *context)
{
    if (!ready(dst,pitch,pitch,height) || !pattern || count < 1 || count > 36 ||
        spread < -4096 || spread > 4096 || cross < 0 || cross > 1 ||
        x < -16384 || x > 16384 || y < -16384 || y > 16384) return 0;
    if (!kfx_wgpu_native_read_barrier(pattern, count * 2 * sizeof(*pattern))) return 0;
    uint8_t bytes[36*8];
    for (int i=0;i<count;i++) {
        int32_t dx=pattern[2*i],dy=pattern[2*i+1];
        if (dx < -32 || dx > 32 || dy < -32 || dy > 32) return 0;
        int64_t offset=((int64_t)y+dy)*pitch+x+dx;
        int64_t margin=cross?(int64_t)abs(spread)*pitch:0;
        if (offset-margin < 0 || offset+margin >= (int64_t)pitch*height) return 0;
        for (int j=0;j<4;j++) {
            bytes[8*i+j]=(uint32_t)dx>>(8*j);
            bytes[8*i+4+j]=(uint32_t)dy>>(8*j);
        }
    }
    struct KfxWgpuDrawCommand c = {0};
    c.width=pitch; c.height=height; c.source_x=3; c.source_y=count; c.colour=colour;
    c.start_low=x; c.start_high=y; c.step_low=spread; c.step_high=cross;
    return submit(dst,pitch,pitch,height,&c,bytes,count*8,oracle,context);
}
