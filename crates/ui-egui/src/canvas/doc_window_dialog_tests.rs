//! A Window › New Window document window under an open dialog: its canvas is as inert as the main
//! window's, so a tool drag there adds no History step (the dialog's Cancel would not undo it).

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, View};

use super::document_windows::extra_windows;

fn app_with_a_layer() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    // Filled where the drag starts; Auto-Select off, so the press moves the active layer whatever
    // lies under it.
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 160, "height": 110})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries().to_vec()
}

/// Where the document window opens, in the harness's screen.
fn window_rect() -> Rect {
    Rect::from_min_size(pos2(100.0, 80.0), vec2(500.0, 400.0))
}

/// The app with a New Window of its document, drawn by `extra_windows` in the harness's own
/// context, where the framework dialogs are drawn too: the modal filter is per context. The window's
/// own view is 1:1 around the document's centre.
fn harness(mut app: PhotocraftApp) -> Harness<'static, (PhotocraftApp, bool)> {
    let document = app.session.documents()[0].doc.id;
    let id = app.ui.document_windows.new_window_for(&document, window_rect());
    let size = app.session.active().unwrap().doc.size;
    let view = View { zoom: 1.0, center: [size.width as f32 / 2.0, size.height as f32 / 2.0], pending: Vec::new(), ..app.ui.views[0].clone() };
    app.ui.window_views.insert(id, view);
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                crate::dialogs::run(app, ui);
                extra_windows(app, ui);
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.state_mut().1 = true;
    h.run_steps(4);
    h
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Drags across the middle of the document window's canvas.
fn drag_in_document_window(h: &mut Harness<'static, (PhotocraftApp, bool)>) {
    let start = window_rect().center();
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(button(start, true));
    h.run_steps(1);
    let mut at = start;
    for _ in 0..6 {
        at += vec2(15.0, 10.0);
        h.event(Event::PointerMoved(at));
        h.run_steps(1);
    }
    h.event(button(at, false));
    h.run_steps(2);
}

#[test]
fn a_document_window_canvas_takes_no_tool_drag_under_a_dialog() {
    let mut app = app_with_a_layer();
    let _dialog = crate::filter_dialog::open(&mut app, "image.rotation.arbitrary");
    assert!(app.dialog_open());
    let before = history(&app);
    let revision = app.session.active().unwrap().revision;
    let mut h = harness(app);
    drag_in_document_window(&mut h);
    let app = &h.state().0;
    assert!(app.dialog_open(), "the dialog is still open");
    assert_eq!(history(app), before, "no History step under the dialog");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document untouched");
}
