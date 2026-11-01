@group(0) @binding(0) var source_map: texture_2d<f32>;
struct Params {
    origin: vec2<u32>,
    linear_srgb: u32,
    _pad: u32,
    source_origin: vec2<u32>,
};
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var p = vec2<f32>(-1.0, -1.0);
    if index == 1u { p = vec2<f32>(3.0, -1.0); }
    if index == 2u { p = vec2<f32>(-1.0, 3.0); }
    return vec4<f32>(p, 0.0, 1.0);
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { return v / 12.92; }
    return pow((v + 0.055) / 1.055, 2.4);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let local = vec2<i32>(pos.xy) - vec2<i32>(params.origin) + vec2<i32>(params.source_origin);
    var pixel = textureLoad(source_map, local, 0);
    if params.linear_srgb != 0u {
        pixel = vec4<f32>(srgb_to_linear(pixel.r), srgb_to_linear(pixel.g), srgb_to_linear(pixel.b), pixel.a);
    }
    return pixel;
}
