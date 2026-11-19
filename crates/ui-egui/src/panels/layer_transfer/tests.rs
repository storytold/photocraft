//! #589: layers dragged onto another document's tab, then dropped on its canvas, are copied there.

use craft::behavior::kurbo::Point;
use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
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
