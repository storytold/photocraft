//! #589: layers dragged onto another document's tab, then dropped on its canvas, are copied there.

use craft::behavior::kurbo::Point;
use craft::ui::canvas::ToolHost;
use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::CanvasHost;
use crate::state::Tool;

/// Document `d`'s view of `rect` (the canvas area, or a tile of it), as the transfer maps the pointer.
fn view_of(app: &PhotocraftApp, d: usize, rect: Rect) -> craft::behavior::CanvasView {
    app.ui.views[d].canvas_view(rect, app.canvas_ppp(), app.ui.view.flip_horizontal, app.ui.view.display_aspect())
}

/// The active document's view of the canvas area.
fn active_view(app: &PhotocraftApp) -> craft::behavior::CanvasView {
    view_of(app, app.session.active_index().unwrap(), app.last_canvas_rect)
}

fn to_screen(view: &craft::behavior::CanvasView, x: f64, y: f64) -> Pos2 {
    view.to_screen(Point::new(x, y))
}

fn to_doc(view: &craft::behavior::CanvasView, p: Pos2) -> [f64; 2] {
    let at = view.to_doc(p);
    [at.x, at.y]
}

/// Lays the open documents out side by side, as Window › Arrange › 2-up Vertical does, and returns the
/// two tiles `(document, rect)`.
fn tile(h: &mut Harness<'_, Box<PhotocraftApp>>) -> Vec<(usize, Rect)> {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.twoUpVertical", json!({})).unwrap();
    h.run_steps(3);
    crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).expect("the documents are tiled")
}

/// "source-doc" (40×30) with a red 10×10 square at (4, 6) on layer "paint", and "dest-doc"
/// (60×50). The source is active.
fn harness() -> (Harness<'static, Box<PhotocraftApp>>, LayerId) {
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
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    (h, paint)
}

/// A document tab of the strip, found by its accessible name (the document's name, then its detail). The document's
/// canvas carries the name too, so the query takes the tab.
fn tab(h: &Harness<'_, Box<PhotocraftApp>>, name: &str) -> Pos2 {
    let name = name.to_owned();
    h.get_by(move |n| n.role() == egui::accesskit::Role::Tab && n.label().is_some_and(|l| l.starts_with(&name))).rect().center()
}

/// Press at `path[0]`, move through the rest (a few frames at each) and release at the last
/// point with `release` held.
fn drag_along(h: &mut Harness<'_, Box<PhotocraftApp>>, path: &[Pos2], release: Modifiers) {
    drag_along_pressed(h, path, Modifiers::NONE, release);
}

/// [`drag_along`], with `press` held from the press on (⌥ for a Move-tool copy).
fn drag_along_pressed(h: &mut Harness<'_, Box<PhotocraftApp>>, path: &[Pos2], press: Modifiers, release: Modifiers) {
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
fn copy(h: &Harness<'_, Box<PhotocraftApp>>) -> (String, Rect) {
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.name, "dest-doc", "the destination is shown");
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let b = photocraft_engine::layer_multi_cmds::layer_bounds(l).unwrap();
    (l.name.clone(), Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

fn source_square(h: &Harness<'_, Box<PhotocraftApp>>, paint: LayerId) -> photocraft_geom::Rect {
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
    let grab = to_screen(&active_view(h.state()), 8.0, 10.0);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);
    let (_, r) = copy(&h);
    assert_eq!(r.size(), vec2(3.0, 4.0));
    let at = to_doc(&active_view(h.state()), drop);
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
    let grab = to_screen(&active_view(h.state()), 8.0, 10.0);
    let dest = tab(&h, "dest-doc");
    drag_along(&mut h, &[grab, dest, Pos2::new(-10.0, -10.0)], Modifiers::NONE);
    assert_eq!(*h.state().session.documents()[0].doc, *before);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 1);
}

#[test]
fn a_selection_can_be_dragged_directly_between_tiled_canvases() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    let cells = tile(&mut h);
    let before = h.state().session.documents()[0].doc.clone();
    let grab = to_screen(&view_of(h.state(), 0, cells[0].1.shrink(1.0)), 8.0, 10.0);
    let drop = cells[1].1.center();
    drag_along(&mut h, &[grab, drop], Modifiers::NONE);
    assert_eq!(copy(&h).1.size(), vec2(3.0, 4.0));
    assert_eq!(*h.state().session.documents()[0].doc, *before);
}

#[test]
fn a_plain_layer_can_be_dragged_directly_between_tiled_canvases() {
    let (mut h, paint) = harness();
    h.state_mut().ui.tool = Tool::Move;
    let cells = tile(&mut h);
    let before = source_square(&h, paint);
    let grab = to_screen(&view_of(h.state(), 0, cells[0].1.shrink(1.0)), 9.0, 11.0);
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
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    let cells = tile(&mut h);
    let before = h.state().session.documents()[0].doc.clone();
    let history = h.state().session.documents()[0].history.past_len();
    let source_view = view_of(h.state(), 0, cells[0].1.shrink(1.0));
    let grab = to_screen(&source_view, 8.0, 10.0);
    let moved = to_screen(&source_view, 13.0, 10.0);
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
    let grab = to_screen(&active_view(h.state()), 13.0, 10.0);
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
fn option_drag_of_a_moved_float(h: &mut Harness<'_, Box<PhotocraftApp>>) -> Pos2 {
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    h.state_mut().run("select.float", json!({"dx": 5, "dy": 0})).unwrap();
    h.run_steps(2);
    let grab = to_screen(&active_view(h.state()), 13.0, 10.0);
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

fn release_at(h: &mut Harness<'_, Box<PhotocraftApp>>, p: Pos2) {
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
    let opacity = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().session.documents()[0].doc.layer(paint).unwrap().opacity;
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
    assert!(crate::layer_transfer::pointer_if_armed(h.state(), &h.ctx).is_some(), "the Move drag is still under way");
    assert!(crate::move_mods::copying(h.state(), false), "the drag's move state was cleared");
    gate.store(true, Ordering::Relaxed);
    release_at(&mut h, over_tab);
}

#[test]
fn a_layers_panel_drag_onto_a_tab_drops_the_layer_where_it_is_released() {
    let (mut h, _) = harness();
    let row = crate::layers_panel_nodes::row(&h, "paint").center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::NONE);
    let at = to_doc(&active_view(h.state()), drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    assert!((r.center().x - at[0] as f32).abs() <= 1.0 && (r.center().y - at[1] as f32).abs() <= 1.0, "centred on the drop point {at:?}: {r:?}");
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 2, "the source keeps its layers");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn shift_centres_the_copy_when_the_documents_differ_in_size() {
    let (mut h, _) = harness();
    let row = crate::layers_panel_nodes::row(&h, "paint").center();
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
    let grab = to_screen(&active_view(h.state()), 9.0, 11.0);
    let grab_doc = to_doc(&active_view(h.state()), grab);
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[grab, over_tab, drop], Modifiers::NONE);
    let at = to_doc(&active_view(h.state()), drop);
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
    let row = crate::layers_panel_nodes::row(&h, "paint").center();
    let (dest_tab, source_tab) = (tab(&h, "dest-doc"), tab(&h, "source-doc"));
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, dest_tab, source_tab, drop], Modifiers::NONE);
    let docs = h.state().session.documents();
    assert_eq!((docs[0].doc.layers.len(), docs[1].doc.layers.len()), (2, 1));
    assert_eq!(h.state().session.active_index(), Some(0));
}

#[test]
fn a_layers_panel_drag_with_floating_windows() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2, "2 floating windows before drag");
    let row = crate::layers_panel_nodes::row(&h, "paint").center();
    // Start drag from the Layers panel
    h.event(egui::Event::PointerMoved(row));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: row, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(row + vec2(-50.0, 0.0)));
    h.run_steps(2);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);
    let dest_rect = h.state().ui.document_windows.floating.iter().find(|(g, _)| g.documents.contains(&h.state().session.documents()[1].doc.id)).unwrap().1;
    let target_center = dest_rect.center();
    h.event(egui::Event::PointerMoved(target_center));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: target_center, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2, "dest document should receive the dragged layer");
    assert_eq!(h.state().session.documents()[1].doc.layers[1].name, "paint");
    assert_eq!(h.state().session.active_index(), Some(1), "dest document becomes active after drop");
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 2, "source retains its layers");
    assert_eq!(h.state().ui.document_windows.floating.len(), 2, "floating windows remain open");
}

#[test]
fn a_move_drag_between_floating_windows() {
    let (mut h, paint) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);
    // Position windows side-by-side so they don't occlude each other
    h.state_mut().ui.document_windows.floating[0].1 = Rect::from_min_size(egui::pos2(50.0, 80.0), vec2(500.0, 400.0));
    h.state_mut().ui.document_windows.floating[1].1 = Rect::from_min_size(egui::pos2(600.0, 80.0), vec2(500.0, 400.0));
    // Set distinct views for source and destination floating windows
    h.state_mut().ui.views[0].zoom = 1.5;
    h.state_mut().ui.views[1].zoom = 2.0;
    h.state_mut().ui.views[1].center = [35.0, 20.0];
    h.state_mut().session.set_active(0);
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);
    let before = source_square(&h, paint);
    let host0 = CanvasHost::primary(h.state(), 0);
    let view0 = ToolHost::shown_space(&h.ctx, host0.canvas_id()).unwrap().view;
    let grab_doc = [9.0, 11.0];
    let grab = to_screen(&view0, grab_doc[0], grab_doc[1]);

    let host1 = CanvasHost::primary(h.state(), 1);
    let view1 = ToolHost::shown_space(&h.ctx, host1.canvas_id()).unwrap().view;
    assert_ne!(view0.zoom, view1.zoom, "source and destination views must have distinct zoom transforms");
    // Known off-center drop point in destination document coordinates (dest-doc size is 60×50, center is (30, 25))
    let drop_doc = [25.0, 35.0];
    let drop = to_screen(&view1, drop_doc[0], drop_doc[1]);

    drag_along(&mut h, &[grab, drop], Modifiers::NONE);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2, "dest document should receive the dragged layer from move tool");
    assert_eq!(h.state().session.documents()[1].doc.layers[1].name, "paint");

    let (name, copied) = copy(&h);
    assert_eq!(name, "paint");
    let offset_x = (drop_doc[0] - grab_doc[0]).round() as i32;
    let offset_y = (drop_doc[1] - grab_doc[1]).round() as i32;
    let expected_bounds = before.translate(offset_x, offset_y);
    let expected_rect =
        Rect::from_min_max(egui::pos2(expected_bounds.x0 as f32, expected_bounds.y0 as f32), egui::pos2(expected_bounds.x1 as f32, expected_bounds.y1 as f32));
    assert_eq!(copied, expected_rect, "copied layer bounds should match source bounds translated by drop-minus-grab offset");

    assert_eq!(h.state().session.active_index(), Some(1), "dest document becomes active after drop");
    assert_eq!(source_square(&h, paint), before, "the move transfer preserves the source layer");
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);
}

#[test]
fn a_move_drag_from_docked_to_floating_window() {
    let (mut h, paint) = harness();
    let ctx = h.ctx.clone();
    let dest_id = h.state().session.documents()[1].doc.id;
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatInWindow", json!({"document": dest_id.0})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 1);

    // Position the floating window
    let dest_rect = Rect::from_min_size(egui::pos2(600.0, 80.0), vec2(500.0, 400.0));
    h.state_mut().ui.document_windows.floating[0].1 = dest_rect;
    h.state_mut().ui.views[1].zoom = 2.0;
    h.state_mut().ui.views[1].center = [35.0, 20.0];
    h.state_mut().session.set_active(0);
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);

    let before = source_square(&h, paint);
    let host0 = CanvasHost::primary(h.state(), 0);
    let view0 = ToolHost::shown_space(&h.ctx, host0.canvas_id()).unwrap().view;
    let grab_doc = [9.0, 11.0];
    let grab = to_screen(&view0, grab_doc[0], grab_doc[1]);

    let host1 = CanvasHost::primary(h.state(), 1);
    let view1 = ToolHost::shown_space(&h.ctx, host1.canvas_id()).unwrap().view;
    let drop_doc = [25.0, 35.0];
    let drop = to_screen(&view1, drop_doc[0], drop_doc[1]);

    drag_along(&mut h, &[grab, drop], Modifiers::NONE);
    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2, "dest document should receive the dragged layer from move tool");
    assert_eq!(h.state().session.documents()[1].doc.layers[1].name, "paint");

    let (name, copied) = copy(&h);
    assert_eq!(name, "paint");
    let offset_x = (drop_doc[0] - grab_doc[0]).round() as i32;
    let offset_y = (drop_doc[1] - grab_doc[1]).round() as i32;
    let expected_bounds = before.translate(offset_x, offset_y);
    let expected_rect =
        Rect::from_min_max(egui::pos2(expected_bounds.x0 as f32, expected_bounds.y0 as f32), egui::pos2(expected_bounds.x1 as f32, expected_bounds.y1 as f32));
    assert_eq!(copied, expected_rect, "copied layer bounds should match source bounds translated by drop-minus-grab offset");

    assert_eq!(h.state().session.active_index(), Some(1), "dest document becomes active after drop");
    assert_eq!(source_square(&h, paint), before, "the move transfer preserves the source layer");
    assert_eq!(h.state().ui.document_windows.floating.len(), 1);
}

#[test]
fn a_move_drag_from_docked_to_floating_viewport_transfers_layer() {
    let (mut h, paint) = harness();
    let ctx = h.ctx.clone();
    let dest_id = h.state().session.documents()[1].doc.id;
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatInWindow", json!({"document": dest_id.0})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 1);

    let group_id = h.state().ui.document_windows.floating[0].0.id;
    let dest_rect = Rect::from_min_size(egui::pos2(600.0, 80.0), vec2(500.0, 400.0));
    h.state_mut().ui.document_windows.floating[0].1 = dest_rect;
    h.state_mut().ui.views[1].zoom = 2.0;
    h.state_mut().ui.views[1].center = [35.0, 20.0];
    h.state_mut().session.set_active(0);
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);

    let vp = egui::ViewportId::from_hash_of(("test-floating-vp", group_id));
    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 900.0))), ..Default::default() },
    );
    viewports
        .insert(vp, egui::ViewportInfo { inner_rect: Some(dest_rect), outer_rect: Some(dest_rect), native_pixels_per_point: Some(1.0), ..Default::default() });

    let before = source_square(&h, paint);
    let host0 = CanvasHost::primary(h.state(), 0);
    let view0 = ToolHost::shown_space(&h.ctx, host0.canvas_id()).unwrap().view;
    let grab_doc = [9.0, 11.0];
    let grab = to_screen(&view0, grab_doc[0], grab_doc[1]);

    let host1 = CanvasHost::primary(h.state(), 1);
    let view1 = ToolHost::shown_space(&h.ctx, host1.canvas_id()).unwrap().view;
    let drop_doc = [25.0, 35.0];
    let drop = to_screen(&view1, drop_doc[0], drop_doc[1]);

    // Press and initiate drag in ROOT
    h.event(egui::Event::PointerMoved(grab));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: grab, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);

    // Release occurs in the floating viewport `vp` (e.g. Linux multi-window), not ROOT
    let drop_local = drop - dest_rect.min.to_vec2();
    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports: viewports.clone(), ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});

    let vp_input = egui::RawInput {
        viewport_id: vp,
        viewports: viewports.clone(),
        events: vec![
            egui::Event::PointerMoved(drop_local),
            egui::Event::PointerButton { pos: drop_local, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ],
        ..Default::default()
    };
    let _ = h.ctx.run_ui(vp_input, |_| {});

    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));
    h.run_steps(4);

    assert_eq!(h.state().session.documents()[1].doc.layers.len(), 2, "dest document should receive the dragged layer from move tool via floating viewport");
    assert_eq!(h.state().session.documents()[1].doc.layers[1].name, "paint");

    let (name, copied) = copy(&h);
    assert_eq!(name, "paint");
    // Dest doc is 60×50, layer is 10×10. Target receives the layer centered (min=(25, 20), max=(35, 30)).
    assert_eq!(copied, Rect::from_min_max(egui::pos2(25.0, 20.0), egui::pos2(35.0, 30.0)));

    assert_eq!(h.state().session.active_index(), Some(1), "dest document becomes active after drop");
    assert_eq!(source_square(&h, paint), before, "the move transfer preserves the source layer");
    assert_eq!(h.state().jobs.focus, None, "jobs.focus should be cleared after transfer");
    assert_eq!(h.state().ui.document_windows.floating.len(), 1);
}

#[test]
fn drop_target_at_uses_maximized_bounds_for_maximized_floating_window() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let group_id = h.state().ui.document_windows.floating[1].0.id;
    let normal_rect = Rect::from_min_size(egui::pos2(50.0, 50.0), vec2(200.0, 200.0));
    h.state_mut().ui.document_windows.floating[1].1 = normal_rect;

    let vp = egui::ViewportId::from_hash_of(("test-maximized-vp", group_id));
    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let max_rect = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1200.0, 900.0));
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() },
    );
    viewports.insert(
        vp,
        egui::ViewportInfo {
            inner_rect: Some(max_rect),
            outer_rect: Some(max_rect),
            maximized: Some(true),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        },
    );

    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports: viewports.clone(), ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});
    let input = egui::RawInput { viewport_id: vp, viewports, ..Default::default() };
    let _ = h.ctx.run_ui(input, |_| {});

    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let p = egui::pos2(400.0, 20.0);
    assert!(!normal_rect.contains(p), "point is outside the normal placement rectangle");
    assert!(max_rect.contains(p), "point is inside the maximized window rectangle");

    let target = super::target_at(h.state(), p);
    assert_eq!(target, Some((1, None)), "drop hits the maximized floating window despite stale normal placement rect");
}

#[test]
fn drop_target_at_skips_minimized_floating_windows() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let group_id = h.state().ui.document_windows.floating[1].0.id;
    let normal_rect = Rect::from_min_size(egui::pos2(50.0, 50.0), vec2(200.0, 200.0));
    h.state_mut().ui.document_windows.floating[1].1 = normal_rect;

    let vp = egui::ViewportId::from_hash_of(("test-minimized-vp", group_id));
    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() },
    );
    viewports.insert(
        vp,
        egui::ViewportInfo {
            inner_rect: Some(normal_rect),
            outer_rect: Some(normal_rect),
            minimized: Some(true),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        },
    );

    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports: viewports.clone(), ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});
    let input = egui::RawInput { viewport_id: vp, viewports, ..Default::default() };
    let _ = h.ctx.run_ui(input, |_| {});

    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let p = normal_rect.center();
    let target = super::target_at(h.state(), p);
    assert_ne!(target.map(|(d, _)| d), Some(1), "drop should not target a minimized floating window");
}

#[test]
fn drop_target_at_translates_root_pointer_to_desktop_coordinates() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let group_id = h.state().ui.document_windows.floating[1].0.id;
    let desktop_bounds = Rect::from_min_size(egui::pos2(500.0, 50.0), vec2(200.0, 200.0));
    h.state_mut().ui.document_windows.floating[1].1 = desktop_bounds;

    let vp = egui::ViewportId::from_hash_of(("test-desktop-vp", group_id));
    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let root_origin = egui::pos2(100.0, 50.0);
    let mut viewports = egui::ViewportIdMap::default();
    viewports
        .insert(egui::ViewportId::ROOT, egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(root_origin, vec2(1200.0, 900.0))), ..Default::default() });
    viewports.insert(
        vp,
        egui::ViewportInfo { inner_rect: Some(desktop_bounds), outer_rect: Some(desktop_bounds), native_pixels_per_point: Some(1.0), ..Default::default() },
    );

    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports: viewports.clone(), ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});
    let input = egui::RawInput { viewport_id: vp, viewports, ..Default::default() };
    let _ = h.ctx.run_ui(input, |_| {});

    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let p = egui::pos2(450.0, 10.0);
    assert!(!desktop_bounds.contains(p), "root-local p is outside desktop bounds");
    assert!(desktop_bounds.contains(p + root_origin.to_vec2()), "translated desktop point is inside bounds");

    let target = super::target_at(h.state(), p);
    assert_eq!(target, Some((1, None)), "drop hits the floating window using translated desktop coordinates");
}

#[test]
fn drop_clears_jobs_focus_when_destination_already_active() {
    let (mut h, paint) = harness();
    let ctx = h.ctx.clone();

    // 1. Cross-document drop with dest already active:
    h.state_mut().session.set_active(1);
    let job = photocraft_engine::jobs::JobId(42);
    h.state_mut().jobs.focus = Some(job);
    assert_eq!(h.state().jobs.focus, Some(job));

    let t = super::Transfer {
        source: h.state().session.documents()[0].doc.id,
        layers: vec![paint.0],
        grab: None,
        label: "paint".to_string(),
        selection_only: false,
    };
    ctx.data_mut(|d| d.insert_temp(super::key(), t));

    super::drop(h.state_mut(), &ctx, 1, super::DropTarget::Tab);
    assert_eq!(h.state().jobs.focus, None, "jobs.focus should be cleared after cross-doc drop even when dest is already active");

    // 2. Same-document drop with dest already active:
    h.state_mut().session.set_active(0);
    h.state_mut().jobs.focus = Some(job);
    assert_eq!(h.state().jobs.focus, Some(job));

    let t_same = super::Transfer {
        source: h.state().session.documents()[0].doc.id,
        layers: vec![paint.0],
        grab: None,
        label: "paint".to_string(),
        selection_only: false,
    };
    ctx.data_mut(|d| d.insert_temp(super::key(), t_same));

    super::drop(h.state_mut(), &ctx, 0, super::DropTarget::Canvas([10.0, 10.0]));
    assert_eq!(h.state().jobs.focus, None, "jobs.focus should be cleared after same-doc drop when dest is already active");
}

#[test]
fn drop_target_at_ignores_floating_canvas_when_outside_window_bounds() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let group_id = h.state().ui.document_windows.floating[1].0.id;
    // Window 1 is placed far away on the desktop
    let desktop_bounds = Rect::from_min_size(egui::pos2(1500.0, 500.0), vec2(400.0, 300.0));
    h.state_mut().ui.document_windows.floating[1].1 = desktop_bounds;

    let vp = egui::ViewportId::from_hash_of(("test-faraway-vp", group_id));
    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() },
    );
    viewports.insert(
        vp,
        egui::ViewportInfo { inner_rect: Some(desktop_bounds), outer_rect: Some(desktop_bounds), native_pixels_per_point: Some(1.0), ..Default::default() },
    );

    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports: viewports.clone(), ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});
    let input = egui::RawInput { viewport_id: vp, viewports, ..Default::default() };
    let _ = h.ctx.run_ui(input, |_| {});

    let vp_key = egui::Id::unique(("floating document viewport", group_id));
    h.ctx.data_mut(|d| d.insert_temp(vp_key, vp));

    // A pointer at (100, 100) on the root window might fall inside window 1's local canvas rect,
    // but desktop_p (100, 100) is outside window 1's desktop bounds (1500, 500).
    let p = egui::pos2(100.0, 100.0);
    let target = super::target_at(h.state(), p);
    assert_ne!(target.map(|(d, _)| d), Some(1), "drop at (100, 100) must not hit floating window located at (1500, 500)");
}

#[test]
fn drop_target_at_uses_root_pointer_for_embedded_floating_windows() {
    let (mut h, _) = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let embedded_rect = Rect::from_min_size(egui::pos2(200.0, 100.0), vec2(300.0, 200.0));
    h.state_mut().ui.document_windows.floating[1].1 = embedded_rect;

    // Simulate root window having an offset on desktop
    let root_origin = egui::pos2(500.0, 400.0);
    let mut viewports = egui::ViewportIdMap::default();
    viewports
        .insert(egui::ViewportId::ROOT, egui::ViewportInfo { inner_rect: Some(Rect::from_min_size(root_origin, vec2(1200.0, 900.0))), ..Default::default() });
    let root_input = egui::RawInput { viewport_id: egui::ViewportId::ROOT, viewports, ..Default::default() };
    let _ = h.ctx.run_ui(root_input, |_| {});

    // Embedded window has no viewport facts (it's embedded).
    let p = egui::pos2(250.0, 150.0);
    assert!(embedded_rect.contains(p), "root-local p is inside embedded_rect");
    assert!(!embedded_rect.contains(p + root_origin.to_vec2()), "desktop_p is outside embedded_rect");

    let target = super::target_at(h.state(), p);
    assert_eq!(target.map(|(d, _)| d), Some(1), "drop hits embedded floating window using root-local coordinates");
}

fn same_size_harness() -> (Harness<'static, Box<PhotocraftApp>>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "source-doc"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "dest-doc"})).unwrap();
    s.set_active(0);
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    (h, paint)
}

#[test]
fn frame_drop_always_centres_layer_even_on_same_size_documents() {
    let (mut h, paint) = same_size_harness();
    let ctx = h.ctx.clone();
    let t = super::Transfer {
        source: h.state().session.documents()[0].doc.id,
        layers: vec![paint.0],
        grab: None,
        label: "paint".to_string(),
        selection_only: false,
    };
    ctx.data_mut(|d| d.insert_temp(super::key(), t));

    super::drop(h.state_mut(), &ctx, 1, super::DropTarget::Frame);

    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    // Dest doc is 40×30, layer is 10×10. Centering puts it at min=(15, 10), max=(25, 20).
    assert_eq!(r, Rect::from_min_max(egui::pos2(15.0, 10.0), egui::pos2(25.0, 20.0)));
}

#[test]
fn tab_drop_preserves_position_on_same_size_documents() {
    let (mut h, paint) = same_size_harness();
    let ctx = h.ctx.clone();
    let t = super::Transfer {
        source: h.state().session.documents()[0].doc.id,
        layers: vec![paint.0],
        grab: None,
        label: "paint".to_string(),
        selection_only: false,
    };
    ctx.data_mut(|d| d.insert_temp(super::key(), t));

    super::drop(h.state_mut(), &ctx, 1, super::DropTarget::Tab);

    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    // Source was at (4, 6) to (14, 16); same-size tab drop keeps position.
    assert_eq!(r, Rect::from_min_max(egui::pos2(4.0, 6.0), egui::pos2(14.0, 16.0)));
}

#[test]
fn shift_canvas_drop_preserves_position_on_same_size_documents() {
    let (mut h, paint) = same_size_harness();
    h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
    h.run_steps(1);
    let ctx = h.ctx.clone();
    let t = super::Transfer {
        source: h.state().session.documents()[0].doc.id,
        layers: vec![paint.0],
        grab: None,
        label: "paint".to_string(),
        selection_only: false,
    };
    ctx.data_mut(|d| d.insert_temp(super::key(), t));

    super::drop(h.state_mut(), &ctx, 1, super::DropTarget::Canvas([30.0, 20.0]));

    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    // Shift-drop on same size keeps original position.
    assert_eq!(r, Rect::from_min_max(egui::pos2(4.0, 6.0), egui::pos2(14.0, 16.0)));
}

#[test]
fn floating_window_frame_drop_centres_on_same_size_document() {
    let (mut h, _) = same_size_harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.document_windows.floating.len(), 2);

    let dest_rect = Rect::from_min_size(egui::pos2(500.0, 80.0), vec2(400.0, 300.0));
    h.state_mut().ui.document_windows.floating[1].1 = dest_rect;
    h.run_steps(2);

    // Verify target_at returns (1, None) for frame point outside canvas
    let frame_point = dest_rect.min + vec2(50.0, 5.0);
    let target = super::target_at(h.state(), frame_point);
    assert_eq!(target, Some((1, None)), "frame point hits floating window frame chrome without canvas point");

    let row = crate::layers_panel_nodes::row(&h, "paint").center();
    drag_along(&mut h, &[row, frame_point], Modifiers::NONE);

    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    assert_eq!(r, Rect::from_min_max(egui::pos2(15.0, 10.0), egui::pos2(25.0, 20.0)));
}
