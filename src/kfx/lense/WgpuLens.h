#pragma once
#include <cstdint>

struct KfxLensLookup { int16_t x, y; };
void KfxLensRemap(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const void* lookup);
void KfxLensMist(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const uint8_t* texture, const uint8_t* fade,
    uint8_t pos_x, uint8_t pos_y, uint8_t sec_x, uint8_t sec_y, unsigned fade_rows = 33);
void KfxLensOverlay(uint8_t* dst, long dstpitch, const uint8_t* src, long srcpitch,
    long width, long height, const uint8_t* overlay, int overlay_width, int overlay_height, short alpha);
