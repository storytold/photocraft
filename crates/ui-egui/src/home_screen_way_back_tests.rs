//! #16802: with a document open, the Home screen has a way back to it: the rail's Home row, and the document's entry
//! at the bottom of the Window menu. With no document open, Home has nothing behind it and both leave it up.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app showing its document area (the Home screen while Home is up), with `documents` documents open. The app's
/// own frame runs around it, so a command the Home screen fires is run as in the app.
fn harness(documents: usize) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for _ in 0..documents {
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_max_steps(64)
        .build_app(|ui, app: &mut PhotocraftApp| crate::canvas::document_area(app, ui), app);
    h.run_steps(4);
    h
}

/// Whether the Home screen is up.
fn home(h: &mut H) -> bool {
    h.state_mut().home_screen_up()
}

/// The options bar's Home button: shows Home over the open documents.
fn show_home(h: &mut H) {
    crate::canvas::start_screen::toggle_home(h.state_mut());
    h.run_steps(4);
}

/// A click on the Home row of the Home screen's rail.
fn click_home_row(h: &mut H) {
    h.get_by_label("Home").click();
    h.run_steps(4);
}

#[test]
fn the_rails_home_row_returns_to_the_open_document() {
    let mut h = harness(1);
    show_home(&mut h);
    assert!(home(&mut h), "Home is up over the document");
    let doc = h.state().session.active().map(|d| d.doc.id);
    click_home_row(&mut h);
    assert!(!home(&mut h), "the Home row leaves Home: {}", h.state().ui.status);
    assert_eq!(h.state().session.active().map(|d| d.doc.id), doc, "back on the same document, unchanged");
    assert_eq!(h.state().session.documents().len(), 1);
}

#[test]
fn choosing_an_open_document_in_the_window_menu_leaves_home() {
    let mut h = harness(2);
    let first = h.state().session.documents()[0].doc.id;
    show_home(&mut h);
    assert!(home(&mut h));
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.document.0", json!({})).unwrap();
    h.run_steps(4);
    assert!(!home(&mut h), "choosing the document leaves Home");
    assert_eq!(h.state().session.active().map(|d| d.doc.id), Some(first), "and shows that document");
}

#[test]
fn with_no_document_open_home_stays_up() {
    let mut h = harness(0);
    assert!(home(&mut h), "Home is all there is with no document");
    click_home_row(&mut h);
    assert!(home(&mut h), "the Home row has nothing to return to");
}
