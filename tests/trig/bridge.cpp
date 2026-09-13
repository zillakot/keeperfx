#include "kfx/renderer/WgpuTerrainBridge.h"
#include <array>
#include <cstdio>
#include <cstdlib>
extern "C" void native_trig_case(unsigned, int, uint8_t*);
int main() {
    std::array<uint8_t,83*63> cpu{}, gpu{};
    unsigned modes[] = {0,1,2,3,4,7,8,11,12,13,14,15,16,17,18,19,22,23};
    WgpuTerrainBridge bridge(UINT64_MAX, false, true);
    for (unsigned mode: modes) {
        native_trig_case(mode, 1, cpu.data());
        native_trig_case(mode, 0, gpu.data());
        if (cpu != gpu || bridge.Failed()) { std::fprintf(stderr,"mode%u: %s\n",mode,bridge.GetError()); return 1; }
    }
    if (bridge.GetCounters().native_commands != 36 || bridge.GetCounters().verified_batches != 36) return 1;
    std::puts("36 native general triangles consumed before CPU setup; exact Metal verification");
    return 0;
}
