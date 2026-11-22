// Texture and Clarity on the GPU (`photocraft_algo::camera_raw::local_detail`, opaque pixels in
// sRGB's primaries): the planes, each scale's Gaussian as three box passes along the rows and
// three along the columns (edges clamped), the moves, and applying them. Compute passes over
// storage buffers of one `vec4` per pixel; `camera_raw_detail.rs` holds the bindings.

struct D {
    // Width, height, box radius, scale index.
    size: vec4<u32>,
    // Scales, amplitudes, colour (1: Texture), unused.
    model: vec4<u32>,
    // The document's transfer curve (0: sRGB's), then the clamp amplitudes (at most two).
    k: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: D;
@group(0) @binding(1) var<storage, read> coeffs: array<f32>;
// The planes: encoded luminance, then (Texture) each channel's colour minus it.
@group(0) @binding(2) var<storage, read_write> planes: array<vec4<f32>>;
// Per pixel: linear luminance, knot index, knot weight.
@group(0) @binding(3) var<storage, read_write> knots: array<vec4<f32>>;
// The moves (Texture: per channel; Clarity: the luminance's in x).
@group(0) @binding(4) var<storage, read_write> moves: array<vec4<f32>>;
// A box pass reads `blur_in` and writes `blur_out`.
@group(0) @binding(5) var<storage, read> blur_in: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> blur_out: array<vec4<f32>>;
@group(0) @binding(7) var src: texture_2d<f32>;
@group(0) @binding(8) var dst: texture_storage_2d<rgba32float, write>;

const KNOTS: u32 = 8u;

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.0031308 {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn decode(v: f32) -> f32 {
    if P.k.x > 0.0 {
        return pow(max(v, 0.0), P.k.x);
    }
    return srgb_to_linear(v);
}

fn encode(v: f32) -> f32 {
    if P.k.x > 0.0 {
        return pow(max(v, 0.0), 1.0 / P.k.x);
    }
    return linear_to_srgb(v);
}

// `Encoding::to_srgb_encoded` in sRGB's primaries.
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    if P.k.x > 0.0 {
        return vec3(linear_to_srgb(decode(c.x)), linear_to_srgb(decode(c.y)), linear_to_srgb(decode(c.z)));
    }
    return c;
}

// `Encoding::back_from_srgb_encoded` in sRGB's primaries.
fn from_srgb(s: vec3<f32>) -> vec3<f32> {
    if P.k.x > 0.0 {
        return vec3(encode(srgb_to_linear(s.x)), encode(srgb_to_linear(s.y)), encode(srgb_to_linear(s.z)));
    }
    return s;
}

fn finite_or_zero(v: f32) -> f32 {
    return select(0.0, v, abs(v) <= 3.4e38);
}

fn lstar(y: f32) -> f32 {
    if y > 216.0 / 24389.0 {
        return 116.0 * pow(y, 1.0 / 3.0) - 16.0;
    }
    return 24389.0 / 27.0 * y;
}

fn index(x: u32, y: u32) -> u32 {
    return y * P.size.x + x;
}

// The coefficient of feature `block` at pixel `i`, interpolated between its knots.
fn coef(block: u32, i: u32) -> f32 {
    let kn = knots[i];
    let j = u32(kn.y);
    let b = block * KNOTS + j;
    return coeffs[b] * (1.0 - kn.z) + coeffs[b + 1u] * kn.z;
}

// `term`: the feature blocks from `block` on, at detail `d`.
fn term(block: u32, i: u32, d: f32) -> f32 {
    var v = coef(block, i) * max(d, 0.0) + coef(block + 1u, i) * min(d, 0.0);
    for (var a = 0u; a < P.model.y; a++) {
        let amp = P.k[1u + a];
        v += coef(block + 2u + a, i) * clamp(d, -amp, amp);
    }
    return v;
}

// The planes, knots and base moves of every pixel.
@compute @workgroup_size(8, 8)
fn cs_planes(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.size.x || id.y >= P.size.y {
        return;
    }
    let i = index(id.x, id.y);
    let q = textureLoad(src, vec2<i32>(id.xy), 0);
    let s = to_srgb(q.rgb);
    let lin = vec3(srgb_to_linear(finite_or_zero(s.x)), srgb_to_linear(finite_or_zero(s.y)), srgb_to_linear(finite_or_zero(s.z)));
    let yl = max(0.2126 * lin.x + 0.7152 * lin.y + 0.0722 * lin.z, 0.0);
    let f = clamp(lstar(yl) / 100.0, 0.0, 1.0) * f32(KNOTS - 1u);
    let j = min(u32(f), KNOTS - 2u);
    knots[i] = vec4(yl, f32(j), f - f32(j), 0.0);
    let sy = linear_to_srgb(yl);
    let sc = vec3(finite_or_zero(s.x), finite_or_zero(s.y), finite_or_zero(s.z));
    if P.model.z == 1u {
        planes[i] = vec4(sy, sc.x - sy, sc.y - sy, sc.z - sy);
    } else {
        planes[i] = vec4(sy, 0.0, 0.0, 0.0);
    }
    moves[i] = vec4(vec3(coef(0u, i)), 0.0);
}

// Copies the planes into the blur's buffer.
@compute @workgroup_size(8, 8)
fn cs_copy(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.size.x || id.y >= P.size.y {
        return;
    }
    let i = index(id.x, id.y);
    blur_out[i] = planes[i];
}

// One box pass of radius `P.size.z` along a row (`rows`) or column, edges clamped: a running
// sum (compensated, as the CPU's f64 sum), one invocation per line.
fn box_line(line: u32, rows: bool) {
    let n = select(P.size.y, P.size.x, rows);
    let lines = select(P.size.x, P.size.y, rows);
    if line >= lines {
        return;
    }
    let r = i32(P.size.z);
    let last = i32(n) - 1;
    let norm = 1.0 / f32(2 * r + 1);
    var acc = vec4(0.0);
    var comp = vec4(0.0);
    for (var t = -r; t <= r; t++) {
        let x = u32(clamp(t, 0, last));
        let v = blur_in[select(index(line, x), index(x, line), rows)];
        let y = v - comp;
        let s = acc + y;
        comp = (s - acc) - y;
        acc = s;
    }
    for (var x = 0; x < i32(n); x++) {
        blur_out[select(index(line, u32(x)), index(u32(x), line), rows)] = acc * norm;
        let add = blur_in[select(index(line, u32(clamp(x + r + 1, 0, last))), index(u32(clamp(x + r + 1, 0, last)), line), rows)];
        let sub = blur_in[select(index(line, u32(clamp(x - r, 0, last))), index(u32(clamp(x - r, 0, last)), line), rows)];
        let y = (add - sub) - comp;
        let s = acc + y;
        comp = (s - acc) - y;
        acc = s;
    }
}

@compute @workgroup_size(64)
fn cs_box_rows(@builtin(global_invocation_id) id: vec3<u32>) {
    box_line(id.x, true);
}

@compute @workgroup_size(64)
fn cs_box_cols(@builtin(global_invocation_id) id: vec3<u32>) {
    box_line(id.x, false);
}

// One box pass of radius `P.size.z` summed directly at each pixel (rows or columns), for radii
// where that beats a line's running sum: every pixel in parallel, edges clamped.
fn box_direct(id: vec3<u32>, rows: bool) {
    if id.x >= P.size.x || id.y >= P.size.y {
        return;
    }
    let r = i32(P.size.z);
    let last = i32(select(P.size.y, P.size.x, rows)) - 1;
    let at = i32(select(id.y, id.x, rows));
    var acc = vec4(0.0);
    for (var t = -r; t <= r; t++) {
        let x = u32(clamp(at + t, 0, last));
        acc += blur_in[select(index(id.x, x), index(x, id.y), rows)];
    }
    blur_out[index(id.x, id.y)] = acc / f32(2 * r + 1);
}

@compute @workgroup_size(8, 8)
fn cs_box_rows_direct(@builtin(global_invocation_id) id: vec3<u32>) {
    box_direct(id, true);
}

@compute @workgroup_size(8, 8)
fn cs_box_cols_direct(@builtin(global_invocation_id) id: vec3<u32>) {
    box_direct(id, false);
}

// Adds scale `P.size.w`'s terms to the moves; `blur_in` holds the planes blurred at that scale
// (the planes themselves for a scale too small to blur).
@compute @workgroup_size(8, 8)
fn cs_moves(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.size.x || id.y >= P.size.y {
        return;
    }
    let i = index(id.x, id.y);
    let ps = 2u + P.model.y;
    let si = P.size.w;
    let ns = P.model.x;
    let p = planes[i];
    let b = blur_in[i];
    let dl = term(1u + ps * si, i, p.x - b.x);
    var m = moves[i];
    for (var c = 0u; c < 3u; c++) {
        m[c] += dl;
        if P.model.z == 1u {
            m[c] += term(1u + ps * ns + ps * si, i, p[1u + c] - b[1u + c]);
        }
    }
    moves[i] = m;
}

// Applies the moves: Texture per channel, Clarity on the luminance with the chromaticity kept.
@compute @workgroup_size(8, 8)
fn cs_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= P.size.x || id.y >= P.size.y {
        return;
    }
    let i = index(id.x, id.y);
    let q = textureLoad(src, vec2<i32>(id.xy), 0);
    let m = moves[i];
    let yl = knots[i].x;
    let inside = all(q.rgb >= vec3(0.0)) && all(q.rgb <= vec3(1.0));
    let s = to_srgb(q.rgb);
    var out = vec3(0.0);
    if P.model.z == 1u {
        for (var c = 0u; c < 3u; c++) {
            let v = s[c] + clamp(m[c], -1.0, 1.0);
            out[c] = select(v, clamp(v, 0.0, 1.0), inside);
        }
    } else {
        let t0 = linear_to_srgb(yl) + clamp(m.x, -1.0, 1.0);
        let target_ = srgb_to_linear(select(t0, clamp(t0, 0.0, 1.0), inside));
        let r = select(1.0, target_ / yl, yl > 1e-6);
        for (var c = 0u; c < 3u; c++) {
            let lin = srgb_to_linear(s[c]) * r;
            out[c] = linear_to_srgb(select(lin, clamp(lin, 0.0, 1.0), inside));
        }
    }
    textureStore(dst, vec2<i32>(id.xy), vec4(from_srgb(out), q.a));
}
