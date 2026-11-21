//! Layers panel Smart Filters rows (#4271) and the Delete key with a filter row selected (#4751).
//! The click, drop and double-click rows are `panels/layers_events/tests.rs`'s.

use egui::Pos2;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// Smart objects "Smart A" (Gaussian Blur and Offset) and "Smart B" (no filters), and a pixel
/// layer "Pixels" on top: (harness, A, B, pixels).
fn panel() -> (egui_kittest::Harness<'static, Box<PhotocraftApp>>, u64, u64, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let smart = |s: &mut photocraft_engine::Session, name: &str| {
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
        id
    };
    let a = smart(&mut s, "Smart A");
    s.execute("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    s.execute("filter.other.offset", json!({"horizontal": 2})).unwrap();
    let b = smart(&mut s, "Smart B");
    let c = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": c, "name": "Pixels"})).unwrap();
    // 60 fps steps, so a double-click's two clicks fall inside egui's 0.3 s window.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(4);
    (h, a, b, c)
}

fn filters(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, id: u64) -> Vec<String> {
    match &h.state().session.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Smart(sm) => sm.smart_filters.iter().map(|f| f.command.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The centre of the Layers panel row named `label`, or of the footer button.
fn at(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, label: &str) -> Pos2 {
    if crate::layers_panel_nodes::row_drawn(h, label) { crate::layers_panel_nodes::row(h, label).center() } else { h.get_by_label(label).rect().center() }
}

fn selected(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, label: &str) -> bool {
    h.get_by_role_and_label(Role::TreeItem, label).accesskit_node().is_selected() == Some(true)
}

fn click_at(h: &mut egui_kittest::Harness<'static, Box<PhotocraftApp>>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(4);
}

/// With a filter's row selected, Delete or Backspace removes that filter, not the smart object;
/// on the Smart Filters row it clears them all (#4751). Where the key belongs to something else (a
/// rename field, Quick Mask) the filter stays; where the row is filtered out, which deselects it,
/// Delete deletes the active layer as usual.
#[test]
fn the_delete_key_and_a_selected_filter() {
    type Case = (&'static str, &'static str, fn(&mut egui_kittest::Harness<'static, Box<PhotocraftApp>>, u64), egui::Key, usize, bool);
    let none: fn(&mut egui_kittest::Harness<'static, Box<PhotocraftApp>>, u64) = |_, _| {};
    let cases: [Case; 5] = [
        ("Delete removes the selected filter", "Gaussian Blur", none, egui::Key::Delete, 1, false),
        ("Backspace on the Smart Filters row clears them", "Smart Filters", none, egui::Key::Backspace, 0, false),
        (
            "a rename field keeps the key",
            "Gaussian Blur",
            |h, a| {
                h.state_mut().layers_rename = Some(LayerId(a));
                h.run_steps(4);
                assert!(selected(h, "Gaussian Blur"), "the filter is still selected while renaming");
            },
            egui::Key::Delete,
            2,
            false,
        ),
        (
            "Quick Mask keeps the key for Edit › Clear on the mask",
            "Gaussian Blur",
            |h, _| {
                h.state_mut().run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
                assert!(h.state().session.active().unwrap().doc.selection.is_none());
                h.run_steps(4);
                assert!(selected(h, "Gaussian Blur"), "the filter is still selected in Quick Mask");
            },
            egui::Key::Delete,
            2,
            false,
        ),
        (
            "a filter row filtered out is deselected: Delete deletes the layer",
            "Gaussian Blur",
            |h, a| {
                let ctx = h.ctx.clone();
                for name in ["Pixels", ""] {
                    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"layerFilter": {"by": "name", "name": name}}));
                    crate::control::handle(h.state_mut(), &ctx, &r);
                    h.run_steps(4);
                }
                assert!(!selected(h, "Gaussian Blur"), "shown again, the row is no longer selected");
                h.state_mut().session.execute("layer.select", json!({"layer": a})).unwrap();
                h.run_steps(4);
                assert!(!selected(h, "Gaussian Blur"), "selecting the layer does not bring the filter row back");
            },
            egui::Key::Delete,
            0,
            true,
        ),
    ];
    for (case, pick, before, key, left, deleted) in cases {
        let (mut h, a, _, _) = panel();
        let p = at(&h, pick);
        click_at(&mut h, p);
        before(&mut h, a);
        h.key_press(key);
        h.run_steps(4);
        let gone = h.state().session.active().unwrap().doc.layer(LayerId(a)).is_none();
        assert_eq!(gone, deleted, "{case}: the smart object deleted");
        if !gone {
            assert_eq!(filters(&h, a).len(), left, "{case}: filters left");
        }
    }
}

/// With the Layers panel closed the selected filter row isn't on screen, so Delete deletes the
/// active layer as usual.
#[test]
fn with_the_layers_panel_closed_delete_leaves_the_filter() {
    let (mut h, a, _, _) = panel();
    let p = at(&h, "Gaussian Blur");
    click_at(&mut h, p);
    crate::panel_docking::show_panel(h.state_mut(), "layers", Some(false)).unwrap();
    h.run_steps(4);
    assert!(h.query_all_by_label("Gaussian Blur").next().is_none(), "the panel is closed");
    h.key_press(egui::Key::Delete);
    h.run_steps(4);
    assert!(h.state().session.active().unwrap().doc.layer(LayerId(a)).is_none(), "Delete reached Smart A itself");
}
