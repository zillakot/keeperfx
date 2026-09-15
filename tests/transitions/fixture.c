#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "kfx/renderer/software/WgpuTransition.h"
#define MAX (650*480)
#define min(a,b) ((a)<(b)?(a):(b))
#define SYNCDBG(...) ((void)0)
static int32_t xtab[640][2],ytab[480][2];
static uint64_t map_fade_src_snapshot,map_fade_dest_snapshot;
static const uint8_t *map_fade_src_owner,*map_fade_dest_owner;
static int map_fade_snapshot_width,map_fade_snapshot_height,map_fade_snapshot_pitch;
static int map_fade_buffers_valid=1;
static struct { uint8_t *WScreen; int GraphicsScreenWidth,GraphicsScreenHeight; } lbDisplay;
static struct { uint8_t ghost[65536],fade_tables[33*256]; } pixmap;
static int MyScreenWidth,MyScreenHeight,pixel_size=1;
struct PlayerInfo { int view_mode_restore,view_mode,instance_num,instance_remain_turns,id_number,allocflags; };
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
static unsigned count,enabled=1,accepted=1,barrier=1,oracle_calls,invalid_frames,frame_valid=1;
static uint8_t initial[MAX],expected[MAX],screen[MAX],first[MAX],second[MAX],fade[33*256];
static int fw,fh,fp,progress,sx,sy,sw,sh;
static void word(uint32_t n){uint8_t b[]={n,n>>8,n>>16,n>>24};if(fwrite(b,1,4,output)!=4)abort();}
int kfx_wgpu_native_enabled(void){return enabled;}
int kfx_wgpu_native_read_barrier(const void* bytes, size_t length) { (void)bytes; (void)length; return 1; }
void kfx_wgpu_native_flush(void) {}
int kfx_wgpu_native_cpu_barrier(void){return barrier;}
void kfx_wgpu_native_invalidate_frame(void){invalid_frames++;frame_valid=0;}
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
    /* A map fade names the fade rows and the ghost table where they live; smoothing has
       only the one table. Either way nothing is concatenated before the call. */
    if(c->source_x==0 ? (table->length!=33*256 || table->tail_length!=65536 || !table->tail)
        : (table->length!=65536 || table->tail || table->tail_length))abort();
    word(table->length+table->tail_length);fwrite(table->bytes,1,table->length,output);
    if(table->tail_length)fwrite(table->tail,1,table->tail_length,output);
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
#define PALETTE_COLORS 256
static uint8_t poly_pool[65536+128000],engine_palette[768];
static uint8_t *map_fade_src,*map_fade_dest,*map_fade_ghost_table;
static unsigned ghost_generations,mode_changes,engine_changes,menu_changes,maintains;
static void generate_map_fade_ghost_table(const char* name,uint8_t* palette,uint8_t* table)
{(void)name;(void)palette;ghost_generations++;for(unsigned i=0;i<65536;i++)table[i]=(i*3+(i>>8)*7)&255;}
#include "progress.inc"
enum {PVT_MapScreen=3,PVM_ParchmentView=4,PVM_ParchFadeIn=5,PVM_ParchFadeOut=6,PI_Unset=0,PlaF_MouseInputDisabled=1,PLAYER_INSTANCES_COUNT=3};
static int my_player_number;
static struct {int tooltips_on;} settings;
static struct {int tooltips_restore,status_menu_restore;} local_state;
static int is_my_player(struct PlayerInfo* p){return p==&player;}
static struct PlayerInfo* get_player(int n){(void)n;return &player;}
static void set_player_mode(struct PlayerInfo* p,int mode){if(mode!=PVT_MapScreen)abort();p->view_mode=PVM_ParchmentView;mode_changes++;}
static void set_engine_view(struct PlayerInfo* p,int view){p->view_mode=view;engine_changes++;}
static void toggle_status_menu(int visible){(void)visible;menu_changes++;}
#include "finish_in.inc"
#include "finish_out.inc"
typedef long (*InstncInfo_Func)(struct PlayerInfo*,int32_t*);
static struct PlayerInstanceInfo { InstncInfo_Func maintain_cb,end_cb;int32_t maintain_end_callback_parameter; } player_instance_info[3];
static long maintain(struct PlayerInfo* p,int32_t* n){(void)p;(void)n;maintains++;return 0;}
#include "instance.inc"
static void check_failed_progress(void)
{
    fw=320;fh=200;fp=327;MyScreenWidth=320;MyScreenHeight=200;
    for(int direction=0;direction<2;direction++)for(int retry=0;retry<2;retry++) {
        reset();accepted=0;enabled=retry?0:1;barrier=retry?1:0;
        player.instance_num=direction+1;player.instance_remain_turns=8;
        player.view_mode=direction?PVM_ParchFadeOut:PVM_ParchFadeIn;
        player.view_mode_restore=PVM_IsoStraightView;player.allocflags=PlaF_MouseInputDisabled;
        player_instance_info[direction+1]=(struct PlayerInstanceInfo){maintain,direction?pinstfe_fade_from_map:pinstfe_fade_to_map,0};
        unsigned v=views,p=parchments,l=loads,g=ghost_generations,m=maintains;
        unsigned ends=direction?engine_changes:mode_changes;
        long step=direction?32:0;
        for(int frame=0;frame<8;frame++) {
            frame_valid=1;
            process_player_instance(&player);
            if(player.view_mode==PVM_ParchFadeIn)step=map_fade_in(step);
            else if(player.view_mode==PVM_ParchFadeOut)step=map_fade_out(step);
            else {
                if(player.view_mode!=PVM_ParchmentView && player.view_mode!=PVM_IsoStraightView)abort();
                if(frame!=7 || !frame_valid)abort();
            }
            if(frame<7 && frame_valid!=(unsigned)retry)abort();
        }
        if(views!=v+1 || parchments!=p+1 || loads!=l+1 || ghost_generations!=g+1 || maintains!=m+8 ||
            (direction?engine_changes:mode_changes)!=ends+1 || player.instance_num!=PI_Unset ||
            player.allocflags&PlaF_MouseInputDisabled || map_fade_buffers_valid!=retry || step!=(direction?4:28))abort();
    }
}
int main(int argc,char** argv)
{
    if(argc!=2)return 2;
    output=fopen(argv[1],"wb");
    if(!output)return 2;
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
    fw=256;fh=3;fp=263;progress=16;
    for(int alias=1;alias<=3;alias++) {
        reset();memcpy(expected,screen,(size_t)fp*fh);
        map_fade_native(expected,alias&1?expected:first,alias&2?expected:second,
            fade,pixmap.ghost,progress,fw,fh,fp);
        unsigned calls=oracle_calls;
        map_fade(screen,alias&1?screen:first,alias&2?screen:second,
            fade,pixmap.ghost,progress,fw,fh,fp);
        if(oracle_calls!=calls || memcmp(screen,expected,(size_t)fp*fh))abort();
    }
    fw=320;fh=200;fp=327;MyScreenWidth=fw;
    for(int mode=0;mode<3;mode++) {
        reset();player.view_mode_restore=mode;unsigned v=views,l=loads,p=parchments;
        prepare_map_fade_buffers(first,second,fw,fh);
        if(views!=v+1 || loads!=l+1 || parchments!=p+1)abort();
        for(int i=0;i<fw*fh;i++)if(first[i]!=(mode?17:39) || second[i]!=211)abort();
    }
    fw=256;fh=3;fp=263;progress=16;reset();
    map_fade(screen,second,first,fade,pixmap.ghost,progress,fw,fh,fp);
    kfx_wgpu_native_snapshot_release(map_fade_src_snapshot);kfx_wgpu_native_snapshot_release(map_fade_dest_snapshot);
    map_fade_src_snapshot=map_fade_dest_snapshot=0;
    reset();accepted=barrier=0;
    unsigned v=views,l=loads,p=parchments;
    prepare_map_fade_buffers(first,second,256,3);
    if(map_fade_buffers_valid || views!=v+1 || loads!=l+1 || parchments!=p+1)abort();
    for(int frame=0;frame<3;frame++) {
        reset();enabled=0;unsigned bad=invalid_frames;
        map_fade(screen,second,first,fade,pixmap.ghost,frame*4,256,3,263);
        if(invalid_frames!=bad+1 || memcmp(screen,initial,(size_t)fp*fh))abort();
    }
    check_failed_progress();
    fseek(output,4,SEEK_SET);word(count);fclose(output);
    printf("%u actual native transition cases; disabled, decline, barriers, capture state passed\n",count);return 0;
}
