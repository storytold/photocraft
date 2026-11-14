// Skin Smooth v2: three cumulative, edge-aware recursive scales (2, 7, 20 px).
struct Params {
    size: vec4<u32>,
    phase: vec4<f32>, // spatial, scale gain, original gain, unused
    tone: vec4<f32>, // exposure, warmth, redness, tolerance
    flags: vec4<u32>, // skinOnly, positive count, negative count, unused
    positive: array<vec4<f32>, 16>,
    negative: array<vec4<f32>, 16>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> pixels: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> accumulated: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> edges: array<f32>;
@group(0) @binding(5) var<storage, read_write> result: array<vec4<f32>>;

@compute @workgroup_size(64)
fn init(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= p.size.x * p.size.y { return; }
    let s = source[i];
    let transposed = (i % p.size.x) * p.size.y + i / p.size.x;
    pixels[i] = vec4<f32>(s.rgb * 255.0, s.a);
    accumulated[i] = vec4<f32>(s.rgb * 255.0 * p.phase.z, 0.0);
    var e = vec2<f32>(0.0);
    if i % p.size.x > 0u {
        let d = abs(s.rgb - source[i - 1u].rgb);
        e.x = exp(-(d.x + d.y + d.z) * 255.0 / 24.0);
    }
    if i >= p.size.x {
        let d = abs(s.rgb - source[i - p.size.x].rgb);
        e.y = exp(-(d.x + d.y + d.z) * 255.0 / 24.0);
    }
    edges[i] = e.x;
    edges[p.size.x * p.size.y + transposed] = e.y;
}

// Affine prefix scan of x[i] = b[i] + a[i] * x[i-1]. Each 64-pixel block
// is parallel; only the carry between blocks is sequential. Source alpha is immutable.
var<workgroup> prefix: array<vec4<f32>, 64>;
var<workgroup> carry: vec3<f32>;

fn scan_line(line: u32, lane: u32, vertical_axis: bool, reverse: bool) {
    let length = select(p.size.x, p.size.y, vertical_axis);
    let count = p.size.x * p.size.y;
    if lane == 0u { carry = vec3<f32>(0.0); }
    workgroupBarrier();
    for (var block = 0u; block < (length + 63u) / 64u; block++) {
        let step = block * 64u + lane;
        let pos = select(step, length - 1u - step, reverse);
        let i = line * length + pos;
        var item = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        var alpha = 0.0;
        if step < length {
            var value = pixels[i];
            if vertical_axis { value = result[i]; }
            alpha = value.a;
            var weight = 0.0;
            if step > 0u {
                let neighbor = select(pos - 1u, pos + 1u, reverse);
                let original = select(line * p.size.x + neighbor, neighbor * p.size.x + line, vertical_axis);
                let edge = select(i, i + 1u, reverse) + select(0u, count, vertical_axis);
                weight = p.phase.x * edges[edge] * source[original].a;
            }
            item = vec4<f32>(value.rgb * (1.0 - weight), weight);
        }
        prefix[lane] = item;
        for (var offset = 1u; offset < 64u; offset *= 2u) {
            workgroupBarrier();
            var previous = vec4<f32>(0.0);
            if lane >= offset { previous = prefix[lane - offset]; }
            let current = prefix[lane];
            workgroupBarrier();
            if lane >= offset { prefix[lane] = vec4<f32>(current.rgb + current.a * previous.rgb, current.a * previous.a); }
        }
        workgroupBarrier();
        let filtered = prefix[lane].rgb + prefix[lane].a * carry;
        workgroupBarrier();
        if step < length {
            if vertical_axis { result[i] = vec4<f32>(filtered, alpha); }
            else { pixels[i] = vec4<f32>(filtered, alpha); }
        }
        if lane == 63u { carry = filtered; }
        workgroupBarrier();
    }
}

@compute @workgroup_size(64)
fn horizontal(@builtin(workgroup_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if id.x >= p.size.y { return; }
    scan_line(id.x, lane, false, false);
    scan_line(id.x, lane, false, true);
}

@compute @workgroup_size(64)
fn rows(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i < p.size.x * p.size.y {
        result[(i % p.size.x) * p.size.y + i / p.size.x] = pixels[i];
    }
}

@compute @workgroup_size(64)
fn vertical(@builtin(workgroup_id) id: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if id.x >= p.size.x { return; }
    scan_line(id.x, lane, true, false);
    scan_line(id.x, lane, true, true);
}

@compute @workgroup_size(64)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < p.size.x * p.size.y {
        let value = result[(id.x % p.size.x) * p.size.y + id.x / p.size.x];
        accumulated[id.x] += vec4<f32>(value.rgb * p.phase.y, 0.0);
        pixels[id.x] = value;
    }
}

fn range_weight(chroma: vec2<f32>, center: vec2<f32>) -> f32 {
    let spread = 0.6 + p.tone.w / 100.0;
    let d = (chroma - center) / (vec2<f32>(19.0, 24.0) * spread);
    return 1.0 - smoothstep(0.35, 1.6, dot(d,d));
}

fn skin_weight(rgb: vec3<f32>) -> f32 {
    let y = 0.299 * rgb.r + 0.587 * rgb.g + 0.114 * rgb.b;
    let cb = 128.0 - 0.168736 * rgb.r - 0.331264 * rgb.g + 0.5 * rgb.b;
    let cr = 128.0 + 0.5 * rgb.r - 0.418688 * rgb.g - 0.081312 * rgb.b;
    let chroma = vec2<f32>(cb, cr);
    var selected = range_weight(chroma, vec2<f32>(108.0, 151.0));
    if p.flags.y > 0u {
        selected = 0.0;
        for (var i = 0u; i < p.flags.y; i++) { selected = max(selected, range_weight(chroma, p.positive[i].xy)); }
    }
    var excluded = 0.0;
    for (var i = 0u; i < p.flags.z; i++) { excluded = max(excluded, range_weight(chroma, p.negative[i].xy)); }
    return selected * (1.0 - excluded) * smoothstep(12.0, 38.0, y) * (1.0 - smoothstep(237.0, 254.0, y));
}

@compute @workgroup_size(64)
fn finish(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= p.size.x * p.size.y { return; }
    let s = source[i];
    var weight = 1.0;
    let rgb = s.rgb * 255.0;
    if p.flags.x != 0u { weight = skin_weight(rgb); }
    if s.a == 0.0 || weight == 0.0 { result[i] = s; return; }
    let red = p.tone.z * max(0.0, rgb.r - (rgb.g + rgb.b) * 0.5) * 0.5;
    let tone = vec3<f32>(p.tone.y - red, red * 0.25, -p.tone.y);
    let adjusted = (rgb + accumulated[i].rgb) * p.tone.x + tone;
    result[i] = vec4<f32>(clamp(rgb + (adjusted - rgb) * weight, vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0, s.a);
}
