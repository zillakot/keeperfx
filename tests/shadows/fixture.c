#include "kfx/renderer/software/bflib_render_trig.c"
#include "kfx/renderer/software/WgpuShadow.h"
#include <stdarg.h>
#include "creature_graphics.h"
#include "engine_render.h"
unsigned char vec_mode, vec_colour;
unsigned char *vec_screen, *vec_map, *poly_screen, *big_scratch;
unsigned long vec_screen_width = 83;
long vec_window_width = 79, vec_window_height = 61;
struct PolyPoint scans[1024];
struct PolyPoint *polyscans = scans;
struct TbColorTables pixmap;
unsigned char block_mem[TEXTURE_VARIATIONS_COUNT * TEXTURE_BLOCKS_STAT_COUNT * 32 * 32];
uint32_t get_gameturn(void) { return 0; }
int LbErrorLog(const char *format, ...) { (void)format; return 0; }
static struct KeeperSprite selection_sprites[10];
static unsigned selection_index, heap_calls, selection_missing;
static int fixture_disabled;
TbSpriteData *keepsprite[KEEPSPRITE_LENGTH];
TbSpriteData keepersprite_add[KEEPERSPRITE_ADD_NUM];
unsigned long keepersprite_index(unsigned short n) { (void)n; return selection_index; }
struct KeeperSprite *keepersprite_array(unsigned short n) { (void)n; return selection_missing ? NULL : selection_sprites; }
static long heap_manage_keepersprite(unsigned short n) { (void)n; heap_calls++; return 1; }
#include "shadow_oracle.inc"
static unsigned char pixels[83 * 63], shadow_storage[65536 + 8], rle[132000];
static uint32_t seed = 0x971413;
static uint32_t random32(void) { seed = seed * 1664525u + 1013904223u; return seed; }
static FILE *output;
static unsigned emitted;
uint64_t shadow_hash;
static void word(FILE *f, uint32_t n) { unsigned char b[] = {n,n>>8,n>>16,n>>24}; if(fwrite(b,1,4,f)!=4)abort(); }
#ifndef KFX_SHADOW_NATIVE
int kfx_wgpu_native_enabled(void) { return !fixture_disabled; }
int kfx_wgpu_native_read_barrier(const void* bytes, size_t length) { (void)bytes; (void)length; return 1; }
void kfx_wgpu_native_flush(void) { kfx_wgpu_terrain_boundary(0); }
int kfx_wgpu_native_cpu_barrier(void) { return 1; }
void kfx_wgpu_terrain_boundary(int allow) { (void)allow; }
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target, const struct KfxWgpuDrawCommand *command,
    const struct KfxWgpuNativeResource *source, const struct KfxWgpuNativeResource *table,
    KfxWgpuNativeOracle oracle, void *context) {
    (void)target;(void)command;(void)source;(void)table;(void)oracle;(void)context;abort();
}
int kfx_wgpu_native_shadow(const struct KfxGpolyTarget *target, const struct KfxWgpuDrawCommand *command,
    const struct KfxWgpuNativeResource *source, const struct KfxWgpuNativeResource *table,
    uint8_t *mirror, KfxWgpuNativeOracle oracle, void *context) {
    (void)table;
    word(output,source->length);word(output,command->colour);
    fwrite(source->bytes,1,source->length,output);
    oracle(target->pixels,target->pitch,context);
    fwrite(mirror,1,65536,output);
    for(unsigned y=0;y<61;y++)fwrite(target->pixels+y*target->pitch,1,79,output);
    emitted++;
    return 1;
}
#endif
int shadow_cases(FILE *file, int expected_accept) {
    output=file; seed=0x971413; shadow_hash=0;
    memset(pixels,167,sizeof(pixels));memset(shadow_storage,203,sizeof(shadow_storage));
    vec_screen=pixels+83;poly_screen=pixels;big_scratch=shadow_storage+4;vec_map=big_scratch;
    vec_mode=10;
    for(unsigned i=0;i<81920;i++)((uint8_t *)&pixmap)[i]=random32()>>24;
    if(file) {word(file,192);fwrite(pixmap.fade_tables,1,16384,file);fwrite(pixmap.ghost,1,65536,file);}
    for(unsigned c=0;c<192;c++) {
        unsigned padding=1+c%4;
        memset(shadow_storage,203,sizeof(shadow_storage));
        big_scratch=shadow_storage+padding;vec_map=big_scratch;
        unsigned w=1+c%31,h=1+(c*7)%29;
        if(c%16<2)w=256;
        unsigned flip=c&1, offx=flip?w:0, offy=c%4;
        if(w<32)offx+=c%9;
        unsigned fw=w<256?w+12:256,fh=h+offy;
        struct ShadowOracle oracle={0};
        oracle.sprite=(struct KfxShadowSprite){rle,fw,fh,w,h,offx,offy,flip};
        oracle.scratch=big_scratch;
        unsigned n=0;
        for(unsigned y=0;y<h;y++) {
            for(unsigned x=0;x<w;) {
                unsigned run=1+random32()%11;if(run>w-x)run=w-x;
                unsigned solid=random32()&1;
                rle[n++]=solid?run:-(int)run;
                if(solid)for(unsigned j=0;j<run;j++)rle[n++]=j&1?255:0;
                x+=run;
                if(c%11==0 && x>w/2)break;
            }
            rle[n++]=0;
        }
        for(unsigned i=0;i<65536;i++)big_scratch[i]=random32()>>24;
        struct PolyPoint v[]={{3,53,0,(fh-1)<<16,0},{7,4,0,0,0},
            {72,2,(fw-1)<<16,0,0},{75,49,(fw-1)<<16,(fh-1)<<16,0}};
        if(c%3==0){v[0].X-=20;v[1].X-=20;v[2].X+=20;v[3].X+=20;}
        if(c%5==0){v[0].Y-=12;v[3].Y-=12;v[1].Y+=15;v[2].Y+=15;}
        memcpy(oracle.vertices,v,sizeof(v));vec_colour=c%64;
        int accepted=kfx_wgpu_shadow_sprite(&oracle.sprite,v,big_scratch,shadow_oracle,&oracle);
        if(accepted!=(expected_accept==2 ? c==0 : expected_accept)) {fprintf(stderr,"shadow acceptance mismatch case %u\n",c);return 1;}
        if(!accepted)shadow_oracle(vec_screen,83,&oracle);
        for(unsigned i=0;i<65536;i++)shadow_hash=shadow_hash*33+big_scratch[i];
        for(unsigned y=0;y<61;y++)for(unsigned x=0;x<79;x++)shadow_hash=shadow_hash*33+vec_screen[y*83+x];
        for(unsigned y=0;y<63;y++)for(unsigned x=(y==0||y==62)?0:79;x<83;x++)if(pixels[y*83+x]!=167)abort();
        for(unsigned i=0;i<padding;i++)if(shadow_storage[i]!=203)abort();
        for(unsigned i=padding+65536;i<sizeof(shadow_storage);i++)if(shadow_storage[i]!=203)abort();
        if(vec_screen!=pixels+83||poly_screen!=pixels||vec_map!=big_scratch||vec_screen_width!=83)abort();
    }
    return 0;
}
#ifndef KFX_SHADOW_NATIVE
static int selection_cases(void) {
    unsigned char artwork[10][10], *pointers[10], expected[65536];
    const short angles[]={0,1151,1152,1918,1919,2047};
    const unsigned frames[]={0,1,255};
    struct PolyPoint vertices[4]={{0}};
    fixture_disabled=1;
    for(unsigned i=0;i<10;i++) {
        artwork[i][0]=1+i%5;
        memset(artwork[i]+1,255,1+i%5);artwork[i][2+i%5]=0;
        pointers[i]=artwork[i];keepsprite[i]=&pointers[i];keepersprite_add[i]=pointers[i];
        selection_sprites[i]=(struct KeeperSprite){.SWidth=1+i%5,.SHeight=1,.FrameWidth=12,.FrameHeight=3,.FramesCount=2,.FrameOffsW=2,.FrameOffsH=1};
    }
    for(unsigned added=0;added<2;added++)for(unsigned rot=0;rot<2;rot++)for(unsigned a=0;a<6;a++)for(unsigned f=0;f<3;f++) {
        selection_index=added?KEEPERSPRITE_ADD_OFFSET:0;selection_sprites[0].Rotable=rot?2:0;
        unsigned frame=frames[f]>1?1:frames[f],quarter=abs(4-(((angles[a]+128)&2047)>>8));
        unsigned selected=frame+(rot?quarter*2:0),flip=angles[a]>1151&&angles[a]<1919;
        struct KeeperSprite *sp=&selection_sprites[selected];
        struct KfxShadowSprite desc={artwork[selected],rot?sp->SWidth:12,rot?1:3,sp->SWidth,1,rot?(flip?sp->SWidth:0):(flip?10:2),rot?0:1,flip};
        memset(big_scratch,101,65536);memset(expected,101,65536);heap_calls=0;
        shadow_native_mask(&desc,expected);
        if(draw_keepsprite_unscaled_in_buffer(0,angles[a],frames[f],big_scratch,vertices)!=0 || heap_calls!=1 || memcmp(expected,big_scratch,65536))return 1;
    }
    selection_missing=1;heap_calls=0;
    if(draw_keepsprite_unscaled_in_buffer(0,0,0,big_scratch,vertices)!=0||heap_calls)return 1;
    selection_missing=0;fixture_disabled=0;
    fprintf(stderr,"72 native frame/orientation selections verified\n");
    return 0;
}
int main(int argc,char **argv) {
    big_scratch=shadow_storage+4;
    if(selection_cases())return 2;
    if(argc!=2)return 1;FILE *f=fopen(argv[1],"wb");if(!f)return 1;
    int result=shadow_cases(f,1);fclose(f);
    fprintf(stderr,"%u actual native shadow fixtures\n",emitted);
    return result;
}
#endif
