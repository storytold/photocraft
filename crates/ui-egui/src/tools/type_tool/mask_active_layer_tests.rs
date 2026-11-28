//! #12078: while a Horizontal or Vertical Type Mask is edited, the layer active before the click
//! stays active (the tab title and Properties follow it) and the Layers panel is disabled; commit
//! and cancel leave that layer active, and the edit does not end just because its hidden working
//! layer is not the active one. The Type tool's new text layer still becomes active.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::json;

use super::mask_replace_selection_tests::{bounds, click, new_app, selection};
use crate::PhotocraftApp;
use crate::state::Tool;

const MASKS: [Tool; 2] = [Tool::HorizontalTypeMask, Tool::VerticalTypeMask];

/// [`new_app`] with a pixel layer "Pix" active above the Background and `tool` in hand.
fn pix(tool: Tool) -> (Box<PhotocraftApp>, LayerId) {
    let mut app = new_app();
    let id = LayerId(app.run("layer.new.layer", json!({"name": "Pix"})).unwrap()["layer"].as_u64().unwrap());
    app.ui.tool = tool;
    (app, id)
}

/// The active layer, the selected layers and the layer count.
fn layers(app: &PhotocraftApp) -> (Option<LayerId>, Vec<LayerId>, usize) {
    let st = app.session.active().unwrap();
    (st.active_layer, st.selected_layers().to_vec(), st.doc.layers.len())
}

/// A mask started on the canvas with "MASK" typed, as the frame then checks it.
fn masking(tool: Tool) -> (Box<PhotocraftApp>, LayerId) {
    let (mut app, id) = pix(tool);
    click(&mut app, 100.0, 200.0, Modifiers::NONE);
    super::host::type_text(&mut app, "MASK");
    super::end_if_left(&mut app);
    (app, id)
}

#[test]
fn a_mask_edit_keeps_the_layer_active_before_the_click() {
    for tool in MASKS {
        let (app, id) = masking(tool);
        let ed = app.ui.text_edit.as_ref().unwrap_or_else(|| panic!("{tool:?}: the edit outlives the frame"));
        assert!(ed.mask && ed.layer != id.0, "{tool:?}");
        let (active, selected, _) = layers(&app);
        assert_eq!((active, selected), (Some(id), vec![id]), "{tool:?}: Pix stays active");
        let title = crate::canvas::tabs::tab_title_at(&app, 0, 100.0);
        assert!(title.contains("(Pix, ") && !title.contains("MASK"), "{tool:?}: {title}");
    }
}

#[test]
fn commit_and_cancel_leave_the_same_layer_active() {
    for tool in MASKS {
        for commit in [true, false] {
            let (mut app, id) = masking(tool);
            if commit {
                super::commit(&mut app)
            } else {
                super::cancel(&mut app)
            }
            assert!(app.ui.text_edit.is_none(), "{tool:?}, commit={commit}");
            assert_eq!(layers(&app), (Some(id), vec![id], 2), "{tool:?}, commit={commit}: Pix active, no layer left behind");
            let text = bounds(&selection(&app)).is_some_and(|r| r.x1 < 600);
            assert_eq!(text, commit, "{tool:?}, commit={commit}: the glyphs are selected only on commit");
        }
    }
}

#[test]
fn the_layers_panel_is_disabled_while_a_mask_is_edited() {
    for tool in MASKS {
        let (app, id) = masking(tool);
        let ctx = app.ctx().clone();
        let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).with_context(ctx).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app
        });
        h.run_steps(8);
        let row = |h: &Harness<'static, Box<PhotocraftApp>>, name: &str| {
            h.query_all_by_label(name).find(|n| n.accesskit_node().role() == egui::accesskit::Role::TreeItem).map(|n| n.accesskit_node().is_disabled())
        };
        assert_eq!(row(&h, "Pix"), Some(true), "{tool:?}: the rows are greyed");
        let at = crate::layers_panel_nodes::row(&h, "Background").center();
        crate::layers_panel_nodes::click(&mut h, at, Modifiers::NONE);
        let app = h.state();
        assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "{tool:?}: a row click does not end the mask");
        assert_eq!(layers(app).0, Some(id), "{tool:?}: nor selects the row");
    }
}

#[test]
fn the_type_tool_still_makes_its_new_layer_active() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let (mut app, id) = pix(tool);
        click(&mut app, 100.0, 200.0, Modifiers::NONE);
        let ed = app.ui.text_edit.as_ref().unwrap_or_else(|| panic!("{tool:?}: new text"));
        assert!(!ed.mask && ed.layer != id.0, "{tool:?}");
        assert_eq!(layers(&app).0, Some(LayerId(ed.layer)), "{tool:?}: the new text layer is active");
    }
}

/// The bottom layer, below Pix.
fn background(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().doc.layers[0].id
}

/// Whether the selection is the typed glyphs rather than [`new_app`]'s rectangle at x 600.
fn glyphs_selected(app: &PhotocraftApp) -> bool {
    bounds(&selection(app)).is_some_and(|r| r.x1 < 600)
}

#[test]
fn choosing_another_layer_still_ends_the_mask() {
    // The vertical mask starts empty: ending it undoes its session, and Background still stands.
    for (tool, text) in [(Tool::HorizontalTypeMask, "MASK"), (Tool::VerticalTypeMask, "MASK"), (Tool::VerticalTypeMask, "")] {
        let (mut app, _) = pix(tool);
        click(&mut app, 100.0, 200.0, Modifiers::NONE);
        if !text.is_empty() {
            super::host::type_text(&mut app, text);
        }
        let bg = background(&app);
        app.run("layer.select", json!({"layer": bg.0})).unwrap();
        super::end_if_left(&mut app);
        assert!(app.ui.text_edit.is_none(), "{tool:?} {text:?}: the mask ends");
        assert_eq!(layers(&app), (Some(bg), vec![bg], 2), "{tool:?} {text:?}: Background stands, no working layer left");
        assert_eq!(glyphs_selected(&app), !text.is_empty(), "{tool:?} {text:?}");
        assert!(app.ui.type_mask_layer_changes.is_none(), "{tool:?} {text:?}");
    }
}

#[test]
fn undo_and_redo_of_a_committed_mask_keep_the_layer_active() {
    for tool in MASKS {
        let (mut app, id) = masking(tool);
        super::commit(&mut app);
        assert!(app.ui.type_mask_layer_changes.is_none(), "{tool:?}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(layers(&app), (Some(id), vec![id], 2), "{tool:?}: undo");
        assert!(bounds(&selection(&app)).is_some_and(|r| r.x0 >= 600), "{tool:?}: undo brings back the rectangle");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(layers(&app), (Some(id), vec![id], 2), "{tool:?}: redo");
        assert!(glyphs_selected(&app), "{tool:?}: redo selects the glyphs again");
    }
}

#[test]
fn warp_and_the_type_menu_work_on_the_mask_over_a_pixel_layer() {
    for tool in MASKS {
        let (mut app, _) = masking(tool);
        let working = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        for id in ["type.warpText", "type.antiAlias.sharp", "type.orientation.vertical"] {
            assert!(crate::menus::is_enabled(&app, id), "{tool:?}: {id}");
        }
        let dialog = crate::filter_dialog::open(&mut app, "type.warpText").expect("Warp Text opens");
        let ctx = app.ctx().clone();
        app.framework.dialogs.set_json_field(&ctx, dialog, "style", json!("arc")).unwrap();
        PhotocraftApp::setup_context(&ctx, Default::default());
        crate::dialogs::testing::confirm(&mut app, dialog).expect("OK is accepted");
        for _ in 0..3 {
            crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(&mut app, ui));
        }
        let st = app.session.active().unwrap();
        assert!(super::text_layer(&st.doc, working).is_some_and(|t| t.warp.is_some()), "{tool:?}: the working layer is warped");
    }
}

#[test]
fn cancel_after_editing_the_kept_layer_removes_the_working_layer_and_keeps_the_edit() {
    for tool in MASKS {
        let (mut app, id) = masking(tool);
        // Properties edits the layer that stayed active, which ends the mask's history run.
        app.run("layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
        super::cancel(&mut app);
        assert_eq!(layers(&app), (Some(id), vec![id], 2), "{tool:?}: Pix active, no working layer left behind");
        let opacity = app.session.active().unwrap().doc.layer(id).unwrap().opacity;
        assert!((opacity - 0.5).abs() < 1e-6, "{tool:?}: the Properties edit stays: {opacity}");
    }
}

#[test]
fn cancel_in_another_document_leaves_its_layer_with_the_same_id() {
    for tool in MASKS {
        let (mut app, id) = masking(tool);
        app.run("layer.setProps", json!({"layer": id.0, "opacity": 0.5})).unwrap();
        let working = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        // A file opened with persisted ids, made active before the edit is checked again.
        let copy = (*app.session.active().unwrap().doc).clone();
        app.session.add_document(copy, None);
        super::cancel(&mut app);
        assert!(app.session.active().unwrap().doc.layer(working).is_some(), "{tool:?}: the other document keeps its layer");
    }
}
