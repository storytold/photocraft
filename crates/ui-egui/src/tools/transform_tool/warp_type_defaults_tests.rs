//! #12499: Warp on an unwarped type layer opens as Arc, Bend 0, H 0, V 0, with no mesh drawn over
//! the type, the Split buttons hidden and Grid greyed; the style list shows Custom greyed and no
//! Cylinder. Enter without a change adds no step; a stored Warp Text still opens as stored.

use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::super::commit::commit;
use super::super::options_bar::options_bar;
use super::*;

/// A 200 × 100 document with a type layer "Hello".
fn type_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Hello"})).unwrap();
    app
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn warp(app: &PhotocraftApp) -> Warp {
    app.ui.transform.as_ref().and_then(|t| t.warp.clone()).expect("Warp mode")
}

#[test]
fn unwarped_type_opens_as_arc_at_zero_and_enter_adds_no_step() {
    let mut app = type_app();
    let ctx = egui::Context::default();
    let before = past(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    let w = warp(&app);
    assert_eq!((w.style, w.bend, w.h_distort, w.v_distort, w.vertical), (WarpStyle::Arc, 0.0, 0.0, 0.0, false));
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "the box closed");
    assert_eq!(past(&app), before, "no history step");
    let st = app.session.active().unwrap();
    assert!(
        matches!(&st.doc.layer(st.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Text(t) if t.warp.is_none()),
        "no Warp Text stored"
    );
}

#[test]
fn a_stored_warp_text_still_opens_as_stored() {
    let mut app = type_app();
    let ctx = egui::Context::default();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("type.warpText", json!({"layer": id, "style": "flag", "bend": 30, "horizontalDistortion": 10, "verticalDistortion": -5})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    let w = warp(&app);
    assert_eq!((w.style, w.bend, w.h_distort, w.v_distort), (WarpStyle::Flag, 30.0, 10.0, -5.0));
}

#[test]
fn the_bar_hides_split_greys_grid_and_lists_custom_greyed() {
    let mut app = type_app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal_centered(|ui| options_bar(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    assert!(h.query_all_by_label("Split:").next().is_none(), "no Split label");
    assert!(h.query_all_by_label("Split Warp Crosswise").next().is_none(), "no Split buttons");
    let grid = h.query_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Grid")).expect("the Grid dropdown");
    assert!(grid.accesskit_node().is_disabled(), "Grid greyed");
    // The style list: Custom greyed, no None, no Cylinder.
    let at = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Warp"))
        .expect("the Warp dropdown")
        .rect()
        .center();
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
    let row = |label: &str| {
        h.query_all_by_role(egui::accesskit::Role::Button)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim() == label))
            .map(|n| n.accesskit_node().is_disabled())
    };
    assert_eq!(row("Custom"), Some(true), "Custom listed, greyed");
    assert_eq!(row("Arc"), Some(false));
    assert_eq!(row("None"), None);
    assert_eq!(row("Cylinder"), None);
}

#[test]
fn no_mesh_is_drawn_over_type() {
    let mut app = type_app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    let w = warp(&app);
    let pv = app.transform_preview.as_ref().expect("the preview");
    let xf = ViewXform::active(&app).unwrap();
    let count = |grid: bool| {
        let mut o = ctx.run_ui(Default::default(), |ui| draw_warp_overlay(ui.painter(), &xf, &w, pv, None, grid));
        o.textures_delta.clear();
        o.shapes.len()
    };
    assert!(count(true) > 0, "a pixel layer's mesh draws lines");
    assert_eq!(count(false), 0, "type draws none");
}
