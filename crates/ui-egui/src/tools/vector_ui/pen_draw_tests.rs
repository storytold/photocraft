//! The Pen through the real canvas (#3306, #6592): a finished path belongs to its document, Delete
//! deletes the finished Work Path, and a Space pan keeps the path in progress.

use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::canvas::testing;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 75.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

/// The screen point of the document point `p` in the harness's canvas.
fn screen(h: &mut Harness<'static, Box<PhotocraftApp>>, p: [f32; 2]) -> Pos2 {
    testing::to_screen(h.state_mut(), [f64::from(p[0]), f64::from(p[1])])
}

fn mods(h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers) {
    h.event(egui::Event::ModifiersChanged(m));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool, m: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: m });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, Box<PhotocraftApp>>, from: [f32; 2], to: [f32; 2]) {
    for i in 1..=6 {
        let t = i as f32 / 6.0;
        let q = screen(h, [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
}

/// A drag from `pts[0]` through the other points; `alt_from` is the index of the point from
/// which ⌥/Alt is held (`Some(0)`: before the press).
fn drag(h: &mut Harness<'static, Box<PhotocraftApp>>, pts: &[[f32; 2]], alt_from: Option<usize>) {
    let held = |i: usize| if alt_from.is_some_and(|a| i >= a) { Modifiers::ALT } else { Modifiers::NONE };
    let p = screen(h, pts[0]);
    mods(h, held(0));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, held(0));
    for (i, w) in pts.windows(2).enumerate() {
        mods(h, held(i + 1));
        move_to(h, w[0], w[1]);
    }
    let end = *pts.last().unwrap();
    let q = screen(h, end);
    button(h, q, false, held(pts.len() - 1));
    mods(h, Modifiers::NONE);
    h.run_steps(1);
}

pub(super) fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: [f32; 2]) {
    drag(h, &[p], None);
}

#[test]
fn opening_a_new_document_finishes_the_previous_pen_path() {
    let mut h = harness();
    click(&mut h, [20.0, 20.0]);
    click(&mut h, [120.0, 20.0]);
    assert_eq!(h.state().ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(2));
    let original = h.state().session.active().unwrap().doc.id;

    h.state_mut().run("file.new", json!({"width": 300, "height": 200})).unwrap();
    assert!(h.state().ui.pen.is_none(), "the Pen draft must not overlay the new document");
    let docs = h.state().session.documents();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].doc.id, original);
    assert_eq!(docs[0].doc.work_path.as_ref().unwrap().subpaths[0].knots.len(), 2);
    assert!(docs[1].doc.work_path.is_none(), "the new document starts with no path");
}

/// #5702: a finished work path showed again whenever the Pen came back, and ⌫ / Delete reached
/// Edit › Clear (the pixels under it) instead, so the path looked impossible to delete. With the
/// Pen the key deletes the path, in one undoable step, and leaves the pixels; other tools keep
/// Edit › Clear.
#[test]
fn delete_with_the_pen_deletes_the_finished_work_path_not_the_pixels() {
    let mut h = harness();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut()
        .session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 200, 150), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    for p in [[20.0, 20.0], [120.0, 20.0], [120.0, 100.0]] {
        click(&mut h, p);
    }
    // Esc keeps the finished Work Path selected; ↩ would deselect it (#12892).
    h.key_press(Key::Escape);
    h.run_steps(2);
    let work = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().session.active().unwrap().doc.work_path.is_some();
    let red = |h: &Harness<'static, Box<PhotocraftApp>>| {
        let st = h.state().session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, 60) == [1.0, 0.0, 0.0, 1.0]
    };
    assert!(work(&h) && h.state().ui.pen.is_none(), "a finished work path");
    let steps = h.state().session.active().unwrap().history.past_len();

    h.key_press(Key::Delete);
    h.run_steps(2);
    assert!(!work(&h), "Delete deletes the work path");
    assert!(red(&h), "and leaves the pixels under it");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one History step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(work(&h), "Undo brings it back");

    h.state_mut().ui.tool = Tool::PathSelection;
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert!(!work(&h), "Backspace with Path Selection deletes it too");
    h.state_mut().run("edit.undo", json!({})).unwrap();

    h.state_mut().ui.tool = Tool::Brush;
    h.key_press(Key::Delete);
    h.run_steps(2);
    assert!(work(&h), "another tool's Delete is Edit › Clear, not Delete Path");
}

/// #6592: holding Space to pan with the Hand keeps the Pen's path in progress, as in Photoshop.
#[test]
fn space_pan_keeps_the_pen_path_in_progress_visible() {
    let mut h = harness();
    let space = |h: &mut Harness<'static, Box<PhotocraftApp>>, down: bool| {
        h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    };
    let knots = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().ui.pen.as_ref().map(|p| p.draft.knots.len());
    click(&mut h, [20.0, 20.0]);
    click(&mut h, [100.0, 20.0]);
    assert_eq!(knots(&h), Some(2));
    let p = screen(&mut h, [150.0, 100.0]);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    space(&mut h, true);
    let c0 = h.state().ui.views[0].center;
    button(&mut h, p, true, Modifiers::NONE);
    for i in 1..=5 {
        h.event(egui::Event::PointerMoved(p + vec2(10.0 * i as f32, 0.0)));
        h.run_steps(1);
        assert_eq!(knots(&h), Some(2), "the path in progress is kept while panning (step {i})");
    }
    button(&mut h, p + vec2(50.0, 0.0), false, Modifiers::NONE);
    space(&mut h, false);
    h.run_steps(1);
    let c1 = h.state().ui.views[0].center;
    assert!((c1[0] - (c0[0] - 50.0)).abs() < 0.5, "Space-drag panned: {c0:?} -> {c1:?}");
    assert_eq!(h.state().ui.tool, Tool::Pen);
    assert_eq!(knots(&h), Some(2), "the path in progress survives the pan");
}
