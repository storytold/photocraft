struct Params {
    size: vec2<u32>,
    radius: u32,
    stage: u32,
    opacity: f32,
    blend: i32,
    value: f32,
    input_black: f32,
    gamma: f32,
    input_white: f32,
    output_black: f32,
    output_white: f32,
    fill_bounds: vec4<i32>,
    fill_size: vec2<u32>,
    fill_flags: u32,
    fill_rotation: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var source_tex: texture_2d<f32>;
@group(1) @binding(1) var auxiliary_tex: texture_2d<f32>;
@group(1) @binding(2) var shell_tex: texture_2d<u32>;
@group(1) @binding(3) var<storage, read> kernel_weights: array<f32>;
@group(1) @binding(4) var<storage, read_write> brush_coverage: array<f32>;


struct VOut { @builtin(position) position: vec4<f32> };

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return VOut(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}

fn inside(p: vec2<i32>) -> bool {
    return p.x >= 0 && p.y >= 0 && p.x < i32(params.size.x) && p.y < i32(params.size.y);
}

fn shell_at(p: vec2<i32>) -> u32 {
    if (!inside(p)) { return 0u; }
    return textureLoad(shell_tex, p, 0).r;
}

fn blend_value(below: f32, value: f32, coverage: f32) -> f32 {
    if (params.blend == 1) { return value * coverage + below * (1.0 - coverage); }
    var blended = value;
    switch params.blend {
        case 3: { blended = min(below, value); }
        case 4: { blended = below * value; }
        case 8: { blended = max(below, value); }
        case 9: { blended = below + value - below * value; }
        case 11: { blended = min(below + value, 1.0); }
        case 20: { blended = abs(below - value); }
        case 22: { blended = max(below - value, 0.0); }
        default: { blended = value; }
    }
    return below + (blended - below) * clamp(coverage, 0.0, 1.0);
}

fn soft_exp(u: f32) -> f32 {
    var g = clamp(1.0 - u / 64.0, 0.0, 1.0);
    for (var i = 0; i < 6; i++) { g *= g; }
    return g;
}

@compute @workgroup_size(8, 8, 1)
fn cs_dab(@builtin(global_invocation_id) id: vec3<u32>) {
    let origin = vec2<u32>(u32(params.fill_bounds.x), u32(params.fill_bounds.y));
    let end = vec2<u32>(u32(params.fill_bounds.z), u32(params.fill_bounds.w));
    let p = origin + id.xy;
    if (p.x >= end.x || p.y >= end.y || p.x >= params.size.x || p.y >= params.size.y) { return; }
    let radius = max(params.gamma, 0.0);
    let core = max(params.output_white, 0.0);
    let shoulder = max(params.fill_rotation, 0.001);
    let delta = vec2<f32>(f32(p.x) + 0.5, f32(p.y) + 0.5) - vec2<f32>(params.value, params.input_black);
    let distance = length(delta);
    let reach = core + 2.6 * shoulder;
    if (distance >= reach + 0.5) { return; }
    var shape = 1.0;
    if (distance > core) {
        let x = (distance - core) / shoulder;
        shape = soft_exp(x * x);
    }
    let index = p.y * params.size.x + p.x;
    let previous = brush_coverage[index];
    let ceiling = clamp(params.output_black, 0.0, 1.0);
    let value = shape * clamp(params.input_white, 0.0, 1.0);
    if (ceiling > previous) { brush_coverage[index] = previous + value * (ceiling - previous); }
}

fn fill_sample_axis(coordinate: f32, map_size: u32, repeat: bool, mirror: bool) -> i32 {
    let tile = floor(coordinate);
    var fraction = coordinate - tile;
    if (repeat && mirror && (i32(tile) % 2 != 0)) {
        fraction = 1.0 - fraction;
    }
    return i32(clamp(floor(fraction * f32(map_size)), 0.0, f32(map_size - 1u)));
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(in.position.xy));
    if (!inside(p)) { return vec4<f32>(0.0); }
    let below = textureLoad(source_tex, p, 0).r;

    if (params.stage == 0u) {
        let paint = textureLoad(auxiliary_tex, p, 0);
        let value = blend_value(below, paint.r, paint.a * params.opacity);
        return vec4<f32>(value, value, value, 1.0);
    }
    if (params.stage == 1u) {
        let value = blend_value(below, params.value, params.opacity);
        return vec4<f32>(value, value, value, 1.0);
    }
    if (params.stage == 2u) {
        let normalized = clamp((below - params.input_black) / max(params.input_white - params.input_black, 0.0000001), 0.0, 1.0);
        let adjusted = params.output_black + pow(normalized, 1.0 / params.gamma) * (params.output_white - params.output_black);
        let value = blend_value(below, adjusted, params.opacity);
        return vec4<f32>(value, value, value, 1.0);
    }
    if (params.stage == 5u) {
        let original = textureLoad(auxiliary_tex, p, 0).r;
        let value = blend_value(original, below, params.opacity);
        return vec4<f32>(value, value, value, 1.0);
    }
    if (params.stage == 6u) {
        let has_map = (params.fill_flags & 32u) != 0u;
        let has_bounds = (params.fill_flags & 16u) != 0u;
        let repeat_u = (params.fill_flags & 1u) != 0u;
        let repeat_v = (params.fill_flags & 2u) != 0u;
        let mirror_u = (params.fill_flags & 4u) != 0u;
        let mirror_v = (params.fill_flags & 8u) != 0u;
        var fill_value = params.value;
        if (has_bounds) {
            let x0 = params.fill_bounds.x;
            let y0 = params.fill_bounds.y;
            let x1 = params.fill_bounds.z;
            let y1 = params.fill_bounds.w;
            if (x1 <= x0 || y1 <= y0) { return vec4<f32>(below, below, below, 1.0); }
            var doc = vec2<f32>(f32(p.x) + 0.5, f32(p.y) + 0.5);
            if (params.fill_rotation != 0.0) {
                let angle = (params.fill_rotation - floor(params.fill_rotation / 360.0) * 360.0) * 0.017453292519943295;
                let center = (vec2<f32>(f32(x0), f32(y0)) + vec2<f32>(f32(x1), f32(y1))) * 0.5;
                let delta = doc - center;
                let c = cos(angle);
                let s = sin(angle);
                doc = vec2<f32>(c * delta.x + s * delta.y, -s * delta.x + c * delta.y) + center;
            }
            let span = vec2<f32>(f32(x1) - f32(x0), f32(y1) - f32(y0));
            let uv = (doc - vec2<f32>(f32(x0), f32(y0))) / span;
            let effective_repeat_u = repeat_u && has_map;
            let effective_repeat_v = repeat_v && has_map;
            if ((!effective_repeat_u && (uv.x < 0.0 || uv.x >= 1.0)) || (!effective_repeat_v && (uv.y < 0.0 || uv.y >= 1.0))) {
                return vec4<f32>(below, below, below, 1.0);
            }
            if (has_map) {
                let sx = fill_sample_axis(uv.x, params.fill_size.x, effective_repeat_u, mirror_u);
                let sy = fill_sample_axis(uv.y, params.fill_size.y, effective_repeat_v, mirror_v);
                fill_value = textureLoad(auxiliary_tex, vec2<i32>(sx, sy), 0).r;
            }
        } else if (has_map) {
            var sx = 0;
            var sy = 0;
            if (repeat_u) {
                let size = i32(params.fill_size.x);
                let tile = p.x / size;
                let offset = p.x % size;
                sx = select(offset, size - 1 - offset, mirror_u && (tile % 2 != 0));
            } else {
                sx = (p.x * i32(params.fill_size.x)) / 2147483647;
            }
            if (repeat_v) {
                let size = i32(params.fill_size.y);
                let tile = p.y / size;
                let offset = p.y % size;
                sy = select(offset, size - 1 - offset, mirror_v && (tile % 2 != 0));
            } else {
                sy = (p.y * i32(params.fill_size.y)) / 2147483647;
            }
            fill_value = textureLoad(auxiliary_tex, vec2<i32>(sx, sy), 0).r;
        }
        let value = blend_value(below, fill_value, params.opacity);
        return vec4<f32>(value, value, value, 1.0);
    }
    if (params.stage == 8u) {
        let index = u32(p.y) * params.size.x + u32(p.x);
        let mask = clamp(brush_coverage[index] * params.opacity, 0.0, 1.0);
        let source = vec4<f32>(params.value, params.input_black, params.gamma, params.input_white);
        let destination = textureLoad(source_tex, p, 0);
        let source_alpha = source.a * mask;
        let output_alpha = source_alpha + destination.a * (1.0 - source_alpha);
        var output_color = vec3<f32>(0.0);
        if (output_alpha > 0.0) {
            output_color = (source.rgb * source_alpha + destination.rgb * destination.a * (1.0 - source_alpha)) / output_alpha;
        }
        return vec4<f32>(output_color, output_alpha);
    }
    if (params.stage == 10u) { return vec4<f32>(0.0); }
    let center_shell = shell_at(p);
    let radius = i32(params.radius);
    let kernel_base = params.radius * 129u;
    if (params.stage == 3u) {
        var same_shell_value = 0.0;
        var same_shell_weight = 0.0;
        var outward_value = 0.0;
        for (var dx = -radius; dx <= radius; dx++) {
            let q = p + vec2<i32>(dx, 0);
            if (inside(q)) {
                let source_shell = shell_at(q);
                let weight = kernel_weights[kernel_base + u32(dx + 64)];
                let value = textureLoad(source_tex, q, 0).r;
                if (source_shell > 0u) { outward_value += value * weight; }
                if (center_shell > 0u && source_shell == center_shell) {
                    same_shell_value += value * weight;
                    same_shell_weight += weight;
                }
            }
        }
        return vec4<f32>(same_shell_value, same_shell_weight, outward_value, 1.0);
    }
    if (params.stage == 4u) {
        var same_shell_value = 0.0;
        var same_shell_weight = 0.0;
        var outward_value = 0.0;
        for (var dy = -radius; dy <= radius; dy++) {
            let q = p + vec2<i32>(0, dy);
            if (inside(q)) {
                let weight = kernel_weights[kernel_base + u32(dy + 64)];
                let sample = textureLoad(source_tex, q, 0);
                let source_shell = shell_at(q);
                if (center_shell == 0u) {
                    outward_value += sample.b * weight;
                } else if (source_shell == center_shell) {
                    same_shell_value += sample.r * weight;
                    same_shell_weight += sample.g * weight;
                }
            }
        }
        var blurred = outward_value;
        if (center_shell > 0u) {
            blurred = 0.0;
            if (same_shell_weight > 0.0) { blurred = same_shell_value / same_shell_weight; }
        }
        return vec4<f32>(blurred, blurred, blurred, 1.0);
    }
    return vec4<f32>(below, below, below, 1.0);
}
