struct Span { bounds: vec4<u32>, accumulator: vec4<u32> }
struct Layout { base: u32, y_lo: u32, rows: u32, width: u32, height: u32 }
struct Parameters { count: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<storage, read> spans: array<Span>;
@group(0) @binding(3) var<uniform> parameters: Parameters;
@group(0) @binding(5) var<storage, read> extents: array<Layout>;
@group(0) @binding(6) var<storage, read_write> status: array<atomic<u32>, 8>;
const STATUS_FRAME: u32 = 0u;
const STATUS_TERRAIN_SPAN: u32 = 3u;
fn raise(cause: u32) { atomicStore(&status[STATUS_FRAME], 1u); atomicStore(&status[cause], 1u); }
@compute @workgroup_size(64)
fn validate(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.y >= parameters.count || gid.x >= extents[gid.y].rows { return; }
    let row = spans[extents[gid.y].base + gid.x];
    var low = row.accumulator.x;
    for (var x = 0u; x < row.bounds.z; x++) {
        if (low & 0xff00u) >= 16384u { raise(STATUS_TERRAIN_SPAN); return; }
        low += row.accumulator.z;
    }
}
