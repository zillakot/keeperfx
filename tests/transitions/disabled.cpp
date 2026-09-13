#include "kfx/renderer/software/WgpuTransition.h"
#include <cassert>
#include <array>
static void oracle(uint8_t*, uint32_t, void*) { assert(false); }
int main()
{
    std::array<uint8_t,65536> bytes={};
    assert(!kfx_wgpu_map_fade(bytes.data(),320,320,200,bytes.data(),bytes.data(),0,0,bytes.data(),bytes.data(),0,oracle,nullptr));
    assert(!kfx_wgpu_smooth(bytes.data(),320,200,0,0,320,200,bytes.data(),oracle,nullptr));
    KfxGpolyTarget target={bytes.data(),320,200,320};
    assert(!kfx_wgpu_native_snapshot(&target,320,200,320,bytes.data()));
    kfx_wgpu_native_snapshot_release(1);
    for(auto pixel:bytes)assert(pixel==0);
}
