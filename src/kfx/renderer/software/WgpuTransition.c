#include "kfx/renderer/software/WgpuTransition.h"
#include <string.h>

static int overlap(const uint8_t* a, size_t an, const uint8_t* b, size_t bn)
{
    uintptr_t x=(uintptr_t)a,y=(uintptr_t)b;
    return x <= y ? y-x < an : x-y < bn;
}

int kfx_wgpu_map_fade(uint8_t* dst, int pitch, int width, int height,
    const uint8_t* first, const uint8_t* second, uint64_t first_snapshot, uint64_t second_snapshot,
    const uint8_t* fade, const uint8_t* ghost, int progress,
    KfxWgpuNativeOracle oracle, void* context)
{
    if (!kfx_wgpu_native_enabled() || !dst || !first || !second || !fade || !ghost || !oracle ||
        width < 256 || width > 640 || height < 1 || height > 480 || pitch < width || pitch > 8192 ||
        progress < 0 || progress > 32) return 0;
    size_t size=(size_t)width*height,outsize=(size_t)pitch*(height-1)+width;
    if (overlap(dst,outsize,first,size) || overlap(dst,outsize,second,size) ||
        overlap(dst,outsize,fade,33*256) || overlap(dst,outsize,ghost,65536)) return 0;
    if (!kfx_wgpu_native_read_barrier(fade, 33*256) ||
        !kfx_wgpu_native_read_barrier(ghost, 65536)) return 0;
    int own_first=!first_snapshot,own_second=!second_snapshot;
    struct KfxGpolyTarget target={dst,width,height,pitch};
    struct KfxGpolyTarget a={(uint8_t*)first,width,height,width};
    struct KfxGpolyTarget b={(uint8_t*)second,width,height,width};
    if (own_first) first_snapshot=kfx_wgpu_native_snapshot(&a,width,height,width,NULL);
    if (own_second && first_snapshot) second_snapshot=kfx_wgpu_native_snapshot(&b,width,height,width,NULL);
    int accepted=0;
    if (first_snapshot && second_snapshot) {
        uint8_t tables[33*256+65536];
        memcpy(tables,fade,33*256);
        memcpy(tables+33*256,ghost,65536);
        struct KfxWgpuNativeResource table={tables,sizeof(tables),1,1,1, NULL, 0, 0};
        struct KfxWgpuDrawCommand c={0};
        c.abi_version=KFX_WGPU_DRAW_ABI_VERSION; c.kind=KFX_WGPU_DRAW_TRANSITION;
        c.width=c.clip_width=c.source_width=width;
        c.height=c.clip_height=c.source_height=height;
        c.source=first_snapshot; c.start_low=second_snapshot; c.start_high=second_snapshot>>32;
        c.step_low=progress; c.transparent=KFX_WGPU_DRAW_OPAQUE;
        accepted=kfx_wgpu_native_draw(&target,&c,NULL,&table,oracle,context);
    }
    if (own_first) kfx_wgpu_native_snapshot_release(first_snapshot);
    if (own_second) kfx_wgpu_native_snapshot_release(second_snapshot);
    return accepted;
}

int kfx_wgpu_smooth(uint8_t* dst, int pitch, int height, int x, int y, int right, int bottom,
    const uint8_t* ghost, KfxWgpuNativeOracle oracle, void* context)
{
    if (!kfx_wgpu_native_enabled() || !dst || !ghost || !oracle || pitch < 1 || pitch > 8192 ||
        height < 1 || height > 8192 || x < 0 || x >= pitch || y < 0 || y >= height || right > pitch || bottom > height ||
        right <= x+1 || bottom <= y+1 || overlap(dst,(size_t)pitch*height,ghost,65536)) return 0;
    struct KfxGpolyTarget target={dst,pitch,height,pitch};
    uint64_t snapshot=kfx_wgpu_native_snapshot(&target,pitch,height,pitch,NULL);
    if (!snapshot) return 0;
    struct KfxWgpuNativeResource table={ghost,65536,256,256,256, NULL, 0, 0};
    struct KfxWgpuDrawCommand c={0};
    c.abi_version=KFX_WGPU_DRAW_ABI_VERSION; c.kind=KFX_WGPU_DRAW_TRANSITION;
    c.x=x; c.y=y; c.width=right-x-1; c.height=bottom-y-1;
    c.clip_width=pitch; c.clip_height=height;
    c.source=snapshot; c.source_x=1; c.transparent=KFX_WGPU_DRAW_OPAQUE;
    int accepted=kfx_wgpu_native_draw(&target,&c,NULL,&table,oracle,context);
    kfx_wgpu_native_snapshot_release(snapshot);
    return accepted;
}
