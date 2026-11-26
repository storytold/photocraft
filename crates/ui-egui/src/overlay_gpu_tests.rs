//! The GPU compositor draws a live Brush stroke's overlay document (the target inside an isolated
//! group beside a colour layer through a mask) as the CPU flatten of the document the commit
//! writes. The overlay is the doc the canvas hands the GPU while a stroke is drawn.
//!
//! Tolerance: 3/255 in premultiplied channels, the GPU parity tests' one 8-bit step
//! (`crates/gpu/tests/it/parity.rs`) plus the 2/255 the preview may differ from the commit by (the
//! commit's quantisation and 8-bit dither).

use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_engine::paint::StrokePoint;
use photocraft_geom::Rect;
use serde_json::{Value, json};

const W: i32 = 120;
const H: i32 = 80;
const TOLERANCE: f32 = 1.0 / 255.0 + 2.0 / 255.0 + 4.0 * f32::EPSILON;
const PTS: [[f64; 3]; 6] = [[14.0, 40.0, 1.0], [34.0, 20.0, 0.8], [56.0, 52.0, 1.0], [78.0, 24.0, 0.6], [100.0, 44.0, 1.0], [70.0, 70.0, 1.0]];

fn scene(setup: &dyn Fn(&mut Session)) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let r = Rect::new(8, 8, 90, 64);
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
                let a = if (x + y) % 5 == 0 { 0.5 } else { 1.0 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, a]));
            }
        }
        surf.write_region(r, &data);
        Ok(())
    })
    .unwrap();
    setup(&mut s);
    s
}

fn tweak(s: &mut Session, f: impl FnOnce(&mut Layer)) {
    s.edit("tweak", |doc, active| {
        f(doc.layer_mut(active.unwrap()).unwrap());
        Ok(())
    })
    .unwrap();
}

/// The largest premultiplied difference, and where.
fn worst(cpu: &[[f32; 4]], gpu: &[[f32; 4]]) -> f32 {
    assert_eq!(cpu.len(), gpu.len());
    cpu.iter()
        .zip(gpu)
        .map(|(p, q)| {
            let d = (0..3).map(|c| (p[c] * p[3] - q[c] * q[3]).abs()).chain([(p[3] - q[3]).abs()]).fold(0.0f32, f32::max);
            if d.is_nan() { 9.0 } else { d }
        })
        .fold(0.0, f32::max)
}

fn points(v: &[[f64; 3]]) -> Vec<StrokePoint> {
    v.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect()
}

fn overlay_doc(build: &dyn Fn() -> Session, extra: Value) -> (std::sync::Arc<Document>, Session) {
    let s = build();
    let mut p = json!({"brush": {"size": 22, "hardness": 0.3, "flow": 0.6}, "color": "#d02060", "opacity": 0.8, "smoothing": 0.0, "target": "pixels", "points": [PTS[0]]});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &p).unwrap();
    assert!(live.is_overlay());
    for c in PTS[1..].chunks(2) {
        live.push(&points(c)).unwrap();
    }
    let mut committed = build();
    p["points"] = json!(PTS);
    p["seed"] = json!(live.seed);
    committed.execute("paint.stroke", p).unwrap();
    (live.doc.clone(), committed)
}

#[test]
fn the_gpu_draws_the_overlay_as_the_committed_stroke() {
    let _guard = photocraft_testkit::gpu_lock();
    let ctx = match photocraft_gpu::NativeContext::new() {
        Ok(c) => c,
        Err(e) => {
            assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no GPU adapter: {e}");
            eprintln!("skipping the overlay GPU test: no adapter ({e})");
            return;
        }
    };
    let mut comp = match ctx.create_compositor() {
        Ok(c) => c,
        Err(e) => {
            assert!(std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_none(), "no GPU compositor: {e}");
            eprintln!("skipping the overlay GPU test: {e}");
            return;
        }
    };
    type Setup = Box<dyn Fn(&mut Session)>;
    let masked: Setup = Box::new(|s| {
        tweak(s, |l| {
            let mut m = LayerMask::reveal_all();
            let data: Vec<f32> = (0..H).flat_map(|y| (0..W).map(move |x| 0.3 + 0.7 * (((x + 2 * y) % 17) as f32 / 16.0))).collect();
            m.surface.write_region(Rect::new(0, 0, W, H), &data);
            l.mask = Some(m);
            l.blend = photocraft_color::BlendMode::Multiply;
            l.opacity = 0.5;
            l.fill_opacity = 0.8;
        })
    });
    let plain: Setup = Box::new(|_| ());
    let erase_setup: Setup = Box::new(|_| ());
    let locked: Setup = Box::new(|s| tweak(s, |l| l.locks.transparency = true));
    let cases: Vec<(&str, Setup, Value)> = vec![
        ("plain Normal", plain, json!({})),
        ("lock transparency", locked, json!({})),
        ("erase", erase_setup, json!({"erase": true})),
        ("opacity, Multiply and mask", masked, json!({})),
    ];
    for (name, setup, extra) in cases {
        let (live, committed) = overlay_doc(&|| scene(&*setup), extra);
        let cpu = photocraft_compose::flatten(&committed.active().unwrap().doc);
        let gpu = photocraft_gpu::render_to_vec(&mut comp, ctx.device(), ctx.queue(), &live, live.bounds()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let d = worst(&cpu.px, &gpu);
        eprintln!("overlay GPU vs committed CPU, {name}: worst difference {:.2}/255", d * 255.0);
        assert!(d <= TOLERANCE, "{name}: the GPU and the committed CPU flatten differ by {:.2}/255", d * 255.0);
    }
}
