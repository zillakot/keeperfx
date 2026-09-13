@group(0) @binding(0) var<storage, read_write> indices: array<u32>;
@group(0) @binding(1) var<storage, read> data: array<u32>;
fn word(n: u32) -> u32 {
    let a = n * 4u;
    return data[a] | (data[a+1u] << 8u) | (data[a+2u] << 16u) | (data[a+3u] << 24u);
}
fn source(offset: u32) -> u32 {
    if word(6u) != 0u {
        let relative = bitcast<i32>(word(5u)) + i32(offset);
        if relative >= 0 {
            let y = u32(relative) / word(4u);
            let x = u32(relative) % word(4u);
            if y < word(2u) && x < word(1u) { return indices[y * word(1u) + x]; }
        }
    }
    return data[word(11u) + offset];
}
fn pixel(i: u32) {
    let x = i % word(1u);
    let y = i / word(1u);
    let asset = word(12u);
    var result: u32;
    if word(0u) == 0u {
        let a = asset + i * 4u;
        let sx = data[a] | (data[a+1u] << 8u);
        let sy = data[a+2u] | (data[a+3u] << 8u);
        result = source(sy * word(3u) + sx);
    } else if word(0u) == 1u {
        let vx = (x * word(7u)) >> 16u;
        let vy = (y * word(8u)) >> 16u;
        let phase = word(10u);
        let a = (((phase >> 8u) + vy) & 255u) * 256u + ((phase + vx) & 255u);
        let b = (((phase >> 24u) + 65536u - vx) & 255u) * 256u + (((phase >> 16u) + 65536u - vy) & 255u);
        let shade = min((data[asset+a] + data[asset+b]) >> 3u, 32u);
        result = data[word(13u) + shade * 256u + source(y * word(3u) + x)];
    } else {
        let ox = min((x * word(7u)) >> 16u, word(14u)-1u);
        let oy = min((y * word(8u)) >> 16u, word(15u)-1u);
        let overlay = data[asset + oy * word(14u) + ox];
        let input = source(y * word(3u) + x);
        result = input;
        if overlay != 255u { result = (overlay * word(9u) + input * (256u-word(9u))) >> 8u; }
    }
    indices[i] = result;
}
@compute @workgroup_size(8, 8)
fn effect(@builtin(global_invocation_id) id: vec3<u32>) {
    let count = word(1u) * word(2u);
    if word(6u) != 0u {
        if id.x != 0u || id.y != 0u { return; }
        for (var i=0u; i<count; i+=1u) { pixel(i); }
    } else if id.x < word(1u) && id.y < word(2u) { pixel(id.y * word(1u) + id.x); }
}
