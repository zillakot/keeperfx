#define main capture_main
#include "capture_test.c"
#undef main

static FILE *output;
static unsigned triangle_count;

static void emit_triangle(struct PolyPoint a, struct PolyPoint b, struct PolyPoint c)
{
    struct KfxGpolyCapture capture;
    require(kfx_gpoly_capture_init(&capture, 200, 1), "triangle capture allocation");
    memset(native, 0xa7, TOTAL);
    memcpy(captured, native, TOTAL);
    vec_screen = native + PREFIX;
    kfx_gpoly_set_sink(NULL, NULL);
    draw_gpoly(&a, &b, &c);
    vec_screen = captured + PREFIX;
    kfx_gpoly_set_sink(kfx_gpoly_capture_sink, &capture);
    draw_gpoly(&a, &b, &c);
    kfx_gpoly_set_sink(NULL, NULL);
    require(!capture.failed, "triangle capture failed");
    require(memcmp(native, captured, TOTAL) == 0, "triangle observation changed native output");
    for (unsigned i = 0; i < PREFIX; i++)
        require(native[i] == 0xa7 && native[TOTAL - i - 1] == 0xa7, "triangle changed target guards");
    const struct PolyPoint vertices[] = {a, b, c};
    for (unsigned i = 0; i < 3; i++) {
        const struct PolyPoint *p = &vertices[i];
        write_u32(output, p->X); write_u32(output, p->Y);
        const int64_t attributes[] = {p->U, p->V, p->S};
        for (unsigned j = 0; j < 3; j++) {
            write_u32(output, (uint64_t)attributes[j]);
            write_u32(output, (uint64_t)attributes[j] >> 32);
        }
    }
    for (int y = 0; y < vec_window_height; y++) {
        const struct KfxGpolySpan *span = NULL;
        for (unsigned j = 0; j < capture.span_count; j++)
            if (capture.spans[j].span.y == y) span = &capture.spans[j].span;
        const uint32_t words[] = {span ? span->x : 0, span ? span->y : 0,
            span ? span->count : 0, 0, span ? span->start_low : 0,
            span ? span->start_high : 0, span ? span->step_low : 0,
            span ? span->step_high : 0};
        for (unsigned j = 0; j < 8; j++) write_u32(output, words[j]);
    }
    require(fwrite(native + PREFIX, 1, SIZE, output) == SIZE, "write expected triangle");
    kfx_gpoly_capture_free(&capture);
    triangle_count++;
}

int main(int argc, char **argv)
{
    require(argc == 2, "provide triangle fixture output path");
    output = fopen(argv[1], "wb");
    require(output != NULL, "open triangle fixture");
    vec_map = texture;
    render_fade_tables = fade;
    for (unsigned i = 0; i < sizeof(texture); ++i) texture[i] = next_random() >> 24;
    for (unsigned i = 0; i < sizeof(fade); ++i) fade[i] = next_random() >> 24;
    for (int y = 0; y < 32; y++) for (int x = -32; x < 32; x++)
        require(gpoly_divtable[y][x + 32] == (y ? x * 65536 / y : (x < 0 ? -8388607 : x > 0 ? 8388607 : 0)), "slope table identity");
    for (unsigned i = 0; i < 256; i++)
        require(gpoly_reptable[i] == (i ? 0x7fffffff / i : 0), "reciprocal table identity");
    require(fwrite("KFXGTRI1", 1, 8, output) == 8, "write triangle magic");
    write_u32(output, vec_window_width); write_u32(output, vec_window_height);
    write_u32(output, vec_screen_width); write_u32(output, 0);
    require(fwrite(texture, 1, sizeof(texture), output) == sizeof(texture), "write triangle texture");
    require(fwrite(fade, 1, sizeof(fade), output) == sizeof(fade), "write triangle fade");
    emit_triangle(point(1, 2, 0, 0, 0), point(77, 2, 31, 0, 63), point(38, 59, 16, 31, 31));
    emit_triangle(point(4, 3, 0, 0, 63), point(75, 59, 31, 31, 0), point(4, 59, 0, 31, 31));
    emit_triangle(point(-30, -20, -96, 64, 17), point(100, 5, 96, -64, 47), point(20, 90, 0, 0, 24));
    emit_triangle(point(3, 3, 0, 0, 31), point(3, 3, 0, 0, 31), point(3, 3, 0, 0, 31));
    emit_triangle(point(1, 5, 0, 0, 31), point(77, 5, 0, 0, 31), point(20, 5, 0, 0, 31));
    emit_triangle(point(90, 3, 0, 0, 31), point(120, 20, 0, 0, 31), point(100, 40, 0, 0, 31));
    emit_triangle(point(1, 70, 0, 0, 31), point(60, 90, 0, 0, 31), point(20, 100, 0, 0, 31));
    emit_triangle(point(0, 0, 0, 0, 31), point(16384, 10, 0, 0, 31), point(30, 40, 0, 0, 31));
    const struct PolyPoint orders[3] = {
        point(5, 4, -257, 513, 1), point(76, 4, 1025, -1025, 62), point(-9, 58, -513, 257, 31)
    };
    for (unsigned a = 0; a < 3; a++) for (unsigned b = 0; b < 3; b++)
        if (a != b) emit_triangle(orders[a], orders[b], orders[3 - a - b]);
    emit_triangle(point(4, -150, -32768, 32767, 31), point(60, -80, 32767, -32768, 31), point(20, 16233, -1, 1, 31));
    emit_triangle(point(-16000, -3, -2147483647L, 2147483647L, 31), point(79, 1, 2147483647L, -2147483647L, 31), point(0, 60, -1, 1, 31));
    emit_triangle(point(-16300, -3, 2147483647L, -2147483647L, 31), point(79, 2, -2147483647L, 2147483647L, 31), point(0, 60, 0, 0, 31));
    emit_triangle(point(0, 0, 0, 0, 31), point(1, 0, 1, 1, 31), point(0, 16383, 31, 31, 31));
    emit_triangle(point(0, 0, 0, 0, 31), point(1, 0, 1, 1, 31), point(0, 16384, 31, 31, 31));
    emit_triangle(point(4, 2, 0, 0, 31), point(4, 2, 31, 31, 31), point(75, 58, -31, -31, 31));
    emit_triangle(point(5, 5, 0, 0, 31), point(15, 15, 31, -31, 31), point(25, 25, -31, 31, 31));
    emit_triangle(point(0, 0, 0, 0, 31), point(-16384, 10, 31, 0, 31), point(0, 60, 0, 31, 31));
    emit_triangle(point(0, 0, 0, 0, 31), point(-16385, 10, 31, 0, 31), point(0, 60, 0, 31, 31));
    struct PolyPoint wide_a = point(-9, -3, 0, 0, 31);
    struct PolyPoint wide_b = point(70, 10, 0, 0, 31);
    struct PolyPoint wide_c = point(35, 60, 0, 0, 31);
    wide_a.U = INT64_MIN; wide_a.V = INT64_MAX;
    wide_b.U = INT64_MAX; wide_b.V = INT64_MIN;
    wide_c.U = INT64_C(0x7654321089abcdef); wide_c.V = -INT64_C(0x123456789abcdef);
    emit_triangle(wide_a, wide_b, wide_c);
    emit_triangle(wide_c, wide_b, wide_a);
    for (unsigned i = 0; i < 1200; i++) {
        struct PolyPoint p[3];
        for (unsigned j = 0; j < 3; j++) {
            int x = (int)(next_random() % 180) - 50;
            int y = (int)(next_random() % 130) - 35;
            int u = (int)(next_random() % 65536) - 32768;
            int v = (int)(next_random() % 65536) - 32768;
            int s = (int)(next_random() % 33) + 15;
            p[j] = point(x, y, u, v, s);
            p[j].U += next_random() & 65535;
            p[j].V += next_random() & 65535;
            p[j].S += next_random() & 65535;
        }
        emit_triangle(p[0], p[1], p[2]);
    }
    require(fseek(output, 20, SEEK_SET) == 0, "seek triangle count");
    write_u32(output, triangle_count);
    require(fclose(output) == 0, "close triangle fixture");
    printf("PASS: %u native triangle fixtures\n", triangle_count);
    return 0;
}
