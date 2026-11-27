//! #306: ⌘↩ (Ctrl+Enter off the Mac) loads a path as a selection, as in Photoshop: the path
//! being drawn with the Pen, the path selected in the Paths panel, or the work path. The key is
//! rebindable in Edit › Keyboard Shortcuts, and with no path it says why instead of doing nothing.

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use craft::behavior::geometry::SelectionOperation;
use craft::draw::ui::paths::PathsEvent;

use super::overlay::overlay_paths;
use super::paths_panel::apply;
use crate::PhotocraftApp;
use crate::direct_select::PathRef;
use crate::state::Tool;
use crate::vector_ui::PenPath;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(6);
    h
}

/// ⌘ on the Mac, Ctrl elsewhere, as egui-winit reports them.
fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers { command: true, mac_cmd: true, ..Default::default() }
    } else {
        Modifiers { command: true, ctrl: true, ..Default::default() }
    }
}

fn press(h: &mut Harness<'_, Box<PhotocraftApp>>, key: Key, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

/// A three-anchor open draft owned by the harness's current document.
fn triangle(h: &Harness<'_, Box<PhotocraftApp>>) -> PenPath {
    PenPath::corners(&[[20.0, 20.0], [120.0, 20.0], [120.0, 80.0]], crate::vector_ui::capture_pen_owner(h.state()))
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> serde_json::Value {
    let k = |x: f64, y: f64| json!({"anchor": [x, y]});
    json!({"subpaths": [{"closed": true, "knots": [k(x, y), k(x + w, y), k(x + w, y + h), k(x, y + h)]}]})
}

/// The selection's bounds (x, y, width, height), if any.
fn selection(h: &Harness<'_, Box<PhotocraftApp>>) -> Option<[i64; 4]> {
    let r = h.state().session.active()?.doc.selection.as_ref()?.content_bounds();
    Some([r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0].map(i64::from))
}

#[test]
fn cmd_enter_with_the_pen_loads_the_work_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "⌘↩ made a selection: {}", h.state().ui.status);
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
}

#[test]
fn cmd_enter_while_drawing_finishes_the_path_and_loads_it() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let pen = triangle(&h);
    h.state_mut().ui.pen = Some(pen);
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.ui.pen.is_none(), "the pen path was finished");
    assert!(app.session.active().unwrap().doc.work_path.is_some(), "into the work path");
    assert!(app.session.active().unwrap().doc.selection.is_some(), "and loaded as a selection: {}", app.ui.status);
    // Then deselected and no longer drawn, as in Photoshop; the work path stays (#6079).
    assert_eq!(app.ui.selected_path, None);
    assert!(!crate::vector_ui::work_path_shown(app));
    // Plain ↩ still only finishes a path.
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let pen = triangle(&h);
    h.state_mut().ui.pen = Some(pen);
    h.run_steps(2);
    press(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some());
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

#[test]
fn selecting_a_saved_path_keeps_the_hidden_work_path_hidden() {
    // #6079 review: only selecting the Work Path in the Paths panel shows it again.
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "Saved", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    let pen = triangle(&h);
    h.state_mut().ui.pen = Some(pen);
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert!(!crate::vector_ui::work_path_shown(h.state()), "⌘↩ hid the work path");
    panel(&mut h, 2);
    let saved = h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Saved").rect();
    click(&mut h, egui::pos2(saved.left() + 70.0, saved.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"));
    assert!(!crate::vector_ui::work_path_shown(h.state()), "a saved path doesn't show the Work Path");
    // So Delete with the Pen doesn't delete the invisible work path either.
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the hidden work path survives Delete");
    // Selecting the Work Path shows it; now Delete deletes it.
    let work = h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Work Path").rect();
    click(&mut h, egui::pos2(work.left() + 70.0, work.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(crate::vector_ui::work_path_shown(h.state()), "selecting the Work Path shows it");
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "{}", h.state().ui.status);
}

#[test]
fn reselecting_the_hidden_work_path_ends_the_hide() {
    // #6079 review: once the Work Path is selected again in the Paths panel, deselecting it there
    // leaves an ordinary unselected Work Path (drawn and edited with a path tool, #6551), not the
    // hidden one from before.
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    let pen = triangle(&h);
    h.state_mut().ui.pen = Some(pen);
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert!(!crate::vector_ui::work_path_shown(h.state()), "⌘↩ hid the work path");
    panel(&mut h, 2);
    let work = h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Work Path").rect();
    click(&mut h, egui::pos2(work.left() + 70.0, work.center().y), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    // Clicking the panel's empty area deselects it.
    click(&mut h, egui::pos2(work.left() + 70.0, work.bottom() + 40.0), Modifiers::NONE);
    assert_eq!(h.state().ui.selected_path, None);
    assert!(crate::vector_ui::work_path_shown(h.state()), "the unchanged Work Path didn't go back into hiding");
    assert_eq!(crate::vector_ui::active_path_name(h.state()).as_deref(), Some("work"));
    // So Delete with the Pen deletes it, like any unselected Work Path.
    press(&mut h, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().doc.work_path.is_none(), "{}", h.state().ui.status);
}

#[test]
fn cmd_enter_loads_the_path_selected_in_the_paths_panel() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    // Any tool, with a path selected in the Paths panel (as in Photoshop).
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]), "{}", h.state().ui.status);
    // A selected path that went away falls back to the work path once that is selected again.
    h.state_mut().run("path.delete", json!({"name": "Path 1"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([0, 0, 30, 30]));
}

#[test]
fn cmd_enter_on_a_saved_path_leaves_no_target_path() {
    // #6079: after loading a saved path nothing is the target path; the shown Work Path must not
    // take over (a second ⌘↩ or the canvas menu's Delete Path would act on it).
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([100, 40, 60, 50]), "{}", h.state().ui.status);
    assert_eq!(h.state().ui.selected_path, None);
    assert_eq!(crate::vector_ui::active_path_name(h.state()), None, "the Work Path didn't become the target path");
    assert!(!crate::vector_ui::work_path_shown(h.state()), "the Work Path isn't drawn");
    // A second ⌘↩ has no path to load.
    h.state_mut().run("select.deselect", json!({})).unwrap();
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), None, "the unselected Work Path wasn't loaded");
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the Work Path stays");
}

#[test]
fn without_a_path_it_says_why_and_the_key_can_be_rebound() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Pen;
    press(&mut h, Key::Enter, command());
    let app = h.state();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status_error && app.ui.status.contains("No path"), "status: {}", app.ui.status);
    // It's a listed, rebindable shortcut.
    let cmd_enter = crate::shortcuts::parse("Cmd+Enter").unwrap();
    let bound = crate::shortcuts::effective_shortcut(app, "path.toSelection", crate::shortcuts::default_shortcut("path.toSelection").as_deref());
    assert_eq!(bound.as_deref().and_then(crate::shortcuts::parse), Some(cmd_enter));
    h.state_mut().run("edit.keyboardShortcuts", json!({"set": {"path.toSelection": "Cmd+Shift+Y"}})).unwrap();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 20.0, 20.0)})).unwrap();
    press(&mut h, Key::Enter, command());
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "⌘↩ was moved");
    let mut m = command();
    m.shift = true;
    press(&mut h, Key::Y, m);
    assert_eq!(selection(&h), Some([10, 10, 20, 20]), "{}", h.state().ui.status);
}

fn panel(h: &mut Harness<'_, Box<PhotocraftApp>>, tab: usize) {
    let ctx = h.ctx.clone();
    let (req, _) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":"promedium", "dockTabs":{"layers":tab}, "dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, Box<PhotocraftApp>>, at: egui::Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.event(Event::ModifiersChanged(modifiers));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// A Cmd/Ctrl-click on a row's thumbnail, as the Paths panel reports it to the app.
// PLAN: assumed `paths_panel::apply(app: &mut PhotocraftApp, event: PathsEvent<PathRef>)` (tools-w36)
fn load(h: &mut Harness<'_, Box<PhotocraftApp>>, key: PathRef, operation: SelectionOperation) {
    apply(h.state_mut(), PathsEvent::load_selection(key, operation));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_targets_thumbnail_and_clicked_path() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name":"Saved","path":rect_path(100.0,40.0,60.0,50.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    load(&mut h, PathRef::Work, SelectionOperation::New);
    assert_eq!(selection(&h), Some([10, 10, 50, 40]), "{}", h.state().ui.status);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"), "loading a thumbnail does not change the selected path");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(selection(&h).is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
    h.state_mut().run("select.deselect", json!({})).unwrap();
    load(&mut h, PathRef::Saved(0), SelectionOperation::New);
    assert_eq!(selection(&h), Some([100, 40, 60, 50]));
}

#[test]
fn thumbnail_selection_ctrl_click_paths_combines_with_shift_and_alt() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(10.0,10.0,50.0,40.0)})).unwrap();
    h.state_mut().run("select.rect", json!({"x":100,"y":40,"width":20,"height":20})).unwrap();
    load(&mut h, PathRef::Work, SelectionOperation::Add);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) > 0.99 && selected.sample_channel(110, 50, 0) > 0.99);
    h.state_mut().run("select.all", json!({})).unwrap();
    load(&mut h, PathRef::Work, SelectionOperation::Subtract);
    let selected = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selected.sample_channel(20, 20, 0) < 0.01 && selected.sample_channel(110, 50, 0) > 0.99);
    h.state_mut().run("select.rect", json!({"x":30,"y":0,"width":100,"height":100})).unwrap();
    load(&mut h, PathRef::Work, SelectionOperation::Intersect);
    assert_eq!(selection(&h), Some([30, 10, 30, 40]));
}

#[test]
fn thumbnail_selection_ctrl_click_layer_path_preserves_panel_target() {
    let mut h = harness();
    let shape =
        h.state_mut().run("shape.create", json!({"kind":"rect","rect":[40,20,60,40],"fill":"#cc3333","name":"Badge"})).unwrap()["layer"].as_u64().unwrap();
    h.state_mut().run("path.set", json!({"name":"work","path":rect_path(0.0,0.0,10.0,10.0)})).unwrap();
    h.state_mut().ui.selected_path = Some("work".into());
    load(&mut h, PathRef::Layer(shape), SelectionOperation::New);
    assert_eq!(selection(&h), Some([40, 20, 60, 40]));
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target);
}

/// #3915: Clicking a shape with the Path Selection tool selects its layer.
#[test]
fn clicking_shape_with_path_selection_selects_its_layer() {
    let mut h = harness();
    let rect_layer =
        h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [20, 20, 80, 60], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    // Add a new empty layer above so rect is not the active layer.
    let empty_layer = h.state_mut().run("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, empty_layer);

    // Clicking inside the rectangle with Path Selection selects the rect layer.
    h.state_mut().ui.tool = Tool::PathSelection;
    crate::vector_ui::path_selection_finish(h.state_mut(), [50.0, 50.0], [50.0, 50.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);

    // Clicking outside does not change active layer.
    crate::vector_ui::path_selection_finish(h.state_mut(), [150.0, 90.0], [150.0, 90.0]);
    assert_eq!(h.state().session.active().unwrap().active_layer.unwrap().0, rect_layer);
}

/// A work path selected in the Paths panel remains visible regardless of the active tool (one that is
/// not selected is hidden, #6551): the overlay holds it with Brush active.
#[test]
fn path_overlay_drawn_for_non_vector_tools() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("path.set", json!({"name": "work", "path": rect_path(50.0, 10.0, 100.0, 80.0)})).unwrap();
    app.ui.selected_path = Some("work".into());
    app.ui.tool = Tool::Brush;
    let doc = app.session.active().unwrap().doc.id;
    assert!(overlay_paths(&app, doc).iter().any(|(which, ..)| *which == PathRef::Work), "the selected Work Path is drawn with any tool");
}

#[test]
fn loading_a_path_after_hiding_extras_restores_outline() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(10.0, 10.0, 50.0, 40.0)})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
    assert!(!h.state().ui.view.extras);
    press(&mut h, Key::Enter, command());
    assert_eq!(selection(&h), Some([10, 10, 50, 40]));
    assert!(h.state().ui.view.shows(h.state().ui.view.show.selection_edges));
}

/// ⌘⇧↩ / ⌘⌥↩ (#8015) take the same path as ⌘↩: a path being drawn with the Pen is finished first
/// and combined, not the old work path.
#[test]
fn cmd_shift_enter_while_drawing_finishes_the_path_and_adds_it() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 10.0, 10.0)})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 150, "y": 60, "width": 20, "height": 20})).unwrap();
    h.state_mut().ui.tool = Tool::Pen;
    // The work path isn't selected in the Paths panel, so the Pen starts a new work path.
    h.state_mut().ui.selected_path = None;
    let pen = triangle(&h);
    h.state_mut().ui.pen = Some(pen);
    h.run_steps(2);
    press(&mut h, Key::Enter, Modifiers { shift: true, ..command() });
    assert!(h.state().ui.pen.is_none(), "the pen path was finished: {}", h.state().ui.status);
    // The new triangle (20..120 × 20..80) joins the selected square; the old work path is not used.
    assert_eq!(selection(&h), Some([20, 20, 150, 60]));
    assert_eq!(h.state().ui.selected_path, None, "loaded path deselected, as with ⌘↩");
}

#[test]
fn cmd_shift_and_alt_enter_use_the_path_selected_in_the_paths_panel() {
    let mut h = harness();
    h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(0.0, 0.0, 30.0, 30.0)})).unwrap();
    h.state_mut().run("path.set", json!({"name": "Path 1", "path": rect_path(100.0, 40.0, 60.0, 50.0)})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap();
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().ui.selected_path = Some("Path 1".into());
    h.run_steps(2);
    press(&mut h, Key::Enter, Modifiers { shift: true, ..command() });
    assert_eq!(selection(&h), Some([40, 40, 120, 50]), "{}", h.state().ui.status);
    // ⌘⌥↩ with the saved path selected again takes it back out, leaving the square.
    h.state_mut().ui.selected_path = Some("Path 1".into());
    press(&mut h, Key::Enter, Modifiers { alt: true, ..command() });
    assert_eq!(selection(&h), Some([40, 40, 20, 20]), "{}", h.state().ui.status);
}

/// The selected area in pixels (partly selected pixels count in part).
fn selected_area(h: &Harness<'_, Box<PhotocraftApp>>) -> f32 {
    let d = &h.state().session.active().unwrap().doc;
    d.selection.as_ref().map_or(0.0, |s| s.read_region(d.bounds()).iter().sum())
}

/// #7726: as in Photoshop, ⇧⌘↩ adds the path to the selection, ⌥⌘↩ subtracts it and ⇧⌥⌘↩
/// intersects them. A 100×60 selection and a 100×20 path overlapping it by 40×20; the areas are
/// Photoshop 2025's for the same document.
#[test]
fn shift_and_option_with_cmd_enter_combine_the_path_with_the_selection() {
    let alt = Modifiers { alt: true, ..command() };
    let shift = Modifiers { shift: true, ..command() };
    let both = Modifiers { shift: true, ..alt };
    for (mods, area) in [(shift, 7200.0), (alt, 5200.0), (both, 800.0), (command(), 2000.0)] {
        let mut h = harness();
        h.state_mut().run("path.set", json!({"name": "work", "path": rect_path(80.0, 40.0, 100.0, 20.0)})).unwrap();
        h.state_mut().run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 60, "antiAlias": false})).unwrap();
        h.state_mut().ui.tool = Tool::Brush;
        h.run_steps(2);
        press(&mut h, Key::Enter, mods);
        assert!((selected_area(&h) - area).abs() < 0.5, "{mods:?}: {} px, wanted {area}: {}", selected_area(&h), h.state().ui.status);
    }
}
