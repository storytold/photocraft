//! #16785: Ctrl+click on a group's folder icon in the Layers panel loads no selection (a group has no pixels of its
//! own), with or without Shift and Alt; the other thumbnails, and a group's own mask, keep loading as before.

use craft::layers::ui::tree::LayerPart;
use egui::Modifiers;
use egui_kittest::Harness;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layers_panel_nodes::{click, layer_id, thumb};

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with `s` open and the Layers panel showing.
fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

/// 800 × 500: "Group" holds "Rect" (black at 300, 200, 200 × 80); "Top", active, above it. With `mask`, the group has
/// a layer mask revealing 100, 50, 60 × 40.
fn grouped(mask: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Rect"})).unwrap();
    s.execute("select.rect", json!({"x": 300, "y": 200, "width": 200, "height": 80})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": group.0, "name": "Group"})).unwrap();
    s.execute("layer.setExpanded", json!({"layer": group.0, "expanded": true})).unwrap();
    if mask {
        s.execute("select.rect", json!({"x": 100, "y": 50, "width": 60, "height": 40})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
        // Revealing the selection may already have used it up.
        let _ = s.execute("select.deselect", json!({}));
    }
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s
}

/// The selection's bounds as (x, y, width, height).
fn selection(h: &H) -> Option<(u32, u32, u32, u32)> {
    let b = h.state().session.active().unwrap().doc.selection.as_ref()?.content_bounds();
    Some((b.x0 as u32, b.y0 as u32, b.width(), b.height()))
}

/// The active layer's name.
fn active(h: &H) -> Option<String> {
    let st = h.state().session.active().unwrap();
    st.active_layer.map(|id| st.doc.layer(id).unwrap().name.clone())
}

/// The number of undoable steps.
fn steps(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn a_ctrl_click_on_a_groups_folder_icon_loads_no_selection() {
    let combos = [
        ("Ctrl", Modifiers::COMMAND),
        ("Ctrl+Shift", Modifiers::COMMAND | Modifiers::SHIFT),
        ("Ctrl+Alt", Modifiers::COMMAND | Modifiers::ALT),
        ("Ctrl+Shift+Alt", Modifiers::COMMAND | Modifiers::SHIFT | Modifiers::ALT),
    ];
    for (name, mods) in combos {
        let mut h = harness(grouped(false));
        let (before_active, before_steps) = (active(&h), steps(&h));
        let at = thumb(&h, "Group", LayerPart::Pixels).center();
        click(&mut h, at, mods);
        assert_eq!(selection(&h), None, "{name}: a group's folder icon loads no selection");
        assert_eq!(active(&h), before_active, "{name}: the active layer stays");
        assert_eq!(steps(&h), before_steps, "{name}: no history step");
    }
}

/// Neighbours: a pixel layer inside the group still loads its pixels, and the group's own layer mask still loads.
#[test]
fn a_ctrl_click_still_loads_a_layer_in_the_group_and_the_groups_mask() {
    let mut h = harness(grouped(false));
    let at = thumb(&h, "Rect", LayerPart::Pixels).center();
    click(&mut h, at, Modifiers::COMMAND);
    assert_eq!(selection(&h), Some((300, 200, 200, 80)), "the layer in the group loads its pixels: {}", h.state().ui.status);

    let mut h = harness(grouped(true));
    assert!(h.state().session.active().unwrap().doc.layer(layer_id(h.state(), "Group")).is_some_and(|g| g.mask.is_some()), "the group has a mask");
    let at = thumb(&h, "Group", LayerPart::LayerMask).center();
    click(&mut h, at, Modifiers::COMMAND);
    assert_eq!(selection(&h), Some((100, 50, 60, 40)), "the group's layer mask loads: {}", h.state().ui.status);
}
