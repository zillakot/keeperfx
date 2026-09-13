#pragma once
#include "bflib_vidraw.h"
#include "bflib_sprite.h"

int kfx_wgpu_sprite(long posx, long posy, const struct TbSourceBuffer *source,
    const struct TbSprite *sprite, const TbPixel *remap, TbPixel colour, unsigned mode);
