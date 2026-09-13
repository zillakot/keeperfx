#include "bflib_video.h"
#include "front_simple.h"
#include "gui_draw.h"
#include "kfx/renderer/software/WgpuRawImage.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct DisplayStruct lbDisplay;
unsigned short MyScreenHeight, pixel_size;
enum { WIDTH = 211, HEIGHT = 139, SIZE = WIDTH * HEIGHT };
static uint8_t pixels[SIZE], initial[SIZE], expected[SIZE], asset[256 * 256];
static unsigned submissions, boundaries, count;
static int enabled = 1, decline;
static FILE *fixture;
static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}
int kfx_wgpu_native_enabled(void) { return enabled; }
void kfx_wgpu_terrain_boundary(int allow) { require(allow == 0, "raw image allowed pending terrain"); boundaries++; }
int kfx_wgpu_native_draw(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuNativeResource *source,
    const struct KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    (void)table;
    submissions++;
    if (decline) return 0;
    require(target->pixels == pixels && target->pitch == WIDTH, "wrong target");
    require(!memcmp(pixels, initial, SIZE), "CPU destination writes before GPU submission");
    uint8_t *screen = lbDisplay.WScreen, *window = lbDisplay.GraphicsWindowPtr;
    long pitch = lbDisplay.GraphicsScreenWidth;
    unsigned before = submissions;
    memcpy(expected, initial, SIZE);
    oracle(expected, target->pitch, context);
    require(before == submissions, "recursive oracle submission");
    require(lbDisplay.WScreen == screen && lbDisplay.GraphicsWindowPtr == window &&
        lbDisplay.GraphicsScreenWidth == pitch, "oracle changed aliases");
    require(!memcmp(pixels, initial, SIZE), "oracle changed native target");
    if (fixture) {
        uint32_t info[] = {target->width, target->height, target->pitch,
            source ? source->width : 0, source ? source->height : 0, source ? source->pitch : 0,
            source ? source->length : 0};
        fwrite(info, sizeof(info), 1, fixture);
        fwrite(command, sizeof(*command), 1, fixture);
        if (source) fwrite(source->bytes, source->length, 1, fixture);
        fwrite(expected, SIZE, 1, fixture);
        count++;
    }
    return 1;
}

static void raw(int sw, int sh, int dw, int dh, int x, int y)
{
    copy_raw8_image_buffer(pixels, WIDTH, HEIGHT, dw, dh, x, y, asset, sw, sh);
    require(!memcmp(pixels, initial, SIZE), "accepted raw image ran legacy stores");
}

int main(int argc, char **argv)
{
    if (argc != 2) return 1;
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / WIDTH * 13) & 255;
    for (unsigned i = 0; i < sizeof(asset); i++) asset[i] = (i * 37 + i / 7 * 13) & 255;
    memcpy(pixels, initial, SIZE);
    lbDisplay.WScreen = pixels;
    lbDisplay.GraphicsWindowPtr = pixels + WIDTH + 3;
    lbDisplay.GraphicsScreenWidth = WIDTH;
    lbDisplay.GraphicsScreenHeight = HEIGHT;
    lbDisplay.PhysicalScreenWidth = WIDTH;
    MyScreenHeight = HEIGHT;
    pixel_size = 1;
    gui_slab = asset;
    enabled = 0;
    copy_raw8_image_buffer(pixels, WIDTH, HEIGHT, 39, 27, 7, 6, asset, 17, 13);
    require(submissions == 0, "default/disabled mode submitted");
    uint8_t fallback[SIZE];
    memcpy(fallback, pixels, SIZE);
    memcpy(pixels, initial, SIZE);
    enabled = 1;
    decline = 1;
    copy_raw8_image_buffer(pixels, WIDTH, HEIGHT, 39, 27, 7, 6, asset, 17, 13);
    require(!memcmp(pixels, fallback, SIZE), "declined raw image changed fallback");
    decline = 0;
    memcpy(pixels, initial, SIZE);
    unsigned before = submissions;
    copy_raw8_image_buffer(pixels, WIDTH, HEIGHT, WIDTH, HEIGHT, 0, 0, pixels, WIDTH, HEIGHT);
    require(submissions == before && boundaries == 2, "aliased source must flush terrain then decline");
    memcpy(pixels, initial, SIZE);
    if (!(fixture = fopen(argv[1], "wb"))) return 1;
    uint32_t header[] = {0x3152464b, 0, WIDTH, HEIGHT, sizeof(struct KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    int sizes[][2] = {{1, 1}, {2, 3}, {7, 5}, {17, 13}, {83, 61}, {127, 89}};
    int destinations[][4] = {{83,61,0,0}, {39,27,7,6}, {17,13,0,0}, {149,111,-19,-23},
        {99,79,11,17}, {1,1,37,29}, {7,5,76,56}, {70,60,-70,-60}};
    for (unsigned s = 0; s < sizeof(sizes)/sizeof(sizes[0]); s++)
        for (unsigned d = 0; d < sizeof(destinations)/sizeof(destinations[0]); d++)
            raw(sizes[s][0], sizes[s][1], destinations[d][0], destinations[d][1],
                destinations[d][2], destinations[d][3]);
    for (int i = 0; i < 100; i++) {
        int sw = 1 + (i * 37) % 97, sh = 1 + (i * 19) % 89;
        int dw = 1 + (i * 53) % 317, dh = 1 + (i * 41) % 233;
        int x = -dw + (i * 73) % (WIDTH + dw + 1);
        int y = -dh + (i * 97) % (HEIGHT + dh + 1);
        raw(sw, sh, dw, dh, x, y);
    }
    for (int scale = 1; scale <= 2; scale++) {
        pixel_size = scale;
        lbDisplay.PhysicalScreenWidth = WIDTH / scale;
        for (int x = -7; x <= 7; x += 7)
            for (int y = -9; y <= 9; y += 9)
                for (int w = 1; w <= 205; w += 31) {
                    if (w + x <= 0) continue;
                    draw_slab64k_background_immediate(x * scale, y * scale, w * scale, 133 * scale);
                    require(!memcmp(pixels, initial, SIZE), "accepted tile ran legacy stores");
                }
    }
    require(kfx_wgpu_raw_clear(pixels, WIDTH, WIDTH - 4, HEIGHT, 213), "clear declined");
    require(kfx_wgpu_raw_clear(pixels, WIDTH, WIDTH, HEIGHT, 0), "full clear declined");
    fseek(fixture, 4, SEEK_SET);
    fwrite(&count, sizeof(count), 1, fixture);
    fclose(fixture);
    printf("%u actual native raw image/tile/clear fixtures\n", count);
    return 0;
}
