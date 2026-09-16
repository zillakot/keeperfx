#include "kfx/renderer/software/bflib_vidraw.c"
#include "bflib_sprfnt.c"

struct DisplayStruct lbDisplay;
volatile struct DisplayStructEx lbDisplayEx;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
static FILE *fixture;
static unsigned submissions, count, barriers;
static int enabled = 1, decline, barrier_ok = 1;
static struct { int keyed; uint32_t kind; uint64_t hi, lo, generation; } last_key;
struct PartRecord { unsigned count; size_t length; uint32_t kind; uint64_t hi, lo, generation;
    const uint8_t *bytes; };
static struct PartRecord pending_part, last_part;
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
    return kfx_wgpu_native_draw_named(target, command, source, NULL, table, oracle, context);
}
int kfx_wgpu_native_draw_parts(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativePart *parts, unsigned count,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    require(count == 1, "unexpected part count");
    pending_part = (struct PartRecord){count, parts[0].resource.length, parts[0].name.kind,
        parts[0].name.hi, parts[0].name.lo, parts[0].name.generation, parts[0].resource.bytes};
    return kfx_wgpu_native_draw_named(target, command, source, NULL, table, oracle, context);
}
int kfx_wgpu_native_draw_named(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeKey *name, const struct KfxWgpuNativeResource *table,
    KfxWgpuNativeOracle oracle, void *context)
{
    (void)table;
    submissions++;
    last_part = pending_part;
    pending_part = (struct PartRecord){0};
    last_key.keyed = name != NULL;
    last_key.kind = name ? name->kind : 0;
    last_key.hi = name ? name->hi : 0;
    last_key.lo = name ? name->lo : 0;
    last_key.generation = name ? name->generation : 0;
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
            source ? source->width : 0, source ? source->height : 0, source ? source->pitch : 0,
            source ? source->length : 0, (uint32_t)last_part.length};
        fwrite(info, sizeof(info), 1, fixture);
        fwrite(command, sizeof(*command), 1, fixture);
        if (source) fwrite(source->bytes, source->length, 1, fixture);
        if (last_part.length) fwrite(last_part.bytes, last_part.length, 1, fixture);
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
    /* Named artwork: the runs are the sprite's own, the scroll and clip are the call's. */
    kfx_render_asset_range(rle, sizeof(rle));
    kfx_render_asset_range(lines, sizeof(lines));
    int scales[] = {8, 16, 24, 32, 48, 80};
    for (unsigned s = 0; s < 6; s++)
        for (int x = -4; x <= 4; x += 2)
            for (int y = -3; y <= 3; y += 2) {
                unsigned old = submissions;
                LbHugeSpriteDraw(&huge, length, pixels, WIDTH, HEIGHT, x, y, scales[s]);
                require(submissions == old + 1, "valid huge sprite declined");
                require(last_part.count == 1 && last_part.kind == KFX_WGPU_DRAW_KEY_HUGE_SPRITE &&
                    last_part.hi == (uint64_t)(uintptr_t)lines &&
                    last_part.lo == (uint64_t)(uintptr_t)rle &&
                    last_part.generation == kfx_render_asset_generation,
                    "huge artwork name wrong");
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
                require(last_part.count == 0, "unnamed huge artwork was named");
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

    /* Identity: a glyph is named by its font and character, and the three colour words
     * are part of the asset, so they are part of the name. */
    memcpy(pixels, initial, SIZE);
    flags = 0;
    active_dbcfont = &fontdata;
    font(bits, 16, 16, 24, 24, 3, 2, 255, -1, 0);
    require(!last_key.keyed, "glyph outside the font table was named");
    dbcfonts[0] = fontdata;
    active_dbcfont = &dbcfonts[0];
    struct AsianFontWindow window = {71, 53, WIDTH, pixels + WIDTH * 3 + 5};
    long x = 5;
    draw_dbc_char(0x4e2d, &window, &x, 5, 24);
    require(last_key.keyed && last_key.kind == KFX_WGPU_DRAW_KEY_GLYPH &&
        last_key.hi == ((uint64_t)1 << 16 | 0x4e2d) &&
        last_key.generation == kfx_render_asset_generation, "glyph name wrong");
    uint64_t colours = last_key.lo, generation = last_key.generation;
    x = 25;
    draw_dbc_char(0x4e2d, &window, &x, 17, 24);
    require(last_key.hi == ((uint64_t)1 << 16 | 0x4e2d) && last_key.lo == colours,
        "glyph position changed the name");
    x = 5;
    draw_dbc_char(0xac00, &window, &x, 5, 24);
    require(last_key.hi == ((uint64_t)1 << 16 | 0xac00), "character not in the name");
    dbc_colour0 = 71;
    x = 5;
    draw_dbc_char(0x4e2d, &window, &x, 5, 24);
    require(last_key.lo != colours, "glyph colours not in the name");
    dbc_colour0 = 0;
    kfx_render_assets_changed();
    x = 5;
    draw_dbc_char(0x4e2d, &window, &x, 5, 24);
    require(last_key.generation == generation + 1, "font reload did not rename");
    dbcfonts[1] = fontdata;
    active_dbcfont = &dbcfonts[1];
    x = 5;
    draw_dbc_char(0x4e2d, &window, &x, 5, 24);
    require(last_key.hi == ((uint64_t)2 << 16 | 0x4e2d), "font not in the name");
    return 0;
}
