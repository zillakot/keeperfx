struct Span { bounds: vec4<u32>, accumulator: vec4<u32> }
struct Parameters { width: u32, height: u32, count: u32, padding: u32, pitch: u32, offset: u32, pad0: u32, pad1: u32 }
@group(0) @binding(0) var<storage, read> spans: array<Span>;
@group(0) @binding(1) var<storage, read> assets: array<u32>;
@group(0) @binding(2) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(3) var<uniform> parameters: Parameters;
@group(0) @binding(4) var<storage, read> resources: array<vec2<u32>>;
@group(0) @binding(5) var<storage, read_write> invalid: atomic<u32>;
@group(0) @binding(6) var<storage, read_write> status: array<atomic<u32>, 8>;
const STATUS_FRAME: u32 = 0u;
const STATUS_TERRAIN_SHADE: u32 = 2u;
fn raise(cause: u32) { atomicStore(&status[STATUS_FRAME], 1u); atomicStore(&status[cause], 1u); }
fn high_product(a: u32, b: u32) -> u32 {
    let a0 = a & 65535u;
    let a1 = a >> 16u;
    let b0 = b & 65535u;
    let b1 = b >> 16u;
    let t = a1 * b0 + ((a0 * b0) >> 16u);
    let u = a0 * b1 + (t & 65535u);
    return a1 * b1 + (t >> 16u) + (u >> 16u);
}
@compute @workgroup_size(64)
fn validate(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= parameters.height || gid.y >= parameters.count { return; }
    let row = spans[gid.y * parameters.height + gid.x];
    var low = row.accumulator.x;
    for (var x = 0u; x < row.bounds.z; x++) {
        if (low & 0xff00u) >= 16384u { atomicStore(&invalid, 1u); return; }
        low += row.accumulator.z;
    }
}
@compute @workgroup_size(8, 8)
fn render(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= parameters.width || gid.y >= parameters.height { return; }
    let pixel = parameters.offset + gid.y * parameters.pitch + gid.x;
    var color = pixels[pixel];
    for (var triangle = 0u; triangle < parameters.count; triangle++) {
        let row = spans[triangle * parameters.height + gid.y];
        if gid.x < row.bounds.x || gid.x - row.bounds.x >= row.bounds.z { continue; }
        let offset = gid.x - row.bounds.x;
        let low = row.accumulator.x + row.accumulator.z * offset;
        if (low & 0xff00u) >= 16384u { raise(STATUS_TERRAIN_SHADE); continue; }
        let high = row.accumulator.y + row.accumulator.w * offset + high_product(row.accumulator.z, offset)
            + u32(low < row.accumulator.x);
        let uv = ((high << 8u) | (high >> 24u)) & 0x1f1fu;
        let offsets = resources[triangle];
        color = assets[offsets.y + (assets[offsets.x + uv] | (low & 0xff00u))];
    }
    pixels[pixel] = color;
}
