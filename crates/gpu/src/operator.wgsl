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

// ---- Displace (`distort2::displace`): each pixel samples the source where the map shifts it ----

struct DisplacePass {
    // The output, in document pixels: its size and where it starts.
    size: vec2<u32>,
    out_origin: vec2<i32>,
    // The source window: where it starts in document pixels, and its size.
    win_origin: vec2<i32>,
    win_size: vec2<u32>,
    // The filter's bounds (x0, y0, x1, y1), document pixels.
    bounds: vec4<i32>,
    map_size: vec2<u32>,
    // The largest shift along each axis, pixels.
    scale: vec2<f32>,
    stretch: u32,
    // 0 Transparent, 1 Repeat Edge Pixels, 2 Wrap Around.
    edge: u32,
    has_alpha: u32,
    _pad: u32,
}

@group(0) @binding(0) var dsrc: texture_2d<f32>;
@group(0) @binding(1) var dmap: texture_2d<f32>;
@group(0) @binding(2) var dout: texture_storage_2d<rgba32float, write>;
@group(0) @binding(3) var<uniform> q: DisplacePass;

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

@compute @workgroup_size(8, 8)
fn displace_pass(@builtin(global_invocation_id) g: vec3<u32>) {
    if g.x >= q.size.x || g.y >= q.size.y {
        return;
    }
    let b = q.bounds;
    let x = f32(q.out_origin.x + i32(g.x)) + 0.5;
    let y = f32(q.out_origin.y + i32(g.y)) + 0.5;
    let lx = x - f32(b.x);
    let ly = y - f32(b.y);
    let mw = f32(q.map_size.x);
    let mh = f32(q.map_size.y);
    var u = rem_euclid_f(lx, mw);
    var v = rem_euclid_f(ly, mh);
    if q.stretch != 0u {
        u = lx / f32(max(b.z - b.x, 1)) * mw;
        v = ly / f32(max(b.w - b.y, 1)) * mh;
    }
    let m = map_sample(u, v);
    let shift = (m - vec2<f32>(0.5)) * 2.0 * q.scale;
    textureStore(dout, vec2<i32>(g.xy), src_sample(x + shift.x, y + shift.y));
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
