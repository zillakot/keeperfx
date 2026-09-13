#define KFX_TRIG_NATIVE 1
#define main fixture_main
#include "fixture.c"
#undef main

void native_trig_case(unsigned mode, int cpu, uint8_t *output)
{
    memset(pixels, 167, sizeof(pixels));
    vec_screen = pixels + 83; poly_screen = pixels; vec_map = texture;
    for (unsigned i = 0; i < sizeof(texture); i++) texture[i] = (i * 13) ^ (i >> 8);
    for (unsigned i = 0; i < sizeof(pixmap.fade_tables); i++) pixmap.fade_tables[i] = (i * 7) ^ (i >> 8);
    for (unsigned i = 0; i < sizeof(pixmap.ghost); i++) pixmap.ghost[i] = (i * 17) ^ (i >> 8);
    vec_mode = mode; vec_colour = 17;
    struct PolyPoint a = {-12,-8,0,0,65536}, b = {73,4,31*65536,0,30*65536}, c = {20,65,0,31*65536,15*65536};
    wgpu_trig_oracle_active = cpu;
    trig(&a,&b,&c);
    a.X += 15; a.Y += 10; b.X -= 5; c.Y -= 8;
    trig(&a,&b,&c);
    wgpu_trig_oracle_active = 0;
    memcpy(output, pixels, sizeof(pixels));
}

void native_trig_scratch_case(uint8_t *output)
{
    big_scratch = texture;
    native_trig_case(7, 0, output);
    big_scratch = NULL;
}
