#include "kfx/renderer/software/WgpuMapView.h"
#include <cassert>
#include <cstring>

static void oracle(uint8_t *, uint32_t, void *) { assert(false); }
int main()
{
    uint8_t pixels[16*16], source[512*512] = {};
    int styles[] = {257, 260};
    int32_t pattern[] = {0, 0};
    memset(pixels, 42, sizeof(pixels));
    assert(!kfx_wgpu_native_enabled());
    assert(kfx_wgpu_native_cpu_barrier());
    assert(!kfx_wgpu_map_row(pixels,16,16,0,0,8,styles,2,source,source,oracle,nullptr));
    assert(!kfx_wgpu_map_texture(pixels,16,16,16,0,0,16,16,0,source,nullptr,oracle,nullptr));
    assert(!kfx_wgpu_map_zoom(pixels,16,16,16,8,8,128,128,256,source,512,512,oracle,nullptr));
    assert(!kfx_wgpu_map_marker(pixels,16,16,8,8,pattern,1,1,1,12,oracle,nullptr));
    for (uint8_t pixel : pixels) assert(pixel == 42);
}
