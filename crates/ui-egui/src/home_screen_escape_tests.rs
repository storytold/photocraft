use crate::PhotocraftApp;
use craft::host::target::{Backend, Os, Runtime, WindowKind};
use craft::window::{
    desktop::Desktop,
    policy::{TitleBarPref, decide},
};
use egui::Key;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

fn home(documents: &[&str]) -> AppHarness {
    let mut session = photocraft_engine::Session::new();
    session.edit_prefs(|prefs| prefs.general.auto_show_home_screen = true);
    for name in documents {
        session.execute("file.new", json!({"name": name, "width": 8, "height": 8})).unwrap();
    }
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |cc| {
        let desktop = Desktop::detect(Backend::X11);
        let frame = decide(Os::Windows, Runtime::Native, Backend::X11, &desktop, WindowKind::Main, TitleBarPref::Integrated, None);
        PhotocraftApp::setup_window(&cc.egui_ctx, Default::default(), frame);
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session, Default::default())
    });
    harness.run_steps(4);
    if !documents.is_empty() {
        harness.get_by_label("Home").click();
        harness.run_steps(4);
    }
    assert!(harness.state_mut().home_screen_up());
    harness
}

fn escape(harness: &mut AppHarness) {
    harness.key_press(Key::Escape);
    harness.run_steps(4);
}

#[test]
fn escape_returns_to_the_active_document_without_deselecting() {
    let mut harness = home(&["First", "Second"]);
    harness.state_mut().run("select.all", json!({})).unwrap();
    harness.get_by_label("Learn").focus();
    harness.run_steps(2);
    escape(&mut harness);
    assert!(!harness.state_mut().home_screen_up(), "Escape should leave Home");
    let document = harness.state().session.active().unwrap();
    assert_eq!(document.doc.name, "Second");
    assert!(document.doc.selection.is_some(), "Escape on Home must not deselect on the canvas");
}

#[test]
fn escape_stays_on_home_without_a_document() {
    let mut harness = home(&[]);
    escape(&mut harness);
    assert!(harness.state_mut().home_screen_up());
}

#[test]
fn escape_closes_the_dialog_before_leaving_home() {
    let mut harness = home(&["Keep"]);
    crate::new_doc_ui::open(harness.state_mut());
    harness.run_steps(4);
    assert!(harness.state().dialog_open());
    escape(&mut harness);
    assert!(!harness.state().dialog_open());
    assert!(harness.state_mut().home_screen_up());
    escape(&mut harness);
    assert!(!harness.state_mut().home_screen_up());
}

#[test]
fn escape_closes_the_menu_before_leaving_home() {
    let mut harness = home(&["Keep"]);
    assert!(harness.query_all_by_label_contains("Close All").next().is_none());
    harness.get_by_label("File").click();
    harness.run_steps(4);
    assert!(harness.query_all_by_label_contains("Close All").next().is_some());
    escape(&mut harness);
    assert!(harness.query_all_by_label_contains("Close All").next().is_none());
    assert!(harness.state_mut().home_screen_up());
    escape(&mut harness);
    assert!(!harness.state_mut().home_screen_up());
}
