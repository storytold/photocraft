//! The Appearance and Language menus in the whole app: a pick changes the theme the framework draws with and the
//! language, whether it comes from a click on the in-window bar or from the macOS menu, which fires the row's command.
//! Both menus work with or without a document open.

use craft::keymap::CommandRef;
use craft::menu::{LANGUAGE_COMMAND, ThemeChoice};
use craft::ui::runtime::commands::{FireSource, fire};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{AccessKitNode, Queryable};

use crate::PhotocraftApp;
use crate::theme::AppTheme;

/// The whole app, empty or with one document open.
fn harness(document: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut session = photocraft_engine::Session::new();
    if document {
        session.execute("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
    }
    crate::shortcut_dispatch::tests::app_harness(PhotocraftApp::new(session, crate::Services::default()))
}

/// The theme the framework's root takes each frame (`frame_input`) and the panel fill the theme installs.
fn shown(h: &Harness<'_, Box<PhotocraftApp>>) -> (String, bool, egui::Color32) {
    let theme = &h.state().ui.theme;
    (theme.id().as_str().to_string(), theme.is_dark(), h.ctx.global_style().visuals.panel_fill)
}

/// Classic in light mode: a light theme of its own row, unlike the dark theme the app starts with.
fn classic() -> (AppTheme, ThemeChoice) {
    let theme = AppTheme::classic();
    let choice = ThemeChoice::theme(theme.id().clone(), false);
    (theme, choice)
}

/// Runs the frames that take a fired command and show its result.
fn settle(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    h.run_steps(6);
}

/// The centre of the top-most menu title or row labelled `label`.
fn top(h: &Harness<'_, Box<PhotocraftApp>>, label: &str) -> egui::Pos2 {
    let named = |node: &AccessKitNode<'_>| {
        node.label().as_deref() == Some(label) && matches!(node.role(), Role::MenuItem | Role::MenuItemRadio | Role::MenuItemCheckBox)
    };
    let node = h.get_all_by(named).min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).unwrap_or_else(|| panic!("no menu row {label}"));
    node.rect().center()
}

/// Moves the pointer over the top-most node labelled `label`.
fn hover_top(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    let at = top(h, label);
    h.event(egui::Event::PointerMoved(at));
    settle(h);
}

/// Clicks the top-most node labelled `label` with the pointer, a frame for the press and one for the release.
fn click_top(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    let at = top(h, label);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }
    settle(h);
}

/// Asserts the app shows Classic light after a pick, with the panel fill it installs.
fn assert_classic(h: &Harness<'_, Box<PhotocraftApp>>, before: &(String, bool, egui::Color32), document: bool) {
    let (theme, _) = classic();
    let after = shown(h);
    assert_eq!((after.0.as_str(), after.1), (theme.id().as_str(), false), "document {document}; status: {}", h.state().ui.status);
    assert_ne!(after.2, before.2, "document {document}: the panels take the theme's fill");
    assert_eq!(after.2, theme.tokens().chrome, "document {document}");
}

#[test]
fn a_theme_fired_from_the_menu_shows_it() {
    for document in [true, false] {
        let mut h = harness(document);
        let (theme, choice) = classic();
        let before = shown(&h);
        assert_ne!(before.0, theme.id().as_str(), "the app starts on another theme");
        assert!(fire(&h.ctx.clone(), choice.command(), FireSource::Menu), "the runtime takes the menu's theme row");
        settle(&mut h);
        assert_classic(&h, &before, document);
    }
}

#[test]
fn a_theme_clicked_in_the_window_menu_shows_it() {
    for document in [true, false] {
        let mut h = harness(document);
        let (theme, _) = classic();
        let before = shown(&h);
        click_top(&mut h, "Window");
        hover_top(&mut h, "Appearance");
        let name = h.state().framework.theme_name(theme.id(), false);
        click_top(&mut h, &name);
        assert_classic(&h, &before, document);
    }
}

#[test]
fn a_language_fired_from_the_menu_is_saved() {
    // A language other than English where the build has the catalogs; English (from Automatic) where it has only that.
    let code = crate::i18n::Lang::all().map(crate::i18n::Lang::code).find(|code| *code != "en").unwrap_or("en");
    for document in [true, false] {
        let mut h = harness(document);
        assert_eq!(h.state().session.prefs().interface.language, "auto");
        let row = CommandRef::with_params(LANGUAGE_COMMAND, serde_json::json!({"lang": code}));
        assert!(fire(&h.ctx.clone(), row, FireSource::Menu), "the runtime takes the menu's language row");
        settle(&mut h);
        assert_eq!(h.state().session.prefs().interface.language, code, "document {document}; status: {}", h.state().ui.status);
    }
}
