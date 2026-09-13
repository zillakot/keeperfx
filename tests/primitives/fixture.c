#include "kfx/renderer/software/bflib_vidraw.c"

struct DisplayStruct lbDisplay;
static unsigned short flags;
static FILE *fixture;
static unsigned count;
enum { WIDTH = 83, HEIGHT = 61, SIZE = WIDTH * HEIGHT };
static uint8_t target_pixels[SIZE], expected[SIZE], glass[65536];
unsigned short RendererGetDrawFlags(void) { return flags; }

int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    (void)source; (void)table;
    for (unsigned i = 0; i < SIZE; i++) target_pixels[i] = (i * 19 + i / WIDTH * 13) & 255;
    memcpy(expected, target_pixels, SIZE);
    oracle(expected, target->pitch, context);
    fwrite(command, sizeof(*command), 1, fixture);
    fwrite(expected, SIZE, 1, fixture);
    count++;
    return 1;
}

int main(int argc, char **argv)
{
    if (argc != 2 || !(fixture = fopen(argv[1], "wb"))) return 1;
    for (unsigned i = 0; i < 65536; i++) glass[i] = ((i >> 8) * 7 + (i & 255) * 3 + 17) & 255;
    lbDisplay.WScreen = target_pixels;
    lbDisplay.GraphicsScreenWidth = WIDTH;
    lbDisplay.GraphicsScreenHeight = HEIGHT;
    lbDisplay.GraphicsWindowX = 3;
    lbDisplay.GraphicsWindowY = 5;
    lbDisplay.GraphicsWindowWidth = 73;
    lbDisplay.GraphicsWindowHeight = 49;
    lbDisplay.GraphicsWindowPtr = target_pixels + 5 * WIDTH + 3;
    lbDisplay.GlassMap = glass;
    uint32_t header[] = {0x3250464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    unsigned modes[] = {0, Lb_SPRITE_TRANSPAR4, Lb_SPRITE_TRANSPAR8,
        Lb_SPRITE_TRANSPAR4 | Lb_SPRITE_TRANSPAR8};
    for (unsigned mode = 0; mode < 4; mode++) {
        flags = modes[mode];
        LbDrawPixel(-5, 19, 57);
        LbDrawPixel(WIDTH + 5, 19, 57);
        for (long r = -2; r < 29; r++) {
            LbDrawCircleFilled(31, 23, r, 109);
            LbDrawCircleOutline(31, 23, r, 109);
            LbDrawCircleFilled(-3, 2, r, 171);
            LbDrawCircleOutline(70, 47, r, 171);
        }
        for (long x = -3; x < 78; x += 8) {
            LbDrawPixelClip(x, 17, 49);
            LbDrawPixel(x, 19, 57);
            LbDrawBoxClip(x, -4, 9, 58, 117);
            LbDrawBoxClip(x, 38, 7, 8, 37);
            LbDrawHVLine(x, 9, 70, 9, 97);
            LbDrawHVLine(70, 13, x, 13, 98);
            LbDrawHVLine(x, -2, x, 51, 99);
            if (x >= 0 && x < 49) LbDrawHVLine(x, 40, x, 3, 100);
            LbDrawHVLine(x, 10, x, 10, 101);
            flags = modes[mode] | Lb_SPRITE_OUTLINE;
            LbDrawBoxImmediate(x, 3, 1, 1, 121);
            LbDrawBoxImmediate(x, 5, 11, 9, 122);
            flags = modes[mode];
        }
    }
    fseek(fixture, 4, SEEK_SET);
    fwrite(&count, sizeof(count), 1, fixture);
    fclose(fixture);
    printf("%u actual legacy primitive oracle cases\n", count);
    return 0;
}
