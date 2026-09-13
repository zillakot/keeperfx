#include "kfx/renderer/WgpuTerrainBridge.h"
#include <array>
#include <cstdio>
#include <cstdlib>
extern "C" void native_trig_case(unsigned, int, uint8_t*);
extern "C" void native_trig_extent_case(unsigned, int, uint8_t*);
extern "C" void native_trig_thin_case(unsigned, int, uint8_t*);
extern "C" void native_trig_scratch_case(uint8_t*);
int main() {
    std::array<uint8_t,83*63> cpu{}, gpu{};
    unsigned modes[] = {0,1,2,3,4,5,6,7,8,9,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26};
    WgpuTerrainBridge bridge(UINT64_MAX, false, true);
    for (unsigned mode: modes) {
        native_trig_case(mode, 1, cpu.data());
        native_trig_case(mode, 0, gpu.data());
        if (cpu != gpu || bridge.Failed()) { std::fprintf(stderr,"mode%u: %s\n",mode,bridge.GetError()); return 1; }
    }
    for (unsigned mode: modes) {
        native_trig_thin_case(mode, 1, cpu.data());
        native_trig_thin_case(mode, 0, gpu.data());
        if (cpu != gpu || bridge.Failed()) { std::fprintf(stderr,"thin mode%u: %s\n",mode,bridge.GetError()); return 1; }
    }
    native_trig_case(7, 1, cpu.data());
    native_trig_scratch_case(gpu.data());
    if (cpu != gpu || bridge.Failed()) return 1;
    for (unsigned variant = 0; variant < 5; variant++) {
        native_trig_extent_case(variant, 1, cpu.data());
        native_trig_extent_case(variant, 0, gpu.data());
        if (cpu != gpu || bridge.Failed()) return 1;
    }
    if (bridge.GetCounters().native_commands != 628 || bridge.GetCounters().verified_batches != 628) return 1;
    std::puts("628 native general triangles consumed before CPU setup; exact Metal verification");
    return 0;
}
