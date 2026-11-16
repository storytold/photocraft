//! #14985: with a Satin page selected, a drag on the image changes the Satin's Angle and
//! Distance, and the options bar says so.
use super::*;

/// The press start time the canvas passes for one gesture.
const PRESS: f64 = 1.0;

fn fields(kind: &str) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("selected".into(), json!("fx1"));
    f.insert("globalLight".into(), json!(120.0));
    f.insert(
        "effects".into(),
        json!([
            {"id": "fx1", "kind": kind, "on": true, "params": {"angle": 90.0, "distance": 20.0, "useGlobalLight": false}},
            {"id": "fx2", "kind": "dropShadow", "on": true, "params": {"angle": 120.0, "distance": 10.0, "useGlobalLight": true}},
        ]),
    );
    f
}

fn params(f: &Map<String, Value>, id: &str) -> (f64, f64) {
    let p = &entry(f, id).unwrap()["params"];
    (p["angle"].as_f64().unwrap(), p["distance"].as_f64().unwrap())
}

/// Resets the Satin to 90 / 20, presses, and drags by `total`.
fn drag_from_90_20(f: &mut Map<String, Value>, total: [f64; 2]) -> (f64, f64) {
    set_param(f, "fx1", "angle", json!(90.0));
    set_param(f, "fx1", "distance", json!(20.0));
    drag_satin(f, true, [0.0, 0.0], PRESS);
    drag_satin(f, false, total, PRESS);
    params(f, "fx1")
}

/// #14985: each drag from the press gives the measured angle (truncated) and distance (truncated).
#[test]
fn a_satin_drag_sets_the_truncated_angle_and_distance() {
    let mut f = fields("satin");
    assert!(drags_satin(&f));
    drag_satin(&mut f, true, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0), "a plain press keeps the satin where it is");
    for (total, want) in [
        ([30.0, 0.0], (146.0, 36.0)),
        ([0.0, -30.0], (-90.0, 10.0)),
        ([0.0, 30.0], (90.0, 50.0)),
        ([-30.0, 0.0], (33.0, 36.0)),
        ([20.0, -20.0], (180.0, 20.0)),
        ([5.0, 0.0], (104.0, 20.0)),
        // Measured later on the issue: negative angles truncate toward zero, and Shift (which the
        // drag ignores) gave (+30, +10) -> 135 / 42.
        ([5.0, -30.0], (-116.0, 11.0)),
        ([-5.0, -30.0], (-63.0, 11.0)),
        ([30.0, 10.0], (135.0, 42.0)),
    ] {
        assert_eq!(drag_from_90_20(&mut f, total), want, "drag {total:?}");
    }
}

/// #14985: a drag that brings the vector to (0, 0) gives Distance 1 and keeps the previous Angle.
#[test]
fn a_satin_drag_to_the_zero_vector_keeps_the_angle_at_distance_one() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [0.0, -20.0]), (90.0, 1.0));
}

/// #14985: Distance stays within the slider's 1 to 250 px, and a non-finite drag changes nothing.
#[test]
fn a_satin_drag_clamps_the_distance_and_ignores_non_finite_input() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [0.0, 380.0]).1, 250.0);
    assert_eq!(drag_from_90_20(&mut f, [f64::NAN, 0.0]), (90.0, 20.0));
}

/// #14985: only a Satin page takes the drag, and only it has the options bar hint.
#[test]
fn only_the_satin_page_takes_the_drag_and_the_hint() {
    let hint = "Click and drag to adjust the satin effect.";
    assert_eq!(canvas_drag_hint(&fields("satin")), Some(hint));
    for kind in ["dropShadow", "innerShadow", "gradientOverlay"] {
        let mut f = fields(kind);
        assert!(!drags_satin(&f), "{kind}");
        assert_eq!(canvas_drag_hint(&f), None, "{kind}");
        drag_satin(&mut f, true, [0.0, 0.0], PRESS);
        drag_satin(&mut f, false, [30.0, 0.0], PRESS);
        assert_eq!(params(&f, "fx1"), (90.0, 20.0), "{kind}: nothing moves");
    }
    let mut f = fields("satin");
    f.insert("selected".into(), json!(BLENDING));
    assert!(!drags_satin(&f));
    assert_eq!(canvas_drag_hint(&f), None);
    drag_satin(&mut f, true, [0.0, 0.0], PRESS);
    drag_satin(&mut f, false, [30.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0));
}

/// #14985: a Satin ignores Global Light, so its drag leaves the shared angle and other effects
/// alone, and the shadow drag does not take Satin pages.
#[test]
fn a_satin_drag_leaves_global_light_and_the_shadow_drag_alone() {
    let mut f = fields("satin");
    assert!(!drags_shadow(&f));
    drag_shadow(&mut f, true, [0.0, 0.0]);
    drag_shadow(&mut f, false, [30.0, 0.0]);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0), "the shadow drag ignores a Satin page");
    drag_from_90_20(&mut f, [30.0, 0.0]);
    assert_eq!(f["globalLight"], json!(120.0));
    assert!(f.get("pendingGlobalLight").is_none());
    assert_eq!(params(&f, "fx2"), (120.0, 10.0));
}

/// #14985: the canvas's one offset drag takes Satin and shadow pages alike and hands each to its
/// own drag.
#[test]
fn the_canvas_offset_drag_routes_satin_and_shadow_pages() {
    let mut f = fields("satin");
    assert!(drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    drag_offset(&mut f, false, [5.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (104.0, 20.0), "truncated, as the satin drag does");
    f.insert("selected".into(), json!("fx2"));
    assert!(drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx2"), (120.0, 10.0));
    let mut f = fields("gradientOverlay");
    assert!(!drags_offset(&f));
    drag_offset(&mut f, true, [0.0, 0.0], PRESS);
    drag_offset(&mut f, false, [30.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.0, 20.0));
}

/// #14985: one drag continuing past the zero vector keeps the Angle shown (146), not the press's.
#[test]
fn a_continuing_drag_through_the_zero_vector_keeps_the_displayed_angle() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    drag_satin(&mut f, false, [0.0, -20.0], PRESS);
    assert_eq!(params(&f, "fx1"), (146.0, 1.0));
}

/// #14985: a press the drag did not see begin (another press time) moves the satin only by the travel
/// since the drag took over, from the values shown, never from an earlier gesture's.
#[test]
fn a_press_the_drag_did_not_see_begin_rebaselines_from_the_shown_values() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    set_param(&mut f, "fx1", "angle", json!(0.0));
    set_param(&mut f, "fx1", "distance", json!(40.0));
    let b = 2.0;
    drag_satin(&mut f, false, [10.0, 0.0], b);
    assert_eq!(params(&f, "fx1"), (0.0, 40.0), "nothing jumps to the old gesture's values");
    // Offset (-40, 0) + travel (10, 30) = (-30, 30): 45 degrees, 42 px.
    drag_satin(&mut f, false, [20.0, 30.0], b);
    assert_eq!(params(&f, "fx1"), (45.0, 42.0));
    // The same press goes on from that baseline: (-40, 0) + (0, 40) = (-40, 40).
    drag_satin(&mut f, false, [10.0, 40.0], b);
    assert_eq!(params(&f, "fx1"), (45.0, 56.0));
}

/// #14985: a new gesture that begins at the same spot as the last one (a middle-button pan pressed
/// there during a held press, then released) is still another press: the drag re-baselines from
/// the values shown instead of restoring the earlier gesture's.
#[test]
fn a_new_press_at_the_same_spot_rebaselines_from_the_shown_values() {
    let mut f = fields("satin");
    assert_eq!(drag_from_90_20(&mut f, [30.0, 0.0]), (146.0, 36.0));
    set_param(&mut f, "fx1", "angle", json!(0.0));
    set_param(&mut f, "fx1", "distance", json!(40.0));
    // Same position, so the same travel as the old gesture's last frame, but a later press.
    drag_satin(&mut f, false, [30.0, 0.0], PRESS + 1.0);
    assert_eq!(params(&f, "fx1"), (0.0, 40.0), "nothing jumps back to 146 / 36");
    drag_satin(&mut f, false, [40.0, 0.0], PRESS + 1.0);
    assert_eq!(params(&f, "fx1"), (0.0, 30.0), "(-40, 0) + (10, 0) = (-30, 0)");
}

/// #14985: a press with no travel keeps fractional values (from an imported file, say) as they
/// are, and so does a drag that comes back to where it began: nothing was dragged.
#[test]
fn no_travel_keeps_fractional_values() {
    let mut f = fields("satin");
    set_param(&mut f, "fx1", "angle", json!(90.5));
    set_param(&mut f, "fx1", "distance", json!(20.75));
    drag_satin(&mut f, true, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.5, 20.75));
    drag_satin(&mut f, false, [30.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (145.0, 36.0));
    drag_satin(&mut f, false, [0.0, 0.0], PRESS);
    assert_eq!(params(&f, "fx1"), (90.5, 20.75));
}

mod on_the_canvas {
    //! #14985 through the canvas: the drag counts in document pixels at every zoom, Shift
    //! changes nothing, the preview shows the drag while the document keeps its style, and
    //! Cancel leaves the document as it was.
    use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use crate::PhotocraftApp;

    /// A 400×300 document with a pixel layer at `zoom`, Layer Style open on a Satin page set
    /// to Angle 90 / Distance 20.
    fn harness(zoom: f32) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::shortcuts::handle(app, &ctx);
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
                crate::dialogs::show(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let v = &mut h.state_mut().ui.views[0];
        (v.zoom, v.center, v.fit_pending) = (zoom, [200.0, 150.0], false);
        crate::layer_style::open(h.state_mut(), Some("satin")).unwrap();
        let f = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        let id = f["selected"].as_str().unwrap().to_string();
        super::set_param(f, &id, "angle", json!(90.0));
        super::set_param(f, &id, "distance", json!(20.0));
        h.run_steps(4);
        h
    }

    fn satin(h: &Harness<'static, PhotocraftApp>) -> (f64, f64) {
        let f = &h.state().ui.dialogs.last().unwrap().fields;
        let p = &super::entry(f, f["selected"].as_str().unwrap()).unwrap()["params"];
        (p["angle"].as_f64().unwrap(), p["distance"].as_f64().unwrap())
    }

    fn step(h: &mut Harness<'static, PhotocraftApp>, e: egui::Event) {
        h.event(e);
        h.run_steps(1);
    }

    /// A press on the free image left of the dialog, moved by `by` screen points, held.
    fn drag(h: &mut Harness<'static, PhotocraftApp>, by: egui::Vec2, modifiers: Modifiers) {
        let r = h.state().last_canvas_rect;
        let a: Pos2 = pos2(r.left() + 60.0, r.center().y);
        step(h, egui::Event::ModifiersChanged(modifiers));
        step(h, egui::Event::PointerMoved(a));
        step(h, egui::Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers });
        step(h, egui::Event::PointerMoved(a + by * 0.5));
        step(h, egui::Event::PointerMoved(a + by));
    }

    #[test]
    fn a_drag_at_200_percent_counts_in_document_pixels() {
        let mut h = harness(2.0);
        drag(&mut h, vec2(60.0, 0.0), Modifiers::NONE);
        assert_eq!(satin(&h), (146.0, 36.0), "60 screen px at 200 % is 30 px");
        let mut h = harness(2.0);
        drag(&mut h, vec2(10.0, -60.0), Modifiers::NONE);
        assert_eq!(satin(&h), (-116.0, 11.0));
    }

    #[test]
    fn shift_changes_nothing() {
        let mut h = harness(1.0);
        drag(&mut h, vec2(30.0, 10.0), Modifiers::SHIFT);
        assert_eq!(satin(&h), (135.0, 42.0), "Shift neither snaps the angle nor changes the result");
        let mut h = harness(1.0);
        drag(&mut h, vec2(5.0, -30.0), Modifiers::SHIFT);
        assert_eq!(satin(&h), (-116.0, 11.0));
    }

    #[test]
    fn the_preview_follows_the_drag_and_cancel_leaves_the_document() {
        let mut h = harness(1.0);
        drag(&mut h, vec2(30.0, 0.0), Modifiers::NONE);
        assert_eq!(satin(&h), (146.0, 36.0));
        let app = h.state();
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let fx = |doc: &photocraft_doc::Document| doc.layer(layer).unwrap().effects.items.clone();
        let f = &app.ui.dialogs.last().unwrap().fields;
        let shown = super::preview_document(&st.doc, &app.session.patterns, f).unwrap();
        let satin_of = |items: &[photocraft_doc::Effect]| {
            items.iter().find_map(|e| match e {
                photocraft_doc::Effect::Satin(s) => Some((s.angle, s.distance)),
                _ => None,
            })
        };
        assert_eq!(satin_of(&fx(&shown)), Some((146.0, 36.0)), "the preview shows the dragged satin");
        assert_eq!(satin_of(&fx(&st.doc)), None, "the document is untouched while the dialog is open");
        let id = app.ui.dialogs.last().unwrap().id;
        crate::dialogs::cancel(h.state_mut(), id).unwrap();
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(satin_of(&fx(&st.doc)), None, "Cancel leaves the document as it was");
    }
}
