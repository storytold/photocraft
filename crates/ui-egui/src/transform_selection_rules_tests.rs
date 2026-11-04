//! Free Transform's rules for the Layers panel's selection (#11101): the Background among several
//! selected layers refuses with the locked-layer alert, and any change to the selected layers
//! while the box is open applies the box first, as in Photoshop.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::transform_tool::LOCKED;

/// A 200 × 200 document on a white Background: "A" with a red block at x 10..50, y 10..30, and
/// "B" with a blue one at x 100..140, y 100..120. Only B is selected.
fn app() -> (PhotocraftApp, LayerId, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    let mut block = |name: &str, rect: [i32; 4], color: &str| {
        let id = LayerId(app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        id
    };
    let a = block("A", [10, 10, 40, 20], "#ff0000");
    let b = block("B", [100, 100, 40, 20], "#0000ff");
    (app, bg, a, b)
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let st = app.session.active().unwrap();
    photocraft_engine::transform_cmds::transform_bounds(&st.doc, st.doc.layer(id).unwrap())
}

fn select(app: &mut PhotocraftApp, id: LayerId, mode: &str) {
    app.run("layer.select", json!({"layer": id.0, "mode": mode})).unwrap();
}

fn invoke(app: &mut PhotocraftApp, id: &str) -> Result<(), String> {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).map(|_| ())
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

/// Drag the open box `dx` px right, with nearest neighbour so the blocks keep their edges.
fn nudge(app: &mut PhotocraftApp, dx: f64) {
    let t = app.ui.transform.as_mut().expect("a box is open");
    t.interpolation = "nearest".into();
    t.quad = t.quad.map(|[x, y]| [x + dx, y]);
}

#[test]
fn the_background_among_several_layers_shows_the_locked_alert_and_opens_no_box() {
    for id in ["edit.freeTransform", "edit.transform.scale", "edit.transform.rotate"] {
        let (mut app, bg, a, _) = app();
        select(&mut app, a, "replace");
        select(&mut app, bg, "add");
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(invoke(&mut app, id), Err(LOCKED.into()), "{id}");
        assert!(app.ui.transform.is_none(), "{id}: no box opens");
        assert_eq!(alerts(&app), [LOCKED], "{id}: the same alert as a position-locked layer");
        assert!(app.session.active().unwrap().doc == before, "{id}: nothing moved");
    }
}

#[test]
fn the_background_is_refused_whichever_of_them_is_active() {
    let (mut app, bg, a, _) = app();
    select(&mut app, a, "replace");
    select(&mut app, bg, "add");
    select(&mut app, a, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(a));
    assert_eq!(invoke(&mut app, "edit.freeTransform"), Err(LOCKED.into()));
    assert!(app.ui.transform.is_none());
}

#[test]
fn the_engines_default_targets_refuse_the_background_but_an_explicit_list_is_kept() {
    let (mut app, bg, a, b) = app();
    select(&mut app, a, "replace");
    select(&mut app, bg, "add");
    let err = app.run("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap_err();
    assert!(err.contains("locked"), "{err}");
    // A recorded or scripted step naming its layers moves those, as before.
    app.run("edit.transform", json!({"layers": [a.0, b.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(bounds(&app, a), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&app, b), Rect::new(105, 100, 145, 120));
}

#[test]
fn several_layers_without_the_background_still_open_one_box() {
    let (mut app, _, a, b) = app();
    select(&mut app, a, "add");
    invoke(&mut app, "edit.freeTransform").unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.layers, [a.0, b.0]);
    assert!(alerts(&app).is_empty());
}

#[test]
fn taking_a_layer_out_of_the_selection_applies_the_box_to_both_first() {
    // The reporter's case: the box over A and B (B active), then ⌘-click A in the Layers panel.
    let (mut app, _, a, b) = app();
    select(&mut app, a, "replace");
    select(&mut app, b, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b));
    invoke(&mut app, "edit.freeTransform").unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    nudge(&mut app, 10.0);
    select(&mut app, a, "toggle");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b), "the active layer didn't change");
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none(), "the box is applied and not opened again");
    assert_eq!(bounds(&app, a), Rect::new(20, 10, 60, 30), "A moved with the box");
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120));
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one Free Transform step");
    assert_eq!(st.selected_layers(), [b], "then A leaves the selection");
    // Nudging B now moves B alone: the old box is gone.
    invoke(&mut app, "edit.freeTransform").unwrap();
    assert!(app.ui.transform.as_ref().unwrap().layers.is_empty(), "a box over B alone");
}

#[test]
fn adding_a_layer_to_the_selection_applies_the_box_first() {
    let (mut app, _, a, b) = app();
    invoke(&mut app, "edit.freeTransform").unwrap();
    nudge(&mut app, 10.0);
    // ⌘-click A, then make B active again: the set changed though the active layer is B.
    select(&mut app, a, "add");
    select(&mut app, b, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b));
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120), "B moved");
    assert_eq!(bounds(&app, a), Rect::new(10, 10, 50, 30), "A wasn't in the box");
    assert_eq!(app.session.active().unwrap().selected_layers(), [a, b]);
}

#[test]
fn an_unchanged_selection_keeps_the_box_open() {
    let (mut app, _, a, b) = app();
    select(&mut app, a, "add");
    invoke(&mut app, "edit.freeTransform").unwrap();
    nudge(&mut app, 10.0);
    // Selecting what is selected already, or editing a moving layer, isn't a change.
    select(&mut app, a, "add");
    app.run("layer.setProps", json!({"layer": b.0, "opacity": 0.5})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_some(), "the box stays open");
}

#[test]
fn the_background_alone_keeps_todays_box() {
    let (mut app, bg, _, _) = app();
    select(&mut app, bg, "replace");
    let r = invoke(&mut app, "edit.freeTransform");
    assert_ne!(r, Err(LOCKED.into()), "not the several-layers alert");
    assert!(alerts(&app).is_empty());
}
