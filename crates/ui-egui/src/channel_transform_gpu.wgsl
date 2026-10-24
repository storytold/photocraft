// Native samples stay immutable. A transformed scalar is merged into just one component.
struct Params {
    h0: vec4<f32>, h1: vec4<f32>, h2: vec4<f32>,
    size: vec4<f32>, rect: vec4<f32>, view: vec4<f32>, opts: vec4<f32>, bounds: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var selection: texture_2d<f32>;
@group(1) @binding(0) var moved: texture_2d<f32>;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) src: vec2<f32> }
@vertex fn vs_full(@builtin(vertex_index) i: u32) -> Vertex {
    let uv = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Vertex(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv * p.size.xy);
}
@vertex fn vs_mesh(@location(0) data: vec4<f32>) -> Vertex {
    return Vertex(vec4(data.xy / p.size.xy * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), data.zw);
}
fn quant(x: f32) -> f32 {
    if p.opts.z == 0.0 { return x; }
    return floor(clamp(x, 0.0, 1.0) * p.opts.z + 0.5) / p.opts.z;
}
fn lifted(pos: vec2<i32>, repeat_edge: bool) -> vec2<f32> {
    var at = pos;
    if repeat_edge { at = clamp(at, vec2<i32>(p.rect.xy), vec2<i32>(p.rect.zw) - vec2(1)); }
    let xy = vec2<f32>(at);
    if any(at < vec2(0)) || any(at >= vec2<i32>(p.size.xy)) || any(xy < p.rect.xy) || any(xy >= p.rect.zw) { return vec2(0.0); }
    let alpha = quant(textureLoad(selection, at, 0).r);
    return vec2(textureLoad(source, at, 0)[u32(p.size.z)] * alpha, alpha);
}
fn cubic(t: f32) -> vec4<f32> {
    let t2 = t * t; let t3 = t2 * t;
    // Match transform::bicubic: Keys a = -0.75, including Warp's scalar sampler.
    return vec4(-0.75*t3+1.5*t2-0.75*t, 1.25*t3-2.25*t2+1.0, -1.25*t3+1.5*t2+0.75*t, 0.75*t3-0.75*t2);
}
fn sample_at(at: vec2<f32>, footprint: vec2<f32>, repeat_edge: bool) -> vec2<f32> {
    if p.opts.x == 0.0 { return lifted(vec2<i32>(floor(at + 0.5)), false); }
    if any(p.rect.zw <= p.rect.xy) { return vec2(0.0); }
    if repeat_edge && (any(at < p.rect.xy - vec2(1.0)) || any(at > p.rect.zw)) { return vec2(0.0); }
    let pos = vec2<i32>(floor(at)); let frac = fract(at);
    // Interior kernels already lie inside the source rectangle; their taps need no clamping.
    let first = pos - select(vec2(0), vec2(1), p.opts.x == 2.0);
    let last = pos + select(vec2(1), vec2(2), p.opts.x == 2.0);
    let clamp_samples = repeat_edge && (any(vec2<f32>(first) < p.rect.xy) || any(vec2<f32>(last) >= p.rect.zw));
    var value = vec2(0.0);
    if p.opts.x == 1.0 {
        value = mix(mix(lifted(pos, clamp_samples), lifted(pos + vec2(1,0), clamp_samples), frac.x), mix(lifted(pos + vec2(0,1), clamp_samples), lifted(pos + vec2(1,1), clamp_samples), frac.x), frac.y);
    } else {
        let wx = cubic(frac.x); let wy = cubic(frac.y);
        for (var j = 0; j < 4; j++) {
            for (var i = 0; i < 4; i++) { value += lifted(pos + vec2(i-1,j-1), clamp_samples) * wx[u32(i)] * wy[u32(j)]; }
        }
    }
    if repeat_edge {
        // Free Transform repeats the source edge; only its geometric outline is antialiased.
        // Warp's mesh sampler keeps its own transparent-border and triangle-coverage semantics.
        let distance = min(at + 0.5 - p.rect.xy, p.rect.zw - (at + 0.5));
        let covered = clamp(distance / footprint + 0.5, vec2(0.0), vec2(1.0));
        value *= covered.x * covered.y;
    }
    let coverage = clamp(value.y, 0.0, 1.0);
    let a = quant(coverage);
    var gray = 0.0;
    // The CPU divides the premultiplied sample by the original alpha before clipping it.
    // Bicubic alpha may overshoot 1; dividing by clipped coverage lightens the sample.
    if value.y > 0.000001 { gray = quant(value.x / value.y); }
    return vec2(gray * a, a);
}
fn srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(c, vec3(0.0)), vec3(1.0/2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
fn finish(at: vec2<i32>, transformed: vec2<f32>) -> vec4<f32> {
    var rgba = textureLoad(source, at, 0);
    let c = u32(p.size.z); let weight = textureLoad(selection, at, 0).r;
    let rest = quant(rgba[c] * (1.0-weight) + p.size.w * weight);
    if p.opts.w == 0.0 { rgba[c] = quant(transformed.x + rest * (1.0-transformed.y)); }
    if p.opts.w == 2.0 && all(vec2<f32>(at) >= p.bounds.xy) && all(vec2<f32>(at) < p.bounds.zw) {
        // Move changes only the source layer's content rectangle. Its vacated samples take
        // the background component; samples outside that rectangle remain untouched.
        rgba[c] = quant(transformed.x + p.size.w * (1.0-transformed.y));
    }
    rgba.a = clamp(rgba.a * p.view.z, 0.0, 1.0);
    let visible = u32(p.view.x);
    if visible != 7u {
        var rgb = rgba.rgb * rgba.a + vec3(1.0-rgba.a);
        if visible == 0u { return vec4(1.0); }
        if countOneBits(visible) == 1u && p.view.y == 0.0 {
            return vec4(vec3(rgb[firstTrailingBit(visible)]), 1.0);
        }
        for (var i = 0u; i < 3u; i++) { if (visible & (1u << i)) == 0u { rgb[i] = 0.0; } }
        return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), 1.0);
    }
    if p.view.w != 0.0 { rgba = vec4(srgb(rgba.rgb), rgba.a); }
    // Match the CPU display's 8-bit straight samples before premultiplication.
    rgba = floor(clamp(rgba, vec4(0.0), vec4(1.0)) * 255.0 + 0.5) / 255.0;
    return vec4(rgba.rgb * rgba.a, rgba.a);
}
@fragment fn fs_affine(v: Vertex) -> @location(0) vec4<f32> {
    let dst = vec3(v.position.xy, 1.0);
    let denom = dot(p.h2.xyz, dst);
    let src = vec2(dot(p.h0.xyz, dst), dot(p.h1.xyz, dst)) / denom - 0.5;
    // Integer translations retain the original nearest-sample path, without coverage work.
    if p.opts.x == 0.0 { return finish(vec2<i32>(v.position.xy), lifted(vec2<i32>(floor(src + 0.5)), false)); }
    if any(v.position.xy < p.bounds.xy) || any(v.position.xy >= p.bounds.zw) { return finish(vec2<i32>(v.position.xy), vec2(0.0)); }
    var footprint = vec2(1.0);
    if p.opts.x != 0.0 {
        if p.h2.x == 0.0 && p.h2.y == 0.0 {
            footprint = vec2(dot(abs(p.h0.xy / p.h2.z), vec2(1.0)), dot(abs(p.h1.xy / p.h2.z), vec2(1.0)));
        } else {
            // Match transform::warp: inverse steps at the midpoint of the bounded CPU tile.
            let tile = floor(v.position.xy / p.h0.w) * p.h0.w;
            let middle = (max(tile, p.bounds.xy) + min(tile + p.h0.w, p.bounds.zw)) * 0.5;
            let o = inverse_at(middle);
            footprint = abs(inverse_at(middle + vec2(1.0, 0.0)) - o) + abs(inverse_at(middle + vec2(0.0, 1.0)) - o);
        }
        footprint = vec2(valid_extent(footprint.x), valid_extent(footprint.y));
    }
    return finish(vec2<i32>(v.position.xy), sample_at(src, footprint, p.opts.w != 2.0));
}
fn inverse_at(at: vec2<f32>) -> vec2<f32> {
    let dst = vec3(at, 1.0);
    return vec2(dot(p.h0.xyz, dst), dot(p.h1.xyz, dst)) / dot(p.h2.xyz, dst);
}
fn valid_extent(value: f32) -> f32 {
    if value > 0.000000000001 && value <= 3.4e38 { return value; }
    return 1.0;
}
@fragment fn fs_warp(v: Vertex) -> @location(0) vec4<f32> { return vec4(sample_at(v.src - 0.5, vec2(1.0), false), 0.0, 0.0); }
@fragment fn fs_finish(v: Vertex) -> @location(0) vec4<f32> {
    let at = vec2<i32>(v.position.xy);
    return finish(at, textureLoad(moved, at, 0).xy);
}
