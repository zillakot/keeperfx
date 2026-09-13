#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "kfx/renderer/software/WgpuTransition.h"
#define MAX (650*480)
#define min(a,b) ((a)<(b)?(a):(b))
#define SYNCDBG(...) ((void)0)
static int32_t xtab[640][2],ytab[480][2];
static uint8_t *map_fade_src,*map_fade_dest;
static uint64_t map_fade_src_snapshot,map_fade_dest_snapshot;
static struct { uint8_t *WScreen; int GraphicsScreenWidth,GraphicsScreenHeight; } lbDisplay;
static struct { uint8_t ghost[65536]; } pixmap;
static int MyScreenWidth,pixel_size=1;
struct PlayerInfo { int view_mode_restore; };
static struct PlayerInfo player;
enum { PVM_IsoWibbleView=1,PVM_IsoStraightView=2 };
static unsigned views,loads,parchments;
static struct PlayerInfo* get_my_player(void){return &player;}
static void redraw_isometric_view(void){views++;memset(lbDisplay.WScreen,17,(size_t)lbDisplay.GraphicsScreenWidth*lbDisplay.GraphicsScreenHeight);}
static void redraw_frontview(void){views++;memset(lbDisplay.WScreen,39,(size_t)lbDisplay.GraphicsScreenWidth*lbDisplay.GraphicsScreenHeight);}
static void load_parchment_file(void){loads++;}
static void redraw_minimal_overhead_view(void){parchments++;memset(lbDisplay.WScreen,211,(size_t)lbDisplay.GraphicsScreenWidth*lbDisplay.GraphicsScreenHeight);}
#include "fade.inc"
#include "capture.inc"
#include "smooth.inc"
struct Snapshot { uint8_t* pixels; unsigned width,height,pitch; };
static struct Snapshot snapshots[8];
static FILE* output;
static unsigned count,enabled=1,accepted=1,barrier=1,oracle_calls;
static uint8_t initial[MAX],expected[MAX],screen[MAX],first[MAX],second[MAX],fade[33*256];
static int fw,fh,fp,progress,sx,sy,sw,sh;
static void word(uint32_t n){uint8_t b[]={n,n>>8,n>>16,n>>24};if(fwrite(b,1,4,output)!=4)abort();}
int kfx_wgpu_native_enabled(void){return enabled;}
int kfx_wgpu_native_cpu_barrier(void){return barrier;}
uint64_t kfx_wgpu_native_snapshot(const struct KfxGpolyTarget* target,uint32_t width,uint32_t height,uint32_t pitch,uint8_t* checkpoint)
{
    if(!accepted)return 0;
    for(unsigned i=1;i<8;i++)if(!snapshots[i].pixels) {
        struct Snapshot* s=&snapshots[i];
        s->pixels=calloc(pitch,height);if(!s->pixels)abort();s->width=width;s->height=height;s->pitch=pitch;
        for(unsigned y=0;y<height;y++) {
            memcpy(s->pixels+y*pitch,target->pixels+y*target->pitch,width);
            if(checkpoint)memcpy(checkpoint+y*pitch,target->pixels+y*target->pitch,width);
        }
        return i;
    }
    abort();
}
void kfx_wgpu_native_snapshot_release(uint64_t id){if(id){free(snapshots[id].pixels);snapshots[id].pixels=NULL;}}
static void write_snapshot(uint64_t id)
{
    if(!id){word(0);word(0);word(0);return;}
    struct Snapshot* s=&snapshots[id];word(s->width);word(s->height);word(s->pitch);
    fwrite(s->pixels,1,s->pitch*s->height,output);
}
int kfx_wgpu_native_draw(const struct KfxGpolyTarget* target,const struct KfxWgpuDrawCommand* c,
    const struct KfxWgpuNativeResource* source,const struct KfxWgpuNativeResource* table,KfxWgpuNativeOracle oracle,void* context)
{
    if(!accepted)return 0;
    size_t size=(size_t)target->pitch*target->height;
    if(source || !table || c->kind!=KFX_WGPU_DRAW_TRANSITION || memcmp(target->pixels,initial,size))abort();
    memcpy(expected,initial,size);oracle(expected,target->pitch,context);oracle_calls++;
    if(memcmp(target->pixels,initial,size))abort();
    word(target->width);word(target->height);word(target->pitch);
    const uint32_t* words=(const uint32_t*)c;for(unsigned i=0;i<28;i++)word(words[i]);
    write_snapshot(c->source);
    write_snapshot(c->source_x==0?c->start_low:0);
    word(table->length);fwrite(table->bytes,1,table->length,output);
    fwrite(initial,1,size,output);fwrite(expected,1,size,output);
    count++;return 1;
}
static void reset(void)
{
    for(unsigned i=0;i<(unsigned)(fp*fh);i++)initial[i]=(i*19+i/fp*13)&255;
    memcpy(screen,initial,(size_t)fp*fh);enabled=accepted=barrier=1;
    lbDisplay.WScreen=screen;lbDisplay.GraphicsScreenWidth=fp;lbDisplay.GraphicsScreenHeight=fh;
}
static void draw_fade(void){map_fade(screen,first,second,fade,pixmap.ghost,progress,fw,fh,fp);}
static void draw_smooth(void){smooth_screen_area(screen,sx,sy,sw,sh,fp);}
static void fallback(void(*draw)(void))
{
    uint8_t* reference=malloc((size_t)fp*fh);if(!reference)abort();memcpy(reference,expected,(size_t)fp*fh);
    for(int mode=0;mode<3;mode++) {
        reset();if(mode==0)enabled=0;else accepted=0;if(mode==2)barrier=0;
        unsigned before=oracle_calls;draw();
        if(before!=oracle_calls || memcmp(screen,mode==2?initial:reference,(size_t)fp*fh))abort();
    }
    free(reference);
}
int main(int argc,char** argv)
{
    if(argc!=2)return 2;output=fopen(argv[1],"wb");if(!output)return 2;
    word(0x3154464b);word(0);
    for(unsigned i=0;i<MAX;i++){first[i]=(i*29+i/317)&255;second[i]=(i*47+i/131+17)&255;}
    const int dimensions[][2]={{256,3},{319,199},{640,480}};
    for(unsigned d=0;d<3;d++)for(int dir=0;dir<2;dir++)for(int n=0;n<=32;n++) {
        fw=dimensions[d][0];fh=dimensions[d][1];fp=fw+7;progress=dir?32-n:n;
        for(unsigned i=0;i<sizeof(fade);i++)fade[i]=(i*13+(i>>8)*(n+1))&255;
        for(unsigned i=0;i<sizeof(pixmap.ghost);i++)pixmap.ghost[i]=(i*7+(i>>8)*(n+9))&255;
        reset();draw_fade();fallback(draw_fade);
    }
    fw=31;fh=23;fp=37;
    for(int p=0;p<3;p++)for(int x=0;x<4;x++)for(int y=0;y<3;y++)for(int edge=0;edge<2;edge++) {
        sx=x;sy=y;sw=edge?fp:19;sh=edge?fh:17;
        for(unsigned i=0;i<65536;i++)pixmap.ghost[i]=p==0?i:p==1?i>>8:(i*7+(i>>8)*31)&255;
        reset();draw_smooth();fallback(draw_smooth);
    }
    reset();struct MapFadeOracle o={screen,second,fade,pixmap.ghost,16,256,3};
    if(kfx_wgpu_map_fade(screen,263,256,3,screen,second,0,0,fade,pixmap.ghost,16,map_fade_oracle,&o))abort();
    fw=320;fh=200;fp=327;MyScreenWidth=fw;
    for(int mode=0;mode<3;mode++) {
        reset();player.view_mode_restore=mode;unsigned v=views,l=loads,p=parchments;
        prepare_map_fade_buffers(first,second,fw,fh);
        if(views!=v+1 || loads!=l+1 || parchments!=p+1)abort();
        for(int i=0;i<fw*fh;i++)if(first[i]!=(mode?17:39) || second[i]!=211)abort();
    }
    kfx_wgpu_native_snapshot_release(map_fade_src_snapshot);kfx_wgpu_native_snapshot_release(map_fade_dest_snapshot);
    fseek(output,4,SEEK_SET);word(count);fclose(output);
    printf("%u actual native transition cases; disabled, decline, barriers, capture state passed\n",count);return 0;
}
