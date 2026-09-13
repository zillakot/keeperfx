#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "kfx/renderer/WgpuMinimap.h"
typedef unsigned char TbPixel;
typedef int RealScreenCoord;
struct Around {int delta_x,delta_y;};
static struct {uint8_t *WScreen;long GraphicsScreenWidth,GraphicsScreenHeight;} lbDisplay;
static struct {int map_subtiles_x,map_subtiles_y;} game;
#include "minimap_enums.inc"
#define TERRAIN_ITEMS_MAX 256
#define TRAPDOOR_TYPES_MAX 2000
#define PLAYERS_COUNT 9
enum {PLAYER0,PLAYER1,PLAYER2,PLAYER3,PLAYER_GOOD,PLAYER_NEUTRAL,PLAYER4,PLAYER5,PLAYER6};
typedef int PlayerNumber;
static uint8_t player_room_colours[9]={1,17,33,49,65,81,97,113,129};
static uint8_t player_path_colours[9]={2,18,34,50,66,82,98,114,130};
static int turn,gui_blink_rate=2,neutral_flash_rate=3;
static int gui_room_type_highlighted=-1,gui_door_type_highlighted=-1;
static long PrevRoomHighlight=-1,PrevDoorHighlight=-1;
static int get_gameturn(void){return turn;}
static int get_player_color_idx(int p){return (p+2)%9;}
static struct {uint8_t ghost[65536],map_abyss[256];} pixmap;
#define PANEL_MAP_RADIUS 58
static long MapDiagonalLength,PanelMapX,PanelMapY,NumBackColours;
static uint8_t *MapBackground, MapBackColours[256], PanelColours[16*PnC_End];
static int32_t *MapShapeStart,*MapShapeEnd;
static uint16_t PanelMap[257*257];
static long LbSqrL(long n){return n>0?(long)sqrt((double)n):0;}
static FILE *output;
static unsigned count;
static uint32_t seed=73;
static uint32_t random32(void){seed=1664525u*seed+1013904223u;return seed;}
static void word(uint32_t n){uint8_t b[]={n,n>>8,n>>16,n>>24};if(fwrite(b,1,4,output)!=4)abort();}
int kfx_wgpu_native_enabled(void){return 1;}
int kfx_wgpu_native_cpu_barrier(void){return 1;}
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,const struct KfxWgpuDrawCommand *command,
    const struct KfxWgpuNativeResource *source,const struct KfxWgpuNativeResource *table,
    KfxWgpuNativeOracle oracle,void *context)
{
    (void)table;
    if(command->kind!=KFX_WGPU_DRAW_MINIMAP)abort();
    word(source->length);fwrite(source->bytes,1,source->length,output);
    for(unsigned y=0;y<target->height;y++)fwrite(target->pixels+y*target->pitch,1,target->width,output);
    oracle(target->pixels,target->pitch,context);
    for(unsigned y=0;y<target->height;y++)fwrite(target->pixels+y*target->pitch,1,target->width,output);
    count++;return 1;
}
#include "minimap.inc"
int main(int argc,char **argv)
{
    if(argc!=2)return 2;
    output=fopen(argv[1],"wb");if(!output)return 2;
    word(0);word(161);word(147);
    uint8_t pixels[161*147+2];memset(pixels,39,sizeof(pixels));
    lbDisplay.WScreen=pixels+1;lbDisplay.GraphicsScreenWidth=161;lbDisplay.GraphicsScreenHeight=147;
    game.map_subtiles_x=39;game.map_subtiles_y=31;PanelMapX=7;PanelMapY=11;
    for(int i=0;i<65536;i++)pixmap.ghost[i]=random32()>>24;
    for(int i=0;i<256;i++)pixmap.map_abyss[i]=random32()>>24;
    for(int scene=0;scene<8;scene++)
    {
        for(int i=0;i<161*147;i++)pixels[i+1]=(random32()>>24)%16*17;
        setup_background(8+scene);
        turn=scene;
        setup_panel_colors();
        for(int i=0;i<40*32;i++)
        {
            int owner=i%9;
            switch(i%5) {
            case 0:PanelMap[i]=i%13;break;
            case 1:PanelMap[i]=PnC_RoomsStart+owner+9*(i%256);break;
            case 2:PanelMap[i]=PnC_DoorsStart+owner+18*(i%2000);break;
            case 3:PanelMap[i]=PnC_DoorsStartLocked+owner+18*(i%2000);break;
            default:PanelMap[i]=PnC_PathStart+owner;break;
            }
        }
        for(int r=0;r<16;r++)
        {
            turn=scene*19+r;
            gui_room_type_highlighted=r%3?7:-1;
            gui_door_type_highlighted=r%2?11:-1;
            update_panel_colors();
            if(r==3)update_panel_color_player_color(2,7);
            int zoom=73+(r*97)%751;
            int32_t sx=(int32_t)(-sin(r*0.392699081698724)*65536)*zoom/256;
            int32_t sy=(int32_t)(cos(r*0.392699081698724)*65536)*zoom/256;
            int cx=(r%4==0?0:r%4==1?39:19)*65536;
            int cy=(r%3==0?0:r%3==1?31:15)*65536;
            uint32_t h[24]={0};h[6]=sx;h[7]=sy;
            h[8]=cx-MapDiagonalLength*sx/2-MapDiagonalLength*sy/2;
            h[9]=cy-MapDiagonalLength*sy/2+MapDiagonalLength*sx/2;
            map_command(h);
        }
        for(int r=0;r<18;r++)
        {
            map_pattern((r*11)%MapDiagonalLength,(r*17)%MapDiagonalLength,r==17?36:1+r*2, r%3?0:(r==15?-3:3), r*15);
            uint32_t h[24]={0};h[0]=2;h[16]=MapDiagonalLength/2+r-9;h[17]=MapDiagonalLength/2-r+9;
            h[18]=r*3;h[20]=255-r*13;h[21]=10*(8+scene)/16;map_command(h);
            memset(h,0,sizeof(h));h[0]=3;h[16]=(MapDiagonalLength/2)<<8;h[17]=(MapDiagonalLength/2)<<8;
            h[6]=(r-9)*101;h[7]=(9-r)*103;h[18]=r==17?36:1+r*2;h[20]=15;h[21]=127;map_command(h);
        }
    }
    if(pixels[0]!=39||pixels[sizeof(pixels)-1]!=39)abort();
    rewind(output);word(count);fclose(output);
    free(MapBackground);free(MapShapeStart);free(MapShapeEnd);
    printf("%u actual native minimap command fixtures\n",count);
    return 0;
}
