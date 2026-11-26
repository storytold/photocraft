//! #16798: the Reset All Tools confirmation reads "Reset all tools to the default settings?" under
//! the app's name, with a warning icon beside the text, as the reference does. OK stays the default
//! button (Enter resets every tool) and Escape resets nothing.

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::ToolOptions;
use crate::tool_reset::RESET_ALL;

const MESSAGE: &str = "Reset all tools to the default settings?";

type H = Harness<'static, Box<PhotocraftApp>>;

/// The dialog layer over an 8 x 8 document, with the marquees' Feather moved off its default.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.ui.tool_options.feather = 12.0;
    let h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Opens the confirmation as the options bar's Reset All Tools does, and draws it.
fn ask(h: &mut H) {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, RESET_ALL, json!({})).unwrap();
    assert_eq!(reply["confirmationPending"], true, "{reply}");
    h.run_steps(4);
}

#[test]
fn the_question_reads_the_reference_text_under_the_app_name_with_a_warning_icon() {
    let mut h = harness();
    ask(&mut h);
    h.get_by_role_and_label(Role::Dialog, "PhotoCraft");
    let title = h.get_by_role_and_label(Role::Heading, "PhotoCraft").rect();
    let message = h.get_by_label(MESSAGE).rect();
    assert!(h.query_by_label("Reset All Tools").is_none(), "the menu entry's name is no longer the title");
    // The title starts at the content's start; the icon sits before the message on its row.
    assert!(message.left() > title.left() + 8.0, "the message starts after the icon: {message:?} vs title {title:?}");
}

#[test]
fn enter_resets_every_tool_and_escape_resets_none() {
    let mut h = harness();
    ask(&mut h);
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(!h.state().dialog_open(), "Escape closes the question");
    assert_eq!(h.state().ui.tool_options.feather, 12.0, "Escape resets nothing");

    ask(&mut h);
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert!(!h.state().dialog_open(), "Enter answers the question");
    assert_eq!(h.state().ui.tool_options, ToolOptions::default(), "Enter is OK: every tool is reset");
}
