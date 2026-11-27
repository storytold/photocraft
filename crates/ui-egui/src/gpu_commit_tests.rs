//! The GPU-commit prototype of #17133 end to end on a GPU: a live Brush stroke whose coverage the
//! GPU draws (`LiveStroke::begin_gpu`, dabs stamped into the compositor's external mask), read
//! back as 64×64 tiles at the end, fed into the stroke and committed. The commit must be the
//! pixels `paint.stroke` writes on the CPU to within the coverage's precision: R32F is within a
//! few 1e-6 of the CPU's coverage, so within one 8-bit step of its pixels; R16F rounds each blend
//! (lavapipe worse than a GPU, see `crates/gpu/tests/it/stamp.rs`), so it is allowed three.

use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, LiveStrokeExt};
use photocraft_engine::paint::{StrokePoint, TipGeometry};
use photocraft_geom::Rect;
use photocraft_gpu::{GpuDab, MaskPrecision, MaskReadback};
use serde_json::{Value, json};

const W: i32 = 600;
const H: i32 = 300;

fn scene() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(0, 0, W, H);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
                data.extend(photocraft_raster::from_rgba(&fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, 1.0]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn gpu_dab(g: &TipGeometry) -> GpuDab {
    GpuDab {
        rect: g.rect,
        cx: g.cx,
        cy: g.cy,
        ro: g.ro,
        core: g.core,
        shoulder: g.shoulder,
        reach2: g.reach2,
        small_reach2: g.small_reach2,
        cs: g.cs,
        sn: g.sn,
        fx: g.fx,
        fy: g.fy,
        proj: g.proj,
        small: g.small,
        flow: g.flow,
        ceil: g.ceil,
    }
}

fn path(n: usize) -> Vec<[f64; 3]> {
    (0..n).map(|i| [30.0 + i as f64 * 540.0 / n as f64, 150.0 + (i as f64 * 0.35).sin() * 100.0, 1.0]).collect()
}

fn stamp(ctx: &photocraft_gpu::NativeContext, comp: &mut photocraft_gpu::Compositor, layer: photocraft_doc::LayerId, dabs: &[TipGeometry]) {
    let dabs: Vec<GpuDab> = dabs.iter().map(gpu_dab).collect();
    let mut encoder = ctx.device().create_command_encoder(&Default::default());
    comp.stamp_dabs(ctx.device(), ctx.queue(), &mut encoder, layer, &dabs).unwrap();
    ctx.queue().submit([encoder.finish()]);
}

/// Read back `keys` completely (the queue holds a few buffers at a time).
fn read_all(
    ctx: &photocraft_gpu::NativeContext,
    comp: &photocraft_gpu::Compositor,
    rb: &MaskReadback,
    layer: photocraft_doc::LayerId,
    mut keys: Vec<(i32, i32)>,
) -> Vec<((i32, i32), Vec<f32>)> {
    let want = keys.len();
    let mut out = Vec::new();
    while out.len() < want {
        let taken = rb.request(comp, ctx.device(), ctx.queue(), layer, &keys);
        keys.drain(..taken);
        out.extend(rb.poll(ctx.device(), true));
    }
    out
}

#[test]
fn a_gpu_stroke_commits_paint_strokes_pixels() {
    let _guard = photocraft_testkit::gpu_lock();
    let ctx = match photocraft_gpu::NativeContext::new() {
        Ok(c) => c,
        Err(e) => {
            assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no GPU adapter: {e}");
            eprintln!("skipping the GPU commit test: no adapter ({e})");
            return;
        }
    };
    let mut comp = match ctx.create_compositor() {
        Ok(c) => c,
        Err(e) => {
            assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no GPU compositor: {e}");
            eprintln!("skipping the GPU commit test: {e}");
            return;
        }
    };
    // Pages of one tile, so the stroke crosses cells.
    comp.set_texture_limit(256);
    let cases = [(5.0, 0.5, 120), (22.0, 0.3, 60), (120.0, 0.0, 24), (40.0, 1.0, 40)];
    for (size, hardness, n) in cases {
        for precision in [Some(MaskPrecision::Half), None] {
            let mut s = scene();
            let pts = path(n);
            let mut p: Value = json!({"brush": {"size": size, "hardness": hardness, "flow": 0.6}, "color": "#d02060", "opacity": 0.8, "smoothing": 0.0, "target": "pixels", "points": [pts[0]]});
            let (mut live, first) = LiveStroke::begin_gpu(&s, "paint.stroke", &p).unwrap();
            let preview = live.gpu_preview().unwrap();
            let used = comp.begin_external_mask(ctx.device(), preview.doc.id, preview.layer, preview.doc.bounds(), precision);
            stamp(&ctx, &mut comp, preview.layer, &first);
            let rb = MaskReadback::new();
            for (i, chunk) in pts[1..].chunks(3).enumerate() {
                let step: Vec<StrokePoint> = chunk.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect();
                let dabs = live.push_dabs(&step).unwrap();
                stamp(&ctx, &mut comp, preview.layer, &dabs);
                if i == n / 6 {
                    // Mid-stroke: coverage read back and fed early is replaced by the final read.
                    for (key, values) in read_all(&ctx, &comp, &rb, preview.layer, live.touched_keys()) {
                        live.feed_coverage(key, &values);
                    }
                }
            }
            let last = live.finish_dabs();
            stamp(&ctx, &mut comp, preview.layer, &last);
            let keys = live.touched_keys();
            assert!(keys.len() > 1);
            let tiles = read_all(&ctx, &comp, &rb, preview.layer, keys.clone());
            assert_eq!(tiles.len(), keys.len());
            for (key, values) in &tiles {
                live.feed_coverage(*key, values);
            }
            assert!(rb.bytes_requested() > 0);
            let seed = live.seed;
            s.offer_live_stroke(live).unwrap();
            p["points"] = json!(pts);
            p["seed"] = json!(seed);
            let taken = s.execute("paint.stroke", p.clone()).unwrap();
            assert!(taken.get("fromLive").is_some());
            let mut plain = scene();
            plain.execute("paint.stroke", p).unwrap();
            let layer = |s: &Session| {
                let d = s.active().unwrap();
                d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().clone()
            };
            let (a, b) = (layer(&s).read_region(Rect::new(0, 0, W, H)), layer(&plain).read_region(Rect::new(0, 0, W, H)));
            let worst = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
            let differing = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(x, y)| x != y).count();
            let limit = if used == MaskPrecision::Full { 1.0 / 255.0 } else { 3.0 / 255.0 };
            eprintln!("GPU commit vs paint.stroke, size {size} hardness {hardness} {used:?}: worst {:.2}/255, {differing} pixels differ", worst * 255.0);
            assert!(
                worst <= limit + 4.0 * f32::EPSILON,
                "size {size} hardness {hardness} {used:?}: the commit differs from paint.stroke by {:.2}/255",
                worst * 255.0
            );
            comp.clear_external_mask(preview.layer);
        }
    }
}
