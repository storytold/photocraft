// Camera Raw's per-pixel develop stages (`photocraft_algo::camera_raw::develop`, constants from
// `camera_raw::per_pixel`), one fragment per pixel. Each function mirrors the CPU code it is
// named after, in the same order of operations; `camera_raw.rs` holds the bindings.

struct Params {
    size: vec2<u32>,
    flags: u32,
    // The document's transfer curve: 0 for sRGB's, else a power-law gamma.
    encoding: f32,
    // White balance gains (rgb).
    gains: vec4<f32>,
    // Linear RGB to linear ProPhoto (rows) and back.
    to_prophoto: array<vec4<f32>, 3>,
    from_prophoto: array<vec4<f32>, 3>,
    // Color Grading wheels: tint (rgb), luminance (w).
    tints: array<vec4<f32>, 4>,
    // Blending, Balance.
    grade: vec4<f32>,
    // [amount, highlights, style, power], [centre, axes], [edges].
    vignette: array<vec4<f32>, 3>,
    // Cell, roughness, amount, full-resolution pixels per pixel.
    grain: vec4<f32>,
    // The two octaves' seeds as (low, high) words.
    seeds: vec4<u32>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
// R32Float rows of `LUT_WIDTH`: parametric, master, red, green, blue curves; Whites, Blacks rows;
// the Exposure and Contrast rows; from row `WHITES_FIX_ROW` on, Whites down's correction grid (RGB
// per node, nodes one after another across the rows); from `MIXER_ROW` the Color Mixer's
// Luminance, Saturation and Hue tables and from `PRESENCE_ROW` Saturation's and Vibrance's (hue
// shift, saturation and log-value change per node of hue x saturation x value).
@group(0) @binding(2) var luts: texture_2d<f32>;

const GAINS: u32 = 0u;
const CONTRAST: u32 = 1u;
const WHITES: u32 = 2u;
const BLACKS: u32 = 3u;
const WHITES_CHANNELS: u32 = 4u;
const PRESENCE: u32 = 5u;
const PARAMETRIC: u32 = 6u;
const MASTER: u32 = 7u;
const RED: u32 = 8u;
const HSL: u32 = 11u;
const GRADE: u32 = 12u;
const VIGNETTE: u32 = 13u;
const GRAIN: u32 = 14u;
const CLAMP: u32 = 15u;
const EXPOSURE: u32 = 16u;
const BLACKS_CHANNELS: u32 = 17u;
const WHITES_FIX: u32 = 19u;

const WHITES_FIX_ROW: i32 = 9;
const MIXER_ROW: i32 = 24;
const PRESENCE_ROW: i32 = 76;
const PRESENCE_H: i32 = 48;
const PRESENCE_S: i32 = 21;
const PRESENCE_V: i32 = 21;
const MIXER_H: i32 = 72;
const MIXER_S: i32 = 9;
const MIXER_V: i32 = 9;
const FIX_N: i32 = 17;

const CURVE_LUT: i32 = 1024;
const TONE_STEPS: i32 = 65;

struct VOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}

fn on(flag: u32) -> bool {
    return (P.flags & (1u << flag)) != 0u;
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
}

fn smoothstep_(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn rem_euclid(x: f32, m: f32) -> f32 {
    let r = x % m;
    return select(r, r + m, r < 0.0);
}

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
    if P.encoding > 0.0 {
        return pow(max(v, 0.0), P.encoding);
    }
    return srgb_to_linear(v);
}

fn encode(v: f32) -> f32 {
    if P.encoding > 0.0 {
        return pow(max(v, 0.0), 1.0 / P.encoding);
    }
    return linear_to_srgb(v);
}

fn lut_at(row: i32, i: i32) -> f32 {
    return textureLoad(luts, vec2(i, row), 0).r;
}

// `lut_eval`: linear inside 0..1, extended linearly outside (float documents).
// Value `k` of the correction grid starting at `row`.
fn fix_at(row: i32, k: i32) -> f32 {
    return lut_at(row + k / CURVE_LUT, k % CURVE_LUT);
}

fn to_srgb(v: f32) -> f32 {
    return select(v, linear_to_srgb(decode(v)), P.encoding > 0.0);
}

fn from_srgb(v: f32) -> f32 {
    return select(v, encode(srgb_to_linear(v)), P.encoding > 0.0);
}

// `base` with the correction grid at `row` looked up at `input` added, as the CPU's
// `colour_lut::Correction::apply`.
fn grid_fix(row: i32, input: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    let m = f32(FIX_N - 1);
    var i = vec3<i32>(0);
    var t = vec3(0.0);
    for (var k = 0; k < 3; k++) {
        let f = clamp(to_srgb(input[k]), 0.0, 1.0) * m;
        i[k] = min(i32(f), FIX_N - 2);
        t[k] = f - f32(i[k]);
    }
    var d = vec3(0.0);
    for (var corner = 0; corner < 8; corner++) {
        let dr = (corner >> 2u) & 1;
        let dg = (corner >> 1u) & 1;
        let db = corner & 1;
        let w = select(1.0 - t.x, t.x, dr == 1) * select(1.0 - t.y, t.y, dg == 1) * select(1.0 - t.z, t.z, db == 1);
        let node = (((i.x + dr) * FIX_N + i.y + dg) * FIX_N + i.z + db) * 3;
        d += w * vec3(fix_at(row, node), fix_at(row, node + 1), fix_at(row, node + 2));
    }
    if max(max(base.x, base.y), base.z) - min(min(base.x, base.y), base.z) < 1e-5 {
        d = vec3((d.x + d.y + d.z) / 3.0);
    }
    let inside = all(input >= vec3(0.0)) && all(input <= vec3(1.0));
    var o = vec3(0.0);
    for (var k = 0; k < 3; k++) {
        var s = to_srgb(base[k]);
        if inside {
            s = clamp(clamp(s, 0.0, 1.0) + d[k], 0.0, 1.0);
        } else {
            s += d[k];
        }
        o[k] = from_srgb(s);
    }
    return o;
}

fn lut_eval(row: i32, v: f32) -> f32 {
    if v < 0.0 {
        return v + lut_at(row, 0);
    }
    if v > 1.0 {
        return lut_at(row, CURVE_LUT - 1) + (v - 1.0);
    }
    let x = v * f32(CURVE_LUT - 1);
    let i = min(i32(x), CURVE_LUT - 2);
    let t = x - f32(i);
    let a = lut_at(row, i);
    return a + (lut_at(row, i + 1) - a) * t;
}

// `exposure::curve`: the curve in table row `row` at linear level `v`, in linear light.
fn tone_curve(row: i32, v: f32) -> f32 {
    if v <= 0.0 {
        return v;
    }
    if v >= 1.0 {
        return srgb_to_linear(lut_at(row, TONE_STEPS - 1)) + (v - 1.0);
    }
    let p = linear_to_srgb(v) * f32(TONE_STEPS - 1);
    let j = min(i32(p), TONE_STEPS - 2);
    let s = p - f32(j);
    let a = lut_at(row, j);
    return srgb_to_linear(a + (lut_at(row, j + 1) - a) * s);
}

// `exposure::tone`: the largest and smallest channels through the curve, the middle one kept
// at its fraction of the way between them.
fn tone_rgb(row: i32, c: vec3<f32>) -> vec3<f32> {
    var hi = 0;
    var lo = 0;
    for (var k = 1; k < 3; k++) {
        if c[k] > c[hi] {
            hi = k;
        }
        if c[k] < c[lo] {
            lo = k;
        }
    }
    if hi == lo {
        return vec3(tone_curve(row, c.x), tone_curve(row, c.y), tone_curve(row, c.z));
    }
    let mid = 3 - hi - lo;
    let fh = tone_curve(row, c[hi]);
    let fl = tone_curve(row, c[lo]);
    var out = vec3(0.0);
    out[hi] = fh;
    out[lo] = fl;
    out[mid] = fl + (fh - fl) * (c[mid] - c[lo]) / (c[hi] - c[lo]);
    return out;
}

fn rows_times(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}

// `exposure::pixel` with table row `row` (7 Exposure, 8 Contrast).
fn tone_pixel(row: i32, q: vec3<f32>) -> vec3<f32> {
    let lin = vec3(decode(q.x), decode(q.y), decode(q.z));
    let o = rows_times(P.from_prophoto, tone_rgb(row, rows_times(P.to_prophoto, lin)));
    return vec3(encode(o.x), encode(o.y), encode(o.z));
}

// `whites_blacks::row_pixel` with `RowOn::Luminance`: the pixel scaled by the change of its
// luminance through table row `row`.
fn luminance_pixel(row: i32, q: vec3<f32>) -> vec3<f32> {
    let lin = vec3(decode(q.x), decode(q.y), decode(q.z));
    let y = max(luma(lin), 0.0);
    if y <= 1e-7 {
        return q;
    }
    let k = tone_curve(row, y) / y;
    return vec3(encode(lin.x * k), encode(lin.y * k), encode(lin.z * k));
}

// `whites_blacks::row_pixel`: row 5 Whites, 6 Blacks, on each channel or the luminance.
fn white_black_pixel(row: i32, channels: bool, q: vec3<f32>) -> vec3<f32> {
    if channels {
        return tone_pixel(row, q);
    }
    return luminance_pixel(row, q);
}

fn rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let mx = max(max(c.x, c.y), c.z);
    let mn = min(min(c.x, c.y), c.z);
    let d = mx - mn;
    var h = 0.0;
    if d <= 1e-9 {
        h = 0.0;
    } else if mx == c.x {
        h = 60.0 * rem_euclid((c.y - c.z) / d, 6.0);
    } else if mx == c.y {
        h = 60.0 * ((c.z - c.x) / d + 2.0);
    } else {
        h = 60.0 * ((c.x - c.y) / d + 4.0);
    }
    return vec3(h, select(0.0, d / mx, mx > 1e-9), mx);
}

fn hsv_to_rgb(hue: f32, s: f32, v: f32) -> vec3<f32> {
    let h = rem_euclid(hue, 360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - abs((h % 2.0) - 1.0));
    var rgb: vec3<f32>;
    switch u32(h) {
        case 0u: { rgb = vec3(c, x, 0.0); }
        case 1u: { rgb = vec3(x, c, 0.0); }
        case 2u: { rgb = vec3(0.0, c, x); }
        case 3u: { rgb = vec3(0.0, x, c); }
        case 4u: { rgb = vec3(x, 0.0, c); }
        default: { rgb = vec3(c, 0.0, x); }
    }
    let m = v - c;
    return rgb + vec3(m);
}

// A table grid (`hsv_table::Grid`): table rows start at `row`; `g` = (hues, sats, vals).
struct HsvGrid {
    row: i32,
    h: i32,
    s: i32,
    v: i32,
}

// `hsv_table::Steps::lookup`: table `k` at (h, s, v), trilinear, hue wrapping round.
fn hsv_lookup(g: HsvGrid, k: i32, h: f32, s: f32, v: f32) -> vec3<f32> {
    let fh = clamp(rem_euclid(h, 360.0) / 360.0 * f32(g.h), 0.0, f32(g.h));
    let fs = clamp(s, 0.0, 1.0) * f32(g.s - 1);
    let fv = clamp(v, 0.0, 1.0) * f32(g.v - 1);
    let ih = min(i32(fh), g.h - 1);
    let isat = min(i32(fs), g.s - 2);
    let iv = min(i32(fv), g.v - 2);
    let t = vec3(fh - f32(ih), fs - f32(isat), fv - f32(iv));
    var d = vec3(0.0);
    for (var corner = 0; corner < 8; corner++) {
        let dh = (corner >> 2u) & 1;
        let ds = (corner >> 1u) & 1;
        let dv = corner & 1;
        let w = select(1.0 - t.x, t.x, dh == 1) * select(1.0 - t.y, t.y, ds == 1) * select(1.0 - t.z, t.z, dv == 1);
        let node = (k * g.h * g.s * g.v + (((ih + dh) % g.h) * g.s + isat + ds) * g.v + iv + dv) * 3;
        d += w * vec3(fix_at(g.row, node), fix_at(g.row, node + 1), fix_at(g.row, node + 2));
    }
    return d;
}

// `hsv_table::Steps::apply_encoded`: `kinds` tables one after the other in HSV of linear
// ProPhoto, each at the input's hue and the previous one's saturation and value.
fn hsv_steps(g: HsvGrid, kinds: i32, c: vec3<f32>) -> vec3<f32> {
    let lin = vec3(decode(c.x), decode(c.y), decode(c.z));
    let x = rows_times(P.to_prophoto, lin);
    let hsv = rgb_to_hsv(max(x, vec3(0.0)));
    // `hsv_table::GREY`.
    if hsv.z <= 1e-6 || hsv.y <= 1e-4 {
        return c;
    }
    var h = hsv.x;
    var s = hsv.y;
    var v = hsv.z;
    var changed = false;
    for (var k = 0; k < kinds; k++) {
        let d = hsv_lookup(g, k, hsv.x, s, v);
        if all(d == vec3(0.0)) {
            continue;
        }
        changed = true;
        h += d.x;
        s = clamp(s + d.y, 0.0, 1.0);
        v *= exp(clamp(d.z, -20.0, 20.0));
    }
    if !changed {
        return c;
    }
    let y = hsv_to_rgb(h, s, v);
    let back = rows_times(P.from_prophoto, y);
    let inside = all(c >= vec3(0.0)) && all(c <= vec3(1.0));
    var o = vec3(encode(back.x), encode(back.y), encode(back.z));
    if inside {
        o = clamp(o, vec3(0.0), vec3(1.0));
    }
    return o;
}

// `photo_util::hash01`: 64-bit words as (low, high).
fn mul_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2((p00 & 0xffffu) | (mid << 16u), p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let w = mul_wide(a.x, b.x);
    return vec2(w.x, w.y + a.x * b.y + a.y * b.x);
}

fn shr64(z: vec2<u32>, n: u32) -> vec2<u32> {
    return vec2((z.x >> n) | (z.y << (32u - n)), z.y >> n);
}

fn i64_(v: i32) -> vec2<u32> {
    return vec2(bitcast<u32>(v), select(0u, 0xffffffffu, v < 0));
}

fn hash01(x: i32, y: i32, seed: vec2<u32>) -> f32 {
    var z = mul64(i64_(x), vec2(0x7f4a7c15u, 0x9e3779b9u)) ^ mul64(i64_(y), vec2(0x27d4eb4fu, 0xc2b2ae3du)) ^ mul64(seed, vec2(0x9e3779f9u, 0x165667b1u));
    z = mul64(z ^ shr64(z, 30u), vec2(0x1ce4e5b9u, 0xbf58476du));
    z = mul64(z ^ shr64(z, 27u), vec2(0x133111ebu, 0x94d049bbu));
    z = z ^ shr64(z, 31u);
    return f32(z.y >> 8u) / 16777216.0;
}

fn value_noise(x: f32, y: f32, seed: vec2<u32>) -> f32 {
    let xi = floor(x);
    let yi = floor(y);
    let tx = x - xi;
    let ty = y - yi;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let ix = i32(xi);
    let iy = i32(yi);
    let h00 = hash01(ix, iy, seed);
    let h10 = hash01(ix + 1, iy, seed);
    let h01 = hash01(ix, iy + 1, seed);
    let h11 = hash01(ix + 1, iy + 1, seed);
    let top = h00 + (h10 - h00) * sx;
    let bot = h01 + (h11 - h01) * sx;
    return top + (bot - top) * sy;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let q = textureLoad(src, p, 0);
    var c = q.rgb;
    // 1. White balance, then Exposure and Contrast.
    if on(GAINS) {
        for (var i = 0; i < 3; i++) {
            c[i] = linear_to_srgb(max(srgb_to_linear(c[i]) * P.gains[i], 0.0));
        }
    }
    if on(EXPOSURE) {
        c = tone_pixel(7, c);
    }
    if on(CONTRAST) {
        c = tone_pixel(8, c);
    }
    // 3b. Whites and Blacks.
    if on(WHITES) {
        let input = c;
        c = white_black_pixel(5, on(WHITES_CHANNELS), c);
        if on(WHITES_FIX) {
            c = grid_fix(WHITES_FIX_ROW, input, c);
        }
    }
    if on(BLACKS) {
        c = white_black_pixel(6, on(BLACKS_CHANNELS), c);
    }
    // 4. Vibrance and Saturation: Saturation's table, then Vibrance's.
    if on(PRESENCE) {
        c = hsv_steps(HsvGrid(PRESENCE_ROW, PRESENCE_H, PRESENCE_S, PRESENCE_V), 2, c);
    }
    // 5. Tone curves.
    if on(PARAMETRIC) {
        c = vec3(lut_eval(0, c.x), lut_eval(0, c.y), lut_eval(0, c.z));
    }
    if on(MASTER) {
        c = vec3(lut_eval(1, c.x), lut_eval(1, c.y), lut_eval(1, c.z));
    }
    for (var i = 0; i < 3; i++) {
        if on(RED + u32(i)) {
            c[i] = lut_eval(2 + i, c[i]);
        }
    }
    // 6. Color Mixer.
    if on(HSL) {
        c = hsv_steps(HsvGrid(MIXER_ROW, MIXER_H, MIXER_S, MIXER_V), 3, c);
    }
    // 7. Color Grading.
    if on(GRADE) {
        let l = clamp(luma(c), 0.0, 1.0);
        let blend = P.grade.x;
        let bal = P.grade.y;
        let sh = 1.0 - smoothstep_(0.0, 0.5 + bal + blend, l);
        let hi = smoothstep_(0.5 + bal - blend, 1.0, l);
        let mid = max(1.0 - sh - hi, 0.0);
        let masks = vec4(sh, mid, hi, 1.0);
        for (var k = 0; k < 4; k++) {
            c += masks[k] * P.tints[k].xyz;
            if P.tints[k].w != 0.0 {
                c = c * exp2(masks[k] * P.tints[k].w / 100.0 * 0.6);
            }
        }
    }
    // 9. Vignette.
    if on(VIGNETTE) {
        let amt = P.vignette[0].x;
        let hl = P.vignette[0].y;
        let style = u32(P.vignette[0].z);
        let pw = P.vignette[0].w;
        let centre = P.vignette[1].xy;
        let axes = P.vignette[1].zw;
        let dx = abs((f32(p.x) + 0.5 - centre.x) / axes.x);
        let dy = abs((f32(p.y) + 0.5 - centre.y) / axes.y);
        let r = pow(pow(dx, pw) + pow(dy, pw), 1.0 / pw);
        let m = smoothstep_(P.vignette[2].x, P.vignette[2].y, r);
        if m > 0.0 {
            let l = clamp(luma(c), 0.0, 1.0);
            if style == 2u {
                let goal = select(1.0, 0.0, amt < 0.0);
                let k = m * abs(amt);
                c += (vec3(goal) - c) * k;
            } else {
                var k = exp2(amt * 2.0 * m);
                if amt < 0.0 && hl > 0.0 {
                    let protect = smoothstep_(0.6, 1.0, l) * hl;
                    k += (1.0 - k) * protect;
                }
                if style == 1u {
                    let nl = l * k;
                    c += vec3(nl - l);
                } else {
                    c = c * k;
                }
            }
        }
    }
    // 9. Grain, in full-resolution coordinates.
    if on(GRAIN) {
        let cell = P.grain.x;
        let rough = P.grain.y;
        let fx = f32(p.x) * P.grain.w / cell;
        let fy = f32(p.y) * P.grain.w / cell;
        let n = value_noise(fx, fy, P.seeds.xy) * (1.0 - rough) + (value_noise(fx * 2.3, fy * 2.3, P.seeds.zw) - 0.0) * rough;
        let l = clamp(luma(c), 0.0, 1.0);
        let g = (n - 0.5) * 2.0 * P.grain.z * (0.4 + 2.4 * l * (1.0 - l));
        c += vec3(g);
    }
    if on(CLAMP) {
        c = clamp(c, vec3(0.0), vec3(1.0));
    }
    return vec4(c, q.a);
}
