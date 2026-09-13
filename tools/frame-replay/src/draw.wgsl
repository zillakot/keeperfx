struct Command {
    operation: vec4<u32>,
    bounds: vec4<i32>,
    clip: vec4<i32>,
    assets: vec4<u32>,
    source: vec4<u32>,
    accumulator: vec4<u32>,
    options: vec4<u32>,
}
@group(0) @binding(0) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(1) var<storage, read> commands: array<Command>;
@group(0) @binding(2) var<storage, read> assets: array<u32>;
@group(0) @binding(4) var<storage, read> tiles: array<u32>;
@group(0) @binding(3) var<uniform> parameters: vec4<u32>;

fn mul_high(a: u32, b: u32) -> u32 {
    let a0 = a & 65535u;
    let a1 = a >> 16u;
    let b0 = b & 65535u;
    let b1 = b >> 16u;
    let t = a1 * b0 + ((a0 * b0) >> 16u);
    let u = a0 * b1 + (t & 65535u);
    return a1 * b1 + (t >> 16u) + (u >> 16u);
}

fn circle_octants(p: vec2<i32>, a: i32, b: i32) -> u32 {
    return u32(all(p == vec2(-a, -b))) + u32(all(p == vec2(a, -b)))
        + u32(all(p == vec2(-a, b))) + u32(all(p == vec2(a, b)))
        + u32(all(p == vec2(-b, -a))) + u32(all(p == vec2(b, -a)))
        + u32(all(p == vec2(-b, a))) + u32(all(p == vec2(b, a)));
}

fn circle_rows(p: vec2<i32>, extent: i32, row: i32) -> u32 {
    return u32(abs(p.x) <= extent) * (u32(p.y == -row) + u32(p.y == row));
}

fn circle_hits(p: vec2<i32>, radius: i32, outline: bool) -> u32 {
    if radius == 0 { return u32(all(p == vec2(0))); }
    var r = radius;
    var n = 3 - 2 * radius;
    var hits = 0u;
    if outline {
        var a = 0;
        while a < r {
            hits += circle_octants(p, a, r);
            if n >= 0 { n += 10 + 4 * (a - r); r--; }
            else { n += 6 + 4 * a; }
            a++;
        }
        if r == a { hits += circle_octants(p, a, r); }
        return hits;
    }
    if radius == 1 { return u32(abs(p.x) + abs(p.y) <= 1); }
    hits = u32(p.y == 0 && abs(p.x) <= radius);
    if n >= 0 {
        hits += circle_rows(p, 0, radius);
        r--;
        n += 10 - (4 * (radius - 1) + 4);
    } else { n += 6; }
    var dx = 1;
    while dx < r {
        hits += circle_rows(p, r, dx);
        if n >= 0 {
            hits += circle_rows(p, dx, r);
            let delta = dx - r;
            r--;
            n += 4 * delta + 10;
        } else { n += 4 * dx + 6; }
        dx++;
    }
    if r == dx { hits += circle_rows(p, r, dx); }
    return hits;
}

@compute @workgroup_size(8, 8)
fn draw(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= parameters.x || id.y >= parameters.y { return; }
    let pixel = vec2<i32>(id.xy);
    let index = id.y * parameters.x + id.x;
    let tile = ((id.y / 16u) * parameters.w + id.x / 16u) * 2u;
    var destination = pixels[index];
    for (var i = 0u; i < tiles[tile + 1u]; i++) {
        let c = commands[tiles[tiles[tile] + i]];
        if any(pixel < c.clip.xy) || any(pixel >= c.clip.zw) { continue; }
        if any(pixel < c.bounds.xy) || any(pixel >= c.bounds.zw) { continue; }
        var source = c.operation.w;
        let local = vec2<u32>(pixel - c.bounds.xy);
        if c.operation.x == 2u {
            let size = vec2<u32>(c.bounds.zw - c.bounds.xy);
            let sample = c.source.xy + (local * c.source.zw) / size;
            source = assets[c.assets.x + sample.y * c.assets.z + sample.x];
        }
        if c.operation.x == 3u {
            let low_product = c.accumulator.z * local.x;
            let low = c.accumulator.x + low_product;
            let high = c.accumulator.y + c.accumulator.w * local.x
                + mul_high(c.accumulator.z, local.x) + select(0u, 1u, low < c.accumulator.x);
            let uv = ((high << 8u) | (high >> 24u)) & 0x1f1fu;
            let shade = low & 0xff00u;
            source = assets[c.assets.y + shade + assets[c.assets.x + uv]];
        }
        if c.operation.x == 6u { source = sprite_sample(c, id.xy); }
        if source == c.options.x { continue; }
        var hits = 1u;
        if c.operation.x == 4u || c.operation.x == 5u {
            let radius = i32(c.source.z);
            hits = circle_hits(vec2<i32>(local) - vec2(radius), radius, c.operation.x == 5u);
        }
        for (var hit = 0u; hit < hits; hit++) {
        if c.operation.y == 1u {
            destination = assets[c.assets.y + (source << 8u) + destination];
        } else if c.operation.y == 2u {
            destination = assets[c.assets.y + (destination << 8u) + source];
        } else {
            destination = source;
        }
        }
    }
    pixels[index] = destination;
}
