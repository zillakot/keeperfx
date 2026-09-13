#include "kfx/renderer/software/bflib_vidraw.c"

struct DisplayStruct lbDisplay;
static unsigned short flags;
static FILE *fixture;
static unsigned count, submissions;
static int decline;
enum { WIDTH = 83, HEIGHT = 61, SIZE = WIDTH * HEIGHT };
static uint8_t target_pixels[SIZE], expected[SIZE], initial[SIZE], glass[65536];

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}

unsigned short RendererGetDrawFlags(void) { return flags; }

int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    (void)source;
    submissions++;
    if (decline) return 0;
    require(target->pixels == target_pixels && target->pitch == WIDTH, "wrong native target");
    require(!memcmp(target_pixels, initial, SIZE), "accepted primitive ran CPU pixel writes");
    require(!command->blend || (table && table->bytes == glass && table->length == 65536),
        "missing live blend table");
    unsigned char *screen = lbDisplay.WScreen, *window = lbDisplay.GraphicsWindowPtr;
    long pitch = lbDisplay.GraphicsScreenWidth;
    unsigned before = submissions;
    memcpy(expected, initial, SIZE);
    oracle(expected, target->pitch, context);
    require(submissions == before, "oracle recursively submitted a native command");
    require(lbDisplay.WScreen == screen && lbDisplay.GraphicsWindowPtr == window &&
        lbDisplay.GraphicsScreenWidth == pitch && !wgpu_primitive_oracle_active,
        "oracle failed to restore target aliases or recursion guard");
    require(!memcmp(target_pixels, initial, SIZE), "oracle modified live target");
    if (fixture) {
        fwrite(command, sizeof(*command), 1, fixture);
        fwrite(expected, SIZE, 1, fixture);
        count++;
    }
    return 1;
}

static void routing_checks(void)
{
    unsigned char *window = lbDisplay.GraphicsWindowPtr;
    unsigned before = submissions;
    lbDisplay.GraphicsWindowPtr++;
    LbDrawPixel(1, 2, 137);
    require(submissions == before && target_pixels[5 * WIDTH + 3 + 1 + 1 + 2 * WIDTH] == 137,
        "inconsistent window alias did not decline to CPU");
    lbDisplay.GraphicsWindowPtr = window;
    memcpy(target_pixels, initial, SIZE);
    decline = 1;
    LbDrawCircleFilled(23, 19, 8, 109);
    uint8_t fallback[SIZE];
    memcpy(fallback, target_pixels, SIZE);
    memcpy(target_pixels, initial, SIZE);
    decline = 0;
    struct WgpuPrimitive primitive = {WgpuCircleFill, 23, 19, 8, 0, 109};
    wgpu_primitive_oracle(target_pixels, WIDTH, &primitive);
    require(!memcmp(fallback, target_pixels, SIZE), "declined circle changed legacy output");
    memcpy(target_pixels, initial, SIZE);
    before = submissions;
    LbDrawCircleOutline(23, 19, 8192, 109);
    require(submissions == before && !memcmp(target_pixels, initial, SIZE),
        "unsupported outline radius should retain clipped legacy coverage");
    LbDrawPixel(16601, -200, 143);
    require(submissions == before + 1 && !memcmp(target_pixels, initial, SIZE)
        && expected[5 * WIDTH + 4] == 143,
        "valid linear pixel should normalize before GPU coordinate limits");
    memcpy(target_pixels, initial, SIZE);
}

int main(int argc, char **argv)
{
    if (argc != 2) return 1;
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
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / WIDTH * 13) & 255;
    memcpy(target_pixels, initial, SIZE);
    routing_checks();
    if (!(fixture = fopen(argv[1], "wb"))) return 1;
    uint32_t header[] = {0x3250464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    unsigned modes[] = {0, Lb_SPRITE_TRANSPAR4, Lb_SPRITE_TRANSPAR8,
        Lb_SPRITE_TRANSPAR4 | Lb_SPRITE_TRANSPAR8};
    for (unsigned mode = 0; mode < 4; mode++) {
        flags = modes[mode];
        LbDrawCircleFilled(31, 23, 8191, 109);
        LbDrawCircleOutline(31, 23, 8191, 109);
        LbDrawCircleOutline(8191, 23, 8191, 171);
        LbDrawCircleFilled(8191, 23, 8191, 171);
        LbDrawBoxClip(-20000, -20000, 20010, 20010, 109);
        LbDrawBoxClip(1, 1, 0, 1, 109);
        LbDrawBoxClip(1, 1, 1, 0, 109);
        LbDrawBoxClip(1, 1, (unsigned long)-1, 1, 109);
        LbDrawHVLine(-20000, 1, 20000, 1, 109);
        LbDrawHVLine(1, -20000, 1, 20000, 109);
        LbDrawCircleFilled(-100, -100, 2, 109);
        LbDrawCircleOutline(100, 100, 2, 109);
        LbDrawPixelClip(-1, -1, 109);
        LbDrawPixel(16601, -200, 143);
        LbDrawPixel(-16599, 200, 143);
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
    require(!memcmp(target_pixels, initial, SIZE), "final accepted primitive ran CPU writes");
    fseek(fixture, 4, SEEK_SET);
    fwrite(&count, sizeof(count), 1, fixture);
    fclose(fixture);
    printf("%u actual legacy primitive oracle cases\n", count);
    return 0;
}
