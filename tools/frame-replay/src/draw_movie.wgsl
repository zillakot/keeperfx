fn movie_sample(c: Command, pixel: vec2<u32>, view: vec3<u32>) -> u32 {
    let view_width = view.z;
    let mode = c.source.x;
    let double_width = select(1u, 2u, (mode & 1u) != 0u);
    let row_step = select(1u, 2u, (mode & 6u) != 0u);
    let copied_width = select(c.source.z, (c.source.z / 4u) * 4u, (mode & 3u) != 0u) * double_width;
    let origin = bitcast<i32>(c.accumulator.y) * i32(view_width) + bitcast<i32>(c.accumulator.x);
    let delta = i32(pixel.y * view_width + pixel.x) - origin;
    let stride = view_width * row_step;
    // Packed native vertical doubling addresses its second row in u32 units.
    for (var row_copy = 0u; row_copy < select(1u, 2u, (mode & 2u) != 0u); row_copy++) {
        let local = delta - i32(row_copy * ((view_width / 4u) * 4u));
        if local < 0 { continue; }
        let row = u32(local) / stride;
        let column = u32(local) % stride;
        if row < c.source.w && column < copied_width {
            return byte(c.assets.x + row * c.assets.z + column / double_width);
        }
    }
    return 256u;
}
