// Dab stamping into a layer mask: one instanced quad per dab, blended in stroke order. A dab's
// value is the CPU rasteriser's (`BrushContext::rasterize_into`, plain computed tips). The blend
// state (SrcAlpha, OneMinusSrcAlpha) turns the output colour = ceiling and alpha = flow * value
// into c <- c * (1 - v) + ceiling * v, the stroke buffer's accumulation.

struct Cell {
    // The texture's origin in document pixels, and its size.
    origin: vec2<f32>,
    size: vec2<f32>,
}
@group(0) @binding(0) var<uniform> cell: Cell;

struct Dab {
    @builtin(vertex_index) vi: u32,
    // x0, y0, x1, y1 of the pixels the dab can touch (document pixels).
    @location(0) rect: vec4<f32>,
    // cx, cy, roundness, flow.
    @location(1) a: vec4<f32>,
    // core, shoulder, reach2, small_reach2.
    @location(2) b: vec4<f32>,
    // cos, sin of the angle, flip x, flip y.
    @location(3) c: vec4<f32>,
    // projection ax, ay, k; small tip (0 or 1).
    @location(4) d: vec4<f32>,
    // opacity ceiling.
    @location(5) e: vec4<f32>,
}

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) a: vec4<f32>,
    @location(1) @interpolate(flat) b: vec4<f32>,
    @location(2) @interpolate(flat) c: vec4<f32>,
    @location(3) @interpolate(flat) d: vec4<f32>,
    @location(4) @interpolate(flat) e: vec4<f32>,
}

@vertex
fn vs_stamp(dab: Dab) -> VOut {
    // Triangle strip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(dab.vi & 1u), f32((dab.vi >> 1u) & 1u));
    let px = mix(dab.rect.xy, dab.rect.zw, corner) - cell.origin;
    let ndc = vec2<f32>(px.x / cell.size.x * 2.0 - 1.0, 1.0 - px.y / cell.size.y * 2.0);
    var o: VOut;
    o.pos = vec4<f32>(ndc, 0.0, 1.0);
    o.a = dab.a;
    o.b = dab.b;
    o.c = dab.c;
    o.d = dab.d;
    o.e = dab.e;
    return o;
}

// The tip profile: full inside the core, then (1 - u/64)^64 by six squarings (`soft_exp`).
fn falloff(dist: f32, core: f32, shoulder: f32) -> f32 {
    if (dist <= core) {
        return 1.0;
    }
    let x = (dist - core) / shoulder;
    var g = clamp(1.0 - x * x / 64.0, 0.0, 1.0);
    g = g * g;
    g = g * g;
    g = g * g;
    g = g * g;
    g = g * g;
    g = g * g;
    return g;
}

// A pixel offset from the dab centre (y down) in tip space: to y-up, project, rotate, flip.
fn to_tip(dx: f32, dy: f32, c: vec4<f32>, d: vec4<f32>) -> vec2<f32> {
    var ux = dx;
    var uy = -dy;
    let t = (ux * d.x + uy * d.y) * d.z;
    ux = ux + t * d.x;
    uy = uy + t * d.y;
    return vec2<f32>((ux * c.x + uy * c.y) * c.z, (-ux * c.y + uy * c.x) * c.w);
}

fn tip_dist2(dx: f32, dy: f32, ro: f32, c: vec4<f32>, d: vec4<f32>) -> f32 {
    let t = to_tip(dx, dy, c, d);
    let u = t.x * ro;
    return u * u + t.y * t.y;
}

@fragment
fn fs_stamp(v: VOut) -> @location(0) vec4<f32> {
    let dx = v.pos.x + cell.origin.x - v.a.x;
    let dy = v.pos.y + cell.origin.y - v.a.y;
    var value = 0.0;
    if (v.d.w > 0.5) {
        // A tip a few pixels across: the covered area, a 4x4 grid of samples inside the pixel.
        if (tip_dist2(dx, dy, v.a.z, v.c, v.d) < v.b.w) {
            var sum = 0.0;
            for (var j = 0; j < 4; j = j + 1) {
                for (var i = 0; i < 4; i = i + 1) {
                    let sd = sqrt(tip_dist2(dx + (f32(i) - 1.5) * 0.25, dy + (f32(j) - 1.5) * 0.25, v.a.z, v.c, v.d));
                    sum = sum + falloff(sd, v.b.x, v.b.y);
                }
            }
            value = sum / 16.0;
        }
    } else {
        let d2 = tip_dist2(dx, dy, v.a.z, v.c, v.d);
        if (d2 < v.b.z) {
            value = falloff(sqrt(d2), v.b.x, v.b.y);
        }
    }
    return vec4<f32>(v.e.x, 0.0, 0.0, value * v.a.w);
}
