// RAW Develop's preview shown from the GPU (#6916): the developed frame converted for the dialog
// the way `CanvasDisplay::to_rgba8` and egui's `Color32::from_rgba_unmultiplied` do it on the
// CPU, and its histogram counted the way `RgbHistogram::from_rgba` does. Fragment and vertex
// stages only (no compute), as the rest of RAW Develop's passes.

struct View {
    // Width, height, flags (1 sRGB-encode, 2 matrix/shaper), histogram rows.
    head: vec4<u32>,
    // The matrix/shaper's input entries, output entries.
    curves: vec4<u32>,
    matrix: array<vec4<f32>, 3>,
}

@group(0) @binding(0) var<uniform> V: View;
// The develop's result, straight RGBA.
@group(0) @binding(1) var pixels: texture_2d<f32>;
// Row 0: the smallest value each 8-bit code `k` (1 to 255) starts at for `(v * 255 + 0.5)`;
// row 1: the same for the sRGB-encoding table. A value's code is how many edges it reaches, so it
// is exactly the CPU's, whatever the GPU does with `v * 255 + 0.5` (a fused multiply-add rounds
// once where the CPU rounds twice).
@group(0) @binding(2) var edges: texture_2d<f32>;
// The matrix/shaper's input curves then its output curves, `CURVE_WIDTH` texels a row.
@group(0) @binding(3) var curves: texture_2d<f32>;
// The histogram points' rows, summed by `fs_sum`.
@group(0) @binding(4) var counts: texture_2d<f32>;

const BINS: u32 = 256u;
// Bins, then each channel's flags, then samples, transparent and invalid.
const COLUMNS: u32 = 260u;
const POINTS: u32 = 7u;
// An `Rgba16Float` texel holds every whole number up to 2048 exactly.
const ROW_PIXELS: u32 = 2048u;
const CURVE_WIDTH: u32 = 1024u;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
}

fn finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// The CPU's 8-bit code of `v` (`row` 0 plain, 1 sRGB-encoded): 0 below the first edge (and for
// NaN, as Rust's saturating cast), 255 from the last.
fn code(v: f32, row: i32) -> u32 {
    if !finite(v) {
        return select(0u, 255u, v > 0.0 && (bitcast<u32>(v) & 0x007fffffu) == 0u);
    }
    var lo = 0u;
    var hi = 255u;
    for (var i = 0; i < 8; i++) {
        let mid = (lo + hi + 1u) / 2u;
        if v >= textureLoad(edges, vec2(i32(mid), row), 0).r {
            lo = mid;
        } else {
            hi = mid - 1u;
        }
    }
    return lo;
}

fn curve_entry(index: u32) -> vec3<f32> {
    return textureLoad(curves, vec2(i32(index % CURVE_WIDTH), i32(index / CURVE_WIDTH)), 0).rgb;
}

// One table of `entries` texels from `offset`, sampled at `c` in 0..1 with linear interpolation.
fn curve(c: vec3<f32>, offset: u32, entries: u32) -> vec3<f32> {
    let n = f32(entries - 1u);
    let p = clamp(c, vec3(0.0), vec3(1.0)) * n;
    let lo = vec3<u32>(floor(p));
    let hi = min(lo + vec3(1u), vec3(entries - 1u));
    let f = fract(p);
    return vec3(
        mix(curve_entry(offset + lo.r).r, curve_entry(offset + hi.r).r, f.r),
        mix(curve_entry(offset + lo.g).g, curve_entry(offset + hi.g).g, f.g),
        mix(curve_entry(offset + lo.b).b, curve_entry(offset + hi.b).b, f.b),
    );
}

// `craft_cms::GpuMatrixShaper`'s contract: input curves, the matrix, output curves sampled
// at the fourth root of the linear value. Returns 8-bit codes.
fn shaper(rgb: vec3<u32>) -> vec3<u32> {
    let v = vec4(curve(vec3<f32>(rgb) / 255.0, 0u, V.curves.x), 1.0);
    let linear = vec3(dot(V.matrix[0], v), dot(V.matrix[1], v), dot(V.matrix[2], v));
    let out = curve(sqrt(sqrt(clamp(linear, vec3(0.0), vec3(1.0)))), V.curves.x, V.curves.y);
    return vec3<u32>(clamp(out, vec3(0.0), vec3(1.0)) * 255.0 + vec3(0.5));
}

// egui's `mul_frac_round`: `v · f / 255`, rounded, in fixed point.
fn mul_frac_round(v: u32, f: u32) -> u32 {
    let p = v * f + 128u;
    return (p + (p >> 8u)) >> 8u;
}

// The preview's texel: what `CameraRawDialog::image` puts in its `ColorImage` (premultiplied
// sRGB bytes, as egui's own textures hold them), written to an `Rgba8Unorm` view.
@fragment
fn fs_display(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = textureLoad(pixels, vec2<i32>(pos.xy), 0);
    let row = i32(V.head.z & 1u);
    var rgb = vec3(code(p.r, row), code(p.g, row), code(p.b, row));
    let a = code(p.a, 0);
    if (V.head.z & 2u) != 0u {
        rgb = shaper(rgb);
    }
    if a == 0u {
        return vec4(0.0);
    }
    if a != 255u {
        rgb = vec3(mul_frac_round(rgb.r, a), mul_frac_round(rgb.g, a), mul_frac_round(rgb.b, a));
    }
    return vec4<f32>(vec4(rgb, a)) / 255.0;
}

struct Point {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) value: vec4<f32>,
}

fn channel(p: vec4<f32>, c: u32) -> f32 {
    return select(select(p.b, p.g, c == 1u), p.r, c == 0u);
}

// `POINTS` points a pixel, added up in a `COLUMNS × rows` target (`ROW_PIXELS` pixels a row):
// 0 to 2 each channel's bin (one in its own component), 3 to 5 each channel's flags (below 0,
// above 1, at or below 0, at or above 1) in column 256 + channel, 6 whether the pixel is a
// sample, transparent or invalid in column 259. A point with nothing to add lands off the target.
@vertex
fn vs_count(@builtin(vertex_index) vi: u32) -> Point {
    let i = vi / POINTS;
    let part = vi % POINTS;
    let w = V.head.x;
    let p = textureLoad(pixels, vec2(i32(i % w), i32(i / w)), 0);
    // As `RgbHistogram::count`: alpha first, then the colour.
    var kind = 0u;
    if !finite(p.a) {
        kind = 2u;
    } else if p.a <= 0.0 {
        kind = 1u;
    } else if !(finite(p.r) && finite(p.g) && finite(p.b)) {
        kind = 2u;
    }
    var out: Point;
    out.pos = vec4(-2.0, -2.0, 0.0, 1.0);
    out.value = vec4(0.0);
    var column = 0u;
    var on = false;
    if part == 6u {
        column = COLUMNS - 1u;
        on = true;
        out.value = vec4(f32(kind == 0u), f32(kind == 1u), f32(kind == 2u), 0.0);
    } else if kind == 0u {
        let c = part % 3u;
        let v = channel(p, c);
        if part < 3u {
            column = code(v, 0);
            on = true;
            out.value = vec4(f32(c == 0u), f32(c == 1u), f32(c == 2u), 0.0);
        } else {
            let flags = vec4(f32(v < 0.0), f32(v > 1.0), f32(v <= 0.0), f32(v >= 1.0));
            if any(flags != vec4(0.0)) {
                column = BINS + c;
                on = true;
                out.value = flags;
            }
        }
    }
    if on {
        let row = i / ROW_PIXELS;
        let x = (f32(column) + 0.5) / f32(COLUMNS) * 2.0 - 1.0;
        let y = 1.0 - (f32(row) + 0.5) / f32(V.head.w) * 2.0;
        out.pos = vec4(x, y, 0.0, 1.0);
    }
    return out;
}

@fragment
fn fs_point(p: Point) -> @location(0) vec4<f32> {
    return p.value;
}

// Each column's rows added up as whole numbers.
@fragment
fn fs_sum(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    let x = i32(pos.x);
    var s = vec4(0u);
    for (var y = 0; y < i32(V.head.w); y++) {
        s += vec4<u32>(textureLoad(counts, vec2(x, y), 0) + vec4(0.5));
    }
    return s;
}
