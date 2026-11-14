//! #14037: File › New lists the Saved tab second, after Recent (as Photoshop does), and an empty
//! Saved tab greys Create out (Enter does not create either), since nothing is selected there.

use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, UiState};

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
    h.run_steps(3);
    h
}

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn tab(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
    h.get_by_label(name).click();
    h.run_steps(2);
}

fn create_enabled(h: &Harness<'static, PhotocraftApp>) -> bool {
    !h.get_by_label("Create").accesskit_node().is_disabled()
}

#[test]
fn the_saved_tab_is_second() {
    const ORDER: [&str; 8] = ["Recent", "Saved", "Photo", "Print", "Art & Illustration", "Web", "Mobile", "Film & Video"];
    assert_eq!(crate::new_doc_ui::tabs().collect::<Vec<_>>(), ORDER);
    // The tab strip draws them in that order, left to right.
    let h = harness(app());
    let lefts: Vec<f32> = ORDER.iter().map(|name| h.get_by_label(name).rect().left()).collect();
    assert!(lefts.is_sorted_by(|a, b| a < b), "{ORDER:?} at {lefts:?}");
}

#[test]
fn create_is_greyed_on_an_empty_saved_tab() {
    let mut h = harness(app());
    assert!(create_enabled(&h), "Recent has a preset to create from");
    tab(&mut h, "Saved");
    assert!(h.query_by_label("No saved presets yet.").is_some());
    assert!(!create_enabled(&h), "nothing to create from");
    // Neither Create nor Enter makes a document.
    h.get_by_label("Create").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty());
    assert_eq!(h.state().ui.dialogs.len(), 1, "the dialog stays open");
    // The other tabs enable it again.
    for name in ["Photo", "Recent"] {
        tab(&mut h, name);
        assert!(create_enabled(&h), "{name}");
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 1, "Enter creates from Recent");
}

/// With saved presets, the Saved tab keeps today's behaviour (not measured): Create stays enabled.
#[test]
fn a_saved_preset_keeps_create_enabled() {
    let mut app = app();
    app.run("document.presets.save", json!({"name": "Square", "settings": {"width": 64, "height": 64}})).unwrap();
    let mut h = harness(app);
    tab(&mut h, "Saved");
    assert!(h.query_by_label("Square").is_some());
    assert!(create_enabled(&h));
}
