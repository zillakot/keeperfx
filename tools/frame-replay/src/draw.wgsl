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
struct Span { bounds: vec4<u32>, accumulator: vec4<u32> }
@group(0) @binding(5) var<storage, read> rows: array<Span>;
// x/y are the target extent; w is the pass's tile columns; base/box bound its dispatch.
struct DrawParameters {
    x: u32, y: u32, box_x: u32, w: u32, pitch: u32, offset: u32,
    tile_base: u32, base_x: u32, base_y: u32, box_y: u32, pad0: u32, pad1: u32,
}
@group(0) @binding(3) var<uniform> parameters: DrawParameters;
@group(0) @binding(7) var<storage, read_write> status: array<atomic<u32>, 8>;
const STATUS_FRAME: u32 = 0u;
const STATUS_TRIG_LOOKUP: u32 = 1u;
const STATUS_TERRAIN_SHADE: u32 = 2u;
fn raise(cause: u32) { atomicStore(&status[STATUS_FRAME], 1u); atomicStore(&status[cause], 1u); }
fn pixel_address(i: u32) -> u32 { return parameters.offset + (i / parameters.x) * parameters.pitch + i % parameters.x; }
// The frame's views live at the head of the tile buffer, four words each: a record names
// one in operation.z. Only the sampled kinds read it, and they read it once per command.
fn view_of(c: Command) -> vec3<u32> {
    let base = c.operation.z * 4u;
    return vec3(tiles[base], tiles[base + 1u], tiles[base + 2u]);
}
// Bounds and clip are stored in the dispatch's space; samplers work in the view the
// command was issued against, which is that space shifted by the view's origin.
fn view_bounds(c: Command, view: vec3<u32>) -> vec4<i32> {
    let o = vec2<i32>(view.xy);
    return c.bounds - vec4(o, o);
}

fn mul_high(a: u32, b: u32) -> u32 {
    let a0 = a & 65535u;
    let a1 = a >> 16u;
    let b0 = b & 65535u;
    let b1 = b >> 16u;
    let t = a1 * b0 + ((a0 * b0) >> 16u);
    let u = a0 * b1 + (t & 65535u);
    return a1 * b1 + (t >> 16u) + (u >> 16u);
}

// One terrain triangle's row, addressed by the record's arena base and the offset of
// this pixel inside the record's conservative box. 256 means the row does not cover
// this pixel; 257 means the shade left the fade table.
fn terrain_sample(c: Command, viewed: vec2<u32>, pixel: vec2<i32>) -> u32 {
    let row = rows[c.assets.w + u32(pixel.y - c.bounds.y)];
    if viewed.x < row.bounds.x || viewed.x - row.bounds.x >= row.bounds.z { return 256u; }
    let offset = viewed.x - row.bounds.x;
    let low = row.accumulator.x + row.accumulator.z * offset;
    if (low & 0xff00u) >= 16384u { return 257u; }
    let high = row.accumulator.y + row.accumulator.w * offset
        + mul_high(row.accumulator.z, offset) + u32(low < row.accumulator.x);
    let uv = ((high << 8u) | (high >> 24u)) & 0x1f1fu;
    return byte(c.assets.y + (byte(c.assets.x + uv) | (low & 0xff00u)));
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
    // A pass covers only the box its own records touch; base_x/base_y place it in the target.
    let at = id.xy + vec2(parameters.base_x, parameters.base_y);
    if at.x >= parameters.box_x || at.y >= parameters.box_y { return; }
    let pixel = vec2<i32>(at);
    let index = at.y * parameters.x + at.x;
    // One index per frame: tile_base is this pass's header, covering only the tiles its
    // own records reach, and w its tile columns.
    let cell = (at.y / 16u - parameters.base_y / 16u) * parameters.w
        + (at.x / 16u - parameters.base_x / 16u);
    let tile = parameters.tile_base + cell * 2u;
    // A tile no record in this pass reaches keeps its pixels, so it costs no traffic.
    if tiles[tile + 1u] == 0u { return; }
    var destination = pixels[pixel_address(index)];
    let end = tiles[tile] + tiles[tile + 1u];
    for (var i = tiles[tile]; i < end; i++) {
        let c = commands[tiles[i]];
        if any(pixel < c.clip.xy) || any(pixel >= c.clip.zw) { continue; }
        if any(pixel < c.bounds.xy) || any(pixel >= c.bounds.zw) { continue; }
        var source = c.operation.w;
        let local = vec2<u32>(pixel - c.bounds.xy);
        if c.operation.x == 2u {
            let size = vec2<u32>(c.bounds.zw - c.bounds.xy);
            let sample = c.source.xy + (local * c.source.zw) / size;
            source = byte(c.assets.x + sample.y * c.assets.z + sample.x);
        }
        if c.operation.x == 3u {
            let low_product = c.accumulator.z * local.x;
            let low = c.accumulator.x + low_product;
            let high = c.accumulator.y + c.accumulator.w * local.x
                + mul_high(c.accumulator.z, local.x) + select(0u, 1u, low < c.accumulator.x);
            let uv = ((high << 8u) | (high >> 24u)) & 0x1f1fu;
            let shade = low & 0xff00u;
            source = byte(c.assets.y + shade + byte(c.assets.x + uv));
        }
        // Kinds below SPRITE never leave the dispatch's space, so they never read the view.
        if c.operation.x >= 6u {
            let view = view_of(c);
            let local_pixel = pixel - vec2<i32>(view.xy);
            let viewed = vec2<u32>(local_pixel);
            if c.operation.x == 9u {
                let sampled = trig_sample(c, local_pixel, destination);
                if sampled == 257u { raise(STATUS_TRIG_LOOKUP); continue; }
                destination = sampled;
                continue;
            }
            if c.operation.x == 17u {
                let sampled = terrain_sample(c, viewed, pixel);
                if sampled == 256u { continue; }
                if sampled == 257u { raise(STATUS_TERRAIN_SHADE); continue; }
                destination = sampled;
                continue;
            }
            if c.operation.x == 16u { source = transition_sample(c, viewed, view); }
            if c.operation.x == 15u { source = bitmap_sample(c, viewed); }
            if c.operation.x == 14u { source = map_view_sample(c, viewed, destination, view); }
            if c.operation.x == 13u { source = movie_sample(c, viewed, view); }
            if c.operation.x == 6u { source = sprite_sample(c, viewed); }
            if c.operation.x == 7u || c.operation.x == 8u { source = raw_sample(c, viewed, view); }
        }
        if source == c.options.x { continue; }
        var hits = 1u;
        if c.operation.x == 4u || c.operation.x == 5u {
            let radius = i32(c.source.z);
            hits = circle_hits(vec2<i32>(local) - vec2(radius), radius, c.operation.x == 5u);
        }
        for (var hit = 0u; hit < hits; hit++) {
        if c.operation.y == 1u {
            destination = byte(c.assets.y + (source << 8u) + destination);
        } else if c.operation.y == 2u {
            destination = byte(c.assets.y + (destination << 8u) + source);
        } else {
            destination = source;
        }
        }
    }
    pixels[pixel_address(index)] = destination;
}
