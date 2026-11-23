pub(super) const CANVAS_WGSL: &str = r#"
struct View {
    a: vec4<f32>, // screen_w, screen_h, scale (device px per doc px), pixels_per_point
    b: vec4<f32>, // doc origin x, y (device px), doc w, h (doc px)
    c: vec4<f32>, // filter mode, lod, unused, unused
    d: vec4<f32>, // unused, unused, display (0 none, 1 LUT, 2 LUT + gamut, 3 matrix/shaper), output linear
    e: vec4<f32>, // unused rgb, gamut warning opacity
    f: vec4<f32>, // unused rgb, 32-bit preview gain (2^exposure; 0 = off)
    g: vec4<f32>, // gamut warning rgb, 32-bit preview 1 / gamma
    h: vec4<f32>, // cos, sin, center_doc_x, center_doc_y
};
struct Tile { r: vec4<f32>, core: vec4<f32> }; // x, y, w, h in doc px; drawn x0, y0, x1, y1

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var samp: sampler;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var<uniform> tile: Tile;
@group(2) @binding(0) var lut: texture_3d<f32>;
@group(2) @binding(1) var display_curves: texture_2d<f32>;
struct DisplayMatrix { r: vec4<f32>, g: vec4<f32>, b: vec4<f32> };
@group(2) @binding(2) var<uniform> display_matrix: DisplayMatrix;

fn curve_entry(index: u32) -> vec3<f32> {
    let width = textureDimensions(display_curves).x;
    return textureLoad(display_curves, vec2<i32>(i32(index % width), i32(index / width)), 0).rgb;
}

fn display_curve(c: vec3<f32>, offset: u32, intervals: f32) -> vec3<f32> {
    let p = clamp(c, vec3(0.0), vec3(1.0)) * intervals;
    let lo = vec3<u32>(floor(p));
    let hi = min(lo + vec3(1u), vec3(u32(intervals)));
    let f = fract(p);
    return vec3(
        mix(curve_entry(offset + lo.r).r, curve_entry(offset + hi.r).r, f.r),
        mix(curve_entry(offset + lo.g).g, curve_entry(offset + hi.g).g, f.g),
        mix(curve_entry(offset + lo.b).b, curve_entry(offset + hi.b).b, f.b)
    );
}

fn matrix_display(c: vec3<f32>) -> vec3<f32> {
    // Layout matches cms::GpuMatrixShaper: 4097 input entries, then 4096 inverse-curve
    // entries indexed by the fourth root. This preserves near-black precision without
    // interpolating across clipped corners of a coarse RGB cube.
    let v = vec4(display_curve(c, 0u, 4096.0), 1.0);
    let linear = vec3(dot(display_matrix.r, v), dot(display_matrix.g, v), dot(display_matrix.b, v));
    let u = sqrt(sqrt(clamp(linear, vec3(0.0), vec3(1.0))));
    return display_curve(u, 4097u, 4095.0);
}

struct VOut { @builtin(position) pos: vec4<f32> };

fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));
    return c[i];
}

fn px_to_clip(p: vec2<f32>) -> vec4<f32> {
    let n = p / view.a.xy * 2.0 - 1.0;
    return vec4(n.x, -n.y, 0.0, 1.0);
}

fn screen_center() -> vec2<f32> {
    return view.b.xy + view.h.zw * view.a.z;
}

fn doc_to_px(d: vec2<f32>) -> vec2<f32> {
    let cs = view.h.xy;
    let q = d - view.h.zw;
    let r = vec2(q.x * cs.x - q.y * cs.y, q.x * cs.y + q.y * cs.x);
    return screen_center() + r * view.a.z;
}

fn px_to_doc(p: vec2<f32>) -> vec2<f32> {
    let cs = view.h.xy;
    let q = (p - screen_center()) / view.a.z;
    return vec2(q.x * cs.x + q.y * cs.y, -q.x * cs.y + q.y * cs.x) + view.h.zw;
}


fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let hi = 1.055 * pow(max(c, vec3(1e-12)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, c * 12.92, c <= vec3(0.0031308));
}

// --- document tiles ------------------------------------------------------------------------

@vertex
fn vs_tile(@builtin(vertex_index) vi: u32) -> VOut {
    let p = doc_to_px(tile.r.xy + tile.r.zw * corner(vi));
    return VOut(px_to_clip(p));
}

@fragment
fn fs_tile(in: VOut) -> @location(0) vec4<f32> {
    let p = in.pos.xy;
    let scale = view.a.z;
    let d = px_to_doc(p);                  // document pixel coordinates
    // A viewport page's apron is only sampled; its neighbour draws those pixels.
    if (any(d < tile.core.xy) || any(d >= tile.core.zw)) { discard; }
    let size = tile.r.zw;
    let t = d - tile.r.xy;                 // texel coordinates within this tile
    let mode = view.c.x;
    var col: vec4<f32>;
    if (mode < 0.5) {
        // Zoomed out: trilinear through the mip chain.
        col = textureSampleLevel(tex, samp, t / size, view.c.y);
    } else if (mode < 1.5) {
        col = textureSampleLevel(tex, samp, t / size, 0.0);
    } else {
        // Crisp texels with a one-device-pixel anti-aliased edge ("sharp bilinear"), worked out
        // in stored texels: a reduced viewport page's texel covers several document pixels, so
        // the edge must not depend on the reduction, only on where and how large it is drawn
        // (#11122). `per` is texels per document pixel (1 for a full-resolution texture).
        let texels = vec2<f32>(textureDimensions(tex, 0));
        let per = texels / size;
        let tt = t * per - 0.5;
        let i = floor(tt);
        let f = clamp((tt - i - 0.5) * (scale / per) + 0.5, vec2(0.0), vec2(1.0));
        col = textureSampleLevel(tex, samp, (i + 0.5 + f) / texels, 0.0);
    }
    if (view.f.w > 0.0 && col.a > 0.0) {
        // View › 32-bit Preview Options: exposure and gamma in linear light on the sRGB-encoded
        // texture value, before the display LUT. A float canvas keeps values above 1.0, so they
        // are brought into range by the exposure instead of clipped.
        let lin = srgb_to_linear(max(col.rgb / col.a, vec3(0.0))) * view.f.w;
        let v = linear_to_srgb(min(pow(max(lin, vec3(1e-12)), vec3(view.g.w)), vec3(1.0)));
        col = vec4(v * col.a, col.a);
    }
    if (view.d.z > 0.5 && col.a > 0.0) {
        // Colour management, Proof Colors, Gamut Warning: the document's display LUT (alpha flags
        // out-of-gamut colours).
        let c = clamp(col.rgb / col.a, vec3(0.0), vec3(1.0));
        var shown: vec3<f32>;
        if (view.d.z > 2.5) {
            shown = matrix_display(c);
        } else {
            let n = f32(textureDimensions(lut).x);
            let l = textureSampleLevel(lut, samp, (c * (n - 1.0) + 0.5) / n, 0.0);
            shown = select(l.rgb, mix(l.rgb, view.g.xyz, view.e.w), view.d.z > 1.5 && l.a > 0.5);
        }
        col = vec4(shown * col.a, col.a);
    }
    // Document pixels only, premultiplied: the canvas draws the checkerboard under them and the pixel grid over them.
    if (col.a <= 0.0) { discard; }
    var rgb = col.rgb;
    if (view.d.w > 0.5) {
        rgb = srgb_to_linear(rgb / col.a) * col.a;
    }
    return vec4(rgb, col.a);
}
"#;

pub(super) const ENCODE_WGSL: &str = r#"
@group(0) @binding(0) var acc: texture_2d<f32>;
@group(0) @binding(1) var<uniform> offset: vec4<i32>; // x, y, encode sRGB, keep values above 1.0

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(acc, vec2<i32>(floor(pos.xy)) - offset.xy, 0);
    let a = clamp(c.a, 0.0, 1.0);
    var rgb = max(c.rgb, vec3(0.0));
    if (offset.w == 0) {
        rgb = min(rgb, vec3(1.0));
    }
    if (offset.z != 0) {
        // Linear composite: store its sRGB encoding (8-bit precision where it matters).
        rgb = srgb_encode(rgb);
    }
    return vec4(rgb * a, a);
}
"#;

pub(super) const MIP_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> V {
    let uv = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return V(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

@fragment
fn fs(in: V) -> @location(0) vec4<f32> {
    // A bilinear tap at the destination texel centre averages the 2×2 source block.
    return textureSampleLevel(src, samp, in.uv, 0.0);
}
"#;
