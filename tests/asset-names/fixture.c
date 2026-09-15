/* The loader's half of the naming contract: LbDataFree must take a buffer's name away
 * and move the generation before the storage can be handed to something else. */
#include "bflib_filelst.h"
#include "kfx/renderer/GpolyCapture.h"
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require(int condition, const char *message)
{
    if (!condition) { fprintf(stderr, "%s\n", message); exit(1); }
}

int LbJustLog(const char *format, ...) { (void)format; return 0; }
int LbErrorLog(const char *format, ...) { (void)format; return 0; }
long LbFileLengthRnc(const char *fname) { (void)fname; return -1; }
long LbFileLoadAt(const char *fname, void *buffer) { (void)fname; (void)buffer; return -1; }
uint32_t get_gameturn(void) { return 0; }

int main(void)
{
    unsigned char *buffer = calloc(4096, 1);
    require(buffer != NULL, "allocation failed");
    unsigned char *end = NULL;
    struct TbLoadFiles entry = {"fixture.dat", &buffer, &end, 4096, 0, 0};

    kfx_render_asset_range(buffer, 4096);
    require(kfx_render_asset_stable(buffer, 4096), "range did not register");
    const uint64_t generation = kfx_render_asset_generation;

    require(LbDataFree(&entry) == 1, "free refused");
    require(buffer == NULL, "free left the pointer set");
    require(kfx_render_asset_generation == generation + 1, "free did not move the generation");

    /* The reuse this exists to make safe: another allocation of the same size usually
     * lands back on the freed address, and it must not inherit the name. */
    unsigned char *reused = calloc(4096, 1);
    require(reused != NULL, "second allocation failed");
    require(!kfx_render_asset_stable(reused, 4096), "freed range still vouches for its address");
    kfx_render_asset_range(reused, 4096);
    require(kfx_render_asset_stable(reused, 4096), "re-registration failed");

    /* A static entry returns before the free, so it must not move the generation either. */
    unsigned char *statics = reused;
    unsigned char *static_end = NULL;
    struct TbLoadFiles skipped = {"!static.dat", &statics, &static_end, 4096, 0, 0};
    const uint64_t held = kfx_render_asset_generation;
    require(LbDataFree(&skipped) == 1, "static entry refused");
    require(statics == reused, "static entry was freed");
    require(kfx_render_asset_generation == held, "static entry moved the generation");
    require(kfx_render_asset_stable(reused, 4096), "static entry dropped a live name");

    kfx_render_asset_range_forget(reused);
    free(reused);
    puts("asset name lifetime across LbDataFree passed");
    return 0;
}
