fn raw_sample(c: Command, pixel: vec2<u32>) -> u32 {
    if c.operation.x == 8u {
        let sample = vec2<u32>(vec2<i32>(pixel) - view_bounds(c).xy) % c.source.zw;
        return assets[c.assets.x + sample.y * c.assets.z + sample.x];
    }
    let local = vec2<i32>(pixel) - bitcast<vec2<i32>>(c.accumulator.xy);
    let size = c.accumulator.zw;
    if any(local < vec2(0)) || any(vec2<u32>(local) >= size) { return 0u; }
    let sample = ((vec2<u32>(local) + vec2(1u)) * c.source.zw - vec2(1u)) / size;
    return assets[c.assets.x + sample.y * c.assets.z + sample.x];
}
