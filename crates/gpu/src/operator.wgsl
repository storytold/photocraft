// Layer operators (operator.rs, #5772): compute passes recorded into the compositor's encoder.
// Windows hold straight RGBA (the page texels' form) with alpha in `.w`; blur passes run
// premultiplied, as the CPU blur does.
//
// No dynamic vector indexing anywhere: FXC (Direct3D 12) can't assign to a vector component
// chosen at run time.

struct Pass {
    // Window size in pixels.
    size: vec2<u32>,
    // Kernel half-length, or the box radius.
    radius: u32,
    // 0: along rows, 1: along columns.
    axis: u32,
    // Box passes: outputs per invocation.
    segment: u32,
    // 1: premultiply the samples read (the first pass).
    premultiply: u32,
    // 1: unpremultiply the results written (the last pass).
    unpremultiply: u32,
    // Box passes: 1 / width, as the CPU computes it.
    norm: f32,
    // Where the pass's window starts in `src` (the first pass reads part of a cell's window).
    origin: vec2<u32>,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> p: Pass;
@group(0) @binding(3) var<storage, read> weights: array<f32>;

// The texel `i` along line `line`, clamped to the window (whose edge texels already repeat the
// source's edge beyond its extent).
fn texel(line: u32, i: i32) -> vec2<i32> {
    let len = select(p.size.y, p.size.x, p.axis == 0u);
    let j = clamp(i, 0, i32(len) - 1);
    return select(vec2<i32>(i32(line), j), vec2<i32>(j, i32(line)), p.axis == 0u);
}

fn load(at: vec2<i32>) -> vec4<f32> {
    let v = textureLoad(src, at + vec2<i32>(p.origin), 0);
    if p.premultiply == 0u {
        return v;
    }
    return vec4<f32>(v.xyz * v.w, v.w);
}

fn store(at: vec2<i32>, v: vec4<f32>) {
    var o = v;
    if p.unpremultiply != 0u {
        // As `unpremultiply` on the CPU: the colour of a (nearly) transparent pixel is zero.
        if v.w > 1e-7 {
            o = vec4<f32>(v.xyz / v.w, v.w);
        } else {
            o = vec4<f32>(0.0, 0.0, 0.0, v.w);
        }
    }
    textureStore(dst, at, o);
}

// One pass of the kernel in `weights` per pixel, summed in the CPU's tap order.
@compute @workgroup_size(8, 8)
fn kernel_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.size.x || g.y >= p.size.y {
        return;
    }
    let line = select(g.x, g.y, p.axis == 0u);
    let i = i32(select(g.y, g.x, p.axis == 0u));
    let r = i32(p.radius);
    var acc = vec4<f32>(0.0);
    for (var k = 0; k <= 2 * r; k++) {
        acc += load(texel(line, i + k - r)) * weights[k];
    }
    store(vec2<i32>(g.xy), acc);
}

// One running-mean box pass of radius `p.radius` over `p.segment` outputs of one line: the window
// sum is taken once, then slides, adding the sample entering and removing the one leaving.
@compute @workgroup_size(64)
fn box_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    let lines = select(p.size.x, p.size.y, p.axis == 0u);
    let len = select(p.size.y, p.size.x, p.axis == 0u);
    let line = g.x;
    let start = g.y * p.segment;
    if line >= lines || start >= len {
        return;
    }
    let end = min(start + p.segment, len);
    let r = i32(p.radius);
    var acc = vec4<f32>(0.0);
    for (var k = i32(start) - r; k <= i32(start) + r; k++) {
        acc += load(texel(line, k));
    }
    for (var i = start; i < end; i++) {
        store(texel(line, i32(i)), acc * p.norm);
        acc += load(texel(line, i32(i) + r + 1)) - load(texel(line, i32(i) - r));
    }
}

// The extreme (Minimum, or Maximum when `p.norm` is 1) of the straight values within
// `p.radius` texels each side along the pass's axis, per channel, alpha included
// (`other::min_max`).
@compute @workgroup_size(8, 8)
fn extreme_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.size.x || g.y >= p.size.y {
        return;
    }
    let line = select(g.x, g.y, p.axis == 0u);
    let i = i32(select(g.y, g.x, p.axis == 0u));
    let r = i32(p.radius);
    var acc = load(texel(line, i - r));
    for (var k = 1; k <= 2 * r; k++) {
        let v = load(texel(line, i + k - r));
        acc = select(min(acc, v), max(acc, v), p.norm > 0.5);
    }
    store(vec2<i32>(g.xy), acc);
}

// The texel at offset `d` from `at`, clamped to the window.
fn texel2(at: vec2<i32>, d: vec2<i32>) -> vec2<i32> {
    return clamp(at + d, vec2<i32>(0), vec2<i32>(p.size) - vec2<i32>(1));
}

// ---- Minimum / Maximum over a disc (`other::min_max_round`), one step per half-width h ----
// `h_in` is the horizontal extreme over 2h + 1 texels; the step folds the disc's rows of
// half-width h (`rows[d.start ..]`) into the result and, unless it is the last, grows the
// horizontal extreme to 2h + 3 texels.

struct DiscStep {
    size: vec2<u32>,
    // Where the window starts in `h_in` (the first step reads part of a cell's source window).
    origin: vec2<u32>,
    start: u32,
    count: u32,
    // 1: the first step; 2: write the next horizontal extreme.
    flags: u32,
    // 1: Maximum, 0: Minimum.
    maximum: u32,
}

@group(0) @binding(0) var h_in: texture_2d<f32>;
@group(0) @binding(1) var acc_in: texture_2d<f32>;
@group(0) @binding(2) var h_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var acc_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var<uniform> d: DiscStep;
@group(0) @binding(5) var<storage, read> rows: array<i32>;

fn pick(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return select(min(a, b), max(a, b), d.maximum != 0u);
}

// The horizontal extreme so far at `at`, clamped to the window.
fn hload(at: vec2<i32>) -> vec4<f32> {
    let c = clamp(at, vec2<i32>(0), vec2<i32>(d.size) - vec2<i32>(1));
    return textureLoad(h_in, c + vec2<i32>(d.origin), 0);
}

@compute @workgroup_size(8, 8)
fn disc_step(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= d.size.x || g.y >= d.size.y {
        return;
    }
    let at = vec2<i32>(g.xy);
    let first = (d.flags & 1u) != 0u;
    // The extreme's identity: every straight value lies inside it.
    var acc = select(vec4<f32>(3.0e38), vec4<f32>(-3.0e38), d.maximum != 0u);
    if !first {
        acc = textureLoad(acc_in, at, 0);
    }
    for (var k = 0u; k < d.count; k++) {
        acc = pick(acc, hload(at + vec2<i32>(0, rows[d.start + k])));
    }
    textureStore(acc_out, at, acc);
    if (d.flags & 2u) != 0u {
        var grown = pick(hload(at - vec2<i32>(1, 0)), hload(at + vec2<i32>(1, 0)));
        if first {
            grown = pick(grown, hload(at));
        }
        textureStore(h_out, at, grown);
    }
}

// Custom (`other2::custom`): the straight colour values convolved by the taps in
// `weights[2 + 3k ..]` (dx, dy, weight) in order, then `/ scale + offset` (`weights[0]`,
// `weights[1]`); alpha stays the texel's own.
@compute @workgroup_size(8, 8)
fn convolve_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= p.size.x || g.y >= p.size.y {
        return;
    }
    let at = vec2<i32>(g.xy);
    let here = load(texel2(at, vec2<i32>(0, 0)));
    if p.radius == 0u {
        store(at, here);
        return;
    }
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < p.radius; k++) {
        let d = vec2<i32>(i32(weights[2u + 3u * k]), i32(weights[3u + 3u * k]));
        acc += load(texel2(at, d)).xyz * weights[4u + 3u * k];
    }
    store(at, vec4<f32>(acc / weights[0] + vec3<f32>(weights[1]), here.w));
}

// ---- The distort filters (`distort::remap`): each pixel samples the source at the point its
// mapping gives (Displace's from a map), with the CPU's arithmetic ----

struct RemapPass {
    // The output, in document pixels: its size and where it starts.
    size: vec2<u32>,
    out_origin: vec2<i32>,
    // The source window: where it starts in document pixels, and its size.
    win_origin: vec2<i32>,
    win_size: vec2<u32>,
    // The filter's bounds (x0, y0, x1, y1), document pixels.
    bounds: vec4<i32>,
    // Displace's map size (1×1 for the other mappings).
    map_size: vec2<u32>,
    _pad0: vec2<u32>,
    _pad1: u32,
    // Past the bounds: 0 Transparent, 1 Repeat Edge Pixels, 2 Wrap Around.
    edge: u32,
    has_alpha: u32,
    // 0 Displace, 1 Twirl, 2 Pinch, 3 Spherize, 4 Wave, 5 Ripple, 6 Polar Coordinates, 7 ZigZag,
    // 8 Shear (operator.rs `Mapping::uniforms`).
    kind: u32,
    p: vec4<f32>,
    e: vec4<u32>,
}

@group(0) @binding(0) var dsrc: texture_2d<f32>;
@group(0) @binding(1) var dmap: texture_2d<f32>;
@group(0) @binding(2) var dout: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var<uniform> q: RemapPass;
@group(0) @binding(4) var<storage, read> rdata: array<f32>;

const TAU: f32 = 6.2831855;
const PI: f32 = 3.1415927;

// `x` modulo 2π in [0, 2π), with the multiple of 2π taken in two parts (Cody–Waite) so a large
// argument (Wave's and Ripple's phases run to thousands of radians across a big layer) loses no
// accuracy: the CPU's `f32::sin` and `rem_euclid` reduce exactly, a GPU's built-in `sin` doesn't.
fn reduce(x: f32) -> f32 {
    let k = floor(x / TAU);
    var r = (x - k * 6.28125) - k * 0.0019353072;
    if r < 0.0 {
        r += TAU;
    }
    if r >= TAU {
        r -= TAU;
    }
    return r;
}

fn sin_r(x: f32) -> f32 {
    return sin(reduce(x));
}

fn cos_r(x: f32) -> f32 {
    return cos(reduce(x));
}

// The map's red and green at map texel (x, y), clamped to the map.
fn map_texel(x: i32, y: i32) -> vec2<f32> {
    let c = clamp(vec2<i32>(x, y), vec2<i32>(0), vec2<i32>(q.map_size) - vec2<i32>(1));
    return textureLoad(dmap, c, 0).xy;
}

// `distort2::map_sample`: bilinear, pixel centres at .5, as the CPU interpolates.
fn map_sample(u: f32, v: f32) -> vec2<f32> {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let ax = fx - x0;
    let ay = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let g00 = map_texel(xi, yi);
    let g10 = map_texel(xi + 1, yi);
    let g01 = map_texel(xi, yi + 1);
    let g11 = map_texel(xi + 1, yi + 1);
    let top = g00 + (g10 - g00) * ax;
    let bot = g01 + (g11 - g01) * ax;
    return top + (bot - top) * ay;
}

fn rem_euclid_i(v: i32, m: i32) -> i32 {
    let r = v % m;
    return select(r, r + m, r < 0);
}

// `Image::get_edge`: the source pixel at document (x, y), straight; past the bounds the
// Undefined Areas rule.
fn src_pixel(x: i32, y: i32) -> vec4<f32> {
    let b = q.bounds;
    var at = vec2<i32>(x, y);
    let inside = x >= b.x && y >= b.y && x < b.z && y < b.w;
    if !inside {
        if q.edge == 0u {
            return vec4<f32>(0.0);
        } else if q.edge == 1u {
            at = clamp(at, b.xy, b.zw - vec2<i32>(1));
        } else {
            at = b.xy + vec2<i32>(rem_euclid_i(x - b.x, max(b.z - b.x, 1)), rem_euclid_i(y - b.y, max(b.w - b.y, 1)));
        }
    }
    let t = clamp(at - q.win_origin, vec2<i32>(0), vec2<i32>(q.win_size) - vec2<i32>(1));
    return textureLoad(dsrc, t, 0);
}

// `Image::sample`: bilinear of the premultiplied pixels (centres at .5), back to straight.
fn src_sample(x: f32, y: f32) -> vec4<f32> {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let ax = fx - x0;
    let ay = fy - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    var acc = vec3<f32>(0.0);
    var acc_a = 0.0;
    let w = array<f32, 4>((1.0 - ax) * (1.0 - ay), ax * (1.0 - ay), (1.0 - ax) * ay, ax * ay);
    let dx = array<i32, 4>(0, 1, 0, 1);
    let dy = array<i32, 4>(0, 0, 1, 1);
    for (var k = 0; k < 4; k++) {
        let wgt = w[k];
        if wgt <= 0.0 {
            continue;
        }
        let p = src_pixel(xi + dx[k], yi + dy[k]);
        let a = select(1.0, p.w, q.has_alpha != 0u);
        acc += p.xyz * a * wgt;
        acc_a += a * wgt;
    }
    if q.has_alpha == 0u {
        return vec4<f32>(acc, 1.0);
    }
    if acc_a > 0.0 {
        acc = acc / acc_a;
    }
    return vec4<f32>(acc, acc_a);
}

fn rem_euclid_f(v: f32, m: f32) -> f32 {
    let r = v - m * trunc(v / m);
    return select(r, r + abs(m), r < 0.0);
}

// `distort::sphere_1d`.
fn sphere_1d(rn: f32, a: f32) -> f32 {
    var aim = sin(rn * (PI / 2.0));
    if a >= 0.0 {
        aim = asin(rn) * 2.0 / PI;
    }
    return rn + abs(a) * (aim - rn);
}

// `distort::shape`: one wave of `t` (0 Sine, 1 Triangle, 2 Square) at `phase`.
fn wave_shape(t: u32, phase: f32) -> f32 {
    let p = reduce(phase) / TAU;
    if t == 0u {
        return sin_r(phase);
    } else if t == 1u {
        return 1.0 - 4.0 * abs(p - 0.5);
    }
    return select(-1.0, 1.0, p < 0.5);
}

// The point output pixel centre (x, y) samples, per mapping.
fn mapped(x: f32, y: f32) -> vec2<f32> {
    let b = q.bounds;
    let w = f32(b.z - b.x);
    let h = f32(b.w - b.y);
    // `distort::ellipse`.
    let cx = f32(b.x) + w / 2.0;
    let cy = f32(b.y) + h / 2.0;
    let hx = w / 2.0;
    let hy = h / 2.0;
    let lx = x - f32(b.x);
    let ly = y - f32(b.y);
    switch q.kind {
        case 0u: {
            // Displace (`distort2::displace`).
            let mw = f32(q.map_size.x);
            let mh = f32(q.map_size.y);
            var u = rem_euclid_f(lx, mw);
            var v = rem_euclid_f(ly, mh);
            if q.e.x != 0u {
                u = lx / f32(max(b.z - b.x, 1)) * mw;
                v = ly / f32(max(b.w - b.y, 1)) * mh;
            }
            let m = map_sample(u, v);
            let shift = (m - vec2<f32>(0.5)) * 2.0 * q.p.xy;
            return vec2<f32>(x, y) + shift;
        }
        case 1u: {
            // Twirl.
            if hx <= 0.0 || hy <= 0.0 {
                return vec2<f32>(x, y);
            }
            let u = (x - cx) / hx;
            let v = (y - cy) / hy;
            let r = sqrt(u * u + v * v);
            if r >= 1.0 {
                return vec2<f32>(x, y);
            }
            let t = q.p.x * (1.0 - r);
            let s = sin_r(t);
            let c = cos_r(t);
            return vec2<f32>(cx + (u * c - v * s) * hx, cy + (u * s + v * c) * hy);
        }
        case 2u: {
            // Pinch.
            if hx <= 0.0 || hy <= 0.0 {
                return vec2<f32>(x, y);
            }
            let u = (x - cx) / hx;
            let v = (y - cy) / hy;
            let r = sqrt(u * u + v * v);
            if r >= 1.0 || r <= 0.0 {
                return vec2<f32>(x, y);
            }
            let k = pow(r, 1.0 - q.p.x * 0.5) / r;
            return vec2<f32>(cx + u * k * hx, cy + v * k * hy);
        }
        case 3u: {
            // Spherize.
            let a = q.p.x;
            let dx = x - cx;
            let dy = y - cy;
            if q.e.x == 1u {
                if abs(dx) >= hx {
                    return vec2<f32>(x, y);
                }
                return vec2<f32>(cx + sign(dx) * hx * sphere_1d(abs(dx) / hx, a), y);
            }
            if q.e.x == 2u {
                if abs(dy) >= hy {
                    return vec2<f32>(x, y);
                }
                return vec2<f32>(x, cy + sign(dy) * hy * sphere_1d(abs(dy) / hy, a));
            }
            if hx <= 0.0 || hy <= 0.0 {
                return vec2<f32>(x, y);
            }
            let u = dx / hx;
            let v = dy / hy;
            let r = sqrt(u * u + v * v);
            if r >= 1.0 || r <= 0.0 {
                return vec2<f32>(x, y);
            }
            let k = sphere_1d(r, a) / r;
            return vec2<f32>(cx + u * k * hx, cy + v * k * hy);
        }
        case 4u: {
            // Wave: the generators' mean.
            var d = vec2<f32>(0.0);
            let n = q.e.x;
            for (var g = 0u; g < n; g++) {
                let len = rdata[4u * g];
                let amp = rdata[4u * g + 1u];
                d.x += amp * wave_shape(q.e.y, TAU * ly / len + rdata[4u * g + 2u]);
                d.y += amp * wave_shape(q.e.y, TAU * lx / len + rdata[4u * g + 3u]);
            }
            return vec2<f32>(x + d.x / f32(n), y + d.y / f32(n));
        }
        case 5u: {
            // Ripple.
            let amp = q.p.x;
            let len = q.p.y;
            return vec2<f32>(x + amp * sin_r(TAU * ly / len), y + amp * sin_r(TAU * lx / len));
        }
        case 6u: {
            // Polar Coordinates (`distort::polar_map`, at pixel corners).
            let pw = f32(max(b.z - b.x, 1));
            let ph = f32(max(b.w - b.y, 1));
            let pcx = f32(b.x) + pw / 2.0;
            let pcy = f32(b.y) + ph / 2.0;
            let rmax = min(pw, ph) / 2.0;
            let px = x - 0.5;
            let py = y - 0.5;
            var s = vec2<f32>(0.0);
            if q.e.x != 0u {
                let dx = px - pcx;
                let dy = py - pcy;
                var theta = atan2(-dx, -dy);
                if theta <= 0.0 {
                    theta += TAU;
                }
                let r = sqrt(dx * dx + dy * dy);
                s = vec2<f32>(f32(b.x) + theta / TAU * pw, f32(b.y) + r / rmax * ph);
            } else {
                let theta = (px + 1.0 - f32(b.x)) / pw * TAU;
                let r = (py - f32(b.y)) / ph * rmax;
                s = vec2<f32>(pcx - r * sin(theta), pcy - r * cos(theta));
            }
            return s + vec2<f32>(0.5);
        }
        case 7u: {
            // ZigZag (`distort2::zigzag`, in the bounds' inscribed circle).
            let rr = min(w, h) / 2.0;
            let a = q.p.x;
            let k = q.p.y;
            let dx = x - cx;
            let dy = y - cy;
            let r = sqrt(dx * dx + dy * dy);
            if r >= rr || r <= 0.0 || a == 0.0 {
                return vec2<f32>(x, y);
            }
            let rn = r / rr;
            let wave = sin_r(TAU * k * rn) * (1.0 - rn);
            var radial = 0.0;
            var angular = 0.0;
            if q.e.x == 1u {
                radial = a * wave * rr / (4.0 * k);
            } else if q.e.x == 0u {
                angular = a * wave * PI / (2.0 * k);
            } else {
                radial = a * wave * rr / (8.0 * k);
                angular = a * wave * PI / (4.0 * k);
            }
            let th = atan2(dy, dx) + angular;
            let rs = max(r + radial, 0.0);
            return vec2<f32>(cx + rs * cos(th), cy + rs * sin(th));
        }
        default: {
            // Shear: the row's offset.
            let n = i32(q.e.x);
            let i = clamp(i32(y - 0.5) - b.y, 0, n - 1);
            return vec2<f32>(x - rdata[i], y);
        }
    }
}

@compute @workgroup_size(8, 8)
fn remap_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= q.size.x || g.y >= q.size.y {
        return;
    }
    let x = f32(q.out_origin.x + i32(g.x)) + 0.5;
    let y = f32(q.out_origin.y + i32(g.y)) + 0.5;
    let at = mapped(x, y);
    textureStore(dout, vec2<i32>(g.xy), src_sample(at.x, at.y));
}

// ---- Lens Blur (`blur2::lens_blur`) ----
// `lens_prefix`: per window row, the running sums of the premultiplied pixels (highlights above
// the threshold boosted first), one column wider than the window. `lens_mean`: per output pixel,
// the iris for its depth as the sum of its rows' prefix differences, blended between two depth
// levels, back to straight, with noise.

struct LensPass {
    // The window: its size, and where it starts in `lsrc`.
    size: vec2<u32>,
    origin: vec2<u32>,
    // The output area inside the window, window pixels: where it starts, and its size.
    out_origin: vec2<u32>,
    out_size: vec2<u32>,
    // The window's first pixel in document pixels (the noise is seeded by document position).
    doc_origin: vec2<i32>,
    levels: u32,
    // 0 none, 1 the layer's transparency, 2 the depth map in `ldepth`.
    depth: u32,
    boost: f32,
    threshold: f32,
    focal: f32,
    noise: f32,
    // 1 alpha, 2 Grayscale, 4 Gaussian noise, 8 monochromatic, 16 invert the depth.
    flags: u32,
    seed: u32,
    // Where the area summed starts in the result (it is written in window pixels).
    store_offset: vec2<u32>,
}

@group(0) @binding(0) var lsrc: texture_2d<f32>;
@group(0) @binding(1) var lprefix: texture_2d<f32>;
@group(0) @binding(2) var ldepth: texture_2d<f32>;
@group(0) @binding(3) var lout: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var<uniform> lp: LensPass;
@group(0) @binding(5) var<storage, read> lspans: array<i32>;

fn lens_flag(bit: u32) -> bool {
    return (lp.flags & bit) != 0u;
}

// `fxutil::smoothstep`.
fn smooth01(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return select(0.0, 1.0, x >= e1);
    }
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The window pixel at `at`, premultiplied, its highlight boosted (as `lens_blur` does before
// blurring).
fn lens_pixel(at: vec2<i32>) -> vec4<f32> {
    let v = textureLoad(lsrc, at + vec2<i32>(lp.origin), 0);
    let alpha = lens_flag(1u);
    let a = select(1.0, v.w, alpha);
    var p = vec4<f32>(select(v.xyz, v.xyz * a, alpha), select(v.w, a, alpha));
    if lp.boost > 0.0 && a > 0.0 {
        // The straight colour as the CPU recovers it from the premultiplied one.
        let st = select(p.xyz, p.xyz / a, alpha);
        var l = 0.299 * st.x + 0.587 * st.y + 0.114 * st.z;
        if lens_flag(2u) {
            l = st.x;
        }
        let k = 1.0 + lp.boost * smooth01(lp.threshold, lp.threshold + 0.02, l);
        p = vec4<f32>(p.xyz * k, p.w);
    }
    return p;
}

// Each row's chunks' sums, scanned into running sums for the chunks after them.
var<workgroup> lens_chunks: array<vec4<f32>, 128>;

// One workgroup per row: each invocation sums its chunk of the row, the chunks' sums are scanned
// (in log steps), then each writes its chunk's running sums from the chunks before it.
@compute @workgroup_size(128)
fn lens_prefix(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let y = wg.x;
    if y >= lp.size.y {
        return;
    }
    let per = (lp.size.x + 127u) / 128u;
    let x0 = min(li * per, lp.size.x);
    let x1 = min(x0 + per, lp.size.x);
    var acc = vec4<f32>(0.0);
    for (var x = x0; x < x1; x++) {
        acc += lens_pixel(vec2<i32>(i32(x), i32(y)));
    }
    lens_chunks[li] = acc;
    for (var step = 1u; step < 128u; step *= 2u) {
        workgroupBarrier();
        var v = lens_chunks[li];
        if li >= step {
            v += lens_chunks[li - step];
        }
        workgroupBarrier();
        lens_chunks[li] = v;
    }
    workgroupBarrier();
    acc = vec4<f32>(0.0);
    if li > 0u {
        acc = lens_chunks[li - 1u];
    } else {
        textureStore(lout, vec2<i32>(0, i32(y)), acc);
    }
    for (var x = x0; x < x1; x++) {
        acc += lens_pixel(vec2<i32>(i32(x), i32(y)));
        textureStore(lout, vec2<i32>(i32(x) + 1, i32(y)), acc);
    }
}

// `Prefix::mean`: the mean over iris level `level` at window pixel (x, y).
fn lens_level_mean(level: u32, x: i32, y: i32) -> vec4<f32> {
    let start = u32(lspans[level]);
    let end = u32(lspans[level + 1u]);
    let count = bitcast<f32>(lspans[lp.levels + 1u + level]);
    let base = 2u * lp.levels + 1u;
    let w = i32(lp.size.x);
    var acc = vec4<f32>(0.0);
    for (var k = start; k < end; k++) {
        let dy = lspans[base + 3u * k];
        let a = lspans[base + 3u * k + 1u];
        let b = lspans[base + 3u * k + 2u];
        let yy = y + dy;
        if yy < 0 || yy >= i32(lp.size.y) {
            continue;
        }
        let x0 = clamp(x + a, 0, w);
        let x1 = clamp(x + b + 1, 0, w);
        acc += textureLoad(lprefix, vec2<i32>(x1, yy), 0) - textureLoad(lprefix, vec2<i32>(x0, yy), 0);
    }
    return acc / count;
}

// `noise::hash01`.
fn hash01(x: i32, y: i32, c: u32, seed: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8163841u) ^ (c * 0xcb1ab31fu) ^ (seed * 0x9e3779b9u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// The noise for channel `c` at document (x, y).
fn lens_noise(x: i32, y: i32, c: u32) -> f32 {
    let ch = select(c, 0u, lens_flag(8u));
    if lens_flag(4u) {
        let u1 = max(hash01(x, y, ch * 2u + 301u, lp.seed), 1e-7);
        let u2 = hash01(x, y, ch * 2u + 302u, lp.seed);
        return sqrt(-2.0 * log(u1)) * cos(6.2831855 * u2) * lp.noise * 0.5;
    }
    return (hash01(x, y, ch + 200u, lp.seed) - 0.5) * 2.0 * lp.noise;
}

@compute @workgroup_size(8, 8)
fn lens_mean(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= lp.out_size.x || g.y >= lp.out_size.y {
        return;
    }
    let at = vec2<i32>(lp.out_origin + g.xy);
    let levels = lp.levels - 1u;
    var level = f32(levels);
    if lp.depth != 0u {
        var d = 0.0;
        if lp.depth == 1u {
            d = select(0.0, textureLoad(lsrc, at + vec2<i32>(lp.origin), 0).w, lens_flag(1u));
        } else {
            d = textureLoad(ldepth, vec2<i32>(g.xy), 0).x;
        }
        if lens_flag(16u) {
            d = 1.0 - d;
        }
        // No alpha to read: the whole iris, as the CPU's `None`.
        if lp.depth == 1u && !lens_flag(1u) {
            level = f32(levels);
        } else {
            level = min(abs(d - lp.focal) * f32(levels), f32(levels));
        }
    }
    let lo = u32(floor(level));
    let f = level - f32(lo);
    var acc = lens_level_mean(lo, at.x, at.y);
    if f > 1e-4 && lo < levels {
        let acc2 = lens_level_mean(lo + 1u, at.x, at.y);
        acc += (acc2 - acc) * f;
    }
    var px = acc;
    let alpha = lens_flag(1u);
    if alpha {
        px = vec4<f32>(select(vec3<f32>(0.0), px.xyz / px.w, px.w > 1e-7), px.w);
    } else {
        px.w = 1.0;
    }
    if lp.noise > 0.0 && (!alpha || px.w > 0.0) {
        let x = lp.doc_origin.x + at.x;
        let y = lp.doc_origin.y + at.y;
        if lens_flag(2u) {
            let n = lens_noise(x, y, 0u);
            px = vec4<f32>(px.xyz + vec3<f32>(n), px.w);
        } else {
            px = vec4<f32>(px.x + lens_noise(x, y, 0u), px.y + lens_noise(x, y, 1u), px.z + lens_noise(x, y, 2u), px.w);
        }
    }
    textureStore(lout, at + vec2<i32>(lp.store_offset), px);
}

// ---- Apply Image (`channel_cmds::apply_image_pixel`) ----

struct ApplyPass {
    // The output, and where it starts in `asrc` (the inputs start at the output).
    size: vec2<u32>,
    origin: vec2<u32>,
    // `plan::mode_index`, or -1 Add, -2 Subtract.
    blend: i32,
    // 1 alpha, 2 Preserve Transparency, 4 a weight.
    flags: u32,
    colors: u32,
    opacity: f32,
    scale: f32,
    offset: f32,
    _pad: vec2<u32>,
}

@group(0) @binding(0) var asrc: texture_2d<f32>;
@group(0) @binding(1) var ainput: texture_2d<f32>;
@group(0) @binding(2) var aweight: texture_2d<f32>;
@group(0) @binding(3) var aout: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var<uniform> ap: ApplyPass;

// `photocraft_color::blend` (`blend_channel`: integer depths, Linear Dodge and Divide clip at 1).
fn pc_color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        return 1.0;
    }
    if cs <= 0.0 {
        return 0.0;
    }
    return 1.0 - min((1.0 - cb) / cs, 1.0);
}

fn pc_color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        return 0.0;
    }
    if cs >= 1.0 {
        return 1.0;
    }
    return min(cb / (1.0 - cs), 1.0);
}

fn pc_hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        return cb * 2.0 * cs;
    }
    let s = 2.0 * cs - 1.0;
    return cb + s - cb * s;
}

fn pc_soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        return 2.0 * cb * cs + cb * cb * (1.0 - 2.0 * cs);
    }
    let b = max(cb, 0.0);
    var d = sqrt(b);
    if b <= 0.25 {
        d = ((16.0 * b - 12.0) * b + 4.0) * b;
    }
    return 2.0 * cb * (1.0 - cs) + d * (2.0 * cs - 1.0);
}

fn pc_vivid_light_hard_mix(cb: f32, cs: f32) -> f32 {
    let edge = 1e-4;
    if cs <= 0.5 {
        if cb >= 1.0 - edge {
            return 1.0;
        }
        if cs <= 0.0 {
            return 0.0;
        }
        return 1.0 - min((1.0 - cb) / (2.0 * cs), 1.0);
    }
    if cb <= edge {
        return 0.0;
    }
    if cs >= 1.0 {
        return 1.0;
    }
    return min(cb / (2.0 * (1.0 - cs)), 1.0);
}

fn pc_blend(mode: i32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case 3: { return min(cb, cs); }
        case 4: { return cb * cs; }
        case 5: { return pc_color_burn(cb, cs); }
        case 6: { return max(cb + cs - 1.0, 0.0); }
        case 8: { return max(cb, cs); }
        case 9: { return cb + cs - cb * cs; }
        case 10: { return pc_color_dodge(cb, cs); }
        case 11: { return min(cb + cs, 1.0); }
        case 13: { return pc_hard_light(cs, cb); }
        case 14: { return pc_soft_light(cb, cs); }
        case 15: { return pc_hard_light(cb, cs); }
        case 16: {
            if cs <= 0.0 {
                return 0.0;
            }
            if cs >= 1.0 {
                return 1.0;
            }
            if cs <= 0.5 {
                return pc_color_burn(cb, 2.0 * cs);
            }
            return pc_color_dodge(cb, 2.0 * (cs - 0.5));
        }
        case 17: { return clamp(cb + 2.0 * cs - 1.0, 0.0, 1.0); }
        case 18: {
            if cs <= 0.5 {
                return min(cb, 2.0 * cs);
            }
            return max(cb, 2.0 * cs - 1.0);
        }
        case 19: { return select(0.0, 1.0, pc_vivid_light_hard_mix(cb, cs) >= 0.5 - 1e-6); }
        case 20: { return abs(cb - cs); }
        case 21: { return cb + cs - 2.0 * cb * cs; }
        case 22: { return max(cb - cs, 0.0); }
        case 23: {
            if cs <= 0.0 {
                return select(1.0, 0.0, cb <= 0.0);
            }
            return min(cb / cs, 1.0);
        }
        case 28: {
            if cs >= 1.0 {
                return 1.0;
            }
            return min(cb * cb / (1.0 - cs), 1.0);
        }
        case 29: {
            if cb >= 1.0 {
                return 1.0;
            }
            return min(cs * cs / (1.0 - cb), 1.0);
        }
        case 30: { return 1.0 - abs(1.0 - cb - cs); }
        case 31: {
            let b = u32(round(clamp(cb, 0.0, 1.0) * 255.0));
            let s = u32(round(clamp(cs, 0.0, 1.0) * 255.0));
            return f32(b ^ s) / 255.0;
        }
        case 32: {
            if cs <= 0.0 {
                return 0.0;
            }
            return max(1.0 - (1.0 - cb) / cs, 0.0);
        }
        case 33: {
            if cs >= 1.0 {
                return 1.0;
            }
            return min(cb / (1.0 - cs), 1.0);
        }
        default: { return cs; }
    }
}

// `Blending::apply`.
fn apply_blending(cb: f32, cs: f32) -> f32 {
    var v = 0.0;
    if ap.blend == -1 {
        v = (cb + cs) / ap.scale + ap.offset;
    } else if ap.blend == -2 {
        v = (cb - cs) / ap.scale + ap.offset;
    } else {
        v = pc_blend(ap.blend, cb, cs);
    }
    return clamp(v, 0.0, 1.0);
}

// One colour channel `b` of the layer with the source's `t` blended on (`apply_image_pixel`).
fn apply_channel(b: f32, t: f32, w: f32, base_a: f32, out_a: f32, preserve: bool) -> f32 {
    let blended = t + (apply_blending(b, t) - t) * base_a;
    return select((w * blended + base_a * (1.0 - w) * b) / out_a, b + (blended - b) * w, preserve);
}

@compute @workgroup_size(8, 8)
fn apply_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= ap.size.x || g.y >= ap.size.y {
        return;
    }
    let at = vec2<i32>(g.xy);
    var px = textureLoad(asrc, at + vec2<i32>(ap.origin), 0);
    let top = textureLoad(ainput, at, 0);
    let alpha = (ap.flags & 1u) != 0u;
    let preserve = (ap.flags & 2u) != 0u;
    var w = ap.opacity;
    if (ap.flags & 4u) != 0u {
        w = ap.opacity * textureLoad(aweight, at, 0).x;
    }
    let base_a = select(1.0, clamp(px.w, 0.0, 1.0), alpha);
    if preserve {
        w = w * base_a;
    }
    if w > 0.0 {
        let out_a = select(w + base_a * (1.0 - w), base_a, preserve);
        // Each channel by name: no vector component chosen at run time (see the header).
        let r = apply_channel(px.x, top.x, w, base_a, out_a, preserve);
        var c = vec3<f32>(r);
        // A Grayscale layer's one channel stands for all three.
        if ap.colors > 1u {
            c = vec3<f32>(r, apply_channel(px.y, top.y, w, base_a, out_a, preserve), apply_channel(px.z, top.z, w, base_a, out_a, preserve));
        }
        px = vec4<f32>(c, select(px.w, out_a, alpha));
    }
    textureStore(aout, at, px);
}

// ---- The final pass: the filter's output as the layer's new pixels ----

struct Finish {
    // Output size, and where it starts inside the windows.
    size: vec2<u32>,
    offset: vec2<u32>,
    // The filtered area, in output pixels (x0, y0, x1, y1): outside it the source stays.
    area: vec4<i32>,
    // Where the selection texture starts, in output pixels, and its size (0: no selection).
    sel_origin: vec2<i32>,
    sel_size: vec2<u32>,
    // 1: Lock Transparency keeps the source's alpha.
    keep_alpha: u32,
    // Steps per unit the results are rounded to (255, 65535), or 0 for floats.
    levels: f32,
    // 1: the source has an alpha channel (else results keep alpha 1, as the CPU stores none).
    has_alpha: u32,
    // 0: the blur itself; 1: Unsharp Mask's combine; 2: High Pass's (operator.rs `Combine`).
    mode: u32,
    // Where the output starts inside the blurred window (which covers only part of the source's).
    blur_offset: vec2<u32>,
    // Unsharp Mask: Amount / 100 and Threshold / 255.
    amount: f32,
    threshold: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var blurred: texture_2d<f32>;
@group(0) @binding(2) var selection: texture_2d<f32>;
@group(0) @binding(3) var out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var<uniform> f: Finish;

fn quantise(v: vec4<f32>) -> vec4<f32> {
    if f.levels <= 0.0 {
        return v;
    }
    return floor(clamp(v, vec4<f32>(0.0), vec4<f32>(1.0)) * f.levels + 0.5) / f.levels;
}

@compute @workgroup_size(8, 8)
fn finish(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= f.size.x || g.y >= f.size.y {
        return;
    }
    let at = vec2<i32>(g.xy);
    let w = at + vec2<i32>(f.offset);
    let orig = textureLoad(source, w, 0);
    let inside = at.x >= f.area.x && at.y >= f.area.y && at.x < f.area.z && at.y < f.area.w;
    if !inside {
        textureStore(out, at, orig);
        return;
    }
    var v = textureLoad(blurred, at + vec2<i32>(f.blur_offset), 0);
    if f.mode == 1u {
        // Unsharp Mask (`sharpen::exact::run`): the blurred value is already straight, with the
        // CPU's unpremultiply cut-off. A fully transparent source pixel stays as it is.
        if f.has_alpha != 0u && orig.w <= 0.0 {
            v = orig;
        } else {
            let d = orig.xyz - v.xyz;
            let sharpen = abs(d) + vec3<f32>(1e-6) >= vec3<f32>(f.threshold);
            v = vec4<f32>(select(orig.xyz, orig.xyz + f.amount * d, sharpen), orig.w);
        }
    } else if f.mode == 2u {
        // High Pass (`sharpen::high_pass`).
        if f.has_alpha != 0u && orig.w <= 0.0 {
            v = orig;
        } else {
            v = vec4<f32>(vec3<f32>(0.5) + (orig.xyz - v.xyz), orig.w);
        }
    }
    // The selection mix (`mix_selection` in photocraft-algo).
    var k = 1.0;
    if f.sel_size.x > 0u {
        let s = at - f.sel_origin;
        if s.x >= 0 && s.y >= 0 && u32(s.x) < f.sel_size.x && u32(s.y) < f.sel_size.y {
            k = clamp(textureLoad(selection, s, 0).x, 0.0, 1.0);
        } else {
            k = 0.0;
        }
    }
    if k <= 0.0 {
        v = orig;
    } else if k < 1.0 {
        if f.has_alpha != 0u && orig.w != v.w {
            let mixed = orig.w + (v.w - orig.w) * k;
            let wo = orig.w * (1.0 - k);
            let wf = v.w * k;
            var c = vec3<f32>(0.0);
            if mixed > 0.0 {
                c = (orig.xyz * wo + v.xyz * wf) / mixed;
            }
            v = vec4<f32>(c, mixed);
        } else {
            v = orig + (v - orig) * k;
        }
    }
    if f.keep_alpha != 0u {
        v = vec4<f32>(v.xyz, orig.w);
    }
    if f.has_alpha == 0u {
        v = vec4<f32>(v.xyz, 1.0);
    }
    textureStore(out, at, quantise(v));
}
