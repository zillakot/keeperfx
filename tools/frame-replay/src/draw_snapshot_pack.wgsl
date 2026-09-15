@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read_write> destination: array<u32>;
@group(0) @binding(2) var<uniform> parameters: vec4<u32>;
@compute @workgroup_size(64)
fn pack_snapshot(@builtin(global_invocation_id) id: vec3<u32>) {
    let word = id.y * parameters.z + id.x;
    let first = word * 4u;
    if first >= parameters.x { return; }
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane++) {
        if first + lane < parameters.x {
            packed |= (source[first + lane] & 255u) << (lane * 8u);
        }
    }
    destination[parameters.y + word] = packed;
}
