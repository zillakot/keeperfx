#include "kfx/renderer/software/bflib_vidraw.c"

struct DisplayStruct lbDisplay;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
static FILE *fixture;
static unsigned count, submissions, ordered_count;
static int decline, enabled = 1;
enum { WIDTH = 83, HEIGHT = 61, SIZE = WIDTH * HEIGHT };
static uint8_t target_pixels[SIZE], expected[SIZE], initial[SIZE], glass[65536], ghost[65536], alpha[65536], remap[256];

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}
unsigned short RendererGetDrawFlags(void) { return flags; }
int kfx_wgpu_native_enabled(void) { return enabled; }
void kfx_wgpu_terrain_boundary(int allow) { require(!allow, "sprite left terrain batching enabled"); }

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
    require(lbDisplay.WScreen == target_pixels && lbDisplay.GraphicsWindowPtr == target_pixels + lbDisplay.GraphicsWindowY * WIDTH + lbDisplay.GraphicsWindowX,
        "oracle failed to restore target aliases");
    require(!memcmp(target_pixels, initial, SIZE), "oracle modified live target");
    uint32_t length = source->length;
    fwrite(command, sizeof(*command), 1, fixture);
    fwrite(&length, 4, 1, fixture);
    fwrite(source->bytes, length, 1, fixture);
    if (command->blend) fwrite(table->bytes, 65536, 1, fixture);
    fwrite(expected, SIZE, 1, fixture);
    count++;
    ordered_count += (command->source_x & 8) != 0;
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
    unsigned fallback = 0, scaled_fallback = 0;
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
        case 2: LbSpriteDrawOneColourUsingScalingData(0,0,&sprite,mapped ? 0 : 211); break;
        case 3: DrawAlphaSpriteUsingScalingData(0,0,&buffer); break;
        case 4: LbSpriteDrawImmediate(positions[position][0],positions[position][1],&sprite); break;
        case 5: LbSpriteDrawOneColourImmediate(positions[position][0],positions[position][1],&sprite,mapped ? 0 : 211); break;
        }
        if (submissions == before) { fallback++; if (mode < 4) scaled_fallback++; }
        else require(!memcmp(target_pixels,initial,SIZE), "accepted sprite modified native target");
    }
    for (unsigned mode = 0; mode < 4; mode++)
    for (unsigned flip = 0; flip < 4; flip++)
    for (unsigned blend = 0; blend < 3; blend++)
    for (unsigned scale = 0; scale < 3; scale++) {
        flags = ((flip & 1) ? Lb_SPRITE_FLIP_HORIZ : 0) |
            ((flip & 2) ? Lb_SPRITE_FLIP_VERTIC : 0) |
            (blend == 1 ? Lb_SPRITE_TRANSPAR4 : 0) |
            (blend == 2 ? Lb_SPRITE_TRANSPAR8 : 0);
        memcpy(target_pixels, initial, SIZE);
        buffer.height = 3;
        LbSpriteSetScalingData(-2, 8, 11, 9, scales[scale][0] * 2, scales[scale][1] * 2);
        unsigned before = submissions;
        switch (mode) {
        case 0: LbSpriteDrawUsingScalingData(2,3,&buffer); break;
        case 1: LbSpriteDrawRemapUsingScalingData(2,3,&buffer,remap); break;
        case 2: LbSpriteDrawOneColourUsingScalingData(2,3,&sprite,211); break;
        case 3: DrawAlphaSpriteUsingScalingData(2,3,&buffer); break;
        }
        if (submissions == before) { fallback++; if (mode < 4) scaled_fallback++; }
    }
    require(scaled_fallback == 0, "valid scaled sprite declined");
    uint8_t segmented[] = {
        1, 0, 1, 255, -2, 1, 17, 2, 93, 0, 0,
        -1, 1, 127, 1, 0, 2, 4, 8, -2, 0,
        1, 43, 2, 29, 17, 1, 0, 3, 251, 180, 3, 0,
        -3, 1, 222, -3, 0,
        1, 0, -5, 1, 255, 0,
    };
    uint8_t trailing[] = {
        1, 0, 1, 255, 0,
        -2, 1, 0, 1, 17, 0,
        3, 127, 0, 255, 1, 19, 3, 0, 27, 93, 0,
        -5, 1, 255, 1, 0, 0,
        -7, 0,
    };
    for (unsigned pattern = 0; pattern < 3; pattern++)
    for (unsigned mode = 0; mode < 3; mode++)
    for (unsigned vertical = 0; vertical < 2; vertical++)
    for (unsigned mapped = 0; mapped < 2; mapped++)
    for (unsigned scale = 0; scale < 4; scale++)
    for (unsigned position = 0; position < 7; position++) {
        uint8_t *artwork = pattern == 0 ? data : pattern == 1 ? segmented : trailing;
        struct TbSprite special = {artwork, 7, 5};
        struct TbSourceBuffer special_buffer = {artwork, 7, 5, 7};
        lbDisplay.GraphicsWindowX = 0;
        lbDisplay.GraphicsWindowWidth = WIDTH;
        lbDisplay.GraphicsWindowPtr = target_pixels + 5 * WIDTH;
        flags = Lb_SPRITE_FLIP_HORIZ | (vertical ? Lb_SPRITE_FLIP_VERTIC : 0)
            | (mapped ? Lb_SPRITE_REMAP | Lb_SPRITE_TRANSPAR4 | Lb_SPRITE_TRANSPAR8 : 0);
        if (mode != 0) flags &= ~(Lb_SPRITE_TRANSPAR4 | Lb_SPRITE_TRANSPAR8);
        memcpy(target_pixels, initial, SIZE);
        const int special_scales[][2] = {{WIDTH, 15}, {19, 13}, {4, 13}, {35, 25}};
        LbSpriteSetScalingData(positions[position][0], positions[position][1], 7, 5,
            special_scales[scale][0], special_scales[scale][1]);
        unsigned before = submissions;
        if (mode == 0) LbSpriteDrawUsingScalingData(0,0,&special_buffer);
        if (mode == 1) LbSpriteDrawRemapUsingScalingData(0,0,&special_buffer,remap);
        if (mode == 2) LbSpriteDrawOneColourUsingScalingData(0,0,&special,mapped ? 0 : 211);
        require(submissions == before + 1, "ordered sprite edge case declined");
        require(!memcmp(target_pixels,initial,SIZE), "ordered sprite modified CPU target");
    }
    lbDisplay.GraphicsWindowX = 3;
    lbDisplay.GraphicsWindowWidth = 73;
    lbDisplay.GraphicsWindowPtr = target_pixels + 5 * WIDTH + 3;
    for (unsigned ordered = 0; ordered < 2; ordered++) {
        flags = ordered ? Lb_SPRITE_FLIP_HORIZ | Lb_SPRITE_FLIP_VERTIC : 0;
        buffer.height = 5;
        LbSpriteSetScalingData(13,11,7,5,14,15);
        memcpy(target_pixels,initial,SIZE);
        decline = 1;
        LbSpriteDrawUsingScalingData(0,0,&buffer);
        uint8_t fallback_pixels[SIZE];
        memcpy(fallback_pixels,target_pixels,SIZE);
        memcpy(target_pixels,initial,SIZE);
        decline = 0;
        enabled = 0;
        unsigned before = submissions;
        LbSpriteDrawUsingScalingData(0,0,&buffer);
        require(submissions == before && !memcmp(target_pixels,fallback_pixels,SIZE),
            "disabled and declined sprite paths differ");
        enabled = 1;
        memcpy(target_pixels,initial,SIZE);
        LbSpriteDrawUsingScalingData(0,0,&buffer);
        require(!memcmp(expected,fallback_pixels,SIZE), "oracle differs from native fallback");
        lbDisplay.GraphicsWindowPtr++;
        before = submissions;
        LbSpriteDrawUsingScalingData(0,0,&buffer);
        require(submissions == before, "inconsistent graphics target alias accepted");
        lbDisplay.GraphicsWindowPtr--;
    }
    header[1] = count;
    rewind(fixture);
    fwrite(header, sizeof(header), 1, fixture);
    fclose(fixture);
    printf("%u sprite commands; %u explicit scaled fallbacks; %u immediate clipped no-ops\n",
        count, scaled_fallback, fallback - scaled_fallback);
    printf("%u ordered GPU sprite commands\n", ordered_count);
    return 0;
}
