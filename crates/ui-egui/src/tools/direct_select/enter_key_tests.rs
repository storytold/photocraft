//! #13208: Enter deselects anchors on a targeted saved path without editing it.
use crate::{
    PhotocraftApp,
    direct_select::{DirectSelection, PathRef},
    state::Tool,
};
use egui::{Key, Modifiers};
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":800,"height":600})).unwrap();
    app.run("path.set", json!({"name":"Path 1","path":{"subpaths":[{"knots":[[100,100],[300,300],[500,100]]}]}})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    app.ui.selected_path = Some("Path 1".into());
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 300.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 300.0, y: 300.0 }, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    app
}
fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    });
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}
fn snapshot(app: &PhotocraftApp) -> (photocraft_doc::vector::Path, usize) {
    let st = app.session.active().unwrap();
    (st.doc.paths[0].path.clone(), st.history.past_len())
}
#[test]
fn enter_clears_selection_target_outline_and_subsequent_nudge() {
    for multiple in [false, true] {
        let mut app = app();
        if multiple {
            app.ui.direct_selection.anchors.push([0, 0]);
        }
        app.ui.direct_selection.segment = Some([0, 0]);
        let before = snapshot(&app);
        press(&mut app, Key::Enter, Modifiers::NONE);
        assert_eq!(app.ui.direct_selection, DirectSelection::default());
        assert_eq!(app.ui.selected_path, None);
        assert!(!super::active_paths(&app).iter().any(|(r, _)| *r == PathRef::Saved(0)), "outline hidden");
        assert_eq!(snapshot(&app), before, "Enter changes neither geometry nor History");
        press(&mut app, Key::ArrowRight, Modifiers::NONE);
        assert_eq!(snapshot(&app), before, "deselected anchors cannot be nudged");
    }
}
#[test]
fn escape_keeps_selection_and_nudge_still_works() {
    let mut app = app();
    let before = snapshot(&app);
    press(&mut app, Key::Escape, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
    assert_eq!(snapshot(&app), before);
    press(&mut app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(snapshot(&app).0.subpaths[0].knots[1].anchor.x, 301.0);
}
#[test]
fn unmeasured_empty_selection_and_other_tools_keep_the_target() {
    for tool in [Tool::DirectSelection, Tool::Move] {
        let mut app = app();
        app.ui.tool = tool;
        if tool == Tool::DirectSelection {
            app.ui.direct_selection = DirectSelection::default();
        }
        let before = snapshot(&app);
        press(&mut app, Key::Enter, Modifiers::NONE);
        assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
        assert_eq!(snapshot(&app), before);
    }
}
#[test]
fn modified_enter_keeps_selected_anchors() {
    for m in [Modifiers::ALT, Modifiers::SHIFT] {
        let mut app = app();
        press(&mut app, Key::Enter, m);
        assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
        assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
    }
}
#[test]
fn free_transform_points_commits_before_deselection() {
    let mut app = app();
    let before = snapshot(&app);
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().expect("Free Transform Points started");
    let [x0, y0, x1, y1] = t.rect;
    t.quad = [[x0 + 5.0, y0], [x1 + 5.0, y0], [x1 + 5.0, y1], [x0 + 5.0, y1]];
    press(&mut app, Key::Enter, Modifiers::NONE);
    assert!(app.ui.transform.is_none());
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));
    assert_ne!(snapshot(&app).0, before.0);
    assert_eq!(snapshot(&app).1, before.1 + 1);
}

#[test]
fn enter_preserves_unmeasured_work_path_and_segment_only_selection() {
    let mut app = app();
    app.ui.direct_selection.anchors.clear();
    app.ui.direct_selection.segment = Some([0, 0]);
    let before = app.ui.direct_selection.clone();
    press(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection, before);
    assert_eq!(app.ui.selected_path.as_deref(), Some("Path 1"));

    app.run("path.set", json!({"name":"work","path":{"subpaths":[{"knots":[[100,100],[300,300]]}]}})).unwrap();
    app.ui.selected_path = Some("work".into());
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 100.0, y: 100.0 }, Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Work));
    assert!(!app.ui.direct_selection.anchors.is_empty());
    let before = app.ui.direct_selection.clone();
    let n = snapshot(&app);
    let work = app.session.active().unwrap().doc.work_path.clone();
    press(&mut app, Key::Enter, Modifiers::NONE);
    assert_eq!(app.ui.selected_path.as_deref(), Some("work"));
    assert_eq!(app.ui.direct_selection, before);
    assert_eq!(snapshot(&app), n);
    assert_eq!(app.session.active().unwrap().doc.work_path, work);
}

#[test]
fn enter_then_arrow_cannot_edit_the_previously_selected_anchor() {
    let mut app = app();
    let before = snapshot(&app);
    press(&mut app, Key::Enter, Modifiers::NONE);
    press(&mut app, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(snapshot(&app), before, "Enter must prevent a later nudge and History entry");
}
