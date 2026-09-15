struct TrigVertex { xy: vec2<i32>, attr: vec3<i32>, }
fn trig_vertex(base: u32, n: u32) -> TrigVertex {
    let p = base + n * 20u;
    return TrigVertex(vec2(i32(sprite_word(p)), i32(sprite_word(p + 4u))),
        vec3(i32(sprite_word(p + 8u)), i32(sprite_word(p + 12u)), i32(sprite_word(p + 16u))));
}
fn trig_fixed(a: i32, b: i32) -> i32 {
    let hi = mul_high(u32(a), u32(b)) - select(0u, u32(b), a < 0) - select(0u, u32(a), b < 0);
    return i32((hi << 16u) | ((u32(a) * u32(b)) >> 16u));
}
fn trig_weight(r: i32, a: vec3<i32>) -> vec3<i32> {
    return vec3(trig_fixed(r, a.x), trig_fixed(r, a.y), trig_fixed(r, a.z));
}
fn trig_rol(value: i32) -> vec2<u32> {
    let sign = select(0u, 0xffffffffu, value < 0);
    return vec2((u32(value) << 16u) | (u32(value) >> 16u) | (sign << 16u),
        (u32(value) >> 16u) | (sign << 16u) | (sign >> 16u));
}
fn trig_add(a: vec2<u32>, b: vec2<u32>) -> vec3<u32> {
    let lo = a.x + b.x;
    let carry = u32(lo < a.x);
    let hi = a.y + b.y;
    let end = hi + carry;
    return vec3(lo, end, u32(hi < a.y || end < hi));
}
fn trig_shade(initial: i32, step: i32, count: u32, mode: u32) -> u32 {
    var state = trig_rol(initial);
    let increment = vec2(u32(step) << 16u, u32(step >> 16u));
    var shade = state.x & 255u;
    for (var i = 0u; i < count; i++) {
        if mode == 6u { state = vec2(state.x & 0xffff0000u, 0u); }
        let next = trig_add(state, increment);
        shade = (shade + u32(step >> 16u) + next.z) & 255u;
        state = vec2((next.x & 0xffffff00u) | shade, 0u);
    }
    return shade;
}
@group(0) @binding(6) var<storage, read> shadow_slots: array<u32>;
// A non-zero assets.w selects the resident mask slot (1-based) instead of the batch arena.
fn trig_texel(command: Command, uv: u32) -> u32 {
    if command.assets.w != 0u { return shadow_slots[(command.assets.w - 1u) * 65536u + uv]; }
    return byte(command.assets.x + 60u + uv);
}
fn trig_sample(command: Command, pixel: vec2<i32>, destination: u32) -> u32 {
    let p = trig_vertex(command.assets.x, 0u);
    let q = trig_vertex(command.assets.x, 1u);
    let r = trig_vertex(command.assets.x, 2u);
    var a = p; var b = q; var c = r;
    var kind = 0u;
    if p.xy.y == q.xy.y {
        if p.xy.y == r.xy.y { return destination; }
        if p.xy.y >= r.xy.y {
            if p.xy.x <= q.xy.x { return destination; }
            a = r; b = p; c = q; kind = 3u;
        } else {
            if q.xy.x <= p.xy.x { return destination; }
            kind = 4u;
        }
    } else if p.xy.y > q.xy.y {
        if p.xy.y == r.xy.y {
            if r.xy.x <= p.xy.x { return destination; }
            a = q; b = r; c = p; kind = 3u;
        } else if p.xy.y < r.xy.y { a = q; b = r; c = p; kind = 2u; }
        else if q.xy.y == r.xy.y {
            if r.xy.x <= q.xy.x { return destination; }
            a = q; b = r; c = p; kind = 4u;
        } else if q.xy.y < r.xy.y { a = q; b = r; c = p; kind = 1u; }
        else { a = r; b = p; c = q; kind = 2u; }
    } else {
        if p.xy.y == r.xy.y {
            if p.xy.x <= r.xy.x { return destination; }
            a = r; b = p; c = q; kind = 4u;
        } else if p.xy.y >= r.xy.y { a = r; b = p; c = q; kind = 1u; }
        else if q.xy.y == r.xy.y {
            if q.xy.x <= r.xy.x { return destination; }
            kind = 3u;
        } else if q.xy.y <= r.xy.y { kind = 1u; }
        else { kind = 2u; }
    }
    let mode = command.source.x;
    let flat = mode == 0u || mode == 14u || mode == 15u;
    let bottom = select(c.xy.y, b.xy.y, kind == 2u);
    if pixel.y < a.xy.y || pixel.y >= bottom { return destination; }
    let elapsed = pixel.y - a.xy.y;
    let ac_height = c.xy.y - a.xy.y;
    let ac = ((c.xy.x - a.xy.x) << 16u) / ac_height;
    var left = (a.xy.x << 16u) + ac * elapsed;
    var right = 0;
    var value = a.attr + ((c.attr - a.attr) / vec3(ac_height)) * elapsed;
    var step = vec3(0);
    if kind == 1u || kind == 2u {
        let ab_height = b.xy.y - a.xy.y;
        let ab = ((b.xy.x - a.xy.x) << 16u) / ab_height;
        if ab <= ac { return destination; }
        right = (a.xy.x << 16u) + ab * elapsed;
        if kind == 1u {
            if pixel.y >= b.xy.y {
                let bc = ((c.xy.x - b.xy.x) << 16u) / (c.xy.y - b.xy.y);
                right = (b.xy.x << 16u) + bc * (pixel.y - b.xy.y);
            }
            if !flat {
                let ratio = (ab_height << 16u) / ac_height;
                let extent = trig_fixed(ratio, a.xy.x - c.xy.x) + b.xy.x - a.xy.x;
                if extent < 0 { return destination; }
                if extent != 0 {
                    step = (b.attr + trig_weight(ratio, a.attr - c.attr) - a.attr) / vec3(extent + 1);
                }
            }
        } else {
            if pixel.y >= c.xy.y {
                let bc = ((b.xy.x - c.xy.x) << 16u) / (b.xy.y - c.xy.y);
                left = (c.xy.x << 16u) + bc * (pixel.y - c.xy.y);
                value = a.attr + ((c.attr - a.attr) / vec3(ac_height)) * ac_height
                    + ((b.attr - c.attr) / vec3(b.xy.y - c.xy.y)) * (pixel.y - c.xy.y);
            }
            if !flat {
                let ratio = (ac_height << 16u) / ab_height;
                let extent = trig_fixed(ratio, b.xy.x - a.xy.x) + a.xy.x - c.xy.x;
                if extent < 0 { return destination; }
                if extent != 0 {
                    step = (a.attr + trig_weight(ratio, b.attr - a.attr) - c.attr) / vec3(extent + 1);
                } else if mode == 5u || mode == 6u || mode == 20u || mode == 21u || mode >= 24u {
                    step.z = trig_fixed(ratio, b.xy.x - a.xy.x);
                }
            }
        }
    } else if kind == 3u {
        let ab = ((b.xy.x - a.xy.x) << 16u) / ac_height;
        right = (a.xy.x << 16u) + ab * elapsed;
        if !flat { step = (b.attr - c.attr) / vec3(b.xy.x - c.xy.x); }
    } else {
        let bc = ((c.xy.x - b.xy.x) << 16u) / ac_height;
        right = (b.xy.x << 16u) + bc * elapsed;
        if !flat { step = (b.attr - a.attr) / vec3(b.xy.x - a.xy.x); }
    }
    left >>= 16u; right >>= 16u;
    if pixel.x < left || pixel.x >= right { return destination; }
    let row_value = value;
    value += step * (pixel.x - left);
    let colour = command.operation.w;
    let fade = command.assets.y;
    let ghost = fade + 16384u;
    if mode == 0u { return colour; }
    if mode == 1u { return (u32(value.z) >> 16u) & 255u; }
    if mode == 14u { return byte(ghost + (colour << 8u) + destination); }
    if mode == 15u { return byte(ghost + (destination << 8u) + colour); }
    if mode == 4u || mode == 16u || mode == 17u {
        let shade = (u32(value.z) >> 16u) & select(255u, 31u, mode == 16u);
        if shade >= 64u { return 257u; }
        let source = byte(fade + (shade << 8u) + colour);
        if mode == 16u { return byte(ghost + (source << 8u) + destination); }
        if mode == 17u { return byte(ghost + (destination << 8u) + source); }
        return source;
    }
    let clipped = row_value + step * max(-left, 0);
    let count = u32(pixel.x - max(left, 0));
    if mode == 5u || mode == 26u {
        let u = (u32(clipped.x >> 16u) + count * u32(step.x >> 16u)) & 255u;
        let v = (u32(clipped.y >> 16u) + count * u32(step.y >> 16u)) & select(31u, 255u, mode == 26u);
        let uv = (v << 8u) | u;
        if uv >= command.source.y { return 257u; }
        let source = trig_texel(command, uv);
        let shade = ((u32(clipped.z >> 8u) + count * (u32(step.z >> 8u) & 65535u)) >> 8u) & 255u;
        if shade >= 64u { return 257u; }
        let shaded = byte(fade + (shade << 8u) + source);
        if mode == 26u && source <= 12u { return byte(ghost + (destination << 8u) + shaded); }
        return shaded;
    }
    let full_v = mode == 2u || mode == 3u || mode == 10u || ((mode == 7u || mode == 11u) && colour == 32u);
    // Native LP64 ROL4 retains sign bits; CFADDL carries at 64 bits, not 32.
    let clipped_v = row_value.y + step.y * max(-left, 0);
    let fraction = select(u32(clipped_v) & 65535u, 65535u, clipped_v < 0);
    var v = (u32(clipped_v) >> 16u) + count * u32(step.y >> 16u);
    if step.y < 0 && step.y > -65536 {
        v += (fraction + count * (u32(step.y) & 65535u)) >> 16u;
    }
    let uv = ((v & select(31u, 255u, full_v)) << 8u) | ((u32(value.x) >> 16u) & 255u);
    if uv >= command.source.y { return 257u; }
    let source = trig_texel(command, uv);
    if mode == 9u {
        if source == 0u { return destination; }
        if source >= 64u { return 257u; }
        return byte(fade + (source << 8u) + destination);
    }
    if mode == 6u || mode == 20u || mode == 21u || mode == 24u || mode == 25u {
        if (mode == 6u || mode == 24u || mode == 25u) && source == 0u { return destination; }
        let shade = trig_shade(clipped.z, step.z, count, mode);
        if shade >= 64u { return 257u; }
        let shaded = byte(fade + (shade << 8u) + source);
        if mode == 6u { return shaded; }
        if mode == 20u || mode == 24u { return byte(ghost + (shaded << 8u) + destination); }
        return byte(ghost + (destination << 8u) + shaded);
    }
    if mode == 2u || ((mode == 7u || mode == 11u) && colour == 32u) { return source; }
    if mode == 10u {
        if source == 0u { return destination; }
        return byte(fade + (colour << 8u) + destination);
    }
    if mode == 3u { return select(source, destination, source == 0u); }
    if mode == 7u || mode == 8u || mode == 11u {
        if mode == 8u && source == 0u { return destination; }
        return byte(fade + (colour << 8u) + source);
    }
    if mode == 12u { return byte(ghost + (source << 8u) + colour); }
    if mode == 13u { return byte(ghost + (colour << 8u) + source); }
    if (mode == 22u || mode == 23u) && source == 0u { return destination; }
    if mode == 18u || mode == 22u { return byte(ghost + (source << 8u) + destination); }
    return byte(ghost + (destination << 8u) + source);
}

