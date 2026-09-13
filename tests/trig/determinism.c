#define main fixture_main
#include "fixture.c"
#undef main

int main(void)
{
    vec_screen = pixels + 83;
    poly_screen = pixels;
    const int xy[][6] = {{1,0,2,1,3,3}, {3,0,1,4,1,3}};
    const int offsets[][2] = {{0,0}, {-2,0}, {77,0}, {0,-1}, {0,59}};
    unsigned failures = 0, checked = 0, covered = 0;
    for (unsigned mode = 0; mode < 27; mode++) {
        const int flat = mode == 0 || mode == 14 || mode == 15;
        const int shade = mode == 1 || mode == 4 || mode == 16 || mode == 17 ||
            mode == 5 || mode == 6 || mode == 20 || mode == 21 || mode >= 24;
        const int textured = !flat && mode != 1 && mode != 4 && mode != 16 && mode != 17;
        vec_mode = mode;
        for (unsigned kind = 0; kind < 2; kind++) for (unsigned offset = 0; offset < 5; offset++) {
            struct PolyPoint v[3];
            for (unsigned i = 0; i < 3; i++) v[i] = (struct PolyPoint){
                xy[kind][i*2] + offsets[offset][0], xy[kind][i*2+1] + offsets[offset][1],
                (7 + i)*65536 + 123, (8 - i)*65536 + 456, (17 + i)*65536 + 789};
            for (unsigned poison = 0; poison < 2; poison++) {
                struct TrigLocalPrep prep;
                struct TrigLocalRend rend;
                memset(&prep, poison ? 0x5a : 0xa5, sizeof(prep));
                memset(&rend, poison ? 0x5a : 0xa5, sizeof(rend));
                int accepted = kind ? trig_rl_start(&prep, &rend, v, v+1, v+2)
                    : trig_ll_start(&prep, &rend, v, v+1, v+2);
                if (!accepted) abort();
                checked++;
                const long expected_shade = kind && shade && textured ? -2 : 0;
                if ((shade && rend.shade_step != expected_shade) ||
                    (textured && (rend.u_step != 0 || rend.v_step != 0))) {
                    if (offset == 0 && poison == 0)
                        fprintf(stderr, "mode %u %s retained poison: U=%lx V=%lx S=%lx\n",
                            mode, kind ? "RL" : "LL", rend.u_step, rend.v_step, rend.shade_step);
                    failures++;
                }
                for (long y = 0; y < rend.render_height; y++) {
                    long left = scans[y].X >> 16, right = scans[y].Y >> 16;
                    if (right > 0 && left < vec_window_width && right > left) covered++;
                }
            }
        }
    }
    printf("%u poisoned setup cases, %u covered rows, %u failures\n", checked, covered, failures);
    return failures != 0 || covered == 0;
}
