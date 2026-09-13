#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stddef.h>
#include "kfx/renderer/software/WgpuMapView.h"
#define PITCH 211
#define HEIGHT 139
#define LANDVIEW_MAP_WIDTH 512
#define LANDVIEW_MAP_HEIGHT 384
typedef uint8_t TbPixel;
typedef int MapSlabCoord;
static struct { uint8_t *WScreen,*GraphicsWindowPtr; long GraphicsScreenWidth,GraphicsScreenHeight,PhysicalScreenWidth,PhysicalScreenHeight; } lbDisplay;
static struct { uint8_t ghost[65536],map_abyss[256],fade_tables[64*256]; } pixmap;
static struct { long screen_shift_x,screen_shift_y; } map_info;
static uint8_t *vec_screen,*map_screen,*block_ptrs[1];
static unsigned long vec_screen_width;
static long vec_window_width,vec_window_height,units_per_pixel_landview;
static int window_x,window_y;
static long scale_value_landview(long v){return v*units_per_pixel_landview/16;}
static uint8_t *SwTargetWScreen(void){return lbDisplay.WScreen;}
static uint8_t *SwTargetGraphicsWindowPtr(void){return lbDisplay.GraphicsWindowPtr;}
static int32_t SwTargetScanline(void){return lbDisplay.GraphicsScreenWidth;}
static int32_t SwTargetScreenHeight(void){return lbDisplay.GraphicsScreenHeight;}
static int32_t SwTargetWindowX(void){return window_x;}
static int32_t SwTargetWindowY(void){return window_y;}
struct Around {int delta_x,delta_y;};
#include "pattern.inc"
enum {WgpuPixel};
struct WgpuPrimitive {int kind;long x,y,a,b;TbPixel colour;};
static int wgpu_primitive(struct WgpuPrimitive p, uint32_t k,long x,long y,long w,long h,long r){(void)p;(void)k;(void)x;(void)y;(void)w;(void)h;(void)r;return 0;}
#include "pixel.inc"
#include "styles.inc"
#include "row.inc"
#include "texture.inc"
#include "zoom.inc"
#include "marker.inc"
static FILE *output;
static unsigned count,enabled=1,accepted=1,barrier=1,oracle_calls;
static uint8_t initial[PITCH*HEIGHT],expected[PITCH*HEIGHT];
static uint32_t seed=73;
static uint8_t random_byte(void){seed=1664525u*seed+1013904223u;return seed>>24;}
static void word(uint32_t n){uint8_t b[]={n,n>>8,n>>16,n>>24};if(fwrite(b,1,4,output)!=4)abort();}
int kfx_wgpu_native_enabled(void){return enabled;}
int kfx_wgpu_native_cpu_barrier(void){return barrier;}
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,const struct KfxWgpuDrawCommand *command,
    const struct KfxWgpuNativeResource *source,const struct KfxWgpuNativeResource *table,KfxWgpuNativeOracle oracle,void *context)
{
    if(!accepted)return 0;
    if(table || !source || command->kind!=KFX_WGPU_DRAW_MAP_VIEW || memcmp(target->pixels,initial,sizeof(initial)))abort();
    memcpy(expected,initial,sizeof(expected));
    oracle(expected,PITCH,context);oracle_calls++;
    if(memcmp(target->pixels,initial,sizeof(initial)))abort();
    word(target->width);word(target->height);word(target->pitch);
    word(source->width);word(source->height);word(source->pitch);word(source->length);
    const uint32_t *words=(const uint32_t*)command;
    for(unsigned i=0;i<28;i++)word(words[i]);
    fwrite(source->bytes,1,source->length,output);
    fwrite(expected,1,sizeof(expected),output);
    count++;return 1;
}
static void reset(void)
{
    for(unsigned i=0;i<sizeof(initial);i++)initial[i]=(i*19+i/PITCH*13)&255;
    memcpy(lbDisplay.WScreen,initial,sizeof(initial));
    enabled=accepted=barrier=1;
}
static void check_fallback(void (*draw)(void))
{
    uint8_t reference[PITCH*HEIGHT];
    memcpy(reference,expected,sizeof(reference));
    for(int mode=0;mode<2;mode++) {
        reset();if(mode)accepted=0;else enabled=0;
        unsigned before=oracle_calls;draw();
        if(oracle_calls!=before || memcmp(lbDisplay.WScreen,reference,sizeof(reference)))abort();
    }
    if(draw != NULL) {
        reset();accepted=barrier=0;
        draw();
        if(memcmp(lbDisplay.WScreen,initial,sizeof(initial)))abort();
    }
}
static struct OverheadRow row;
static void draw_row(void)
{
    if(!kfx_wgpu_map_row(lbDisplay.WScreen,PITCH,HEIGHT,row.x,row.y,row.block_size,row.styles,row.count,pixmap.ghost,pixmap.map_abyss,overhead_row_native,&row))if(kfx_wgpu_native_cpu_barrier())overhead_row_native(lbDisplay.WScreen,PITCH,&row);
}
static struct MapTextureOracle tex;
static void draw_tex(void){scale_tmap2(tex.texture,tex.flags,tex.fade,tex.x,tex.y,tex.width,tex.height);}
static struct FrontZoomOracle zoom;
static void draw_zoom(void){frontzoom_to_point(zoom.x,zoom.y,zoom.zoom);}
static struct OverheadMarker marker;
static void draw_marker(void){overhead_marker(marker.x,marker.y,marker.count,marker.spread,marker.cross,marker.colour);}
int main(int argc,char **argv)
{
    if(argc!=2)return 2;
    output=fopen(argv[1],"wb");if(!output)return 2;
    word(0x3156464b);word(0);word(PITCH);word(HEIGHT);word(112);
    uint8_t pixels[PITCH*HEIGHT],texture[31*256+32];
    uint8_t *map=malloc(LANDVIEW_MAP_WIDTH*LANDVIEW_MAP_HEIGHT);if(!map)abort();
    lbDisplay.WScreen=pixels;lbDisplay.GraphicsWindowPtr=pixels;lbDisplay.GraphicsScreenWidth=PITCH;lbDisplay.GraphicsScreenHeight=HEIGHT;
    lbDisplay.PhysicalScreenWidth=PITCH-4;lbDisplay.PhysicalScreenHeight=HEIGHT;
    vec_screen=pixels;vec_screen_width=PITCH;vec_window_width=PITCH-4;vec_window_height=HEIGHT-3;
    block_ptrs[0]=texture;map_screen=map;
    for(unsigned i=0;i<sizeof(texture);i++)texture[i]=random_byte();
    for(unsigned i=0;i<LANDVIEW_MAP_WIDTH*LANDVIEW_MAP_HEIGHT;i++)map[i]=random_byte();
    for(unsigned i=0;i<sizeof(pixmap);i++)((uint8_t*)&pixmap)[i]=random_byte();
    int styles[128];
    for(int bs=1;bs<=16;bs*=2)for(int phase=0;phase<7;phase++) {
        int n=(PITCH-7)/bs;if(n>128)n=128;
        for(int i=0;i<n;i++)styles[i]=(i+phase*43)%263;
        row=(struct OverheadRow){3,HEIGHT-bs-2,bs,n,styles};reset();draw_row();check_fallback(draw_row);
        for(int i=0;i<n;i++)styles[i]=256+i%7;
        reset();draw_row();check_fallback(draw_row);
    }
    const int sizes[][2]={{1,1},{2,3},{7,11},{31,32},{32,31},{33,35},{63,57},{207,136},{640,480}};
    const int origins[][2]={{0,0},{-13,-9},{198,128},{-700,-500}};
    for(int flags=0;flags<=0x70;flags+=16)for(unsigned z=0;z<sizeof(sizes)/sizeof(*sizes);z++)for(unsigned o=0;o<4;o++)for(int f=0;f<3;f++) {
        tex=(struct MapTextureOracle){0,flags,f==0?-1:f==1?0:63,origins[o][0],origins[o][1],sizes[z][0],sizes[z][1]};
        reset();draw_tex();check_fallback(draw_tex);
    }
    const int scales[]={16,20,24,32},zooms[]={0,15,100,255,256};
    for(unsigned u=0;u<4;u++)for(unsigned z=0;z<5;z++)for(int corner=0;corner<5;corner++) {
        units_per_pixel_landview=scales[u];zoom=(struct FrontZoomOracle){256,192,zooms[z]};
        int x=corner==0?1:corner==1?206:103,y=corner==2?1:corner==3?138:70;
        map_info.screen_shift_x=256-x*16/units_per_pixel_landview;
        map_info.screen_shift_y=192-y*16/units_per_pixel_landview;
        reset();draw_zoom();check_fallback(draw_zoom);
    }
    for(int n=1;n<=36;n++)for(int c=0;c<3;c++)for(int pos=0;pos<3;pos++) {
        window_x=pos?7:0;window_y=pos?3:0;
        lbDisplay.GraphicsWindowPtr=pixels+window_y*PITCH+window_x;
        marker=(struct OverheadMarker){pos==2?-9:101,61,n,c==2?-6:6,c!=0,60+n};
        reset();draw_marker();check_fallback(draw_marker);
    }
    fseek(output,4,SEEK_SET);word(count);fclose(output);free(map);
    printf("%u actual native map-view cases, disabled and declined fallback matched\n",count);return 0;
}
