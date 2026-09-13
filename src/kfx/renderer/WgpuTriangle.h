#pragma once
#include <stddef.h>
#include <stdint.h>
#pragma pack(push, 8)
struct KfxWgpuVertex { int32_t x, y; int64_t u, v, shade; };
struct KfxWgpuTriangle {
    uint32_t abi_version, reserved;
    uint64_t source, table;
    struct KfxWgpuVertex vertices[3];
};
#pragma pack(pop)
#ifdef __cplusplus
extern "C" {
#endif
int32_t kfx_wgpu_draw_submit_triangles(void* context, uint64_t target,
    const struct KfxWgpuTriangle* triangles, size_t count, char* error, size_t capacity);
#ifdef __cplusplus
}
static_assert(sizeof(KfxWgpuVertex) == 32 && sizeof(KfxWgpuTriangle) == 120);
#endif
