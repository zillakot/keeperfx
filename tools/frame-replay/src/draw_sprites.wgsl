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

fn sprite_copy_forward(source: i32, destination: i32, count: i32, alignment: u32) {
    var at = 0i;
    while at < count && ((u32(destination + at) + alignment) & 3u) != 0u {
        pixels[pixel_address(u32(destination + at))] = pixels[pixel_address(u32(source + at))];
        at++;
    }
    while at + 4 <= count {
        let values = vec4<u32>(pixels[pixel_address(u32(source + at))], pixels[pixel_address(u32(source + at + 1))],
            pixels[pixel_address(u32(source + at + 2))], pixels[pixel_address(u32(source + at + 3))]);
        pixels[pixel_address(u32(destination + at))] = values.x;
        pixels[pixel_address(u32(destination + at + 1))] = values.y;
        pixels[pixel_address(u32(destination + at + 2))] = values.z;
        pixels[pixel_address(u32(destination + at + 3))] = values.w;
        at += 4;
    }
    while at < count {
        pixels[pixel_address(u32(destination + at))] = pixels[pixel_address(u32(source + at))];
        at++;
    }
}

@compute @workgroup_size(1)
fn sprite_ordered() {
    let c = commands[0];
    let w = c.source.z;
    let h = c.source.w;
    let axis = c.assets.x + 2u * w * h;
    let remap = axis + (w + h) * 8u;
    let stride = select(i32(parameters.x), -i32(parameters.x), (c.source.x & 2u) != 0u);
    for (var sy = 0u; sy < h; sy++) {
        let ay = select(sy, h - 1u - sy, stride < 0);
        let ystart = sprite_word(axis + (w + ay) * 8u);
        let ycount = sprite_word(axis + (w + ay) * 8u + 4u);
        if ycount == 0u { continue; }
        let y = select(ystart, ystart + ycount - 1u, stride < 0);
        var run_right = 0i;
        var in_run = false;
        for (var sx = 0u; sx < w; sx++) {
            let artwork = c.assets.x + 2u * (sy * w + sx);
            let coverage = assets[artwork + 1u];
            if coverage == 0u { continue; }
            let ax = w - 1u - sx;
            let xstart = sprite_word(axis + ax * 8u);
            let xcount = sprite_word(axis + ax * 8u + 4u);
            let right = i32(y * parameters.x + xstart + xcount) - 1;
            if !in_run { run_right = right; in_run = true; }
            let colour = select(assets[remap + assets[artwork]], c.operation.w, (c.source.x & 4u) != 0u);
            for (var dx = 0u; dx < xcount; dx++) {
                pixels[pixel_address(u32(right - i32(dx)))] = colour;
            }
            if coverage == 2u {
                let left = i32(y * parameters.x + xstart) - 1;
                for (var dy = 1u; dy < ycount; dy++) {
                    sprite_copy_forward(left, left + i32(dy) * stride,
                        run_right - left + 1, c.source.y);
                }
                in_run = false;
            }
        }
    }
}
