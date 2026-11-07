fn source_at(at: vec2<i32>) -> vec4<f32> {
    var bits = textureLoad(source, at, 0, 0);
    if p.opts.z == 255.0 { return vec4<f32>(bits) / 255.0; }
    bits |= textureLoad(source, at, 1, 0) << vec4<u32>(8u);
    if p.opts.z == 65535.0 { return vec4<f32>(bits) / 65535.0; }
    bits |= textureLoad(source, at, 2, 0) << vec4<u32>(16u);
    bits |= textureLoad(source, at, 3, 0) << vec4<u32>(24u);
    return bitcast<vec4<f32>>(bits);
}
@fragment fn fs_native_move(v: Vertex) -> @location(0) vec4<f32> {
    let at = vec2<i32>(v.position.xy) * i32(p.h0.w);
    let src = at - vec2<i32>(p.h1.xy);
    var value = Sample(0.0);
    if all(src >= vec2<i32>(p.rect.xy)) && all(src < vec2<i32>(p.rect.zw)) {
        let rgba = source_at(src);
        value[0] = rgba[color_index(0u)];
        if COMPONENTS != 1u { value[1] = rgba.g; value[COMPONENTS-1u] = rgba.b; }
        value[COMPONENTS] = 1.0;
    }
    return finish(at, value);
}
