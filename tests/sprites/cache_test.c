/* The emitter's expanded-artwork cache: what a hit is allowed to skip, what separates
   two generations, and the bounds that keep the cache from growing without limit. */
#include "kfx/renderer/software/bflib_vidraw.c"
#include "kfx/renderer/software/WgpuSprite.h"

struct DisplayStruct lbDisplay;
unsigned char *render_ghost, *render_alpha;
static unsigned short flags;
/* 16 KiB of expanded artwork per name, so 512 names reach the cache's byte limit while
   its 1,024 slots are still half empty: the sweep is what bounds this cache, not the
   slot count, and the crowd below is large enough to make both bind. */
enum { WIDTH = 200, HEIGHT = 160, SIZE = WIDTH * HEIGHT, W = 128, H = 64 };
enum { CACHE_SLOTS = 1024, CACHE_BYTE_LIMIT = 8 * 1024 * 1024 };
enum { ARTWORK_BYTES = 2 * W * H, ROW_BYTES = 5, RLE_BYTES = ROW_BYTES * H };
static uint8_t target_pixels[SIZE];

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}

unsigned short RendererGetDrawFlags(void) { return flags; }
int kfx_wgpu_native_enabled(void) { return 1; }
void kfx_wgpu_native_flush(void) {}
int kfx_wgpu_native_cpu_barrier(void) { return 1; }
void kfx_wgpu_terrain_boundary(int allow) { require(!allow, "sprite left terrain batching enabled"); }

/* Every read barrier of one draw, so a hit's single barrier over the walked extent can
   be told from the miss's walk. */
enum { BARRIER_LIMIT = 4096 };
struct Barrier { const void *bytes; size_t length; };
static struct Barrier barriers[BARRIER_LIMIT];
static unsigned barrier_count;

int kfx_wgpu_native_read_barrier(const void *bytes, size_t length)
{
    if (barrier_count < BARRIER_LIMIT) {
        barriers[barrier_count].bytes = bytes;
        barriers[barrier_count].length = length;
    }
    barrier_count++;
    return 1;
}

/* The barriers the last draw took over the RLE's first byte: one on a hit, covering the
   whole extent the walk measured, and the walk's first single byte on a miss. */
static unsigned source_barriers(const void *rle, size_t *length)
{
    unsigned found = 0;
    require(barrier_count <= BARRIER_LIMIT, "barrier record overflowed");
    for (unsigned i = 0; i < barrier_count; i++)
        if (barriers[i].bytes == rle) { if (length) *length = barriers[i].length; found++; }
    return found;
}

static unsigned submissions;
static uint8_t submitted_artwork[ARTWORK_BYTES];
static const void *submitted_identity;
static uint64_t submitted_generation;

int kfx_wgpu_native_draw_sprite(const struct KfxGpolyTarget *target,
    const struct KfxWgpuDrawCommand *command, const struct KfxWgpuSpriteAssets *assets,
    KfxWgpuNativeOracle oracle, void *context)
{
    (void)target; (void)oracle; (void)context;
    require(command->kind == KFX_WGPU_DRAW_SPRITE, "not a sprite command");
    require(assets->artwork->length == ARTWORK_BYTES, "unexpected artwork length");
    require(assets->ranges->length == (size_t)(W + H) * 8, "unexpected range length");
    require(assets->remap->length == 256, "unexpected remap length");
    memcpy(submitted_artwork, assets->artwork->bytes, ARTWORK_BYTES);
    submitted_identity = assets->identity;
    submitted_generation = assets->generation;
    submissions++;
    return 1;
}

/* One literal run of two pixels per row, then transparency to the row's end. */
static void fill_artwork(uint8_t *rle, uint8_t first, uint8_t second)
{
    for (unsigned y = 0; y < H; y++) {
        uint8_t *row = rle + y * ROW_BYTES;
        row[0] = 2; row[1] = first + y; row[2] = second + y;
        row[3] = (uint8_t)(int8_t)-(int)(W - 2);
        row[4] = 0;
    }
}

static void expect_artwork(uint8_t first, uint8_t second, const char *message)
{
    for (unsigned y = 0; y < H; y++) {
        const uint8_t *pixels = submitted_artwork + 2 * y * W;
        require(pixels[0] == (uint8_t)(first + y) && pixels[1] == 1, message);
        require(pixels[2] == (uint8_t)(second + y) && pixels[3] == 2, message);
        for (unsigned x = 2; x < W; x++)
            require(pixels[2 * x] == 0 && pixels[2 * x + 1] == 0, message);
    }
}

/* One draw of one identity, returning the number of read barriers it took. */
static unsigned draw(const uint8_t *rle, const void *identity)
{
    const struct TbSourceBuffer buffer = {rle, W, H, W, identity};
    LbSpriteSetScalingData(4, 6, W, H, W, H);
    barrier_count = 0;
    const unsigned before = submissions;
    LbSpriteDrawUsingScalingData(0, 0, &buffer);
    require(submissions == before + 1, "sprite declined");
    return barrier_count;
}

int main(void)
{
    lbDisplay.WScreen = target_pixels;
    lbDisplay.GraphicsScreenWidth = WIDTH;
    lbDisplay.GraphicsScreenHeight = HEIGHT;
    lbDisplay.GraphicsWindowX = 0;
    lbDisplay.GraphicsWindowY = 0;
    lbDisplay.GraphicsWindowWidth = WIDTH;
    lbDisplay.GraphicsWindowHeight = HEIGHT;
    lbDisplay.GraphicsWindowPtr = target_pixels;
    flags = 0;

    static uint8_t rle[RLE_BYTES];
    fill_artwork(rle, 0x10, 0x60);

    // A miss walks the RLE byte by byte; a hit takes one barrier over the walked extent.
    const unsigned missed = draw(rle, rle);
    expect_artwork(0x10, 0x60, "miss produced the wrong artwork");
    require(submitted_identity == rle && submitted_generation == kfx_render_sprite_generation,
        "identity or generation not carried to the bridge");
    size_t covered = 0;
    require(missed > H, "a miss must walk the RLE");
    require(source_barriers(rle, &covered) == 1 && covered == 1,
        "a miss walks the RLE one run byte at a time");
    const unsigned hit = draw(rle, rle);
    expect_artwork(0x10, 0x60, "hit produced the wrong artwork");
    require(hit < missed, "a hit must take fewer barriers than the walk it skips");
    require(source_barriers(rle, &covered) == 1 && covered == RLE_BYTES,
        "the hit's single barrier must cover exactly the extent the walk measured");

    // Rewriting the bytes behind a live name serves the expansion the name already has;
    // the bump is what makes the new artwork visible.
    fill_artwork(rle, 0x21, 0x71);
    draw(rle, rle);
    expect_artwork(0x10, 0x60, "a rewrite without a bump must not be visible");
    kfx_render_sprites_changed();
    draw(rle, rle);
    expect_artwork(0x21, 0x71, "a bump must make the new artwork visible");
    draw(rle, rle);
    expect_artwork(0x21, 0x71, "the bumped artwork must stay cached");

    // A sprite with no name never enters the cache.
    size_t entries = 0, bytes = 0;
    kfx_wgpu_sprite_cache_stats(&entries, &bytes);
    const size_t named = entries;
    draw(rle, NULL);
    expect_artwork(0x21, 0x71, "a nameless sprite produced the wrong artwork");
    kfx_wgpu_sprite_cache_stats(&entries, &bytes);
    require(entries == named, "a nameless sprite must not be cached");

    /* More names than the cache has slots, each larger than a probe window's share of
       the byte limit: the probe runs out, the sweep runs, and every name still draws its
       own artwork whether it was kept or rebuilt. */
    enum { NAMES = 1500 };
    static uint8_t crowd[NAMES][RLE_BYTES];
    for (unsigned i = 0; i < NAMES; i++) {
        fill_artwork(crowd[i], (uint8_t)(i * 7 + 3), (uint8_t)(i * 13 + 5));
        draw(crowd[i], crowd[i]);
        expect_artwork((uint8_t)(i * 7 + 3), (uint8_t)(i * 13 + 5), "a crowded name drew the wrong artwork");
        kfx_wgpu_sprite_cache_stats(&entries, &bytes);
        require(entries <= CACHE_SLOTS, "the cache exceeded its slot count");
        require(bytes <= CACHE_BYTE_LIMIT, "the cache exceeded its byte limit");
        require(bytes == entries * ARTWORK_BYTES, "cached bytes do not match the entries held");
    }
    kfx_wgpu_sprite_cache_stats(&entries, &bytes);
    require(entries < CACHE_SLOTS, "the byte sweep never ran: the slot count bound first");
    for (unsigned i = 0; i < NAMES; i++) {
        draw(crowd[i], crowd[i]);
        expect_artwork((uint8_t)(i * 7 + 3), (uint8_t)(i * 13 + 5), "a redrawn name drew the wrong artwork");
    }
    kfx_wgpu_sprite_cache_stats(&entries, &bytes);
    require(entries > 0 && entries < CACHE_SLOTS && bytes <= CACHE_BYTE_LIMIT,
        "cache bounds broken");

    printf("%u sprite artwork cache draws; %zu entries and %zu bytes held\n",
        submissions, entries, bytes);
    return 0;
}
