#define KFX_TRIG_NATIVE 1
#define main fixture_main
#include "fixture.c"
#undef main

void native_trig_case(unsigned mode, int cpu, uint8_t *output)
{
    memset(pixels, 167, sizeof(pixels));
    vec_screen = pixels + 83; poly_screen = pixels; vec_map = block_mem;
    for (unsigned i = 0; i < sizeof(texture); i++) texture[i] = ((i * 13) ^ (i >> 8)) & 63;
    memcpy(block_mem, texture, sizeof(texture));
    kfx_render_assets_changed();
    for (unsigned i = 0; i < sizeof(pixmap.fade_tables); i++) pixmap.fade_tables[i] = (i * 7) ^ (i >> 8);
    for (unsigned i = 0; i < sizeof(pixmap.ghost); i++) pixmap.ghost[i] = (i * 17) ^ (i >> 8);
    vec_mode = mode; vec_colour = 17;
    struct PolyPoint a = {-12,-8,0,0,65536}, b = {73,4,31*65536,0,30*65536}, c = {20,65,0,31*65536,15*65536};
    if (mode == 5 || mode == 6 || mode == 20 || mode == 21 || mode >= 24) {
        a = (struct PolyPoint){-3,-2,0,0,30*65536};
        b = (struct PolyPoint){15,2,31*65536,0,32*65536};
        c = (struct PolyPoint){2,19,0,31*65536,28*65536};
    }
    wgpu_trig_oracle_active = cpu;
    trig(&a,&b,&c);
    a.X += 15; a.Y += 10; b.X -= 5; c.Y -= 8;
    trig(&a,&b,&c);
    wgpu_trig_oracle_active = 0;
    memcpy(output, pixels, sizeof(pixels));
}

void native_trig_scratch_case(uint8_t *output)
{
    big_scratch = block_mem;
    native_trig_case(7, 0, output);
    big_scratch = NULL;
}

void native_trig_extent_case(unsigned variant, int cpu, uint8_t *output)
{
    memset(pixels, 167, sizeof(pixels));
    vec_screen = pixels + 83; poly_screen = pixels;
    const size_t offset[] = {0, sizeof(block_mem)-1, sizeof(block_mem)-7968};
    const unsigned index[] = {65535, 0, 7967};
    vec_map = variant == 3 ? texture : block_mem + offset[variant == 4 ? 0 : variant];
    unsigned sample = variant == 3 ? 0 : index[variant == 4 ? 0 : variant];
    vec_map[sample] = 39;
    /* The page is keyed by its offset in block_mem: rewriting it is an asset change. */
    kfx_render_assets_changed();
    vec_mode = variant == 4 ? 26 : 2; vec_colour = 17;
    struct PolyPoint a = {0,0,(sample & 255)*65536,(sample >> 8)*65536,31*65536};
    struct PolyPoint b = a, c = a; b.X = 70; c.Y = 60;
    wgpu_trig_oracle_active = cpu;
    trig(&a,&b,&c);
    wgpu_trig_oracle_active = 0;
    memcpy(output, pixels, sizeof(pixels));
}

void native_trig_thin_case(unsigned mode, int cpu, uint8_t *output)
{
    memset(pixels, 167, sizeof(pixels));
    vec_screen = pixels + 83; poly_screen = pixels; vec_map = block_mem;
    vec_mode = mode; vec_colour = 17;
    const int xy[][6] = {{1,0,2,1,3,3}, {3,0,1,4,1,3}};
    const int offsets[][2] = {{0,0}, {-2,0}, {77,0}, {0,-1}, {0,59}};
    wgpu_trig_oracle_active = cpu;
    for (unsigned prior = 0; prior < 2; prior++) {
        struct PolyPoint previous[3] = {
            {0,0,0,0,(17+prior)*65536}, {79,0,20*65536,12*65536,19*65536},
            {0,61,8*65536,7*65536,18*65536}
        };
        trig(previous, previous+1, previous+2);
        for (unsigned kind = 0; kind < 2; kind++) for (unsigned offset = 0; offset < 5; offset++) {
            struct PolyPoint v[3];
            for (unsigned i = 0; i < 3; i++) v[i] = (struct PolyPoint){
                xy[kind][i*2] + offsets[offset][0], xy[kind][i*2+1] + offsets[offset][1],
                (7 + i)*65536 + 123, (8 - i)*65536 + 456, (17 + i)*65536 + 789};
            trig(v, v+1, v+2);
        }
    }
    wgpu_trig_oracle_active = 0;
    memcpy(output, pixels, sizeof(pixels));
}
