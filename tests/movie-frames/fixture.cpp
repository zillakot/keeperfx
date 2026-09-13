#include "kfx/renderer/software/MovieFrame.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include "bflib_fmvids.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

enum { PITCH = 211, HEIGHT = 139, SIZE = PITCH * HEIGHT };
static uint8_t pixels[SIZE], initial[SIZE], expected[SIZE];
static bool enabled = true, decline;
static unsigned submissions, boundaries, count;
static FILE *fixture;
static void require(bool condition, const char *message)
{
    if (!condition) { std::fprintf(stderr, "%s\n", message); std::exit(1); }
}
extern "C" int kfx_wgpu_native_enabled(void) { return enabled; }
extern "C" void kfx_wgpu_terrain_boundary(int allow)
{
    require(allow == 0, "movie allowed pending terrain"); boundaries++;
}
extern "C" int kfx_wgpu_native_draw(const KfxGpolyTarget *target,
    const KfxWgpuDrawCommand *command, const KfxWgpuNativeResource *source,
    const KfxWgpuNativeResource *table, KfxWgpuNativeOracle oracle, void *context)
{
    require(!table, "unexpected table"); submissions++;
    if (decline) return 0;
    require(target->pixels == pixels && target->pitch == PITCH, "wrong target");
    require(!memcmp(pixels, initial, SIZE), "CPU writes before submission");
    unsigned before = submissions;
    memcpy(expected, initial, SIZE);
    oracle(expected, target->pitch, context);
    require(before == submissions, "oracle recursively submitted");
    require(!memcmp(pixels, initial, SIZE), "oracle mutated live target");
    if (fixture) {
        uint32_t info[] = {uint32_t(target->width), uint32_t(target->height), uint32_t(target->pitch),
            source ? source->width : 0, source ? source->height : 0, source ? source->pitch : 0,
            source ? uint32_t(source->length) : 0};
        fwrite(info, sizeof(info), 1, fixture);
        fwrite(command, sizeof(*command), 1, fixture);
        if (source) fwrite(source->bytes, source->length, 1, fixture);
        fwrite(expected, SIZE, 1, fixture);
        count++;
    }
    return 1;
}
static void run(int width, int height, int flags, unsigned seed = 0)
{
    int pitch = width + 5;
    std::vector<uint8_t> source(pitch * height);
    for (unsigned i = 0; i < source.size(); i++) source[i] = (i * 37 + i / pitch * 17 + seed * 23) & 255;
    KfxMovieFrame frame = {source.data(), width, height, pitch};
    KfxMovieTarget target = {pixels, PITCH, HEIGHT, PITCH - 4, HEIGHT - 2};
    if (flags & (SMK_FullscreenFit | SMK_FullscreenStretch | SMK_FullscreenCrop))
        kfx_movie_copy_scaled(frame, target, flags);
    else kfx_movie_copy(frame, target, flags);
}
int main(int argc, char **argv)
{
    if (argc != 2) return 1;
    for (unsigned i = 0; i < SIZE; i++) initial[i] = (i * 19 + i / PITCH * 13) & 255;
    memcpy(pixels, initial, SIZE);
    const int fallback_flags[] = {0, SMK_InterlaceLine, SMK_PixelDoubleLine | SMK_PixelDoubleWidth,
                      SMK_FullscreenFit, SMK_FullscreenCrop, SMK_FullscreenStretch};
    for (int flags : fallback_flags) {
        enabled = false;
        unsigned before = submissions;
        run(17, 13, flags);
        require(before == submissions, "disabled renderer submitted");
        uint8_t fallback[SIZE]; memcpy(fallback, pixels, SIZE);
        memcpy(pixels, initial, SIZE);
        enabled = true; decline = true;
        run(17, 13, flags);
        require(!memcmp(pixels, fallback, SIZE), "declined fallback mismatch");
        memcpy(pixels, initial, SIZE);
    }
    decline = false;
    if (!(fixture = fopen(argv[1], "wb"))) return 1;
    uint32_t header[] = {0x314d464b, 0, PITCH, HEIGHT, sizeof(KfxWgpuDrawCommand)};
    fwrite(header, sizeof(header), 1, fixture);
    for (int width : {4, 7, 16, 17, 60, 99}) {
        for (unsigned mode = 0; mode < 8; mode++) {
            int flags = ((mode & 1) ? SMK_PixelDoubleWidth : 0) |
                ((mode & 2) ? SMK_PixelDoubleLine : 0) | ((mode & 4) ? SMK_InterlaceLine : 0);
            run(width, 33, flags, mode);
        }
    }
    run(211, 33, 0, 9);
    int sizes[][2] = {{1,1}, {7,13}, {17,9}, {99,61}, {320,200}, {321,201}, {113,179}, {400,500}};
    for (auto &size : sizes) for (unsigned mode = 1; mode < 8; mode++)
        run(size[0], size[1], ((mode & 1) ? SMK_FullscreenFit : 0) |
            ((mode & 2) ? SMK_FullscreenStretch : 0) | ((mode & 4) ? SMK_FullscreenCrop : 0), mode);
    require(!memcmp(pixels, initial, SIZE), "accepted draw ran native stores");
    auto before = submissions;
    KfxMovieFrame aliased = {pixels, 17, 13, PITCH};
    KfxMovieTarget target = {pixels, PITCH, HEIGHT, PITCH, HEIGHT};
    kfx_movie_copy_scaled(aliased, target, SMK_FullscreenStretch);
    require(submissions == before, "aliased movie source submitted");
    fseek(fixture, 4, SEEK_SET); fwrite(&count, sizeof(count), 1, fixture); fclose(fixture);
    printf("%u actual native movie cases; %u boundaries\n", count, boundaries);
}
