//! Mode-switch pops: every short sequence of Free Transform modes, small edits and redraws on a smart object whose
//! colour encodes position (red across, green down), on the real canvas. A mode switch or a redraw must not move the
//! picture, and Enter must leave what the preview showed: each sample point's colour says which point of the object is
//! drawn there, so a pop reads as a jump in content pixels.
//!
//! `cargo test -p photocraft-ui-egui --test it transform_pops -- --ignored --nocapture` (POPS_LEN, default 3; POPS_GPU=1
//! for the GPU canvas; POPS_SUBJECT=smart|pixel|turned|warped for the object, see [`subject`]).

use craft::behavior::kurbo::Point;
use egui::{Modifiers, PointerButton, Pos2};
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The object: x 60..240, y 90..210 of a 400 × 300 document.
const OBJ: [i32; 4] = [60, 90, 240, 210];

fn harness(gpu_canvas: bool) -> Option<Harness> {
    let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = entered.clone();
    let built = std::panic::catch_unwind(move || {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
            if gpu_canvas && let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    match built {
        Ok(h) => Some(h),
        Err(panic) if entered.load(std::sync::atomic::Ordering::SeqCst) => std::panic::resume_unwind(panic),
        Err(_) => None,
    }
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

fn screen(h: &Harness, x: f64, y: f64) -> Pos2 {
    photocraft_ui_egui::canvas::active_view(h.state()).expect("an active document view").to_screen(Point::new(x, y))
}

fn pointer_move(h: &mut Harness, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
}

fn button(h: &mut Harness, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    pointer_move(h, from);
    button(h, from, true);
    for i in 1..=8 {
        pointer_move(h, from + (to - from) * (i as f32 / 8.0));
    }
    button(h, to, false);
    h.run_steps(4);
}

/// A white 400 × 300 document with the position-coded object as a smart object, at 100 %.
fn setup(h: &mut Harness, subject: &str) {
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.run("layer.new.layer", json!({})).expect("new layer");
        app.session
            .edit("paint", |doc, active| {
                let id = active.expect("the new layer is active");
                let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).expect("a raster layer");
                let (w, hh) = (f64::from(OBJ[2] - OBJ[0] - 1), f64::from(OBJ[3] - OBJ[1] - 1));
                for y in OBJ[1]..OBJ[3] {
                    for x in OBJ[0]..OBJ[2] {
                        let c = [f64::from(x - OBJ[0]) / w, f64::from(y - OBJ[1]) / hh, 0.5, 1.0].map(|v| v as f32);
                        surface.fill_rect(Rect::new(x, y, x + 1, y + 1), &c);
                    }
                }
                Ok(())
            })
            .expect("paint");
        if subject != "pixel" {
            app.run("layer.smartObjects.convertToSmartObject", json!({})).expect("smart object");
        }
        if subject == "turned" {
            app.run("edit.transform", json!({"rotate": 15.0})).expect("turn");
        }
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.panels.context_task_bar = false;
    }
    control(h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    h.run_steps(3);
    if subject == "warped" {
        // A smart object that already carries a warp of its own: one pull, applied.
        control(h, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
        h.run_steps(4);
        let (from, to) = (screen(h, 150.5, 150.5), screen(h, 160.5, 157.5));
        drag(h, from, to);
        h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(8);
        let st = h.state().session.active().expect("a document");
        let layer = st.active_layer.and_then(|id| st.doc.layer(id)).expect("the object");
        let photocraft_doc::LayerContent::Smart(sm) = &layer.content else { panic!("a smart object") };
        assert!(sm.warp.as_ref().is_some_and(|w| !w.is_identity()), "the warped subject carries a warp");
    }
}

/// The object the sweep runs on (POPS_SUBJECT): `smart` (default), `pixel` (a plain pixel layer), `turned` (a smart
/// object turned 15°), `warped` (a smart object with a warp of its own).
fn subject() -> String {
    std::env::var("POPS_SUBJECT").unwrap_or_else(|_| "smart".into())
}

/// Document points sampled: a grid well inside the object's start rect.
fn samples() -> Vec<(f64, f64)> {
    let mut v = Vec::new();
    for j in 0..4 {
        for i in 0..5 {
            v.push((95.5 + 28.0 * f64::from(i), 112.5 + 26.0 * f64::from(j)));
        }
    }
    v
}

/// What the canvas shows at each sample: the object's content point (in its pixels) or `None` for the background. The
/// transform's controls (outline, cross, grid) draw over the picture in other colours: each sample reads the nearest
/// pixel, within 4 px, that still carries the object's blue.
fn capture(h: &mut Harness) -> Vec<Option<(f64, f64)>> {
    let points: Vec<Pos2> = samples().iter().map(|&(x, y)| screen(h, x, y)).collect();
    let img = h.render().expect("render");
    let (w, hh) = (f64::from(OBJ[2] - OBJ[0] - 1), f64::from(OBJ[3] - OBJ[1] - 1));
    points
        .iter()
        .map(|p| {
            let (cx, cy) = (p.x.floor() as i64, p.y.floor() as i64);
            let mut best: Option<(i64, (f64, f64))> = None;
            for dy in -4i64..=4 {
                for dx in -4i64..=4 {
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= i64::from(img.width()) || y >= i64::from(img.height()) {
                        continue;
                    }
                    let px = img.get_pixel(x as u32, y as u32);
                    if (i64::from(px[2]) - 128).abs() > 2 {
                        continue;
                    }
                    let d = dx * dx + dy * dy;
                    // The content point at the sample itself: this pixel's, less its offset (1 content px per screen px
                    // near 100 %, so a few px of slack stay well under a pop).
                    let at = (f64::from(px[0]) / 255.0 * w - dx as f64, f64::from(px[1]) / 255.0 * hh - dy as f64);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, at));
                    }
                }
            }
            best.map(|(_, at)| at)
        })
        .collect()
}

/// How far the picture moved between two captures, in content pixels: the median over the samples (a pop moves the
/// whole picture; the transform's lines spoil a sample or two), a sample turning to or from the background counting as
/// `inf`.
fn jump(a: &[Option<(f64, f64)>], b: &[Option<(f64, f64)>]) -> f64 {
    let mut d: Vec<f64> = a
        .iter()
        .zip(b)
        .map(|(p, q)| match (p, q) {
            (Some(p), Some(q)) => (p.0 - q.0).hypot(p.1 - q.1),
            (None, None) => 0.0,
            _ => f64::INFINITY,
        })
        .collect();
    d.sort_by(f64::total_cmp);
    d.get(d.len() / 2).copied().unwrap_or(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Warp,
    Persp,
    Distort,
    Scale,
    Free,
    Redraw,
    PerspDrag,
    DistortDrag,
    Pull,
}

impl Step {
    fn menu(self) -> Option<&'static str> {
        Some(match self {
            Step::Warp => "edit.transform.warp",
            Step::Persp => "edit.transform.perspective",
            Step::Distort => "edit.transform.distort",
            Step::Scale => "edit.transform.scale",
            Step::Free => "edit.freeTransform",
            _ => return None,
        })
    }
}

/// One sequence; `Err` names the first pop. `Ok` carries what the canvas showed before Enter.
/// POPS_DUMP=<dir>: save what the canvas shows under `name`, and print the box, for looking at one sequence.
fn dump(h: &mut Harness, name: &str) {
    let Ok(dir) = std::env::var("POPS_DUMP") else { return };
    let quad = h.state().ui.transform.as_ref().map(|t| t.quad_array());
    eprintln!("dump {name}: box {quad:?}");
    let img = h.render().expect("render");
    img.save(std::path::Path::new(&dir).join(format!("{name}.png"))).expect("save");
}

fn run(gpu: bool, steps: &[Step]) -> Result<Vec<Option<(f64, f64)>>, String> {
    run_on(gpu, &subject(), steps)
}

/// [`run`] on a named subject.
fn run_on(gpu: bool, subject: &str, steps: &[Step]) -> Result<Vec<Option<(f64, f64)>>, String> {
    let Some(mut h) = harness(gpu) else { return Ok(Vec::new()) };
    setup(&mut h, subject);
    // A capture that finds no object proves nothing: the object must show at most sample points before the first step.
    let seen = capture(&mut h).iter().filter(|c| c.is_some()).count();
    if seen * 4 < samples().len() * 3 {
        return Err(format!("setup: the object shows at only {seen} of {} sample points", samples().len()));
    }
    let mut mode: Option<Step> = None;
    for (n, &step) in steps.iter().enumerate() {
        let before = capture(&mut h);
        let edit = match step {
            s if s.menu().is_some() => {
                control(&mut h, "ui.menu.invoke", json!({"id": s.menu().expect("a menu id")}));
                mode = Some(s);
                false
            }
            Step::Redraw => {
                let at = screen(&h, 320.0, 40.0);
                pointer_move(&mut h, at);
                h.run_steps(6);
                false
            }
            Step::PerspDrag if mode == Some(Step::Persp) => {
                let q = h.state().ui.transform.as_ref().map(|t| t.quad_array()).ok_or("no box")?;
                let (from, to) = (screen(&h, q[0][0], q[0][1]), screen(&h, q[0][0] + 60.0, q[0][1]));
                drag(&mut h, from, to);
                true
            }
            Step::DistortDrag if mode == Some(Step::Distort) => {
                let q = h.state().ui.transform.as_ref().map(|t| t.quad_array()).ok_or("no box")?;
                let (from, to) = (screen(&h, q[2][0], q[2][1]), screen(&h, q[2][0] + 20.0, q[2][1] - 15.0));
                drag(&mut h, from, to);
                true
            }
            Step::Pull if mode == Some(Step::Warp) => {
                let (from, to) = (screen(&h, 150.5, 150.5), screen(&h, 165.5, 160.5));
                drag(&mut h, from, to);
                true
            }
            _ => return Ok(Vec::new()),
        };
        h.run_steps(4);
        dump(&mut h, &format!("step{n}-{step:?}"));
        let after = capture(&mut h);
        let j = jump(&before, &after);
        if !edit && j > 3.0 {
            return Err(format!("step {n} {step:?}: the picture moved {j:.1} content px without an edit"));
        }
        // A second look after a forced redraw: nothing may change by itself.
        let at = screen(&h, 330.0, 45.0);
        pointer_move(&mut h, at);
        h.run_steps(6);
        let again = capture(&mut h);
        let j = jump(&after, &again);
        if j > 3.0 {
            return Err(format!("step {n} {step:?}: the picture moved {j:.1} content px on a redraw"));
        }
    }
    let shown = capture(&mut h);
    if mode.is_some() {
        let before = shown.clone();
        h.event(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(8);
        dump(&mut h, "enter");
        let after = capture(&mut h);
        let j = jump(&before, &after);
        if j > 3.0 {
            return Err(format!("Enter:the result differs from the preview by {j:.1} content px"));
        }
    }
    Ok(shown)
}

#[test]
#[ignore = "long: every short sequence of modes and edits (see the module docs)"]
fn transform_pops() {
    let _gpu = photocraft_testkit::gpu_lock();
    let gpu = std::env::var("POPS_GPU").is_ok();
    let len: usize = std::env::var("POPS_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let vocab = [Step::Warp, Step::Persp, Step::Distort, Step::Scale, Step::Free, Step::Redraw, Step::PerspDrag, Step::DistortDrag, Step::Pull];
    let mut seqs: Vec<Vec<Step>> = vec![Vec::new()];
    for _ in 0..len {
        let mut next = Vec::new();
        for s in &seqs {
            for &v in &vocab {
                let mut t = s.clone();
                t.push(v);
                next.push(t);
            }
        }
        seqs.extend(next.clone());
        seqs.retain(|s| s.len() <= len);
        seqs.sort();
        seqs.dedup();
    }
    // Only sequences that start with a mode and whose edits come in their mode.
    let valid = |s: &[Step]| {
        let mut mode = None;
        for &st in s {
            match st {
                m if m.menu().is_some() => mode = Some(m),
                Step::Redraw => {}
                Step::PerspDrag if mode == Some(Step::Persp) => {}
                Step::DistortDrag if mode == Some(Step::Distort) => {}
                Step::Pull if mode == Some(Step::Warp) => {}
                _ => return false,
            }
        }
        s.first().is_some_and(|f| f.menu().is_some())
    };
    seqs.retain(|s| valid(s));
    // POPS_SEQ=Persp,PerspDrag,...: only that sequence (with POPS_DUMP to look at it).
    if let Ok(one) = std::env::var("POPS_SEQ") {
        seqs.retain(|s| format!("{s:?}").replace(' ', "") == format!("[{one}]"));
        assert!(!seqs.is_empty(), "POPS_SEQ names no valid sequence of up to POPS_LEN steps");
    }
    let mut failures = Vec::new();
    for (i, s) in seqs.iter().enumerate() {
        if i % 100 == 0 {
            eprintln!("progress: {i} of {} sequences, {} with a pop", seqs.len(), failures.len());
        }
        match run(gpu, s) {
            Err(e) => {
                eprintln!("POP {s:?}: {e}");
                failures.push((s.clone(), e));
            }
            // An untouched warp is no warp (the reference's picture is the same with and without it): with no pull, the
            // picture must be what the same steps without Warp show.
            Ok(shown) if s.contains(&Step::Warp) && !s.contains(&Step::Pull) => {
                let plain: Vec<Step> = s.iter().copied().filter(|&st| st != Step::Warp).collect();
                if plain.first().is_some_and(|f| f.menu().is_some())
                    && let Ok(want) = run(gpu, &plain)
                    && !want.is_empty()
                {
                    let j = jump(&shown, &want);
                    if j > 3.0 {
                        let e = format!("an untouched warp changed the picture by {j:.1} content px (against {plain:?})");
                        eprintln!("POP {s:?}: {e}");
                        failures.push((s.clone(), e));
                    }
                }
            }
            Ok(_) => {}
        }
    }
    eprintln!("{} sequences, {} with a pop", seqs.len(), failures.len());
    assert!(failures.is_empty(), "{} sequences pop", failures.len());
}

/// The orders recorded on the reference application (none of them moves its picture, on a smart object or a pixel
/// layer): Warp left untouched, then
/// Perspective and a drag, then Warp again; Perspective and a drag, Warp, Perspective; a real pull, then Perspective, a
/// drag and Warp; Distort and a drag, Warp, Distort. Checked on every run, on the CPU canvas.
#[test]
fn the_recorded_orders_keep_the_picture() {
    let _gpu = photocraft_testkit::gpu_lock();
    use Step::*;
    let orders: [&[Step]; 4] =
        [&[Warp, Persp, PerspDrag, Warp], &[Persp, PerspDrag, Warp, Persp], &[Warp, Pull, Persp, PerspDrag, Warp], &[Distort, DistortDrag, Warp, Distort]];
    for subject in ["smart", "pixel"] {
        for order in orders {
            let shown = run_on(false, subject, order).unwrap_or_else(|e| panic!("{subject} {order:?}: {e}"));
            // An untouched warp is no warp.
            if !order.contains(&Pull) {
                let plain: Vec<Step> = order.iter().copied().filter(|&s| s != Warp).collect();
                let want = run_on(false, subject, &plain).unwrap_or_else(|e| panic!("{subject} {plain:?}: {e}"));
                if !shown.is_empty() && !want.is_empty() {
                    let j = jump(&shown, &want);
                    assert!(j <= 3.0, "{subject} {order:?}: an untouched warp changed the picture by {j:.1} content px");
                }
            }
        }
    }
}

/// Deliberate parity: after Perspective, Warp shows the box's exact map until the first warp edit, which then draws the
/// warp from its carried control points and so redistributes the picture once, as the reference does (its recording:
/// Perspective with the top-left corner 90 px in, Warp, a 6 px pull at (330, 280): the red/blue boundary moved from row
/// 310 to 324). Replayed on the reference's probe document; the jump must match its 14 px within 2. The reference saves
/// placed warps in that carried model, so this is how PSD files render too: don't smooth it.
#[test]
fn the_first_warp_pull_after_perspective_redistributes_like_the_reference() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(moved) = first_pull(true, 240.0, [336.0, 280.0], &[200.0, 230.0, 260.0]) else { return };
    for (i, d) in moved.iter().enumerate().skip(1) {
        assert!((d - 14.0).abs() <= 2.0, "column {i}: the boundary moved {d} px on the first pull; the reference's moved 14");
    }
}

/// The same on a pixel layer (the reference's recording: the top-left corner 100 px in, Warp, a pull from (330, 280) to
/// (336, 286): the red/blue boundary at columns 230 and 260 moved 16 and 19 px).
#[test]
fn the_first_warp_pull_after_perspective_redistributes_a_pixel_layer_like_the_reference() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(moved) = first_pull(false, 250.0, [336.0, 286.0], &[230.0, 260.0]) else { return };
    for (d, want) in moved.iter().zip([16.0, 19.0]) {
        assert!((d - want).abs() <= 2.0, "the boundary moved {d} px on the first pull; the reference's moved {want}: {moved:?}");
    }
}

/// The reference's probe document (quadrants red, green, blue, gold over (150, 125)-(450, 325)), as a smart object or a
/// pixel layer: Perspective with the top-left corner dragged to `corner_x`, Warp, a pull from (330, 280) to `pull_to`.
/// How far the red/blue boundary moved at each of `cols`; `None` without a canvas.
fn first_pull(smart: bool, corner_x: f64, pull_to: [f64; 2], cols: &[f64]) -> Option<Vec<f64>> {
    let mut h = harness(false)?;
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 600, "height": 450})).expect("new");
        app.run("layer.new.layer", json!({})).expect("new layer");
        app.session
            .edit("paint", |doc, active| {
                let id = active.expect("active");
                let s = doc.layer_mut(id).and_then(|l| l.surface_mut()).expect("raster");
                s.fill_rect(Rect::new(150, 125, 300, 225), &[1.0, 0.0, 0.0, 1.0]);
                s.fill_rect(Rect::new(300, 125, 450, 225), &[0.0, 0.784, 0.0, 1.0]);
                s.fill_rect(Rect::new(150, 225, 300, 325), &[0.0, 0.0, 1.0, 1.0]);
                s.fill_rect(Rect::new(300, 225, 450, 325), &[1.0, 0.784, 0.0, 1.0]);
                Ok(())
            })
            .expect("paint");
        if smart {
            app.run("layer.smartObjects.convertToSmartObject", json!({})).expect("smart object");
        }
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.panels.context_task_bar = false;
    }
    control(&mut h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [300, 225]}));
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.transform.perspective"}));
    let (a, b) = (screen(&h, 150.0, 125.0), screen(&h, corner_x, 125.0));
    drag(&mut h, a, b);
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
    h.run_steps(6);
    let boundary = |h: &mut Harness| -> Vec<Option<f64>> {
        let cols: Vec<(f64, Pos2)> = cols.iter().map(|&x| (x, screen(h, x, 0.0))).collect();
        let img = h.render().expect("render");
        cols.iter()
            .map(|(_, p)| {
                (0..img.height())
                    .find(|&y| {
                        let px = img.get_pixel(p.x.floor() as u32, y);
                        px[2] > 200 && px[0] < 60 && px[1] < 60
                    })
                    .map(|y| f64::from(y))
            })
            .collect()
    };
    let before = boundary(&mut h);
    let (a, b) = (screen(&h, 330.0, 280.0), screen(&h, pull_to[0], pull_to[1]));
    drag(&mut h, a, b);
    let after = boundary(&mut h);
    Some(
        before
            .iter()
            .zip(&after)
            .enumerate()
            .map(|(i, (b, a))| match (b, a) {
                (Some(b), Some(a)) => a - b,
                _ => panic!("column {i}: no boundary: {before:?} {after:?}"),
            })
            .collect(),
    )
}

/// Perspective with two drags, then Warp left untouched, then Enter: the result is what the preview showed, the box's
/// exact map (an untouched warp is no warp). Stored as an identity warp, the render drew it from control points carried
/// through the perspective, 8 content px off the preview (found by the sweep).
#[test]
fn enter_on_an_untouched_warp_after_perspective_applies_what_the_preview_shows() {
    let _gpu = photocraft_testkit::gpu_lock();
    use Step::*;
    let order = [Persp, PerspDrag, PerspDrag, Warp];
    for subject in ["smart", "pixel"] {
        run_on(false, subject, &order).unwrap_or_else(|e| panic!("{subject} {order:?}: {e}"));
    }
}

/// Perspective with the top edge squeezed to a point (the same corner dragged twice), then Enter: the layer is the
/// triangle the preview showed. The commit's pre-reduction took the scale at the centre and rounded its frame outward
/// past the map's horizon, which left a 2 × 1 px pixel layer (found by the sweep). A warped smart object's preview
/// carried its control points through the raw map while the render carries them through its capped one, so near the
/// point the preview showed the middle rows where the result has the top ones.
#[test]
fn enter_on_a_perspective_squeezed_to_a_point_applies_what_the_preview_shows() {
    let _gpu = photocraft_testkit::gpu_lock();
    use Step::*;
    let order = [Persp, PerspDrag, PerspDrag];
    for subject in ["pixel", "warped"] {
        run_on(false, subject, &order).unwrap_or_else(|e| panic!("{subject} {order:?}: {e}"));
    }
}
