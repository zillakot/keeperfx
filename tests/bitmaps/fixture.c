#include "kfx/renderer/software/bflib_vidraw.c"
#include "bflib_sprfnt.c"

struct DisplayStruct lbDisplay;
volatile struct DisplayStructEx lbDisplayEx;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
static FILE *fixture;
static unsigned submissions, count, barriers;
static int enabled = 1, decline, barrier_ok = 1;
enum { WIDTH = 83, HEIGHT = 61, SIZE = WIDTH * HEIGHT };
static uint8_t pixels[SIZE], expected[SIZE], initial[SIZE];
static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}
unsigned short RendererGetDrawFlags(void) { return flags; }
unsigned char RendererGetDrawColour(void) { return 33; }
int kfx_wgpu_native_enabled(void) { return enabled; }
int kfx_wgpu_native_read_barrier(const void* bytes, size_t length) { (void)bytes; (void)length; return 1; }
void kfx_wgpu_native_flush(void) { kfx_wgpu_terrain_boundary(0); }
int kfx_wgpu_native_cpu_barrier(void) { barriers++; return barrier_ok; }
void kfx_wgpu_terrain_boundary(int allow) { require(!allow, "bitmap allowed terrain"); }
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    (void)table;
    submissions++;
    if (decline) return 0;
    require(target->pixels == pixels && target->pitch == WIDTH, "wrong target");
    require(!memcmp(pixels, initial, SIZE), "CPU destination writes before GPU submission");
    unsigned before = submissions;
    memcpy(expected, initial, SIZE);
    oracle(expected, target->pitch, context);
    require(before == submissions, "recursive oracle submission");
    require(lbDisplay.WScreen == pixels && lbDisplay.GraphicsWindowPtr == pixels + WIDTH * 3 + 5,
        "oracle changed aliases");
    require(!memcmp(pixels, initial, SIZE), "oracle changed native target");
    if (fixture) {
        uint32_t info[] = {target->width, target->height, target->pitch,
            source ? source->width : 0, source ? source->height : 0, source ? source->pitch : 0, source ? source->length : 0};
        fwrite(info, sizeof(info), 1, fixture);
        fwrite(command, sizeof(*command), 1, fixture);
        if (source) fwrite(source->bytes, source->length, 1, fixture);
        fwrite(expected, SIZE, 1, fixture);
        count++;
    }
    return 1;
}

static void word(uint8_t *p, uint32_t v)
{
    memcpy(p, &v, 4);
}
static void font(uint8_t *bits, int sw, int sh, int dw, int dh, int x, int y, int fg, int bg, int shadow)
{
    struct DbcOracle o = {
        {71, 53, WIDTH, pixels + WIDTH * 3 + 5},
        {0, dw, dh, 1, 2, 1, bits}, sw, sh, sw != dw || sh != dh,
        x, y, fg, bg, shadow, 0};
    dbc_dispatch_bitmap(&o);
}

int main(int argc, char **argv)
{
    if (argc != 2) return 1;
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / WIDTH * 13) & 255;
    memcpy(pixels, initial, SIZE);
    lbDisplay.WScreen = pixels;
    lbDisplay.GraphicsWindowPtr = pixels + WIDTH * 3 + 5;
    lbDisplay.GraphicsScreenWidth = WIDTH; lbDisplay.GraphicsScreenHeight = HEIGHT;
    lbDisplay.GraphicsWindowX = 5; lbDisplay.GraphicsWindowY = 3;
    lbDisplay.GraphicsWindowWidth = 71; lbDisplay.GraphicsWindowHeight = 53;
    uint8_t bits[8192];
    for (unsigned i = 0; i < sizeof(bits); i++) bits[i] = (i * 37 + i / 7 * 13) & 255;
    enabled = 0;
    font(bits, 16, 16, 24, 24, -3, 2, 255, -1, 0);
    require(submissions == 0, "disabled GPU submitted");
    uint8_t fallback[SIZE]; memcpy(fallback, pixels, SIZE);
    memcpy(pixels, initial, SIZE); enabled = 1; decline = 1;
    font(bits, 16, 16, 24, 24, -3, 2, 255, -1, 0);
    require(!memcmp(fallback, pixels, SIZE), "decline changed glyph fallback");
    memcpy(pixels, initial, SIZE); barrier_ok = 0;
    font(bits, 16, 16, 24, 24, -3, 2, 255, -1, 0);
    require(!memcmp(pixels, initial, SIZE), "failed barrier permitted CPU drawing");
    barrier_ok = 1; decline = 0;
    unsigned before = submissions;
    font(pixels, 16, 16, 16, 16, 10, 10, 255, -1, 0);
    require(submissions == before, "aliased glyph submitted");
    memcpy(pixels, initial, SIZE);
    fixture = fopen(argv[1], "wb+"); require(fixture != NULL, "fixture open failed");
    uint32_t header[] = {0x3142464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    int shapes[][2] = {{8,12}, {16,16}, {24,16}, {9,12}, {17,16}};
    int sizes[][2] = {{8,8},{16,16},{24,24},{32,40},{48,48}};
    int positions[][2] = {{5,7},{-7,-9},{65,49},{0,-3},{0,0},{-3,12}};
    for (unsigned s = 0; s < 5; s++)
        for (unsigned d = 0; d < 6; d++)
            for (unsigned p = 0; p < 6; p++)
                for (unsigned c = 0; c < 3; c++) {
                    unsigned old = submissions;
                    font(bits, shapes[s][0], shapes[s][1], d == 5 ? shapes[s][0] : sizes[d][0],
                        d == 5 ? shapes[s][1] : sizes[d][1], positions[p][0], positions[p][1],
                        c == 0 ? 0 : c == 1 ? 255 : -1, c == 2 ? 117 : -1, c == 0 ? -1 : 0);
                    require(submissions == old + 1, "valid glyph declined");
                }
    static unsigned short widths[65536];
    static unsigned int offsets[65536];
    uint32_t scripts[] = {0x41, 0x4e2d, 0x3042, 0xac00, 0x03a9};
    for (unsigned i = 0; i < 5; i++) widths[scripts[i]] = i == 0 ? 8 : 16;
    struct AsianFont fontdata = {"fixture", bits, widths, offsets, 16, 1, 2, 3, 2};
    active_dbcfont = &fontdata;
    uint8_t remap[256];
    for (int i = 0; i < 256; i++) remap[i] = 255 - i;
    lbSpriteReMapPtr = remap;
    dbc_colour0 = 0; dbc_colour1 = 255;
    for (unsigned script = 0; script < 5; script++)
        for (unsigned mode = 0; mode < 4; mode++) {
            flags = (mode & 1 ? Lb_TEXT_ONE_COLOR : 0) | (mode & 2 ? Lb_TEXT_REMAP : 0)
                | Lb_TEXT_UNDERLINE | Lb_TEXT_UNDERLNSHADOW;
            long x = 5;
            struct AsianFontWindow window = {71, 53, WIDTH, pixels + WIDTH * 3 + 5};
            unsigned before = submissions;
            draw_dbc_char(scripts[script], &window, &x, 5, 24);
            require(submissions >= before + 3, "glyph underline or bitmap missed GPU");
            require(x > 5 && !memcmp(pixels, initial, SIZE), "glyph advance or GPU ownership");
        }
    flags = 0;
    uint8_t rle[1024]; int32_t lines[5]; size_t length = 0;
    for (int y = 0; y < 5; y++) {
        lines[y] = length;
        word(rle + length, 2); length += 4;
        rle[length++] = y ? y * 43 : 0; rle[length++] = 255;
        word(rle + length, 2); length += 4;
        word(rle + length, 3); length += 4;
        rle[length++] = 19; rle[length++] = 0; rle[length++] = 255;
        word(rle + length, 0); length += 4;
    }
    struct TbHugeSprite huge = {rle, lines, 7, 5};
    int scales[] = {8, 16, 24, 32, 48, 80};
    for (unsigned s = 0; s < 6; s++)
        for (int x = -4; x <= 4; x += 2)
            for (int y = -3; y <= 3; y += 2) {
                unsigned old = submissions;
                LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, x, y, scales[s]);
                require(submissions == old + 1, "valid huge sprite declined");
            }
    uint8_t wide_rle[65536]; int32_t wide_lines[61];
    int huge_sizes[][2] = {{17,23},{83,61},{127,89}};
    for (int dimension = 0; dimension < 2; dimension++) {
        int sw = huge_sizes[dimension][0], sh = huge_sizes[dimension][1];
        size_t used = 0;
        for (int row = 0; row < sh; row++) {
            wide_lines[row] = used;
            int x = 0;
            while (x < sw) {
                int solid = row % 3 == 0 ? 0 : row % 3 == 1 ? sw - x : (x * 7 + row) % 5;
                if (solid > sw - x) solid = sw - x;
                word(wide_rle + used, solid); used += 4;
                for (int p = 0; p < solid; p++) wide_rle[used++] = (x++ * 67 + row * 31) & 255;
                int skip = row % 3 == 0 ? sw - x : row % 3 == 1 ? 0 : 1 + (x + row) % 4;
                if (skip > sw - x) skip = sw - x;
                word(wide_rle + used, skip); used += 4; x += skip;
            }
        }
        struct TbHugeSprite wide = {wide_rle, wide_lines, sw, sh};
        for (unsigned scale = 0; scale < 6; scale++)
            for (int shift = 0; shift < 4; shift++) {
                unsigned old = submissions;
                LbHugeSpriteDraw(&wide, used, pixels, WIDTH, HEIGHT, shift * 7, shift * 11, scales[scale]);
                require(submissions == old + 1, "clipped huge row declined");
            }
    }
    LbSpriteSetScalingData(0, 0, 7, 5, 7, 5);
    unsigned old = submissions;
    LbHugeSpriteDrawUsingScalingUpData(pixels, WIDTH, HEIGHT, xsteps_array, ysteps_array, &huge);
    require(submissions == old + 1, "trusted huge helper declined");
    fseek(fixture, 4, SEEK_SET); fwrite(&count, 4, 1, fixture); fclose(fixture);
    fixture = NULL;
    enabled = 0;
    LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, 0, 0, 32);
    memcpy(fallback, pixels, SIZE);
    memcpy(pixels, initial, SIZE); enabled = 1; decline = 1;
    LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, 0, 0, 32);
    require(!memcmp(fallback, pixels, SIZE), "huge decline changed fallback");
    memcpy(pixels, initial, SIZE); barrier_ok = 0;
    LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, 0, 0, 32);
    require(!memcmp(pixels, initial, SIZE), "huge failed barrier allowed writes");
    barrier_ok = 1; decline = 0;
    before = submissions;
    memcpy(pixels + SIZE - sizeof(rle), rle, length);
    huge.Data = pixels + SIZE - sizeof(rle);
    LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, 0, 0, 16);
    require(submissions == before, "huge mutable source alias submitted");
    huge.Data = rle;
    memcpy(pixels, initial, SIZE);
    struct HugeOracle huge_context = {HEIGHT, xsteps_array, ysteps_array, &huge};
    require(!kfx_wgpu_bitmap_huge(pixels, WIDTH, HEIGHT, xsteps_array, ysteps_array,
        &huge, 4, huge_oracle, &huge_context), "truncated huge asset accepted");
    printf("%u actual native huge sprite/glyph fixtures; %u CPU barriers\n", count, barriers);
    return 0;
}
