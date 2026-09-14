#include "kfx/renderer/software/WgpuTransition.h"
#include "kfx/renderer/WgpuTargetResource.h"
#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstring>
#include <vector>
#define SYNCDBG(...) ((void)0)
static int32_t xtab[640][2],ytab[480][2];
static uint64_t map_fade_src_snapshot,map_fade_dest_snapshot;
static const uint8_t *map_fade_src_owner,*map_fade_dest_owner;
static int map_fade_snapshot_width,map_fade_snapshot_height,map_fade_snapshot_pitch;
static int map_fade_buffers_valid=1;
static struct { int GraphicsScreenHeight; } lbDisplay;
static struct { uint8_t ghost[65536]; } pixmap;
using std::min;
#include "fade.inc"
#include "smooth.inc"
static void fill(uint8_t* pixels,uint32_t pitch,void* context)
{
    int colour=*static_cast<int*>(context);
    for(int y=0;y<200;y++)std::memset(pixels+y*pitch,colour,320);
}
static void paint(WgpuTerrainBridge& bridge,const KfxGpolyTarget& target,int colour)
{
    KfxWgpuDrawCommand command={};
    command.abi_version=KFX_WGPU_DRAW_ABI_VERSION;command.kind=KFX_WGPU_DRAW_RECT;
    command.width=command.clip_width=320;command.height=command.clip_height=200;
    command.colour=colour;command.transparent=KFX_WGPU_DRAW_OPAQUE;
    assert(bridge.SubmitNative(target,command,nullptr,nullptr,fill,&colour));
}
int main()
{
    uint8_t fade[33*256];
    for(unsigned i=0;i<sizeof(fade);i++)fade[i]=(i*17+(i>>8)*3)&255;
    for(unsigned i=0;i<65536;i++)pixmap.ghost[i]=(i*13+(i>>8)*29)&255;
    lbDisplay.GraphicsScreenHeight=200;
    for(bool verify:{false,true})for(bool resident:{false,true}) {
        std::vector<uint8_t> screen(327*200,81),first(320*200),second(320*200),expected;
        WgpuTerrainBridge bridge(0,false,verify,resident);
        KfxGpolyTarget target={screen.data(),320,200,327};
        bridge.BeginResident();paint(bridge,target,17);
        // Batched commands upload the initial index image at their flush, not on submission.
        bridge.Flush();
        auto initial=bridge.GetCounters().bridge_initial_index_bytes;
        auto snapshot=bridge.Snapshot(target,320,200,320,first.data());
        assert(snapshot);
        if(resident)assert(bridge.GetCounters().bridge_initial_index_bytes==initial);
        map_fade_dest_owner=first.data();map_fade_dest_snapshot=snapshot;
        map_fade_snapshot_width=map_fade_snapshot_pitch=320;map_fade_snapshot_height=200;
        bridge.BeginResident();paint(bridge,target,211);
        map_fade_src_owner=second.data();map_fade_src_snapshot=bridge.Snapshot(target,320,200,320,second.data());
        assert(map_fade_src_snapshot);
        for(auto pixel:first)assert(pixel==17);
        for(auto pixel:second)assert(pixel==211);
        for(int progress:{0,4,16,28,32,28,16,4,0}) {
            expected=screen;
            map_fade_native(expected.data(),first.data(),second.data(),fade,pixmap.ghost,progress,320,200,327);
            bridge.BeginResident();
            map_fade(screen.data(),first.data(),second.data(),fade,pixmap.ghost,progress,320,200,327);
            assert(bridge.CpuBarrier());
            if(screen!=expected){std::fprintf(stderr,"map bridge mismatch: %s\n",bridge.GetError());return 1;}
        }
        expected=screen;smooth_screen_area_native(expected.data(),3,5,319,199,327);
        bridge.BeginResident();smooth_screen_area(screen.data(),3,5,319,199,327);
        assert(bridge.CpuBarrier());assert(screen==expected);
        assert(!bridge.Failed());assert(bridge.GetCounters().transition_commands==10);
        assert(bridge.GetCounters().transition_checkpoint_bytes==128000);
        bridge.ReleaseSnapshot(map_fade_src_snapshot);bridge.ReleaseSnapshot(map_fade_dest_snapshot);
        map_fade_src_snapshot=map_fade_dest_snapshot=0;
    }
    {
        std::vector<uint8_t> screen(327*200,81),checkpoint(320*200);
        WgpuTerrainBridge bridge(1,false,false,true);
        KfxGpolyTarget target={screen.data(),327,200,327};
        bridge.BeginResident();paint(bridge,target,17);
        auto snapshot=bridge.Snapshot(target,320,200,320,nullptr);assert(snapshot);
        smooth_screen_area(screen.data(),0,0,320,200,327);
        assert(bridge.Failed() && !bridge.FrameValid() && !bridge.CpuBarrier());
        for(auto pixel:screen)assert(pixel==81);
        bridge.ReleaseSnapshot(snapshot);
        bridge.FullRedraw();assert(bridge.FrameValid());
        map_fade_buffers_valid=0;map_fade_src_owner=checkpoint.data();
        map_fade(screen.data(),checkpoint.data(),checkpoint.data(),fade,pixmap.ghost,16,320,200,327);
        assert(!bridge.FrameValid() && !bridge.CpuBarrier());
        map_fade_buffers_valid=1;
        bridge.FullRedraw();assert(bridge.FrameValid());
    }
    std::puts("real bridge snapshots, retained transitions, smoothing, residency, oracle and failure passed");
}
