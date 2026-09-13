#include "kfx/renderer/software/bflib_render_gpoly.c"

#include <stdarg.h>

unsigned char vec_mode = VM_QuadTextured;
unsigned char *vec_screen, *vec_map, *render_fade_tables;
unsigned long vec_screen_width = 83;
long vec_window_width = 79, vec_window_height = 61;

uint32_t get_gameturn(void) { return 0; }
int LbErrorLog(const char *format, ...) { (void)format; return 0; }

enum { PREFIX = 16384, SIZE = 83 * 61, TOTAL = PREFIX + SIZE + 16384 };
static unsigned char native[TOTAL], captured[TOTAL], replay[TOTAL];
static unsigned char texture[7968], fade[KFX_GPOLY_FADE_BYTES];
static uint32_t random_state = 0x583917ab;
static unsigned tested, spans_tested;

static uint32_t next_random(void)
{
    random_state = random_state * 1664525u + 1013904223u;
    return random_state;
}

static void require(int condition, const char *message)
{
    if (!condition) {
        fprintf(stderr, "FAIL: %s (fixture %u)\n", message, tested);
        exit(1);
    }
}

static struct PolyPoint point(long x, long y, long u, long v, long shade)
{
    return (struct PolyPoint){x, y, u * 65536, v * 65536, shade * 65536};
}

static void compare_triangle(struct PolyPoint a, struct PolyPoint b, struct PolyPoint c,
    int empty)
{
    memset(native, 0xa7, sizeof(native));
    memcpy(captured, native, sizeof(native));
    memcpy(replay, native, sizeof(native));
    kfx_gpoly_set_sink(NULL, NULL);
    vec_screen = native + PREFIX;
    draw_gpoly(&a, &b, &c);

    struct KfxGpolyCapture capture;
    require(kfx_gpoly_capture_init(&capture, 200, 2), "allocate capture");
    vec_screen = captured + PREFIX;
    kfx_gpoly_set_sink(kfx_gpoly_capture_sink, &capture);
    draw_gpoly(&a, &b, &c);
    kfx_gpoly_set_sink(NULL, NULL);
    require(!capture.failed, "capture accepted");
    require(memcmp(native, captured, TOTAL) == 0, "observation changed legacy output");
    require(kfx_gpoly_capture_replay(&capture, replay + PREFIX, SIZE), "oracle replay");
    require(memcmp(native, replay, TOTAL) == 0, "oracle differs from legacy renderer or guards");
    require(!empty || !capture.span_count, "degenerate or rejected triangle drew spans");
    spans_tested += capture.span_count;
    kfx_gpoly_capture_free(&capture);
    ++tested;
}

static int reject_sink(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *map, const uint8_t *table)
{
    (void)context; (void)target; (void)span; (void)map; (void)table;
    return KFX_GPOLY_ERROR;
}

static int consume_sink(void *context, const struct KfxGpolyTarget *target,
    const struct KfxGpolySpan *span, const uint8_t *map, const uint8_t *table)
{
    return kfx_gpoly_capture_sink(context, target, span, map, table) == KFX_GPOLY_DECLINED
        ? KFX_GPOLY_CONSUMED : KFX_GPOLY_ERROR;
}

static void compare_line(uint32_t low, uint32_t high, uint32_t step_low, uint32_t step_high)
{
    struct KfxGpolyCapture capture;
    require(kfx_gpoly_capture_init(&capture, 1, 1), "allocate line capture");
    memset(native, 0xa7, TOTAL);
    memcpy(replay, native, TOTAL);
    vec_screen = native + PREFIX;
    texcoord_delta_x = (TexCoordShort){{step_low, step_high}};
    TexCoord position = {{0xffffffff, low, high}};
    kfx_gpoly_set_sink(kfx_gpoly_capture_sink, &capture);
    draw_gpoly_line(vec_screen, 79, position, 0, 0);
    kfx_gpoly_set_sink(NULL, NULL);
    require(kfx_gpoly_capture_replay(&capture, replay + PREFIX, SIZE), "line replay");
    require(memcmp(native, replay, TOTAL) == 0, "carry or wrap differs from legacy loop");
    spans_tested += capture.span_count;
    kfx_gpoly_capture_free(&capture);
    ++tested;
}

static void write_u32(FILE *file, uint32_t value)
{
    unsigned char bytes[4] = {value, value >> 8, value >> 16, value >> 24};
    require(fwrite(bytes, 1, 4, file) == 4, "write fixture word");
}

static void sequence(const char *fixture)
{
    struct PolyPoint a = point(-11, -7, -33, 65, 12);
    struct PolyPoint b = point(69, 18, 74, -8, 48);
    struct PolyPoint c = point(4, 70, 3, 96, 23);
    struct KfxGpolyCapture capture;
    require(kfx_gpoly_capture_init(&capture, 200, 3), "allocate sequence");
    memset(native, 0xa7, TOTAL);
    memcpy(captured, native, TOTAL);
    memcpy(replay, native, TOTAL);
    vec_screen = native + PREFIX;
    kfx_gpoly_set_sink(kfx_gpoly_capture_sink, &capture);
    draw_gpoly(&a, &b, &c);
    for (unsigned i = 0; i < sizeof(texture); ++i) texture[i] ^= 0x59;
    for (unsigned i = 0; i < sizeof(fade); ++i) fade[i] ^= 0xe3;
    a = point(75, 5, 72, 37, 22);
    b = point(-6, 28, -5, 64, 43);
    c = point(51, 58, 66, -61, 18);
    draw_gpoly(&a, &b, &c);
    kfx_gpoly_set_sink(NULL, NULL);
    require(capture.texture_count == 2 && capture.fade_count == 2, "mutable resources versioned");
    memset(texture, 0, sizeof(texture));
    memset(fade, 0, sizeof(fade));
    require(kfx_gpoly_capture_replay(&capture, replay + PREFIX, SIZE), "immutable sequence replay");
    require(memcmp(native, replay, TOTAL) == 0, "ordering or resource ownership differs");
    if (fixture) {
        FILE *file = fopen(fixture, "wb");
        require(file != NULL, "open fixture");
        require(fwrite("KFXGSPN1", 1, 8, file) == 8, "write fixture magic");
        write_u32(file, vec_window_width); write_u32(file, vec_window_height);
        write_u32(file, vec_screen_width); write_u32(file, capture.span_count);
        write_u32(file, capture.texture_count); write_u32(file, capture.fade_count);
        for (uint32_t i = 0; i < capture.span_count; ++i) {
            const struct KfxGpolyCapturedSpan *s = &capture.spans[i];
            write_u32(file, s->span.x); write_u32(file, s->span.y); write_u32(file, s->span.count);
            write_u32(file, s->span.start_low); write_u32(file, s->span.start_high);
            write_u32(file, s->span.step_low); write_u32(file, s->span.step_high);
            write_u32(file, s->texture); write_u32(file, s->fade);
        }
        for (uint32_t i = 0; i < capture.texture_count; ++i)
            require(fwrite(capture.textures[i], 1, KFX_GPOLY_TEXTURE_BYTES, file) == KFX_GPOLY_TEXTURE_BYTES, "write texture");
        for (uint32_t i = 0; i < capture.fade_count; ++i)
            require(fwrite(capture.fades[i], 1, KFX_GPOLY_FADE_BYTES, file) == KFX_GPOLY_FADE_BYTES, "write fade");
        require(fwrite(captured + PREFIX, 1, SIZE, file) == SIZE, "write initial target");
        require(fwrite(native + PREFIX, 1, SIZE, file) == SIZE, "write legacy target");
        require(fclose(file) == 0, "close fixture");
    }
    spans_tested += capture.span_count;
    kfx_gpoly_capture_free(&capture);
    ++tested;
}

static void sink_outcomes(void)
{
    struct PolyPoint a = point(3, 4, 1, 2, 12), b = point(58, 9, 35, 8, 30),
        c = point(27, 50, 19, 61, 45);
    struct KfxGpolyCapture capture;
    require(kfx_gpoly_capture_init(&capture, 200, 2), "allocate consumed capture");
    memset(native, 0xa7, TOTAL);
    memset(captured, 0xa7, TOTAL);
    memset(replay, 0xa7, TOTAL);
    vec_screen = native + PREFIX;
    draw_gpoly(&a, &b, &c);
    vec_screen = captured + PREFIX;
    kfx_gpoly_set_sink(reject_sink, NULL);
    draw_gpoly(&a, &b, &c);
    require(memcmp(native, captured, TOTAL) == 0, "error did not preserve fallback");
    memset(captured, 0xa7, TOTAL);
    kfx_gpoly_set_sink(consume_sink, &capture);
    draw_gpoly(&a, &b, &c);
    kfx_gpoly_set_sink(NULL, NULL);
    require(memcmp(captured, replay, TOTAL) == 0, "consumed sink ran CPU pixels");
    require(kfx_gpoly_capture_replay(&capture, replay + PREFIX, SIZE), "replay consumed capture");
    require(memcmp(native, replay, TOTAL) == 0, "consumed command replay differs");
    kfx_gpoly_capture_free(&capture);
    require(kfx_gpoly_capture_init(&capture, 1, 1), "allocate bounded capture");
    memset(captured, 0xa7, TOTAL);
    kfx_gpoly_set_sink(kfx_gpoly_capture_sink, &capture);
    draw_gpoly(&a, &b, &c);
    kfx_gpoly_set_sink(NULL, NULL);
    require(capture.failed && !kfx_gpoly_capture_replay(&capture, replay + PREFIX, SIZE), "overflow not rejected");
    require(memcmp(native, captured, TOTAL) == 0, "overflow lost legacy drawing");
    kfx_gpoly_capture_free(&capture);
    ++tested;
}

int main(int argc, char **argv)
{
    vec_map = texture;
    render_fade_tables = fade;
    for (unsigned i = 0; i < sizeof(texture); ++i) texture[i] = next_random() >> 24;
    for (unsigned i = 0; i < sizeof(fade); ++i) fade[i] = next_random() >> 24;
    compare_triangle(point(1, 2, 0, 0, 0), point(77, 2, 31, 0, 63), point(38, 59, 16, 31, 31), 0);
    compare_triangle(point(4, 3, 0, 0, 63), point(75, 59, 31, 31, 0), point(4, 59, 0, 31, 31), 0);
    compare_triangle(point(-30, -20, -96, 64, 17), point(100, 5, 96, -64, 47), point(20, 90, 0, 0, 24), 0);
    compare_triangle(point(3, 3, 0, 0, 31), point(3, 3, 0, 0, 31), point(3, 3, 0, 0, 31), 1);
    compare_triangle(point(1, 5, 0, 0, 31), point(77, 5, 0, 0, 31), point(20, 5, 0, 0, 31), 1);
    compare_triangle(point(90, 3, 0, 0, 31), point(120, 20, 0, 0, 31), point(100, 40, 0, 0, 31), 1);
    compare_triangle(point(1, 70, 0, 0, 31), point(60, 90, 0, 0, 31), point(20, 100, 0, 0, 31), 1);
    compare_triangle(point(0, 0, 0, 0, 31), point(16384, 10, 0, 0, 31), point(30, 40, 0, 0, 31), 1);
    for (unsigned i = 0; i < 600; ++i) {
        struct PolyPoint p[3];
        for (unsigned j = 0; j < 3; ++j) {
            int x = (int)(next_random() % 180) - 50;
            int y = (int)(next_random() % 130) - 35;
            int u = (int)(next_random() % 1024) - 512;
            int v = (int)(next_random() % 1024) - 512;
            int s = (int)(next_random() % 33) + 15;
            p[j] = point(x, y, u, v, s);
        }
        compare_triangle(p[0], p[1], p[2], 0);
    }
    compare_line(0xffff0100, 0xffffffff, 0x10000, 0x00010000);
    compare_line(0x100, 0, 0xffff0000, 0xffffffff);
    compare_line(0x1000, 0x1fffffff, 79, 0xff0000ff);
    compare_line(0x3000, 0xff000000, 0xffffffd0, 0xffff00ff);
    sink_outcomes();
    sequence(argc > 1 ? argv[1] : NULL);
    printf("PASS: %u fixtures, %u captured spans; exact legacy indices, bounds, ownership, sink outcomes\n", tested, spans_tested);
    return 0;
}
