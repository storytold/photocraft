//! Switching Edit › Transform modes without an edit must not change what is applied: a box moved in
//! any mode then switched to Warp, a warp switched back to a box mode, back and forth. Checked on
//! smart objects (upright, rotated, warped, under a perspective), by the applied pixels.
use super::tests::app_with_square;
use super::*;
use crate::state::TransformMode;
use serde_json::json;

fn paint_pattern(app: &mut PhotocraftApp) {
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let r = |x0, y0, x1, y1| photocraft_geom::Rect::new(x0, y0, x1, y1);
            s.fill_rect(r(30, 40, 60, 60), &[1.0, 0.0, 0.0, 1.0]);
            s.fill_rect(r(60, 40, 90, 60), &[0.0, 1.0, 0.0, 1.0]);
            s.fill_rect(r(30, 60, 60, 80), &[0.0, 0.0, 1.0, 1.0]);
            s.fill_rect(r(60, 60, 90, 80), &[1.0, 1.0, 0.0, 1.0]);
            s.fill_rect(r(55, 55, 65, 65), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
}

fn rot(q: [[f64; 2]; 4], deg: f64, k: f64) -> [[f64; 2]; 4] {
    let c = [q.iter().map(|p| p[0]).sum::<f64>() / 4.0, q.iter().map(|p| p[1]).sum::<f64>() / 4.0];
    let (s, co) = deg.to_radians().sin_cos();
    q.map(|p| {
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        [c[0] + k * (co * x - s * y), c[1] + k * (s * x + co * y)]
    })
}

fn moved(mode: TransformMode, q: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let mut q = q;
    match mode {
        TransformMode::Free | TransformMode::Rotate => return rot(q, 10.0, 0.9),
        TransformMode::Skew => {
            q[0][0] += 8.0;
            q[1][0] += 8.0;
        }
        TransformMode::Distort => {
            q[1][0] += 10.0;
            q[1][1] -= 6.0;
        }
        TransformMode::Perspective => {
            let d = [(q[1][0] - q[0][0]) * 0.25, (q[1][1] - q[0][1]) * 0.25];
            q[0] = [q[0][0] + d[0], q[0][1] + d[1]];
            q[1] = [q[1][0] - d[0], q[1][1] - d[1]];
        }
    }
    q
}

fn kind(k: usize) -> (&'static str, PhotocraftApp) {
    let ctx = egui::Context::default();
    let mut app = app_with_square(120, photocraft_geom::Rect::new(30, 40, 90, 80));
    paint_pattern(&mut app);
    if k == 0 {
        return ("pixel", app);
    }
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let r = [30.0, 40.0, 90.0, 80.0];
    let c = [[30.0, 40.0], [90.0, 40.0], [90.0, 80.0], [30.0, 80.0]];
    match k {
        1 => ("smart upright", app),
        2 => {
            app.run("edit.transform", json!({"rect": r, "quad": rot(c, 20.0, 1.0)})).unwrap();
            ("smart rotated", app)
        }
        3 => {
            begin_warp(&mut app, &ctx).unwrap();
            if let Some(m) = app.ui.transform.as_mut().unwrap().warp.as_mut().and_then(|w| w.mesh.as_mut()) {
                m.points[6][1] -= 6.0;
            }
            commit(&mut app);
            ("smart warped", app)
        }
        _ => {
            app.run("edit.transform", json!({"rect": r, "quad": [[42.0, 40.0], [78.0, 40.0], [90.0, 80.0], [30.0, 80.0]]})).unwrap();
            ("smart persp", app)
        }
    }
}

fn pixels(app: &PhotocraftApp) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&app.session.active().unwrap().doc).into_px()
}

fn diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> (usize, f32) {
    let mut n = 0;
    let mut m = 0.0f32;
    for (p, q) in a.iter().zip(b) {
        let d = (0..4).map(|i| (p[i] * p[3] - q[i] * q[3]).abs().max((p[3] - q[3]).abs())).fold(0.0f32, f32::max);
        if d > 3.0 / 255.0 {
            n += 1;
        }
        m = m.max(d);
    }
    (n, m)
}

fn edit_mesh(app: &mut PhotocraftApp) {
    if let Some(m) = app.ui.transform.as_mut().unwrap().warp.as_mut().and_then(|w| w.mesh.as_mut()) {
        m.points[5][0] += 4.0;
        m.points[5][1] += 3.0;
    }
}

fn set_box(app: &mut PhotocraftApp, mode: TransformMode, mv: bool) {
    let t = app.ui.transform.as_mut().unwrap();
    t.mode = mode;
    if mv {
        t.quad = moved(mode, t.quad);
    }
}

/// Every smart-object kind × box mode × switch sequence applies the same pixels with and without
/// the switch. (A pixel layer's box switched to Warp is rebuilt as a mesh, so a Distort or
/// Perspective there changes; not covered here.)
#[test]
fn mode_switches_keep_what_a_smart_object_applies() {
    let ctx = egui::Context::default();
    let modes = [TransformMode::Free, TransformMode::Skew, TransformMode::Distort, TransformMode::Perspective];
    let mut bad = Vec::new();
    for k in 1..5 {
        for mode in modes {
            let run = |seq: usize, variant: bool| -> Result<Vec<[f32; 4]>, String> {
                let (_, mut app) = kind(k);
                match seq {
                    // box move, (Warp), apply
                    0 => {
                        begin(&mut app, &ctx)?;
                        set_box(&mut app, mode, true);
                        if variant {
                            enter_warp(&mut app, &ctx);
                        }
                    }
                    // Warp edit, (box mode), apply
                    1 => {
                        begin_warp(&mut app, &ctx)?;
                        edit_mesh(&mut app);
                        if variant {
                            leave_warp(&mut app);
                            set_box(&mut app, mode, false);
                        }
                    }
                    // Warp edit, box move, (Warp), apply
                    2 => {
                        begin_warp(&mut app, &ctx)?;
                        edit_mesh(&mut app);
                        leave_warp(&mut app);
                        set_box(&mut app, mode, true);
                        if variant {
                            enter_warp(&mut app, &ctx);
                        }
                    }
                    // box move, (Warp and back), apply
                    _ => {
                        begin(&mut app, &ctx)?;
                        set_box(&mut app, mode, true);
                        if variant {
                            enter_warp(&mut app, &ctx);
                            leave_warp(&mut app);
                            set_box(&mut app, mode, false);
                        }
                    }
                }
                commit(&mut app);
                if app.ui.status_error {
                    return Err(app.ui.status.clone());
                }
                Ok(pixels(&app))
            };
            let name = kind(k).0;
            let mut line = format!("{name:<14} {mode:<12?}");
            for (seq, label) in ["box>W", "W>box", "W>box>W", "box>W>box"].iter().enumerate() {
                let cell = match (run(seq, false), run(seq, true)) {
                    (Ok(a), Ok(b)) => {
                        let (n, m) = diff(&a, &b);
                        if n > 0 {
                            bad.push(format!("{name} {mode:?} {label}: {n} px differ (up to {m:.2})"));
                        }
                        format!("{label} {n:>5} px {m:.2}")
                    }
                    (a, b) => {
                        bad.push(format!("{name} {mode:?} {label}: {:?} {:?}", a.as_ref().err(), b.as_ref().err()));
                        format!("{label} ERR")
                    }
                };
                line.push_str(&format!(" | {cell:<24}"));
            }
            eprintln!("{line}");
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
