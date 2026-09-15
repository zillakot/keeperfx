#pragma once
#include <cstdint>

struct KfxLensLookup { int16_t x, y; };
/* Names a lens table so the drawing context keeps it resident instead of taking a copy
 * per frame. A zero id has no name and leaves the table in the per-call source; the
 * generation must move whenever the bytes behind a live id are rewritten. */
struct KfxLensIdentity { uint64_t id, generation; };
/* Built-in table owners, the high half of a lens identity. */
enum KfxLensOwner {
    KFX_LENS_DISPLACEMENT = 1,
    KFX_LENS_FLYEYE = 2,
    KFX_LENS_MIST = 3,
    KFX_LENS_OVERLAY = 4
};
void KfxLensRemap(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const void* lookup, struct KfxLensIdentity name = {0, 0});
void KfxLensMist(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const uint8_t* texture, const uint8_t* fade,
    uint8_t pos_x, uint8_t pos_y, uint8_t sec_x, uint8_t sec_y, unsigned fade_rows = 33,
    struct KfxLensIdentity name = {0, 0});
void KfxLensOverlay(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const uint8_t* overlay, int overlay_width, int overlay_height,
    short alpha, struct KfxLensIdentity name = {0, 0});
/* Identity generation for lens tables, separate so that rebuilding one lookup does not
 * drop terrain textures. Bumped whenever a built-in lens table is built or released. */
uint64_t KfxLensGeneration();
void KfxLensTablesChanged();
