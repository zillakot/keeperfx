struct Vertex {
    xy: vec2<i32>,
    u: vec2<u32>,
    v: vec2<u32>,
    s: vec2<u32>,
}
struct Triangle { a: Vertex, b: Vertex, c: Vertex }
struct Span { bounds: vec4<u32>, accumulator: vec4<u32> }
struct Parameters { count: u32, pad0: u32, pad1: u32, pad2: u32 }
struct Layout { base: u32, y_lo: u32, rows: u32, width: u32, height: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(0) @binding(1) var<storage, read_write> spans: array<Span>;
@group(0) @binding(2) var<uniform> parameters: Parameters;
@group(0) @binding(3) var<storage, read> extents: array<Layout>;

fn add96(a: vec3<u32>, b: vec3<u32>) -> vec3<u32> {
    let lo = a.x + b.x;
    let mid0 = a.y + b.y;
    let mid = mid0 + u32(lo < a.x);
    return vec3(lo, mid, a.z + b.z + u32(mid0 < a.y || mid < mid0));
}
fn neg96(a: vec3<u32>) -> vec3<u32> { return add96(~a, vec3(1u, 0u, 0u)); }
fn scale96(a: vec3<u32>, n: i32) -> vec3<u32> {
    var factor = bitcast<u32>(n);
    var value = a;
    if n < 0 { factor = 0u - factor; value = neg96(value); }
    var result = vec3(0u);
    loop {
        if factor == 0u { break; }
        if (factor & 1u) != 0u { result = add96(result, value); }
        value = add96(value, value);
        factor >>= 1u;
    }
    return result;
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
fn mul_shift(a: i32, b: i32) -> i32 {
    let ua = bitcast<u32>(a);
    let ub = bitcast<u32>(b);
    let lo = ua * ub;
    let hi = mul_high(ua, ub) - select(0u, ub, a < 0) - select(0u, ua, b < 0);
    return bitcast<i32>((lo >> 15u) | (hi << 17u)) - (bitcast<i32>(hi) >> 31u);
}
fn pack(u: i32, v: i32, s: i32) -> vec3<u32> {
    return vec3(bitcast<u32>(s) << 24u,
        (bitcast<u32>(v) << 16u) | ((bitcast<u32>(s) >> 8u) & 65535u),
        (bitcast<u32>(u) << 8u) | ((bitcast<u32>(v) >> 16u) & 255u));
}
fn pack_signed(d: vec3<i32>) -> vec3<u32> {
    let v = d.y + (d.z >> 31u);
    return pack(d.x + (v >> 31u), v, d.z);
}
fn attributes(v: Vertex) -> vec3<i32> {
    return bitcast<vec3<i32>>(vec3((v.u.x >> 16u) | (v.u.y << 16u),
        (v.v.x >> 16u) | (v.v.y << 16u), (v.s.x >> 16u) | (v.s.y << 16u)));
}
fn start(v: Vertex) -> vec3<u32> {
    let a = attributes(v) << vec3(16u);
    return pack(a.x, a.y, a.z);
}
fn reciprocal(n: i32) -> i32 { return select(0, 2147483647 / max(n, 1), n != 0); }
fn slope(dx: i32, dy: i32) -> i32 {
    if dy == 0 { return select(select(0, 8388607, dx > 0), -8388607, dx < 0); }
    return (dx << 16u) / dy;
}
fn mapping(d: vec3<i32>, factor: i32) -> vec3<i32> {
    return vec3(mul_shift(d.x, factor), mul_shift(d.y, factor), mul_shift(d.z, factor));
}

@compute @workgroup_size(64)
fn prepare(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if index >= parameters.count { return; }
    let base = extents[index].base;
    let y_lo = i32(extents[index].y_lo);
    let width = extents[index].width;
    let height = extents[index].height;
    for (var row = 0u; row < extents[index].rows; row++) {
        spans[base + row] = Span(vec4(0u), vec4(0u));
    }
    var a = triangles[index].a;
    var b = triangles[index].b;
    var c = triangles[index].c;
    if a.xy.y > b.xy.y { let t = a; a = b; b = t; }
    if a.xy.y > c.xy.y { let t = a; a = c; c = t; }
    if b.xy.y > c.xy.y { let t = b; b = c; c = t; }
    let ab = b.xy - a.xy;
    let ac = c.xy - a.xy;
    let bc = c.xy - b.xy;
    if ac.y == 0 || any(ab < vec2(-16384)) || any(ab > vec2(16383)) ||
        any(ac < vec2(-16384)) || any(ac > vec2(16383)) ||
        any(bc < vec2(-16384)) || any(bc > vec2(16383)) { return; }
    let sab = slope(ab.x, ab.y);
    let sac = slope(ac.x, ac.y);
    let sbc = slope(bc.x, bc.y);
    let left = ab.y * sac > (ab.x << 16u);
    let ad = attributes(a);
    let bd = attributes(b);
    let cd = attributes(c);
    let abd = bd - ad;
    let acd = cd - ad;
    let cross = ab.y * ac.x - ac.y * (ab.x + select(1, -1, left));
    var dx = vec3(0);
    if cross != 0 { dx = mapping(ab.y * acd - ac.y * abd, 2147483647 / cross); }
    let exact_dx = pack_signed(dx);
    let rounded_dx = pack_signed(vec3(dx.xy, dx.z - ((dx.z >> 31u) << 8u)));
    var dy = pack_signed(mapping(select(acd, abd, left), reciprocal(select(ac.y, ab.y, left))));
    var sl = select(sac, sab, left);
    var sr = select(sab, sac, left);
    var xl = a.xy.x << 16u;
    var xr = xl;
    var x = a.xy.x;
    var y = a.xy.y;
    var coord = start(a);
    let clipped = a.xy.x < 0 || a.xy.x > i32(width) ||
        b.xy.x < 0 || b.xy.x > i32(width) || c.xy.x < 0 || c.xy.x > i32(width);
    for (var half = 0u; half < 2u; half++) {
        let end = min(select(b.xy.y, c.xy.y, half == 1u), i32(height));
        loop {
            if y >= end { break; }
            if y >= 0 {
                let l = select(xl >> 16u, max(xl >> 16u, 0), clipped);
                let r = select(xr >> 16u, min(xr >> 16u, i32(width)), clipped);
                if clipped { coord = add96(coord, scale96(exact_dx, l - x)); x = l; }
                if r > l {
                    spans[base + u32(y - y_lo)] = Span(vec4(u32(l), u32(y), u32(r - l), 0u),
                        vec4(coord.yz, rounded_dx.yz));
                }
            }
            coord = add96(coord, dy);
            x -= xl >> 16u;
            xl += sl;
            xr += sr;
            x += xl >> 16u;
            y++;
        }
        if left {
            sl = sbc;
            xl = b.xy.x << 16u;
            x = b.xy.x;
            dy = pack_signed(mapping(cd - bd, reciprocal(bc.y)));
            coord = start(b);
        } else { sr = sbc; xr = b.xy.x << 16u; }
        y = b.xy.y;
    }
}
