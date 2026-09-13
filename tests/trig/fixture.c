#include "kfx/renderer/software/bflib_render_trig.c"
#include <stdarg.h>

unsigned char vec_mode, vec_colour;
unsigned char *vec_screen, *vec_map, *poly_screen, *big_scratch;
unsigned long vec_screen_width = 83;
long vec_window_width = 79, vec_window_height = 61;
struct PolyPoint scans[1024];
struct PolyPoint *polyscans = scans;
struct TbColorTables pixmap;
unsigned char block_mem[TEXTURE_VARIATIONS_COUNT * TEXTURE_BLOCKS_STAT_COUNT * 32 * 32];
#ifndef KFX_TRIG_NATIVE
int kfx_wgpu_native_enabled(void) { return 0; }
void kfx_wgpu_terrain_boundary(int allow) { (void)allow; }
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target, const struct KfxWgpuDrawCommand *command,
    const struct KfxWgpuNativeResource *source, const struct KfxWgpuNativeResource *table,
    KfxWgpuNativeOracle oracle, void *context) {
    (void)target; (void)command; (void)source; (void)table; (void)oracle; (void)context; return 0;
}
#endif
uint32_t get_gameturn(void) { return 0; }
int LbErrorLog(const char *format, ...) { (void)format; return 0; }
static unsigned char pixels[83 * 63], texture[65536];
static uint32_t seed = 0x98124412;
static uint32_t random32(void) { seed = seed * 1664525u + 1013904223u; return seed; }
static void word(FILE *f, uint32_t n) {
    unsigned char bytes[] = {n, n >> 8, n >> 16, n >> 24};
    if (fwrite(bytes, 1, 4, f) != 4) abort();
}
static void emit(FILE *f, struct PolyPoint vertices[3], unsigned mode, unsigned colour) {
    vec_mode = mode; vec_colour = colour;

    word(f, mode); word(f, colour);
    for (unsigned i = 0; i < 3; i++) {
        word(f, vertices[i].X); word(f, vertices[i].Y);
        word(f, vertices[i].U); word(f, vertices[i].V); word(f, vertices[i].S);
    }
    trig(&vertices[0], &vertices[1], &vertices[2]);
    for (unsigned y = 0; y < 61; y++) {
        if (fwrite(vec_screen + y * 83, 1, 79, f) != 79) abort();
        for (unsigned x = 79; x < 83; x++) if (vec_screen[y * 83 + x] != 167) abort();
    }
    for (unsigned x = 0; x < 83; x++) if (pixels[x] != 167 || pixels[83*62+x] != 167) abort();
}
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    FILE *f = fopen(argv[1], "wb"); if (!f) return 1;
    memset(pixels, 167, sizeof(pixels)); vec_screen = pixels + 83; poly_screen = pixels; vec_map = texture;
    for (unsigned i = 0; i < sizeof(texture); i++) texture[i] = (random32() >> 24) & (i < 8192 ? 63 : 255);
    for (unsigned i = 0; i < sizeof(pixmap.fade_tables); i++) pixmap.fade_tables[i] = random32() >> 24;
    for (unsigned i = 0; i < sizeof(pixmap.ghost); i++) pixmap.ghost[i] = random32() >> 24;
    fwrite("KFXTRIG1", 1, 8, f); word(f, 79); word(f, 61); word(f, 0);
    fwrite(texture, 1, sizeof(texture), f); fwrite(pixmap.fade_tables, 1, sizeof(pixmap.fade_tables), f);
    fwrite(pixmap.ghost, 1, sizeof(pixmap.ghost), f);
    const unsigned modes[] = {0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26};
    const int xy[][6] = {
        {3,2,73,2,39,59}, {3,2,73,59,3,59}, {3,2,73,22,20,59}, {3,2,73,59,5,22},
        {-30,-20,105,10,20,90}, {-30,-20,105,90,0,10}, {-10,-20,103,-20,23,80},
        {0,-20,103,90,-30,90}, {-20,-90,110,-10,20,70}, {-20,-90,110,70,20,-10},
        {-100,1,-10,2,-60,55}, {90,1,190,20,120,59}, {0,80,70,80,20,100},
        {2,3,50,3,70,3}, {5,5,5,5,5,5}, {0,0,1,0,0,61},
        {-32767,-3,0,10,-30000,60}, {0,-32767,100,0,0,0},
    };
    unsigned count = 0;
    for (unsigned mode = 0; mode < sizeof(modes)/sizeof(*modes); mode++) {
        for (unsigned shape = 0; shape < sizeof(xy)/sizeof(*xy); shape++) {
            for (unsigned permutation = 0; permutation < 6; permutation++) {
                const unsigned order[][3] = {{0,1,2},{1,2,0},{2,0,1},{0,2,1},{2,1,0},{1,0,2}};
                struct PolyPoint v[3], original[3];
                for (unsigned i = 0; i < 3; i++) original[i] = (struct PolyPoint){
                    xy[shape][2*i], xy[shape][2*i+1],
                    (long)((int)(random32() % (1024*65536)) - 512*65536),
                    (long)((int)(random32() % (1024*65536)) - 512*65536),
                    (long)(random32() % (63*65536))};
                if (modes[mode] == 5 || modes[mode] == 6 || modes[mode] == 20 || modes[mode] == 21 || modes[mode] >= 24)
                    for (unsigned i = 0; i < 3; i++) original[i].S = 17*65536;
                for (unsigned i = 0; i < 3; i++) v[i] = original[order[permutation][i]];
                emit(f, v, modes[mode], shape % 3 == 0 ? 32 : 17); count++;
            }
        }
    }
    for (unsigned n = 0; n < 1140; n++) {
        struct PolyPoint v[3] = {
            {-30 + (int)(random32()%50), -30 + (int)(random32()%35), 0,0,0},
            {50 + (int)(random32()%70), 10 + (int)(random32()%20), 0,0,0},
            {-40 + (int)(random32()%70), 45 + (int)(random32()%60), 0,0,0}
        };
        for (unsigned i = 0; i < 3; i++) {
            v[i].U = (int)(random32() % (1024*65536)) - 512*65536;
            v[i].V = (int)(random32() % (1024*65536)) - 512*65536;
            v[i].S = random32() % (63*65536);
        }
        if (modes[n % 27] == 5 || modes[n % 27] == 6 || modes[n % 27] == 20 || modes[n % 27] == 21 || modes[n % 27] >= 24)
            for (unsigned i = 0; i < 3; i++) v[i].S = 17*65536;
        emit(f, v, modes[n % 27], n % 3 == 0 ? 32 : 17); count++;
    }
    const long boundary[] = {-65537,-65536,-1,0,65535,65536,31*65536,32*65536,255*65536,256*65536};
    for (unsigned mode = 0; mode < 27; mode++) for (unsigned j = 0; j < 10; j++) {
        struct PolyPoint v[3] = {
            {-3,-2,boundary[j],boundary[j],0},
            {78,0,boundary[(j+1)%10],boundary[(j+2)%10],63*65536},
            {0,61,boundary[(j+2)%10],boundary[(j+1)%10],31*65536}
        };
        if (modes[mode] == 5 || modes[mode] == 6 || modes[mode] == 20 || modes[mode] == 21 || modes[mode] >= 24)
            for (unsigned i = 0; i < 3; i++) v[i].S = (j % 2 ? 63 : 0)*65536;
        emit(f,v,modes[mode],j % 2 ? 32 : 17); count++;
    }
    const unsigned shaded_modes[] = {5,6,20,21,24,25,26};
    for (unsigned mode = 0; mode < 7; mode++) for (unsigned n = 0; n < 120; n++) {
        const int small_xy[][6] = {
            {-3,-2,15,2,2,19}, {-3,-2,15,19,2,2},
            {-3,-2,15,-2,2,19}, {-3,-2,15,19,2,19}
        };
        const unsigned order[][3] = {{0,1,2},{1,2,0},{2,0,1},{0,2,1},{2,1,0},{1,0,2}};
        struct PolyPoint v[3], original[3];
        for (unsigned i = 0; i < 3; i++) {
            v[i].X = small_xy[n % 4][i*2]; v[i].Y = small_xy[n % 4][i*2+1];
            v[i].U = (int)(random32() % (1024*65536)) - 512*65536;
            v[i].V = (int)(random32() % (1024*65536)) - 512*65536;
            v[i].S = 30*65536 + (int)(random32() % (4*65536)) - 2*65536;
        }
        memcpy(original, v, sizeof(v));
        for (unsigned i = 0; i < 3; i++) v[i] = original[order[(n/4)%6][i]];
        emit(f,v,shaded_modes[mode],17); count++;
    }
    const unsigned samples[] = {0,1,12,13,63,64,127,255};
    for (unsigned mode = 0; mode < 27; mode++) for (unsigned j = 0; j < 8; j++) {
        if (mode == 9 && samples[j] >= 64) continue;
        unsigned index = 0;
        while (index < sizeof(texture) && texture[index] != samples[j]) index++;
        if (index == sizeof(texture)) abort();
        struct PolyPoint v[3] = {
            {0,0,(index & 255)*65536,(index >> 8)*65536,31*65536},
            {20,0,(index & 255)*65536,(index >> 8)*65536,31*65536},
            {0,20,(index & 255)*65536,(index >> 8)*65536,31*65536}
        };
        emit(f,v,mode,17); count++;
    }
    const int thin_xy[][6] = {{1,0,2,1,3,3}, {3,0,1,4,1,3}};
    const int thin_offsets[][2] = {{0,0}, {-2,0}, {77,0}, {0,-1}, {0,59}};
    const unsigned thin_order[][3] = {{0,1,2},{1,2,0},{2,0,1},{0,2,1},{2,1,0},{1,0,2}};
    for (unsigned mode = 0; mode < 27; mode++) for (unsigned kind = 0; kind < 2; kind++)
        for (unsigned offset = 0; offset < 5; offset++) for (unsigned prior = 0; prior < 2; prior++)
            for (unsigned permutation = 0; permutation < 6; permutation++) {
                struct PolyPoint previous[3] = {
                    {0,0,0,0,17*65536}, {79,0,20*65536,12*65536,19*65536},
                    {0,61,8*65536,7*65536,18*65536}
                };
                emit(f, previous, prior ? 26 : 5, prior ? 32 : 17); count++;
                struct PolyPoint v[3], original[3];
                for (unsigned i = 0; i < 3; i++) original[i] = (struct PolyPoint){
                    thin_xy[kind][i*2] + thin_offsets[offset][0],
                    thin_xy[kind][i*2+1] + thin_offsets[offset][1],
                    (7 + i)*65536 + 123, (8 - i)*65536 + 456, (17 + i)*65536 + 789};
                for (unsigned i = 0; i < 3; i++) v[i] = original[thin_order[permutation][i]];
                emit(f, v, mode, 17); count++;
            }
    fseek(f,16,SEEK_SET); word(f,count); fclose(f); printf("%u original C triangle fixtures\n", count); return 0;
}
