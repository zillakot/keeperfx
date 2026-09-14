#include "kfx/renderer/software/WgpuRawImage.h"
#include <string.h>

static int oracle_active;
struct RawOracle { KfxWgpuNativeOracle function; void *context; };

static void raw_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    struct RawOracle *oracle = context;
    oracle_active = 1;
    oracle->function(pixels, pitch, oracle->context);
    oracle_active = 0;
}

static int target_valid(uint8_t *dst, int pitch, int width, int height)
{
    return dst && pitch > 0 && width > 0 && width <= pitch && pitch <= 8192 &&
        height > 0 && height <= 8192;
}

static int disjoint(const uint8_t *source, size_t length, const uint8_t *dst, size_t capacity)
{
    uintptr_t a = (uintptr_t)source, b = (uintptr_t)dst;
    return a < b ? length <= b - a : capacity <= a - b;
}

static int submit(uint8_t *dst, int pitch, int height, struct KfxWgpuDrawCommand *command,
    const uint8_t *source, int sw, int sh, KfxWgpuNativeOracle oracle, void *context)
{
    if (!source || sw <= 0 || sh <= 0 || sw > 8192 || sh > 8192 ||
        (size_t)sw * sh > 16 * 1024 * 1024 || !oracle ||
        !disjoint(source, (size_t)sw * sh, dst, (size_t)pitch * height)) return 0;
    struct KfxGpolyTarget target = {dst, pitch, height, pitch};
    struct KfxWgpuNativeResource asset = {source, (size_t)sw * sh, sw, sh, sw, NULL, 0, 0};
    command->abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command->transparent = KFX_WGPU_DRAW_OPAQUE;
    command->clip_width = pitch;
    command->clip_height = height;
    command->source_width = sw;
    command->source_height = sh;
    struct RawOracle wrapper = {oracle, context};
    return kfx_wgpu_native_draw(&target, command, &asset, NULL, raw_oracle, &wrapper);
}

int kfx_wgpu_raw_image(uint8_t *dst, int pitch, int height, int dw, int dh, int x, int y,
    const uint8_t *source, int sw, int sh, KfxWgpuNativeOracle oracle, void *context)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    if (!target_valid(dst, pitch, pitch, height) ||
        dw <= 0 || dh <= 0 || dw > 16384 || dh > 16384 || x < -16384 || x > pitch ||
        y < -16384 || y > height || (int64_t)x + dw < 0 || (int64_t)y + dh < 0) return 0;
    struct KfxWgpuDrawCommand command = {0};
    command.kind = KFX_WGPU_DRAW_RAW_IMAGE;
    command.width = pitch;
    command.height = height;
    command.start_low = (uint32_t)x;
    command.start_high = (uint32_t)y;
    command.step_low = dw;
    command.step_high = dh;
    return submit(dst, pitch, height, &command, source, sw, sh, oracle, context);
}

int kfx_wgpu_raw_tile(uint8_t *dst, int pitch, int height, int x, int y, int width, int rows,
    const uint8_t *source, int size, KfxWgpuNativeOracle oracle, void *context)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    if (!target_valid(dst, pitch, pitch, height) ||
        x < 0 || y < 0 || width <= 0 || rows <= 0 || (int64_t)x + width > pitch ||
        (int64_t)y + rows > height) return 0;
    struct KfxWgpuDrawCommand command = {0};
    command.kind = KFX_WGPU_DRAW_TILED_IMAGE;
    command.x = x; command.y = y; command.width = width; command.height = rows;
    return submit(dst, pitch, height, &command, source, size, size, oracle, context);
}

struct ClearOracle { int width, height; uint8_t colour; };
static void clear_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    struct ClearOracle *clear = context;
    for (int y = 0; y < clear->height; y++) memset(pixels + y * pitch, clear->colour, clear->width);
}

int kfx_wgpu_raw_clear(uint8_t *dst, int pitch, int width, int height, uint8_t colour)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return 0;
    kfx_wgpu_native_flush();
    if (!target_valid(dst, pitch, width, height)) return 0;
    struct KfxGpolyTarget target = {dst, width, height, pitch};
    struct KfxWgpuDrawCommand command = {0};
    command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command.kind = KFX_WGPU_DRAW_CLEAR;
    command.colour = colour;
    command.clip_width = width;
    command.clip_height = height;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    struct ClearOracle context = {width, height, colour};
    return kfx_wgpu_native_draw(&target, &command, NULL, NULL, clear_oracle, &context);
}
