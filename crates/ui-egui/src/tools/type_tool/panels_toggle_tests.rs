//! #12054: the Type options bar ends with the reference application's button that shows and
//! hides the Character and Paragraph panels. It runs Window › Character, so the cases the issue
//! leaves unmeasured keep that command's behaviour.

use crate::app::framework::BuildApp;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::panel_docking;

const TOGGLE: &str = "Toggle Character and Paragraph panels";

/// An app with one selected type layer.
fn app_with_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = crate::state::Tool::Type;
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let made = app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let id = made["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": id, "mode": "replace"})).unwrap();
    app
}

/// The options bar in a row, once the app's fonts are set up (the first pass has none).
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(vec2(1400.0, 200.0)).with_max_steps(16).build_app(
        |ui, app| {
            ui.horizontal(|ui| super::options_bar::options_bar(app, ui));
        },
        app,
    );
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, TOGGLE).click();
    h.run();
}

/// Whether the Character | Paragraph group is shown, and whether Paragraph is its front tab.
fn group(h: &Harness<'static, Box<PhotocraftApp>>) -> (bool, bool) {
    (panel_docking::is_shown(h.state(), "character"), panel_docking::is_front(h.state(), "paragraph"))
}

/// Clicking the button twice: the group opens with Character in front, then closes.
#[test]
fn with_a_type_layer_it_opens_the_group_with_character_in_front_then_closes_it() {
    let mut h = harness(app_with_layer());
    assert!(!group(&h).0, "the group starts hidden");
    click(&mut h);
    assert_eq!(group(&h), (true, false));
    click(&mut h);
    assert!(!group(&h).0, "the second click hides the group");
}

/// Not measured: with Paragraph in front, the button does what Window › Character does and
/// brings Character to the front of the open group.
#[test]
fn with_paragraph_in_front_it_brings_character_forward_like_window_character() {
    let mut app = app_with_layer();
    let ctx = app.ctx().clone();
    // Character and Paragraph start closed: the group opens with Character, then Paragraph comes to its front.
    crate::menus::invoke(&mut app, &ctx, "window.panel.character", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.panel.paragraph", json!({})).unwrap();
    let mut h = harness(app);
    assert_eq!(group(&h), (true, true));
    click(&mut h);
    assert_eq!(group(&h), (true, false));
}
