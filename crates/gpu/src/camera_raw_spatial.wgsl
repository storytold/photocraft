// Camera Raw's spatial stages on the GPU, around the analyses the CPU runs on reduced copies
// (`camera_raw_spatial.rs`): the reductions those analyses read, and the per-pixel passes that
// apply their results. Each function mirrors the CPU code it is named after
// (`photocraft_algo::camera_raw`), in the same order of operations.

struct S {
    // Full width and height, then the output's (the reduced copy, or the full image).
    sizes: vec4<u32>,
    // Block size of a reduction; flags; Highlights' grid width and height.
    grid: vec4<u32>,
    // Shadows' grid width and height.
    grid2: vec4<u32>,
    // The document's transfer curve (0: sRGB's), Dehaze's amount, the colour exponents of
    // Highlights and Shadows.
    k: vec4<f32>,
    // Dehaze's atmospheric light, or negative Dehaze's haze colour (linear).
    air: vec4<f32>,
    // Negative Dehaze's weight quadratic.
    haze: vec4<f32>,
    // The luminance weights of the document's linear RGB.
    lum: vec4<f32>,
}

@group(0) @binding(0) var<uniform> P: S;
@group(0) @binding(1) var src: texture_2d<f32>;
// Highlights' move coefficients (a, b), or Dehaze's transmission (r).
@group(0) @binding(2) var grid_a: texture_2d<f32>;
// Shadows' move coefficients (a, b).
@group(0) @binding(3) var grid_b: texture_2d<f32>;

const HL_ON: u32 = 1u;
const HL_EXACT: u32 = 2u;
const SH_ON: u32 = 4u;
const SH_EXACT: u32 = 8u;
const FLOOR: f32 = 1.0 / 65536.0;
const MAX_MOVE: f32 = 6.0;

struct VOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return VOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0));
}

fn on(flag: u32) -> bool {
    return (P.grid.y & flag) != 0u;
}

fn luma(c: vec3<f32>) -> f32 {
    return 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
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

fn decode3(c: vec3<f32>) -> vec3<f32> {
    return vec3(decode(c.x), decode(c.y), decode(c.z));
}

fn encode3(c: vec3<f32>) -> vec3<f32> {
    return vec3(encode(c.x), encode(c.y), encode(c.z));
}

fn finite(v: f32) -> bool {
    // NaN fails both comparisons; infinities fail the bound.
    return abs(v) <= 3.4e38;
}

// `photo_util::bilinear` on channel `c` of `t` (`size`), edges clamped.
fn bilinear(t: texture_2d<f32>, size: vec2<u32>, c: i32, x: f32, y: f32) -> f32 {
    let xm = f32(size.x - 1u);
    let ym = f32(size.y - 1u);
    let xc = clamp(x, 0.0, xm);
    let yc = clamp(y, 0.0, ym);
    let x0 = i32(floor(xc));
    let y0 = i32(floor(yc));
    let x1 = min(x0 + 1, i32(size.x) - 1);
    let y1 = min(y0 + 1, i32(size.y) - 1);
    let tx = xc - f32(x0);
    let ty = yc - f32(y0);
    let p00 = textureLoad(t, vec2(x0, y0), 0)[c];
    let p10 = textureLoad(t, vec2(x1, y0), 0)[c];
    let p01 = textureLoad(t, vec2(x0, y1), 0)[c];
    let p11 = textureLoad(t, vec2(x1, y1), 0)[c];
    let top = p00 + (p10 - p00) * tx;
    let bot = p01 + (p11 - p01) * tx;
    return top + (bot - top) * ty;
}

// The pixel's block: [start, end) in full-size pixels.
fn block(p: vec2<i32>) -> vec4<i32> {
    let k = i32(P.grid.x);
    let x0 = p.x * k;
    let y0 = p.y * k;
    return vec4(x0, y0, min(x0 + k, i32(P.sizes.x)), min(y0 + k, i32(P.sizes.y)));
}

// `dehaze`'s reduced copy: each block's mean colour, clamped to 0 … 1.
@fragment
fn fs_reduce_dehaze(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = block(vec2<i32>(floor(pos.xy)));
    var sum = vec3(0.0);
    var n = 0.0;
    for (var y = b.y; y < b.w; y++) {
        for (var x = b.x; x < b.z; x++) {
            sum += clamp(textureLoad(src, vec2(x, y), 0).rgb, vec3(0.0), vec3(1.0));
            n += 1.0;
        }
    }
    return vec4(sum / max(n, 1.0), 1.0);
}

// `add_haze`'s reduced copy: each block's sum of linear colour of its visible pixels, and
// their count.
@fragment
fn fs_reduce_haze(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = block(vec2<i32>(floor(pos.xy)));
    var sum = vec3(0.0);
    var n = 0.0;
    for (var y = b.y; y < b.w; y++) {
        for (var x = b.x; x < b.z; x++) {
            let q = textureLoad(src, vec2(x, y), 0);
            if q.a > 0.0 && finite(q.r) && finite(q.g) && finite(q.b) {
                sum += decode3(clamp(q.rgb, vec3(0.0), vec3(1.0)));
                n += 1.0;
            }
        }
    }
    return vec4(sum, n);
}

// The linear luminance `local_tone` reads.
fn tone_lum(q: vec3<f32>) -> f32 {
    return max(luma(decode3(q)), 0.0);
}

// `ToneMove::at` for the grid in `t` (`size`), at full-size pixel `p` with log2 luminance `u`.
fn tone_at(t: texture_2d<f32>, size: vec2<u32>, exact: bool, p: vec2<i32>, u: f32) -> f32 {
    if exact {
        return textureLoad(t, p, 0).g;
    }
    let fx = f32(size.x) / f32(P.sizes.x);
    let fy = f32(size.y) / f32(P.sizes.y);
    let sx = (f32(p.x) + 0.5) * fx - 0.5;
    let sy = (f32(p.y) + 0.5) * fy - 0.5;
    let a = bilinear(t, size, 0, sx, sy);
    let b = bilinear(t, size, 1, sx, sy);
    return clamp(a * u + b, -MAX_MOVE, MAX_MOVE);
}

// Highlights' move at `p`, 0 when it is off.
fn highlights_at(p: vec2<i32>, lum: f32) -> f32 {
    if !on(HL_ON) {
        return 0.0;
    }
    return tone_at(grid_a, P.grid.zw, on(HL_EXACT), p, log2(lum + FLOOR));
}

// `local_tone::shrink_linear`: the block's mean linear luminance (`after`: after Highlights'
// move, for Shadows).
fn reduce_luma(pos: vec4<f32>, after: bool) -> vec4<f32> {
    let b = block(vec2<i32>(floor(pos.xy)));
    var sum = 0.0;
    var n = 0.0;
    for (var y = b.y; y < b.w; y++) {
        for (var x = b.x; x < b.z; x++) {
            var l = tone_lum(textureLoad(src, vec2(x, y), 0).rgb);
            if after {
                l *= exp2(highlights_at(vec2(x, y), l));
            }
            sum += l;
            n += 1.0;
        }
    }
    return vec4(sum / max(n, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn fs_reduce_luma(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return reduce_luma(pos, false);
}

@fragment
fn fs_reduce_luma_after(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return reduce_luma(pos, true);
}

// `tone_points`' cells: each block's sum of luminance (the document's weights) of its visible
// pixels, and their count.
@fragment
fn fs_reduce_cells(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = block(vec2<i32>(floor(pos.xy)));
    var sum = 0.0;
    var n = 0.0;
    for (var y = b.y; y < b.w; y++) {
        for (var x = b.x; x < b.z; x++) {
            let q = textureLoad(src, vec2(x, y), 0);
            if q.a > 0.0 {
                sum += max(dot(P.lum.xyz, decode3(q.rgb)), 0.0);
                n += 1.0;
            }
        }
    }
    return vec4(sum, n, 0.0, 1.0);
}

// `dehaze_pixel` with the transmission `DehazePlan::at` finds there.
@fragment
fn fs_apply_dehaze(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let q = textureLoad(src, p, 0);
    let k = f32(P.grid.x);
    let tv = clamp(bilinear(grid_a, P.grid.zw, 0, (f32(p.x) + 0.5) / k - 0.5, (f32(p.y) + 0.5) / k - 0.5), 0.1, 1.0);
    let amount = P.k.y;
    var o = vec3(0.0);
    for (var c = 0; c < 3; c++) {
        let i = q[c];
        let j = (i - P.air[c]) / tv + P.air[c];
        if amount >= 0.0 {
            o[c] = i + (j - i) * amount;
        } else {
            o[c] = i + (P.air[c] - i) * (-amount) * 0.5 * (1.0 - tv * 0.5);
        }
    }
    return vec4(o, q.a);
}

// `add_haze_pixel`, in linear light.
@fragment
fn fs_apply_haze(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = textureLoad(src, vec2<i32>(floor(pos.xy)), 0);
    let lin = decode3(q.rgb);
    let y = clamp(luma(lin), 0.0, 1.0);
    let wt = clamp(P.haze.x + P.haze.y * y + P.haze.z * y * y, 0.0, 1.0);
    return vec4(encode3(lin * (1.0 - wt) + P.air.rgb * wt), q.a);
}

// `local_tone::apply`'s last step: Highlights' then Shadows' moves and `tone_pixel`.
@fragment
fn fs_apply_tone(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    let q = textureLoad(src, p, 0);
    let lin = decode3(q.rgb);
    let lum = max(luma(lin), 0.0);
    let a = highlights_at(p, lum);
    var b = 0.0;
    if on(SH_ON) {
        b = tone_at(grid_b, P.grid2.xy, on(SH_EXACT), p, log2(lum * exp2(a) + FLOOR));
    }
    let gain = exp2(a + b);
    let l = luma(lin);
    let e = P.k.z * a + P.k.w * b;
    var out = lin * gain;
    if e != 0.0 && l > 1e-6 && all(lin >= vec3(0.0)) {
        let beta = exp2(-e);
        // WGSL's pow is undefined at a zero base, where the CPU's powf gives 0.
        let r = lin / l;
        let o = l * select(pow(r, vec3(beta)), vec3(0.0), r <= vec3(0.0));
        let lo = luma(o);
        if lo > 1e-9 {
            out = o * (l * gain / lo);
        }
    }
    return vec4(encode3(out), q.a);
}
