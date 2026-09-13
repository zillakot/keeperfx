#include "kfx/renderer/software/bflib_vidraw.c"

struct DisplayStruct lbDisplay;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
static FILE *fixture;
static unsigned count, submissions;
static int decline, enabled = 1;
enum { WIDTH = 83, HEIGHT = 61, SIZE = WIDTH * HEIGHT };
static uint8_t target_pixels[SIZE], expected[SIZE], initial[SIZE], glass[65536], ghost[65536], alpha[65536], remap[256];

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}
unsigned short RendererGetDrawFlags(void) { return flags; }
int kfx_wgpu_native_enabled(void) { return enabled; }

int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    submissions++;
    if (decline) return 0;
    require(target->pixels == target_pixels && target->pitch == WIDTH, "wrong native target");
    require(!memcmp(target_pixels, initial, SIZE), "accepted sprite ran CPU pixel writes");
    unsigned before = submissions;
    memcpy(expected, initial, SIZE);
    oracle(expected, target->pitch, context);
    require(submissions == before, "oracle recursively submitted a native command");
    require(lbDisplay.WScreen == target_pixels && lbDisplay.GraphicsWindowPtr == target_pixels + 5 * WIDTH + 3,
        "oracle failed to restore target aliases");
    require(!memcmp(target_pixels, initial, SIZE), "oracle modified live target");
    uint32_t length = source->length;
    fwrite(command, sizeof(*command), 1, fixture);
    fwrite(&length, 4, 1, fixture);
    fwrite(source->bytes, length, 1, fixture);
    if (command->blend) fwrite(table->bytes, 65536, 1, fixture);
    fwrite(expected, SIZE, 1, fixture);
    count++;
    return 1;
}

int main(int argc, char **argv)
{
    if (argc != 2) return 1;
    for (unsigned i = 0; i < 65536; i++) {
        glass[i] = ((i >> 8) * 7 + (i & 255) * 3 + 17) & 255;
        ghost[i] = ((i >> 8) * 5 + (i & 255) * 11 + 3) & 255;
        alpha[i] = ((i >> 8) * 13 + (i & 255) * 7 + 97) & 255;
    }
    for (unsigned i = 0; i < 256; i++) remap[i] = (i * 17 + 53) & 255;
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / WIDTH * 13) & 255;
    lbDisplay.WScreen = target_pixels;
    lbDisplay.GraphicsScreenWidth = WIDTH;
    lbDisplay.GraphicsScreenHeight = HEIGHT;
    lbDisplay.GraphicsWindowX = 3;
    lbDisplay.GraphicsWindowY = 5;
    lbDisplay.GraphicsWindowWidth = 73;
    lbDisplay.GraphicsWindowHeight = 49;
    lbDisplay.GraphicsWindowPtr = target_pixels + 5 * WIDTH + 3;
    lbDisplay.GlassMap = glass;
    render_ghost = ghost;
    render_alpha = alpha;
    lbSpriteReMapPtr = remap;
    uint8_t data[] = {
        2, 0, 255, -2, 3, 17, 93, 0, 0,
        -1, 4, 127, 0, 4, 8, -2, 0,
        7, 43, 29, 17, 0, 251, 180, 3, 0,
        -3, 1, 222, -3, 0,
        1, 0, -5, 1, 255, 0,
    };
    struct TbSprite sprite = {data, 7, 5};
    struct TbSourceBuffer buffer = {data, 7, 5, 7};
    fixture = fopen(argv[1], "wb+");
    require(fixture != NULL, "cannot open fixture");
    uint32_t header[] = {0x3353464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    const int scales[][2] = {{7,5}, {3,2}, {14,15}, {19,3}, {4,13}, {1,1}, {35,25}};
    const int positions[][2] = {{13,11}, {-4,-3}, {69,46}, {-8,10}, {8,-6}, {75,51}, {0,0}};
    unsigned fallback = 0;
    for (unsigned mode = 0; mode < 6; mode++)
    for (unsigned flip = 0; flip < 4; flip++)
    for (unsigned blend = 0; blend < 4; blend++)
    for (unsigned mapped = 0; mapped < 2; mapped++)
    for (unsigned scale = 0; scale < (mode < 4 ? 7 : 1); scale++)
    for (unsigned position = 0; position < 7; position++)
    for (unsigned water = 0; water < (mode == 0 || mode == 1 || mode == 3 ? 2 : 1); water++) {
        flags = ((flip & 1) ? Lb_SPRITE_FLIP_HORIZ : 0) |
            ((flip & 2) ? Lb_SPRITE_FLIP_VERTIC : 0) |
            ((blend & 1) ? Lb_SPRITE_TRANSPAR4 : 0) |
            ((blend & 2) ? Lb_SPRITE_TRANSPAR8 : 0) |
            (mapped ? Lb_SPRITE_REMAP : 0);
        memcpy(target_pixels, initial, SIZE);
        buffer.height = water ? 3 : 5;
        LbSpriteSetScalingData(positions[position][0], positions[position][1], 7, 5,
            scales[scale][0], scales[scale][1]);
        unsigned before = submissions;
        switch (mode) {
        case 0: LbSpriteDrawUsingScalingData(0,0,&buffer); break;
        case 1: LbSpriteDrawRemapUsingScalingData(0,0,&buffer,remap); break;
        case 2: LbSpriteDrawOneColourUsingScalingData(0,0,&sprite,0); break;
        case 3: DrawAlphaSpriteUsingScalingData(0,0,&buffer); break;
        case 4: LbSpriteDrawImmediate(positions[position][0],positions[position][1],&sprite); break;
        case 5: LbSpriteDrawOneColourImmediate(positions[position][0],positions[position][1],&sprite,0); break;
        }
        if (submissions == before) fallback++;
        else require(!memcmp(target_pixels,initial,SIZE), "accepted sprite modified native target");
    }
    header[1] = count;
    rewind(fixture);
    fwrite(header, sizeof(header), 1, fixture);
    fclose(fixture);
    printf("%u sprite commands; %u explicit native fallback cases\n", count, fallback);
    return 0;
}
