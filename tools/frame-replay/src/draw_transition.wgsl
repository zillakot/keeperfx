fn transition_sample(c: Command, pixel: vec2<u32>, view: vec3<u32>) -> u32 {
    if c.source.x == 1u {
        let offset = c.assets.x + pixel.y * c.assets.z + pixel.x;
        return byte(c.assets.y + (byte(offset) << 8u) + byte(offset + 1u));
    }
    let p = vec2<i32>(pixel) - view_bounds(c, view).xy;
    let w = i32(c.source.z);
    let h = i32(c.source.w);
    let step = i32(c.accumulator.z);
    let xb = vec2(4 * (32 - step), 4 * step);
    let yb = ((8 * h / w) * xb) / 8;
    let sx = clamp(xb + p.x * (vec2(w) - 2 * xb) / w, vec2(0), vec2(w));
    let sy = clamp(yb + p.y * (vec2(h) - 2 * yb) / h, vec2(0), vec2(h));
    let a = byte(c.assets.x + u32(sy.x) * c.assets.z + u32(sx.x));
    let b = byte(c.accumulator.x + u32(sy.y) * c.accumulator.y + u32(sx.y));
    let fa = byte(c.assets.y + u32(step) * 256u + a);
    let fb = byte(c.assets.y + u32(32 - step) * 256u + b);
    return byte(c.assets.y + 33u * 256u + 256u * fb + fa);
}
