#include "kfx/renderer/software/bflib_vidraw.c"

struct DisplayStruct lbDisplay;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
static int enabled;
static FILE *fixture;
static unsigned count;
enum { WIDTH = 3, HEIGHT = 16, SIZE = WIDTH * HEIGHT };
static _Alignas(4) uint8_t target_storage[65536 + SIZE + 3], native_storage[65536 + SIZE + 3], oracle_pixels[SIZE];
static uint8_t initial[SIZE], remap[256], *target_pixels, *native_pixels;

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}
unsigned short RendererGetDrawFlags(void) { return flags; }
int kfx_wgpu_native_enabled(void) { return enabled; }
int kfx_wgpu_native_read_barrier(const void* bytes, size_t length) { (void)bytes; (void)length; return 1; }
void kfx_wgpu_native_flush(void) { kfx_wgpu_terrain_boundary(0); }
int kfx_wgpu_native_cpu_barrier(void) { return 1; }
void kfx_wgpu_terrain_boundary(int allow) { require(!allow, "terrain boundary"); }
static void write_resource(const struct KfxWgpuNativeResource *resource)
{
    uint32_t length = resource->length;
    fwrite(&length, 4, 1, fixture);
    fwrite(resource->bytes, length, 1, fixture);
}

int kfx_wgpu_native_draw_sprite(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuSpriteAssets *assets,
    KfxWgpuNativeOracle oracle, void *context)
{
    const struct KfxWgpuNativeResource *source = assets->artwork, *ranges = assets->ranges;
    const struct KfxWgpuNativeResource *remap = assets->remap, *table = assets->table;
    require(target->pixels == target_pixels && target->pitch == WIDTH, "target mismatch");
    require(!table && (command->source_x & 8), "expected solid ordered sprite");
    require(command->source_y == ((uintptr_t)target_pixels & 3), "alignment metadata");
    memcpy(oracle_pixels, initial, SIZE);
    oracle(oracle_pixels, WIDTH, context);
    require(!memcmp(oracle_pixels, native_pixels, SIZE), "unaligned verification oracle mismatch");
    require(lbDisplay.WScreen == target_pixels && !memcmp(target_pixels, initial, SIZE), "oracle side effects");
    fwrite(command, sizeof(*command), 1, fixture);
    write_resource(source);
    write_resource(ranges);
    write_resource(remap);
    fwrite(native_pixels, SIZE, 1, fixture);
    count++;
    return 1;
}

int main(int argc, char **argv)
{
    if (argc != 2) return 1;
    fixture = fopen(argv[1], "wb+");
    require(fixture != NULL, "cannot open fixture");
    uint32_t header[] = {0x3453464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / WIDTH * 13) & 255;
    for (unsigned i = 0; i < 256; i++) remap[i] = (i * 17 + 53) & 255;
    uint8_t artwork[][8] = {{3, 19, 0, 255, 0}, {1, 19, 2, 0, 255, 0}, {5, 19, 0, 255, 43, 79, 0}};
    lbDisplay.GraphicsScreenWidth = lbDisplay.GraphicsWindowWidth = WIDTH;
    lbDisplay.GraphicsScreenHeight = lbDisplay.GraphicsWindowHeight = HEIGHT;
    for (unsigned alignment = 0; alignment < 4; alignment++)
    for (unsigned mode = 0; mode < 3; mode++)
    for (unsigned vertical = 0; vertical < 2; vertical++)
    for (unsigned y = 2; y < 6; y++)
    for (unsigned pattern = 0; pattern < 3; pattern++)
    for (unsigned duplicates = 2; duplicates <= 3; duplicates++) {
        target_pixels = target_storage + alignment;
        native_pixels = native_storage + alignment;
        memcpy(target_pixels, initial, SIZE);
        memcpy(native_pixels, initial, SIZE);
        unsigned width = pattern == 2 ? 5 : 3;
        struct TbSprite sprite = {artwork[pattern], width, 1};
        struct TbSourceBuffer source = {artwork[pattern], width, 1, width, artwork[pattern]};
        flags = Lb_SPRITE_FLIP_HORIZ | (vertical ? Lb_SPRITE_FLIP_VERTIC : 0);
        LbSpriteSetScalingData(0, y, width, 1, WIDTH, duplicates);
        unsigned before = count;
        for (enabled = 0; enabled <= 1; enabled++) {
            lbDisplay.WScreen = lbDisplay.GraphicsWindowPtr = enabled ? target_pixels : native_pixels;
            if (mode == 0) LbSpriteDrawUsingScalingData(0, 0, &source);
            if (mode == 1) LbSpriteDrawRemapUsingScalingData(0, 0, &source, remap);
            if (mode == 2) LbSpriteDrawOneColourUsingScalingData(0, 0, &sprite, 117);
        }
        require(count == before + 1, "narrow sprite declined");
        require(!memcmp(target_pixels, initial, SIZE), "accepted sprite ran native stores");
    }
    for (unsigned alias = 0; alias < 3; alias++) {
        target_pixels = target_storage;
        native_pixels = native_storage;
        for (unsigned i = 0; i < sizeof(target_storage); i++) target_storage[i] = (i * 17 + 53) & 255;
        uint8_t data[] = {3, 8, 9, 10, 0};
        if (alias == 0) memcpy(target_pixels + 45, data, sizeof(data));
        memcpy(native_storage, target_storage, sizeof(target_storage));
        flags = Lb_SPRITE_FLIP_HORIZ | (alias == 2 ? Lb_SPRITE_TRANSPAR4 : 0);
        LbSpriteSetScalingData(0, alias == 0 ? 14 : 2, 3, 1, WIDTH, 2);
        unsigned before = count;
        for (enabled = 0; enabled <= 1; enabled++) {
            uint8_t *pixels = enabled ? target_pixels : native_pixels;
            lbDisplay.WScreen = lbDisplay.GraphicsWindowPtr = pixels;
            render_ghost = pixels;
            // Mutable bytes, so no identity: a target alias must never be kept resident.
        struct TbSourceBuffer source = {alias == 0 ? pixels + 45 : data, 3, 1, 3, NULL};
            if (alias == 1) LbSpriteDrawRemapUsingScalingData(0, 0, &source, pixels);
            else LbSpriteDrawUsingScalingData(0, 0, &source);
        }
        require(count == before, "mutable sprite asset alias accepted");
        require(!memcmp(target_storage, native_storage, sizeof(target_storage)), "asset alias fallback differs from native");
    }
    fseek(fixture, 4, SEEK_SET);
    fwrite(&count, 4, 1, fixture);
    require(fclose(fixture) == 0, "cannot finish fixture");
    printf("%u narrow-pitch native sprite fixtures and three asset-alias fallbacks passed\n", count);
    return 0;
}
