//! The Curves graph's keys (#14438), in the Curves dialog and in Properties: Ctrl+Tab and
//! Ctrl+Shift+Tab step through the points, wrapping around, without switching documents; Delete
//! on an end point deselects it and leaves the curve alone, on an inner point removes it; the
//! arrows keep their steps (1, Shift 10).

use egui::{Key, Modifiers, Pos2, Rect, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::CurvesEditorState;

/// The whole app with two documents, the second one active; `curves_layer` gives it a Curves
/// adjustment layer, shown in Properties.
fn app(curves_layer: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(0.25).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        for _ in 0..2 {
            s.execute("file.new", json!({"width": 96, "height": 64})).unwrap();
        }
        if curves_layer {
            s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
        }
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        // Room for the whole Curves editor in Properties without scrolling.
        app.ui.dock.heights.insert(crate::dock::Group::Properties, 560.0);
        app
    });
    h.run_steps(8);
    h
}

/// What a Ctrl press reports: on Windows and Linux Ctrl is also the command key, on macOS it isn't.
fn ctrl() -> Modifiers {
    Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), ..Modifiers::NONE }
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

/// Where curve value `v` (levels) sits on screen in `graph`.
fn at(graph: Rect, v: [f32; 2]) -> Pos2 {
    Pos2::new(graph.left() + v[0] / 255.0 * graph.width(), graph.bottom() - v[1] / 255.0 * graph.height())
}

fn active(h: &Harness<'_, PhotocraftApp>) -> Option<usize> {
    h.state().session.active_index()
}

fn dialog_selected(h: &Harness<'_, PhotocraftApp>) -> Option<usize> {
    let st: CurvesEditorState = h.ctx.data(|d| d.get_temp(crate::adjust_dialog::dialog_mem("curves").with("curves"))).unwrap_or_default();
    st.gesture.selected
}

fn dialog_points(h: &Harness<'_, PhotocraftApp>) -> serde_json::Value {
    h.state().ui.dialogs.last().and_then(|d| d.fields.get("points")).cloned().unwrap_or_default()
}

/// Image › Adjustments › Curves, as measured in the reference application.
#[test]
fn ctrl_tab_steps_through_the_points_and_delete_deselects_an_end_point_in_the_dialog() {
    let mut h = app(false);
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.curves").unwrap();
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("points".into(), json!([[0, 0], [64, 64], [192, 192], [255, 255]]));
    h.run_steps(4);
    let graph = h.ctx.data(|d| d.get_temp::<Rect>(crate::adjust_dialog::dialog_mem("curves").with("curves-graph"))).unwrap();
    click(&mut h, at(graph, [64.0, 64.0]));
    assert_eq!(dialog_selected(&h), Some(1), "the click selects 64");
    let points = dialog_points(&h);
    for (shift, expected) in [(false, 2), (false, 3), (false, 0), (true, 3), (true, 2), (true, 1)] {
        key(&mut h, Key::Tab, if shift { ctrl() | Modifiers::SHIFT } else { ctrl() });
        assert_eq!(dialog_selected(&h), Some(expected), "shift: {shift}");
        assert_eq!(active(&h), Some(1), "Ctrl+Tab in the graph doesn't switch documents");
    }
    assert_eq!(dialog_points(&h), points, "stepping moves no point");
    // The graph still has the keyboard: the arrows nudge the selected point by 1, Shift by 10.
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    key(&mut h, Key::ArrowRight, Modifiers::SHIFT);
    assert_eq!(dialog_points(&h), json!([[0.0, 0.0], [74.0, 65.0], [192.0, 192.0], [255.0, 255.0]]));
    // Delete on an end point: the curve stays, the point is no longer selected.
    key(&mut h, Key::Tab, ctrl() | Modifiers::SHIFT);
    assert_eq!(dialog_selected(&h), Some(0));
    let points = dialog_points(&h);
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(dialog_points(&h), points, "an end point stays");
    assert_eq!(dialog_selected(&h), None, "and is deselected");
    // Delete on an inner point still removes it.
    click(&mut h, at(graph, [192.0, 192.0]));
    assert_eq!(dialog_selected(&h), Some(2));
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(dialog_points(&h), json!([[0.0, 0.0], [74.0, 65.0], [255.0, 255.0]]));
    assert_eq!(dialog_selected(&h), None);
}

/// The same graph in Properties, where no dialog holds back the shortcuts: with a point
/// selected, Ctrl+Tab is the graph's; with none (not measured) it switches documents as before.
#[test]
fn ctrl_tab_in_the_properties_graph_steps_points_only_while_one_is_selected() {
    let mut h = app(true);
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let mem = egui::Id::unique(("adjust-layer", layer.0));
    let graph = h.ctx.data(|d| d.get_temp::<Rect>(mem.with("curves-graph"))).expect("curves graph drawn");
    let selected = |h: &Harness<'_, PhotocraftApp>| h.ctx.data(|d| d.get_temp::<CurvesEditorState>(mem.with("curves"))).unwrap_or_default().gesture.selected;
    for v in [[64.0, 64.0], [192.0, 192.0], [64.0, 64.0]] {
        click(&mut h, at(graph, v));
    }
    assert_eq!(selected(&h), Some(1), "64 is selected");
    key(&mut h, Key::Tab, ctrl());
    assert_eq!((selected(&h), active(&h)), (Some(2), Some(1)), "Ctrl+Tab selects 192 and keeps the document");
    key(&mut h, Key::Tab, ctrl() | Modifiers::SHIFT);
    key(&mut h, Key::Tab, ctrl() | Modifiers::SHIFT);
    assert_eq!((selected(&h), active(&h)), (Some(0), Some(1)), "Ctrl+Shift+Tab goes back to the black point");
    key(&mut h, Key::Delete, Modifiers::NONE);
    assert_eq!(selected(&h), None, "Delete deselects the end point");
    let doc = h.state().session.active().unwrap();
    assert!(doc.doc.layer(layer).is_some(), "Delete in the focused graph doesn't delete the layer");
    key(&mut h, Key::Tab, ctrl());
    assert_eq!(active(&h), Some(0), "with no point selected Ctrl+Tab switches documents");
}
