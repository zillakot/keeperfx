#include "kfx/renderer/WgpuTargetResource.h"
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(struct KfxWgpuDrawCommand) == 112, "drawing ABI");
_Static_assert(offsetof(struct KfxWgpuDrawCommand, source) == 48, "resource alignment");
_Static_assert(sizeof(struct KfxWgpuTargetResourceCounters) == 24, "counter ABI");

#define CHECK(expression) do { if (!(expression)) { fprintf(stderr, "%s:%d: %s: %s\n", __FILE__, __LINE__, #expression, error); return 1; } } while (0)

int main(void)
{
    char error[256] = {0};
    void *draw = kfx_wgpu_draw_create(error, sizeof(error));
    CHECK(draw != NULL);
    uint64_t target = kfx_wgpu_draw_target_create(draw, 5, 4, error, sizeof(error));
    CHECK(target != 0);
    struct KfxWgpuDrawCommand initial[2] = {
        {.abi_version = KFX_WGPU_DRAW_ABI_VERSION, .kind = KFX_WGPU_DRAW_CLEAR, .colour = 0,
            .clip_width = 5, .clip_height = 4, .transparent = 256},
        {.abi_version = KFX_WGPU_DRAW_ABI_VERSION, .kind = KFX_WGPU_DRAW_RECT, .colour = 255,
            .x = 2, .y = 1, .width = 1, .height = 2,
            .clip_width = 5, .clip_height = 4, .transparent = 256},
    };
    CHECK(kfx_wgpu_draw_submit(draw, target, initial, 2, error, sizeof(error)) == 1);
    uint64_t snapshot = kfx_wgpu_draw_target_snapshot(draw, target, 1, 1, 2, 2, 6, error, sizeof(error));
    CHECK(snapshot != 0);
    initial[0].colour = 99;
    CHECK(kfx_wgpu_draw_submit(draw, target, initial, 1, error, sizeof(error)) == 1);
    struct KfxWgpuDrawCommand images[2] = {
        {.abi_version = KFX_WGPU_DRAW_ABI_VERSION, .kind = KFX_WGPU_DRAW_IMAGE, .source = snapshot,
            .x = 1, .y = 1, .width = 2, .height = 2,
            .clip_width = 5, .clip_height = 4, .source_width = 2, .source_height = 2,
            .transparent = 256},
        {.abi_version = KFX_WGPU_DRAW_ABI_VERSION, .kind = KFX_WGPU_DRAW_IMAGE, .source = 0,
            .width = 2, .height = 2, .clip_width = 5, .clip_height = 4,
            .source_width = 2, .source_height = 2, .transparent = 256},
    };
    CHECK(kfx_wgpu_draw_submit_target_images(draw, target, images, 2, error, sizeof(error)) == -1);
    uint8_t pixels[36];
    memset(pixels, 0xa5, sizeof(pixels));
    CHECK(kfx_wgpu_draw_readback(draw, target, pixels, sizeof(pixels), 9, error, sizeof(error)) == 1);
    for (unsigned y = 0; y < 4; ++y)
        for (unsigned x = 0; x < 9; ++x)
            CHECK(pixels[y * 9 + x] == (x < 5 ? 99 : 0xa5));
    CHECK(kfx_wgpu_draw_submit_target_images(draw, target, images, 1, error, sizeof(error)) == 1);
    CHECK(kfx_wgpu_draw_target_snapshot_release(draw, snapshot, error, sizeof(error)) == 1);
    CHECK(kfx_wgpu_draw_readback(draw, target, pixels, sizeof(pixels), 9, error, sizeof(error)) == 1);
    for (unsigned y = 0; y < 4; ++y) {
        for (unsigned x = 0; x < 9; ++x) {
            uint8_t expected = x < 5 ? 99 : 0xa5;
            if (y >= 1 && y <= 2 && x >= 1 && x <= 2)
                expected = x == 1 ? 0 : 255;
            CHECK(pixels[y * 9 + x] == expected);
        }
    }
    struct KfxWgpuTargetResourceCounters counters;
    CHECK(kfx_wgpu_draw_target_resource_counters(draw, &counters, error, sizeof(error)) == 1);
    struct KfxWgpuDrawCounters drawing;
    CHECK(kfx_wgpu_draw_counters(draw, &drawing, error, sizeof(error)) == 1);
    CHECK(counters.snapshots == 1 && counters.snapshot_copy_bytes == 16);
    CHECK(counters.sampling_copy_bytes == (drawing.arena_representation == 2 ? 12 : 48));
    CHECK(kfx_wgpu_draw_submit_target_images(draw, target, images, 1, error, sizeof(error)) == -1);
    CHECK(kfx_wgpu_draw_target_release(draw, target, error, sizeof(error)) == 1);
    kfx_wgpu_draw_destroy(draw);
    puts("GPU snapshot C ABI: exact indices, padding, ordering, release and host rejection passed");
    return 0;
}
