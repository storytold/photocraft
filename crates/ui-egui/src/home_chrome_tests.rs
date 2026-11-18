//! The Home screen is only the menu bar and the Home content: the tool strip, options bar, status
//! bar and docks are not drawn under it, and come back when Home is dismissed. The saved panel
//! preferences are never touched by showing Home.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::AppTheme;

/// The real app (every panel on) in `theme`, with a document open when `document`.
fn app(theme: AppTheme, document: bool, auto_show: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    if document {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    }
    s.edit_prefs(|p| p.general.auto_show_home_screen = auto_show);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.theme = theme;
        app
    });
    h.run_steps(6);
    h
}

fn chrome_shown(h: &Harness<'static, Box<PhotocraftApp>>, when: &str) -> bool {
    // Tool buttons are named "Move Tool  (V)" (the shortcut follows), the dock's footer button "Create a new layer".
    let toolbar = h.query_all_by_label_contains("Move Tool").next().is_some();
    let dock = h.query_all_by_label_contains("Create a new layer").next().is_some();
    assert_eq!(toolbar, dock, "{when}: the tool strip and the dock come and go together (toolbar {toolbar}, dock {dock})");
    toolbar
}

/// Whether the Home page is up: its drop hint and its Pinned Folders heading.
fn home_content(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.query_all_by_label_contains("Drop an image or PSD anywhere").next().is_some() || h.query_all_by_label("Pinned Folders").next().is_some()
}

/// The options bar and the status bar: a widget of each ("Auto-Select:" of the Move tool's options,
/// the status bar's zoom field).
fn bars_shown(h: &Harness<'static, Box<PhotocraftApp>>) -> (bool, bool) {
    (h.query_all_by_label_contains("Auto-Select").next().is_some(), h.query_all_by_label("Zoom").next().is_some())
}

/// Click the Home entry: the options bar's button with a document in front, the rail's on Home.
fn click_home(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let top = h.query_all_by_label("Home").min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).expect("a Home button");
    top.click();
    h.run_steps(6);
}

#[test]
fn no_documents_with_auto_show_draws_only_the_home_content() {
    let h = app(AppTheme::pro(), false, true);
    assert!(home_content(&h), "Home content");
    assert!(!chrome_shown(&h, "no documents, auto-show"), "no tool strip or dock under Home");
    assert!(h.query_by_label("No document").is_none(), "no status bar");
    assert_eq!(h.query_all_by_label("Home").count(), 1, "only the rail's entry: nothing to go back to");
}

#[test]
fn tab_in_full_screen_does_not_bring_the_chrome_back_over_home() {
    let mut h = app(AppTheme::pro(), true, true);
    click_home(&mut h);
    h.state_mut().ui.view.screen_mode = craft::ui::shell::ScreenMode::FullScreen;
    h.run_steps(2);
    // Tab reveals the chrome in full screen; under Home the shell still draws none of it.
    h.key_press(egui::Key::Tab);
    h.run_steps(6);
    assert!(home_content(&h), "Home content");
    assert!(!chrome_shown(&h, "Tab-revealed chrome on Home"), "no tool strip or dock");
    assert_eq!(bars_shown(&h), (false, false), "no options bar or status bar");
}
