@group(0) @binding(0) var indices: texture_2d<u32>;
@group(0) @binding(1) var palette: texture_2d<u32>;
@group(0) @binding(2) var<uniform> parameters: vec4<u32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = min(vec2<u32>(position.xy * vec2<f32>(parameters.xy) / vec2<f32>(parameters.zw)), parameters.xy - vec2<u32>(1));
    let index = textureLoad(indices, vec2<i32>(pixel), 0).r;
    return vec4<f32>(textureLoad(palette, vec2<i32>(i32(index), 0), 0)) / 255.0;
}
