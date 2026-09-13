struct Span { bounds: vec4<u32>, accumulator: vec4<u32> }
struct Parameters { width: u32, height: u32, count: u32, pitch: u32 }
@group(0) @binding(0) var<storage, read> spans: array<Span>;
@group(0) @binding(1) var<storage, read> assets: array<u32>;
@group(0) @binding(2) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(3) var<uniform> parameters: Parameters;
fn high_product(a: u32, b: u32) -> u32 {
    let a0 = a & 65535u;
    let a1 = a >> 16u;
    let b0 = b & 65535u;
    let b1 = b >> 16u;
    let t = a1 * b0 + ((a0 * b0) >> 16u);
    let u = a0 * b1 + (t & 65535u);
    return a1 * b1 + (t >> 16u) + (u >> 16u);
}
@compute @workgroup_size(8, 8)
fn render(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= parameters.pitch || gid.y >= parameters.height || gid.z >= parameters.count { return; }
    let row = spans[gid.z * parameters.height + gid.y];
    var color = 167u;
    if gid.x >= row.bounds.x && gid.x - row.bounds.x < row.bounds.z {
        let offset = gid.x - row.bounds.x;
        let delta = row.accumulator.z * offset;
        let low = row.accumulator.x + delta;
        let high = row.accumulator.y + row.accumulator.w * offset + high_product(row.accumulator.z, offset)
            + u32(low < row.accumulator.x);
        let uv = ((high << 8u) | (high >> 24u)) & 0x1f1fu;
        let shade = low & 0xff00u;
        color = assets[7968u + (assets[uv] | shade)];
    }
    pixels[(gid.z * parameters.height + gid.y) * parameters.pitch + gid.x] = color;
}
