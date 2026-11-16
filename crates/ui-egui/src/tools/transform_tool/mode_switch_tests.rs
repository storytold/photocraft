//! Switching Edit › Transform modes without an edit must not change what is applied: a box moved in
//! any mode then switched to Warp, a warp switched back to a box mode, back and forth. Checked by
//! everything the transform can change (composite, selection, work path, layer masks), on every
//! kind of thing a box can hold.
//!
//! Known gaps, not asserted yet (main behaves the same): a pixel layer's, a selection's or a
//! path's moved box switched to Warp is rebuilt as a mesh (a Distort or Perspective changes
//! shape, an affine move is resampled differently), and so is a smart object's linked mask; a
//! path's warp is dropped when the box goes back to another mode.
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

/// Every kind of thing a transform box can hold, and the menu command that opens the box on it.
const KINDS: usize = 14;
fn kind_with(k: usize) -> (&'static str, PhotocraftApp, &'static str) {
    let ctx = egui::Context::default();
    let mut app = app_with_square(120, photocraft_geom::Rect::new(30, 40, 90, 80));
    paint_pattern(&mut app);
    let ft = "edit.freeTransform";
    match k {
        0 => return ("pixel", app, ft),
        5 => {
            app.run("select.rect", json!({"x": 34, "y": 44, "width": 50, "height": 30})).unwrap();
            return ("selection", app, "select.transformSelection");
        }
        6 => {
            app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[34, 44], [84, 48], [80, 74], [36, 70]]}]}})).unwrap();
            app.ui.tool = Tool::PathSelection;
            return ("path", app, ft);
        }
        7 => {
            add_mask(&mut app, false);
            app.ui.mask_target = true;
            return ("unlinked mask", app, ft);
        }
        8 => {
            add_mask(&mut app, true);
            return ("pixel+mask", app, ft);
        }
        9 => {
            let mut app = app_with_square(120, photocraft_geom::Rect::new(0, 0, 0, 0));
            app.run("type.create", json!({"x": 30, "y": 70, "text": "Warp", "size": 36})).unwrap();
            return ("type", app, ft);
        }
        10 => return ("pixel copy", app, "edit.freeTransformCopy"),
        _ => {}
    }
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    match k {
        11 => {
            app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [30, 90, 90, 30, 0.3, 0.3, 0.6], "rect": [30, 40, 90, 80]})).unwrap();
            return ("smart cylinder", app, ft);
        }
        12 => {
            add_mask(&mut app, true);
            return ("smart+mask", app, ft);
        }
        13 => return ("smart copy", app, "edit.freeTransformCopy"),
        _ => {}
    }
    let (name, app) = smart_kind(k, app, &ctx);
    (name, app, ft)
}

fn add_mask(app: &mut PhotocraftApp, linked: bool) {
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session
        .edit("mask", |doc, a| {
            let m = doc.layer_mut(a.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(photocraft_geom::Rect::new(30, 40, 50, 80), &[0.0]);
            m.linked = linked;
            Ok(())
        })
        .unwrap();
}

fn smart_kind(k: usize, mut app: PhotocraftApp, ctx: &egui::Context) -> (&'static str, PhotocraftApp) {
    let ctx = ctx.clone();
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

/// Everything a transform can change, as numbers: the composite, the selection, the work path's
/// points and every layer mask.
fn pixels(app: &PhotocraftApp) -> Vec<[f32; 4]> {
    let doc = &app.session.active().unwrap().doc;
    let mut v = photocraft_compose::flatten(doc).into_px();
    let gray = |s: &photocraft_raster::Surface, v: &mut Vec<[f32; 4]>| {
        for y in 0..120 {
            for x in 0..120 {
                let p = s.pixel(x, y);
                let g = p.first().copied().unwrap_or(0.0);
                v.push([g, g, g, 1.0]);
            }
        }
    };
    if let Some(sel) = &doc.selection {
        gray(sel, &mut v);
    }
    if let Some(wp) = &doc.work_path {
        for sp in &wp.subpaths {
            for k in &sp.knots {
                // Path points count in 1/100 px against the 3/255 tolerance.
                let q = |p: &photocraft_geom::Point| [(p.x / 4.0) as f32, (p.y / 4.0) as f32, 1.0, 1.0];
                v.extend([q(&k.anchor), q(&k.in_ctrl), q(&k.out_ctrl)]);
            }
        }
    }
    for (_, _, l) in doc.walk() {
        if let Some(m) = &l.mask {
            gray(&m.surface, &mut v);
        }
    }
    v
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
    let Some(w) = app.ui.transform.as_mut().unwrap().warp.as_mut() else { return };
    // A cylinder is edited by its handles, which rebuild it from its values (deeper here).
    if w.style == photocraft_geom::warp::WarpStyle::Cylinder {
        let mut v = w.values.clone();
        v[4] += 0.2;
        *w = photocraft_geom::warp::Warp::cylinder(&v, w.bounds).unwrap();
        return;
    }
    match w.mesh.as_mut() {
        Some(m) => {
            m.points[5][0] += 4.0;
            m.points[5][1] += 3.0;
        }
        // A type warp: its bend.
        None => w.bend += 20.0,
    }
}

fn set_box(app: &mut PhotocraftApp, mode: TransformMode, mv: bool) {
    let t = app.ui.transform.as_mut().unwrap();
    t.mode = mode;
    if mv {
        t.quad = moved(mode, t.quad);
    }
}

/// Kinds the switches must leave alone: smart objects (upright, rotated, warped, under a
/// perspective, a cylinder, a ⌥⌘T copy), an unlinked mask, type.
const CLEAN: [usize; 8] = [1, 2, 3, 4, 7, 9, 11, 13];

/// Every clean kind × box mode × switch sequence applies the same with and without the switch;
/// live type refuses Distort and Perspective either way. `SWITCH_ALL=1` reports every kind.
#[test]
fn mode_switches_keep_what_a_box_applies() {
    // Opening boxes through the menus runs deep in a debug build.
    std::thread::Builder::new().stack_size(64 << 20).spawn(switch_matrix).unwrap().join().unwrap();
}

fn switch_matrix() {
    let ctx = egui::Context::default();
    let modes = [TransformMode::Free, TransformMode::Skew, TransformMode::Distort, TransformMode::Perspective];
    let mut bad = Vec::new();
    let all = std::env::var("SWITCH_ALL").is_ok();
    for k in 0..KINDS {
        if !all && !CLEAN.contains(&k) {
            continue;
        }
        for mode in modes {
            let run = |seq: usize, variant: bool| -> Result<Vec<[f32; 4]>, String> {
                let (_, mut app, start) = kind_with(k);
                let begin = |app: &mut PhotocraftApp, ctx: &egui::Context| crate::menus::invoke(app, ctx, start, json!({})).map(|_| ());
                let begin_warp = |app: &mut PhotocraftApp, ctx: &egui::Context| -> Result<(), String> {
                    begin(app, ctx)?;
                    enter_warp(app, ctx);
                    Ok(())
                };
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
            let name = kind_with(k).0;
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
                    (Err(a), Err(b)) if name == "type" && matches!(mode, TransformMode::Distort | TransformMode::Perspective) && a == b => {
                        format!("{label} refused")
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
