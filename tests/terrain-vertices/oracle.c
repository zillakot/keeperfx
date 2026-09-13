#include "kfx/renderer/software/bflib_render_gpoly.c"
unsigned char vec_mode = VM_QuadTextured;
unsigned char *vec_screen, *vec_map, *render_fade_tables;
unsigned long vec_screen_width;
long vec_window_width, vec_window_height;
uint32_t get_gameturn(void) { return 0; }
int LbErrorLog(const char* format, ...) { (void)format; return 0; }
void vertex_draw(const struct KfxGpolyTarget* target, const struct KfxWgpuTriangle* triangle,
    const uint8_t* texture, const uint8_t* fade, int software)
{
    if (software) { rasterize_original_triangle(target, triangle, texture, fade); return; }
    vec_screen = target->pixels; vec_screen_width = target->pitch;
    vec_window_width = target->width; vec_window_height = target->height;
    vec_map = (unsigned char*)texture; render_fade_tables = (unsigned char*)fade;
    struct PolyPoint points[3];
    for (int i = 0; i < 3; ++i) {
        points[i] = (struct PolyPoint){triangle->vertices[i].x, triangle->vertices[i].y,
            triangle->vertices[i].u, triangle->vertices[i].v, triangle->vertices[i].shade};
    }
    vertex_a_x = 123456;
    draw_gpoly(&points[0], &points[1], &points[2]);
}
int vertex_cpu_setup_ran(void) { return vertex_a_x != 123456; }
