#include "kfx/renderer/software/bflib_vidraw.c"
struct DisplayStruct lbDisplay;
unsigned char *render_ghost, *render_alpha;
void cursor_native(uint8_t* pixels, int pitch, int height, const struct TbSprite* sprite,
    int32_t* xs, int32_t* ys)
{
    const struct TbSourceBuffer source = {sprite->Data, sprite->SWidth, sprite->SHeight, sprite->SWidth};
    LbSpriteDrawUsingScalingUpDataSolidLR(pixels + xs[0] + pitch * ys[0], pitch, height, xs, ys, &source);
}
