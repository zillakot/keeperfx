#include "kfx/renderer/software/MovieFrame.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "bflib_fmvids.h"
#include <cassert>
#include <cstring>

int main()
{
    uint8_t source[] = {1,2,3,4,99,5,6,7,8,99};
    uint8_t pixels[32];
    memset(pixels, 42, sizeof(pixels));
    assert(!kfx_wgpu_native_enabled());
    kfx_movie_copy_scaled({source,4,2,5}, {pixels,8,4,8,4}, SMK_FullscreenStretch);
    for (int y = 0; y < 4; y++) for (int x = 0; x < 8; x++)
        assert(pixels[y*8+x] == source[(y/2)*5+x/2]);
}
