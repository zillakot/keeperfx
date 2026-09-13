fn sprite_word(offset: u32) -> u32 {
    return assets[offset] | (assets[offset + 1u] << 8u)
        | (assets[offset + 2u] << 16u) | (assets[offset + 3u] << 24u);
}

fn sprite_axis(offset: u32, count: u32, position: u32) -> u32 {
    var lo = 0u;
    var hi = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2u;
        let start = sprite_word(offset + mid * 8u);
        let length = sprite_word(offset + mid * 8u + 4u);
        if position >= start + length { lo = mid + 1u; }
        else { hi = mid; }
    }
    if lo == count { return count; }
    if position < sprite_word(offset + lo * 8u) { return count; }
    return lo;
}

fn sprite_sample(c: Command, pixel: vec2<u32>) -> u32 {
    let w = c.source.z;
    let h = c.source.w;
    let axis = c.assets.x + 2u * w * h;
    var x = sprite_axis(axis, w, pixel.x);
    var y = sprite_axis(axis + w * 8u, h, pixel.y);
    if x == w || y == h { return 256u; }
    if (c.source.x & 1u) != 0u { x = w - 1u - x; }
    if (c.source.x & 2u) != 0u { y = h - 1u - y; }
    let index = c.assets.x + 2u * (y * w + x);
    if assets[index + 1u] == 0u { return 256u; }
    if (c.source.x & 4u) != 0u { return c.operation.w; }
    return assets[axis + (w + h) * 8u + assets[index]];
}
