@group(0) @binding(0) var<storage, read> indices: array<u32>;
@group(0) @binding(1) var<storage, read> palette: array<u32>;
@group(0) @binding(2) var<uniform> parameters: vec4<u32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = ((2u * vec2<u32>(position.xy) + vec2<u32>(1)) * parameters.xy) / (2u * parameters.zw);
    let rgba = palette[indices[pixel.y * parameters.x + pixel.x]];
    return vec4<f32>(f32(rgba & 255u), f32((rgba >> 8u) & 255u),
        f32((rgba >> 16u) & 255u), f32(rgba >> 24u)) / 255.0;
}
