//! #589: layers dragged onto another document's tab, then dropped on its canvas, are copied there.

use egui::accesskit::Role;
use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// "source-doc" (40×30) with a red 10×10 square at (4, 6) on layer "paint", and "dest-doc"
/// (60×50). The source is active.
fn harness() -> (Harness<'static, PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "source-doc"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 50, "name": "dest-doc"})).unwrap();
    s.set_active(0);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    (h, paint)
}

/// A document tab, found by its accessible title ("name @ 100% (…)", see `pro_tabs`).
fn tab(h: &Harness<'_, PhotocraftApp>, name: &str) -> Pos2 {
    h.get_by_label_contains(&format!("{name} @")).rect().center()
}

/// Press at `path[0]`, move through the rest (a few frames at each) and release at the last
/// point with `release` held.
fn drag_along(h: &mut Harness<'_, PhotocraftApp>, path: &[Pos2], release: Modifiers) {
    drag_along_pressed(h, path, Modifiers::NONE, release);
}

/// [`drag_along`], with `press` held from the press on (⌥ for a Move-tool copy).
fn drag_along_pressed(h: &mut Harness<'_, PhotocraftApp>, path: &[Pos2], press: Modifiers, release: Modifiers) {
    let first = path[0];
    h.event(egui::Event::ModifiersChanged(press));
    h.event(egui::Event::PointerMoved(first));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: press });
    h.run_steps(1);
    for w in path.windows(2) {
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(w[0] + (w[1] - w[0]) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        h.run_steps(2);
    }
    let last = path[path.len() - 1];
    h.event(egui::Event::ModifiersChanged(release));
    h.event(egui::Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers: release });
    h.run_steps(3);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// The copy in "dest-doc" (its active layer) and its bounds.
fn copy(h: &Harness<'_, PhotocraftApp>) -> (String, Rect) {
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.name, "dest-doc", "the destination is shown");
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let b = photocraft_engine::layer_multi_cmds::layer_bounds(l).unwrap();
    (l.name.clone(), Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

fn source_square(h: &Harness<'_, PhotocraftApp>, paint: LayerId) -> photocraft_geom::Rect {
    let doc = &h.state().session.documents()[0].doc;
    photocraft_engine::layer_multi_cmds::layer_bounds(doc.layer(paint).unwrap()).unwrap()
}

#[test]
fn move_selection_across_tab_copies_only_selected_pixels_and_keeps_source_history() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.run_steps(2);
    let before = h.state().session.documents()[0].doc.clone();
    let history = h.state().session.documents()[0].history.entries().len();
    let grab = ViewXform::active(h.state()).unwrap().to_screen(8.0, 10.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);
    let (_, r) = copy(&h);
    assert_eq!(r.size(), vec2(3.0, 4.0));
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    assert!((r.min.x - (7.0 + at[0] - 8.0) as f32).abs() <= 1.0);
    assert_eq!(*h.state().session.documents()[0].doc, *before);
    assert_eq!(h.state().session.documents()[0].history.entries().len(), history);
    assert_eq!(source_square(&h, paint), photocraft_geom::Rect::new(4, 6, 14, 16));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1);
}

#[test]
fn a_selection_dropped_outside_documents_copies_nothing() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.run_steps(2);
    let before = h.state().session.documents()[0].doc.clone();
    let grab = ViewXform::active(h.state()).unwrap().to_screen(8.0, 10.0);
    let dest = tab(&h, "dest-doc");
    drag_along(&mut h, &[grab, dest, Pos2::new(-10.0, -10.0)], Modifiers::NONE);
    assert_eq!(*h.state().session.documents()[0].doc, *before);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1);
}

#[test]
fn a_selection_can_be_dragged_directly_between_tiled_canvases() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.run_steps(3);
    let before = h.state().session.documents()[0].doc.clone();
    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    let view = &h.state().ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(h.state(), cells[0].1.shrink(1.0)),
        zoom: view.zoom / h.state().ppp,
        center: view.center,
        flip: false,
        rotation: view.rotation,
        aspect: 1.0,
    };
    let grab = xf.to_screen(8.0, 10.0);
    let drop = cells[1].1.center();
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);
    assert_eq!(copy(&h).1.size(), vec2(3.0, 4.0));
    assert_eq!(*h.state().session.documents()[0].doc, *before);
}

#[test]
fn a_plain_layer_can_be_dragged_directly_between_tiled_canvases() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.run_steps(3);
    let before = source_square(&h, paint);
    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    let view = &h.state().ui.views[0];
    let source_xf = ViewXform {
        rect: crate::rulers::content_rect(h.state(), cells[0].1.shrink(1.0)),
        zoom: view.zoom / h.state().ppp,
        center: view.center,
        flip: false,
        rotation: view.rotation,
        aspect: 1.0,
    };
    let grab = source_xf.to_screen(9.0, 11.0);
    let drop = cells[1].1.center();
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);
    let (name, copied) = copy(&h);
    assert_eq!(name, "paint");
    assert_eq!(copied.size(), vec2(10.0, 10.0));
    assert_eq!(source_square(&h, paint), before, "the tiled transfer preserves the source layer");
}

#[test]
fn a_tiled_selection_drag_that_returns_to_its_source_still_moves_there() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.run_steps(3);
    let before = h.state().session.documents()[0].doc.clone();
    let history = h.state().session.documents()[0].history.past_len();
    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    let view = &h.state().ui.views[0];
    let ppp = h.state().canvas_ppp();
    let source_xf = ViewXform {
        rect: crate::rulers::content_rect(h.state(), cells[0].1.shrink(1.0)),
        zoom: view.zoom / ppp,
        center: view.center,
        flip: false,
        rotation: view.rotation,
        aspect: 1.0,
    }
    .pixel_aligned_for(h.state(), ppp);
    let grab = source_xf.to_screen(8.0, 10.0);
    let moved = source_xf.to_screen(13.0, 10.0);
    assert!(cells[0].1.contains(moved), "release {moved:?} is outside source cell {:?}", cells[0].1);
    drag_along(&mut h, &[grab, cells[1].1.center(), moved], Modifiers::NONE);
    let state = h.state();
    assert_eq!(*state.session.documents()[0].doc, *before, "a selection move remains floating until it is dropped");
    let floating = state.session.documents()[0].floating.as_ref().expect("returning to the source preserves and releases the original move drag");
    assert!(floating.moved);
    assert_ne!(floating.offset, (0, 0));
    assert_eq!(state.session.documents()[0].history.past_len(), history);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1);
}

#[test]
fn an_option_drag_of_a_moved_float_onto_a_tab_keeps_the_source_float() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.state_mut().run("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    h.run_steps(2);
    let source = &h.state().session.documents()[0];
    let (before, history) = (source.doc.clone(), source.history.past_len());
    assert!(photocraft_engine::float_cmds::floating(source).is_some_and(|f| f.offset == (5, 0)));
    let grab = ViewXform::active(h.state()).unwrap().to_screen(13.0, 10.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along_pressed(&mut h, &[grab, over_tab, drop], Modifiers::ALT, Modifiers::NONE);
    assert_eq!(copy(&h).1.size(), vec2(3.0, 4.0));
    let source = &h.state().session.documents()[0];
    assert_eq!(*source.doc, *before);
    assert_eq!(source.history.past_len(), history);
    // The rollback of the ⌥-drag's drop-and-copy raised the revision: the restored float must
    // still count as floating over it, or the earlier move is lost.
    let float = photocraft_engine::float_cmds::floating(source).expect("the moved float survives the transfer");
    assert_eq!(float.offset, (5, 0));
}

/// A moved float in "source-doc", the Move tool, and an ⌥-press on it, moved a little so the
/// drag has dropped the float and floats its copy. Returns the tab of "dest-doc".
fn option_drag_of_a_moved_float(h: &mut Harness<'_, PhotocraftApp>) -> Pos2 {
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.state_mut().run("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    h.run_steps(2);
    let grab = ViewXform::active(h.state()).unwrap().to_screen(13.0, 10.0);
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.event(egui::Event::PointerMoved(grab));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: grab, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(grab + vec2(0.0, 10.0 * k as f32)));
        h.run_steps(1);
    }
    tab(h, "dest-doc")
}

fn release_at(h: &mut Harness<'_, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn an_edit_during_an_option_drag_of_a_float_is_not_taken_back_by_a_tab_hover() {
    let (mut h, paint) = harness();
    let over_tab = option_drag_of_a_moved_float(&mut h);
    // An edit lands while the button is still held (as an opacity key would).
    h.state_mut().run("layer.setProps", json!({"layer": paint.0, "opacity": 0.3})).unwrap();
    h.run_steps(1);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(over_tab + vec2(0.0, k as f32)));
        h.run_steps(1);
    }
    let opacity = |h: &Harness<'_, PhotocraftApp>| h.state().session.documents()[0].doc.layer(paint).unwrap().opacity;
    assert!((opacity(&h) - 0.3).abs() < 1e-6, "the hover took the opacity edit back");
    release_at(&mut h, over_tab);
    assert!((opacity(&h) - 0.3).abs() < 1e-6, "the release took the opacity edit back");
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1, "nothing is copied over a later edit");
}

#[test]
fn a_tab_hover_that_cannot_take_the_option_drag_back_keeps_the_move_going() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut h, _) = harness();
    let over_tab = option_drag_of_a_moved_float(&mut h);
    // A background job locks the source: the rollback can't run.
    let gate = Arc::new(AtomicBool::new(false));
    let open = gate.clone();
    h.state_mut()
        .session
        .start_job(
            "test.slow",
            json!({}),
            "Slow",
            true,
            move |ctx| {
                while !open.load(Ordering::Relaxed) {
                    ctx.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            |_, ()| Ok(json!({})),
        )
        .unwrap();
    assert!(h.state().session.active_job().is_some());
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(over_tab + vec2(0.0, k as f32)));
        h.run_steps(1);
    }
    // Still the same ⌥-drag of the copy, its modifier state intact.
    assert_eq!(h.state().session.active().unwrap().doc.name, "source-doc");
    assert!(h.state().drag.is_some());
    assert!(crate::move_mods::copying(h.state(), false), "the drag's move state was cleared");
    gate.store(true, Ordering::Relaxed);
    release_at(&mut h, over_tab);
}

#[test]
fn a_layers_panel_drag_onto_a_tab_drops_the_layer_where_it_is_released() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::NONE);
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    assert!((r.center().x - at[0] as f32).abs() <= 1.0 && (r.center().y - at[1] as f32).abs() <= 1.0, "centred on the drop point {at:?}: {r:?}");
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 2, "the source keeps its layers");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn shift_centres_the_copy_when_the_documents_differ_in_size() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center() + vec2(80.0, 60.0);
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::SHIFT);
    assert_eq!(copy(&h).1, Rect::from_min_max(egui::pos2(25.0, 20.0), egui::pos2(35.0, 30.0)));
}

#[test]
fn a_move_tool_drag_onto_a_tab_copies_the_layer_under_the_pointer_and_leaves_the_source() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);
    let before = source_square(&h, paint);
    let steps = h.state().session.documents()[0].history.entries().len();
    let grab = ViewXform::active(h.state()).unwrap().to_screen(9.0, 11.0);
    let grab_doc = ViewXform::active(h.state()).unwrap().to_doc(grab);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    // The grabbed pixel lands under the pointer.
    let want = egui::pos2((9.0 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
    assert_eq!(source_square(&h, paint), before, "the source layer did not move");
    assert_eq!(h.state().session.documents()[0].history.entries().len(), steps, "no Move step in the source");
}

#[test]
fn dropping_back_on_the_source_copies_nothing() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let (dest_tab, source_tab) = (tab(&h, "dest-doc"), tab(&h, "source-doc"));
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, dest_tab, source_tab, drop], Modifiers::NONE);
    let docs = h.state().session.documents();
    assert_eq!((docs[0].doc.layers.len(), docs[1].doc.layers.len()), (2, 1));
    assert_eq!(h.state().session.active_index(), Some(0));
}

#[test]
/// Dragging a selection onto another document's tab copies the selected pixels and leaves the source intact.
fn a_move_tool_selection_drag_onto_a_tab_copies_selected_pixels_to_destination() {
    let (mut h, paint) = harness();
    // Create a selection inside the red square
    h.state_mut().session.execute("select.rect", json!({"x": 4, "y": 6, "width": 5, "height": 10})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);

    let grab = ViewXform::active(h.state()).unwrap().to_screen(6.0, 11.0);
    let grab_doc = ViewXform::active(h.state()).unwrap().to_doc(grab);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);

    // Destination now has the copied 5x10 selection layer placed at the drop point
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (_, r) = copy(&h);
    assert_eq!(r.size(), vec2(5.0, 10.0), "only the 5x10 selected region is copied");
    let want = egui::pos2((6.5 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2);
    // Source layer keeps its pixels intact (not cut)
    let src_bounds = source_square(&h, paint);
    assert_eq!(src_bounds, photocraft_geom::Rect::new(4, 6, 14, 16), "source remains intact");
}

#[test]
/// Dragging a mask selection onto a tab copies the mask gray and leaves the layer's pixels.
fn a_move_tool_mask_selection_drag_onto_a_tab_copies_the_mask_and_leaves_layer_pixels() {
    let (mut h, paint) = harness();
    h.state_mut().session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    h.state_mut().session.execute("select.rect", json!({"x": 4, "y": 6, "width": 5, "height": 10})).unwrap();
    h.state_mut().session.execute("edit.fill", json!({"color": "#808080", "target": "mask"})).unwrap();
    h.state_mut().ui.mask_target = true;
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);

    // Snapshot the source mask and selection: arming the transfer and copying the selected
    // pixels must leave both unchanged.
    let src_doc = h.state().session.active().unwrap().doc.id;
    let src_layer = h.state().session.active().unwrap().active_layer;
    let mask_sample = |h: &Harness<'static, PhotocraftApp>| {
        let st = h.state().session.documents().iter().find(|s| s.doc.id == src_doc).unwrap();
        let layer = src_layer.and_then(|id| st.doc.layer(id)).unwrap();
        let mut px = [0.0f32; 4];
        layer.surface().unwrap().read_pixel(6, 8, &mut px);
        px
    };
    let selection_present =
        |h: &Harness<'static, PhotocraftApp>| h.state().session.documents().iter().find(|s| s.doc.id == src_doc).is_some_and(|s| s.doc.selection.is_some());
    let before_mask = mask_sample(&h);
    let before_selection = selection_present(&h);

    let grab = ViewXform::active(h.state()).unwrap().to_screen(6.0, 11.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);

    let (_, r) = copy(&h);
    assert_eq!(r.size(), vec2(5.0, 10.0), "only the 5x10 selected mask region is copied");
    let st = h.state().session.active().unwrap();
    let dst_layer = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let mut sample = [0.0f32; 4];
    dst_layer.surface().unwrap().read_pixel(r.min.x as i32 + 1, r.min.y as i32 + 1, &mut sample);
    assert!((sample[0] - sample[1]).abs() < 0.02 && (sample[0] - 128.0 / 255.0).abs() < 0.02, "copied mask gray, not red layer pixels: {sample:?}");
    assert_eq!(source_square(&h, paint), photocraft_geom::Rect::new(4, 6, 14, 16), "source layer pixels remain intact");
    assert_eq!(mask_sample(&h), before_mask, "arming and copying leave the source mask unchanged");
    assert_eq!(selection_present(&h), before_selection, "arming and copying leave the source selection unchanged");
}

#[test]
/// In Arrange views, dragging a selection into the neighbouring cell transfers it there directly.
fn a_move_tool_selection_drag_in_arranged_view_transfers_directly_to_neighbour_cell() {
    let (mut h, paint) = harness();
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.run_steps(4);

    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    assert_eq!(cells.len(), 2);
    let (src_cell, dest_cell) = (cells[0].1, cells[1].1);

    // Create selection on source
    h.state_mut().session.execute("select.rect", json!({"x": 4, "y": 6, "width": 5, "height": 10})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);

    let view_src = h.state().ui.views[0].clone();
    let xf_src =
        crate::canvas::ViewXform { rect: src_cell, zoom: view_src.zoom / h.state().ppp, center: view_src.center, flip: false, rotation: 0.0, aspect: 1.0 };
    let grab = xf_src.to_screen(6.0, 11.0);
    let grab_doc = xf_src.to_doc(grab);
    let view_dest = h.state().ui.views[1].clone();
    let xf_dest = crate::canvas::ViewXform {
        rect: crate::rulers::content_rect(h.state(), dest_cell),
        zoom: view_dest.zoom / h.state().canvas_ppp(),
        center: view_dest.center,
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let drop = dest_cell.center();
    let at = xf_dest.to_doc(drop);

    // Drag directly from src canvas to dest canvas (no tab hover!)
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);

    // Destination now has the transferred 5x10 selection layer at the drop point!
    let (_, r) = copy(&h);
    assert_eq!(r.size(), vec2(5.0, 10.0), "only the 5x10 selected region is copied");
    let want = egui::pos2((6.5 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
    assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2);
    // Source layer keeps its pixels intact (not cut)
    let src_bounds = source_square(&h, paint);
    assert_eq!(src_bounds, photocraft_geom::Rect::new(4, 6, 14, 16), "source remains intact");
}

#[test]
/// In Arrange views, dragging the Background into the neighbouring cell transfers it there directly.
fn a_move_tool_background_layer_drag_in_arranged_view_transfers_directly_to_neighbour_cell() {
    let (mut h, paint) = harness();
    h.state_mut().session.execute("layer.delete", json!({"layer": paint.0})).unwrap();
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 1, "only Background remains in source");

    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = true;
    h.run_steps(4);

    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    assert_eq!(cells.len(), 2);
    let (src_cell, dest_cell) = (cells[0].1, cells[1].1);

    let view_src = h.state().ui.views[0].clone();
    let xf_src =
        crate::canvas::ViewXform { rect: src_cell, zoom: view_src.zoom / h.state().ppp, center: view_src.center, flip: false, rotation: 0.0, aspect: 1.0 };
    let grab = xf_src.to_screen(15.0, 15.0);
    let drop = dest_cell.center();

    // Drag Background directly from src canvas to dest canvas
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);

    // Destination now has the transferred Background layer copied as an ordinary layer!
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2);
    let (name, _) = copy(&h);
    assert_eq!(name, "Layer 1");
    // Source still has its Background intact
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 1);
}

#[test]
/// A blocked Move drag inside one document shows the lock message even with two documents open.
fn same_document_blocked_move_with_multiple_documents_shows_lock_message() {
    let (mut h, paint) = harness();
    h.state_mut().session.execute("layer.delete", json!({"layer": paint.0})).unwrap();
    assert_eq!(h.state().session.documents().len(), 2, "two documents open");
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;

    use crate::canvas::{ToolEvent, tool_event};
    // Click on Background: no message
    tool_event(h.state_mut(), ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(h.state_mut(), ToolEvent::Up { x: 20.0, y: 20.0 }, Modifiers::NONE);
    let error_dialogs = |app: &crate::PhotocraftApp| -> Vec<String> {
        app.ui
            .dialogs
            .iter()
            .filter(|d| d.kind == crate::state::DialogKind::Error)
            .filter_map(|d| d.fields.get("message")?.as_str().map(String::from))
            .collect()
    };
    assert!(error_dialogs(h.state()).is_empty() && !h.state().move_blocked);

    // Drag and return to press point: shows lock prompt promptly on Move
    tool_event(h.state_mut(), ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(h.state_mut(), ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    // Even if pointer returns to start before Up, the prompt is shown
    tool_event(h.state_mut(), ToolEvent::Move { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(h.state_mut(), ToolEvent::Up { x: 20.0, y: 20.0 }, Modifiers::NONE);
    assert_eq!(error_dialogs(h.state()), [crate::move_lock::MESSAGE]);
    assert!(!h.state().move_blocked && h.state().move_blocked_start.is_none());
}

#[test]
/// With Auto-Select on, a press inside a locked selection is refused, and the press is recorded:
/// the point and the window that owns the gesture, so it can still show the lock message and
/// become a cross-document transfer with two documents open.
fn a_refused_press_inside_a_locked_selection_records_the_press() {
    let (mut h, paint) = harness();
    // The painted layer is locked in place and its pixels are selected.
    h.state_mut().session.execute("layer.select", json!({"layer": paint.0})).unwrap();
    h.state_mut().session.execute("layer.lockLayers", json!({"position": true})).unwrap();
    h.state_mut().session.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = true;
    assert_eq!(h.state().session.documents().len(), 2, "two documents open");

    use crate::canvas::{ToolEvent, tool_event};
    tool_event(h.state_mut(), ToolEvent::Down { x: 8.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(h.state().move_blocked, "the press on the locked selection is refused");
    assert_eq!(h.state().move_blocked_start, Some([8.0, 10.0]), "the press point is recorded");
    assert!(h.state().move_viewport.is_some(), "the window that owns the gesture is recorded");
}

#[test]
/// With Auto-Select on, an All-locked layer above an unlocked one is the transfer candidate:
/// the ordinary pick keeps the same-document selection, and the marquee grabs the locked layer.
fn an_all_locked_layer_above_the_pick_is_the_transfer_candidate() {
    let (mut h, paint) = harness();
    // A locked layer sits above the painted one and covers the press point (an empty raster
    // cannot be hit, so the cover carries its own pixels).
    h.state_mut().session.execute("layer.new.layer", json!({"name": "cover"})).unwrap();
    let cover = LayerId(h.state().session.active().unwrap().doc.layers.last().unwrap().id.0);
    h.state_mut().session.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    h.state_mut().session.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    h.state_mut().session.execute("layer.lockLayers", json!({"all": true})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = true;

    use crate::canvas::{ToolEvent, tool_event};
    tool_event(h.state_mut(), ToolEvent::Down { x: 8.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    let grabbed = h.state().layer_box.as_ref().and_then(|b| b.grabbed_layer);
    assert_eq!(grabbed, Some(cover), "the locked layer under the pointer is the transfer candidate");
    assert!(h.state().session.active().unwrap().is_layer_selected(paint), "the ordinary pick keeps the same-document selection");
}

#[test]
/// Releasing a blocked Move drag clears the blocked state.
fn blocked_move_gesture_release_clears_blocked_state() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().move_blocked = true;
    h.state_mut().move_blocked_start = Some([10.0, 10.0]);

    // When the mouse button is not down or released, finish clears the blocked move state
    let ctx = h.ctx.clone();
    crate::layer_transfer::finish(h.state_mut(), &ctx);
    assert!(!h.state().move_blocked);
    assert_eq!(h.state().move_blocked_start, None);
}

#[test]
/// A gesture begun in another document window survives the main window's `finish`.
fn extra_window_move_gesture_preserved_when_main_window_finishes() {
    let (mut h, _) = harness();
    let main_ctx = h.ctx.clone();
    let extra_vid = egui::ViewportId::from_hash_of("docwin-1");

    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().move_blocked = true;
    h.state_mut().move_blocked_start = Some([10.0, 10.0]);
    crate::tools::move_box::begin_with_layer(h.state_mut(), [10.0, 10.0], false, Some(photocraft_doc::LayerId(1)), false, false);
    h.state_mut().move_viewport = Some(extra_vid);

    // When the main window's finish runs, the extra window's gesture is preserved
    crate::layer_transfer::finish(h.state_mut(), &main_ctx);
    assert!(h.state().move_blocked);
    assert_eq!(h.state().move_blocked_start, Some([10.0, 10.0]));
    assert!(h.state().layer_box.is_some());

    // When the gesture's viewport finishes, the state is cleared
    let extra_ctx = egui::Context::default();
    let mut raw = egui::RawInput { viewport_id: extra_vid, ..Default::default() };
    raw.viewports.insert(extra_vid, egui::ViewportInfo::default());
    let mut out = extra_ctx.run_ui(raw, |ui| {
        crate::layer_transfer::finish(h.state_mut(), ui.ctx());
    });
    out.textures_delta.clear();
    assert!(!h.state().move_blocked);
    assert_eq!(h.state().move_blocked_start, None);
    assert!(h.state().layer_box.is_none());
}

/// #11971 review: a document window's `finish` must not complete a transfer the main window
/// armed. The transfer lives in the shared `Context`, so a second viewport of the same context
/// sees it; only the viewport that armed it may complete it.
#[test]
fn a_document_window_does_not_complete_a_transfer_armed_in_the_main_window() {
    let (mut h, paint) = harness();
    // One context, two viewports: the main window arms the transfer, the document window's
    // `finish` runs with its own (idle) pointer state.
    let ctx = egui::Context::default();
    let extra_vid = egui::ViewportId::from_hash_of("docwin-transfer");

    crate::layer_transfer::begin_from_panel(h.state_mut(), &ctx, paint);
    assert!(crate::layer_transfer::is_transfer_armed(&ctx), "the main window armed a transfer");

    // The document window finishes first: its pointer is up, but the transfer belongs to the
    // main window, so it must stay armed.
    let mut raw = egui::RawInput { viewport_id: extra_vid, ..Default::default() };
    raw.viewports.insert(extra_vid, egui::ViewportInfo::default());
    let mut out = ctx.run_ui(raw, |ui| {
        crate::layer_transfer::finish(h.state_mut(), ui.ctx());
    });
    out.textures_delta.clear();
    assert!(crate::layer_transfer::is_transfer_armed(&ctx), "the main window's transfer survives another viewport's finish");

    // The main window's own release completes it.
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerButton {
        pos: egui::Pos2::new(20.0, 20.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    let mut out = ctx.run_ui(raw, |ui| {
        crate::layer_transfer::finish(h.state_mut(), ui.ctx());
    });
    out.textures_delta.clear();
    assert!(!crate::layer_transfer::is_transfer_armed(&ctx), "the main window completes its transfer");
}

/// #11971 review: releasing the secondary button during a locked-layer drag keeps the marquee,
/// so the primary release still selects the layers it covers.
#[test]
fn a_secondary_button_release_keeps_the_marquee_until_the_primary_release() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;

    // A marquee over a locked layer, as a blocked Move drag leaves it.
    crate::tools::move_box::begin_with_layer(h.state_mut(), [20.0, 20.0], false, Some(paint), false, false);
    h.state_mut().move_blocked = true;
    h.state_mut().move_blocked_start = Some([20.0, 20.0]);

    // The secondary button is released while the primary button is still down.
    let ctx = h.ctx.clone();
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerButton {
        pos: egui::Pos2::new(20.0, 20.0),
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    raw.events.push(egui::Event::PointerButton {
        pos: egui::Pos2::new(20.0, 20.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    let mut out = ctx.run_ui(raw, |ui| {
        crate::layer_transfer::finish(h.state_mut(), ui.ctx());
    });
    out.textures_delta.clear();

    assert!(h.state().layer_box.is_some(), "the marquee survives a secondary-button release");
    assert!(h.state().move_blocked, "the blocked move survives a secondary-button release");
    assert_eq!(h.state().move_blocked_start, Some([20.0, 20.0]));
}

/// #11971 review: a secondary-button release while the drag is held must not complete a
/// cross-document transfer; only the primary release does.
#[test]
fn a_secondary_button_release_does_not_complete_a_transfer() {
    let (mut h, paint) = harness();
    let ctx = egui::Context::default();

    crate::layer_transfer::begin_from_panel(h.state_mut(), &ctx, paint);
    assert!(crate::layer_transfer::is_transfer_armed(&ctx), "the window armed a transfer");

    // The secondary button is released while the primary button is still down.
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerButton {
        pos: egui::Pos2::new(20.0, 20.0),
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    raw.events.push(egui::Event::PointerButton {
        pos: egui::Pos2::new(20.0, 20.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    let mut out = ctx.run_ui(raw, |ui| {
        crate::layer_transfer::finish(h.state_mut(), ui.ctx());
    });
    out.textures_delta.clear();

    assert!(crate::layer_transfer::is_transfer_armed(&ctx), "a secondary release leaves the transfer armed for the intended primary drop");
}

/// #11971 review: with Auto-Select off, pressing a locked layer (e.g. Background) and dragging
/// onto another document tab transfers the layer without firing the lock dialog mid-drag.
#[test]
fn a_move_tool_background_layer_drag_with_auto_select_off_transfers_via_tab_without_lock_dialog() {
    let (mut h, paint) = harness();
    h.state_mut().session.execute("layer.delete", json!({"layer": paint.0})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.run_steps(2);

    let grab = ViewXform::active(h.state()).unwrap().to_screen(15.0, 15.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);

    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2);
    let (name, _) = copy(&h);
    assert_eq!(name, "Layer 1");
    assert!(h.state().ui.dialogs.iter().all(|d| d.kind != crate::state::DialogKind::Error), "no lock dialog during transfer");
}

/// #11971 review: with Auto-Select off, dragging Background directly to an arranged neighbour
/// cell transfers it without popping up the lock dialog.
#[test]
fn a_move_tool_background_layer_drag_with_auto_select_off_transfers_in_arranged_view() {
    let (mut h, paint) = harness();
    h.state_mut().session.execute("layer.delete", json!({"layer": paint.0})).unwrap();
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.run_steps(4);

    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    assert_eq!(cells.len(), 2);
    let (src_cell, dest_cell) = (cells[0].1, cells[1].1);

    let view_src = h.state().ui.views[0].clone();
    let xf_src =
        crate::canvas::ViewXform { rect: src_cell, zoom: view_src.zoom / h.state().ppp, center: view_src.center, flip: false, rotation: 0.0, aspect: 1.0 };
    let grab = xf_src.to_screen(15.0, 15.0);
    let drop = dest_cell.center();
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);

    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2);
    let (name, _) = copy(&h);
    assert_eq!(name, "Layer 1");
    assert!(h.state().ui.dialogs.iter().all(|d| d.kind != crate::state::DialogKind::Error), "no lock dialog during arranged transfer");
}

/// #11971 review: with Auto-Select off and a locked active layer (Background), pressing over
/// another layer (`paint`) captures that pressed layer for cross-document transfer instead of
/// transferring the active locked layer.
#[test]
fn a_blocked_move_with_auto_select_off_transfers_the_pressed_layer() {
    let (mut h, paint) = harness();
    let bg = h.state().session.documents()[0].doc.layers[0].id;
    h.state_mut().session.execute("layer.select", json!({"layer": bg.0})).unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.run_steps(2);

    // Press directly over the 10x10 red square on `paint` while Background is active.
    let grab = ViewXform::active(h.state()).unwrap().to_screen(8.0, 10.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);

    let (name, r) = copy(&h);
    assert_eq!(name, "paint", "transfers the layer hit at the press, not Background");
    assert_eq!(r.size(), vec2(10.0, 10.0));
    assert_eq!(h.state().session.documents()[0].active_layer, Some(bg), "source active layer stayed unchanged");
    assert!(h.state().session.documents()[0].doc.layer(paint).is_some());
}

/// #11971 review: when a blocked Move gesture starts in an extra document window, running the
/// extra-window pass (`extra_windows`) and starting a fresh press in another viewport do not leave
/// stale blocked state consuming later events.
#[test]
fn stale_blocked_move_from_extra_window_cleans_up_before_consuming_other_viewport_events() {
    let (mut h, _paint) = harness();
    let main_vid = h.ctx.viewport_id();
    let extra_vid = egui::ViewportId::from_hash_of("docwin-stale");
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;

    use crate::canvas::{ToolEvent, tool_event};
    // Blocked drag begins in the extra viewport (`move_viewport = Some(extra_vid)`).
    h.state_mut().current_viewport = Some(extra_vid);
    let bg = h.state().session.documents()[0].doc.layers[0].id;
    h.state_mut().session.execute("layer.select", json!({"layer": bg.0})).unwrap();
    tool_event(h.state_mut(), ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(h.state().move_blocked);
    assert_eq!(h.state().move_viewport, Some(extra_vid));

    // A fresh Down on an unlocked layer in the main viewport clears stale extra-window state
    // rather than letting later Move/Up events get eaten by the old blocked gesture.
    h.state_mut().current_viewport = Some(main_vid);
    let paint = h.state().session.documents()[0].doc.layers[1].id;
    h.state_mut().session.execute("layer.select", json!({"layer": paint.0})).unwrap();
    tool_event(h.state_mut(), ToolEvent::Down { x: 8.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(!h.state().move_blocked);
    assert!(h.state().move_blocked_layer.is_none());
    assert!(h.state().drag.is_some(), "unlocked drag starts cleanly in main viewport");
    tool_event(h.state_mut(), ToolEvent::Up { x: 8.0, y: 10.0 }, Modifiers::NONE);
}

/// #11971 review: when Auto-Select is off and the active layer is locked, pressing over empty
/// space (no layer hit under pointer, no selection) must not transfer the locked active layer.
#[test]
fn a_blocked_move_over_empty_space_does_not_transfer_locked_active_layer() {
    let (mut h, paint) = harness();
    // Hide Background and lock `paint` in place so (25, 25) is transparent empty space.
    let bg = h.state().session.documents()[0].doc.layers[0].id;
    h.state_mut()
        .session
        .edit("hide bg and lock paint", |doc, active| {
            doc.layer_mut(bg).unwrap().visible = false;
            doc.layer_mut(paint).unwrap().locks.position = true;
            *active = Some(paint);
            Ok(())
        })
        .unwrap();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.run_steps(2);

    let grab = ViewXform::active(h.state()).unwrap().to_screen(25.0, 25.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);

    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1, "nothing transferred from empty space");
}
