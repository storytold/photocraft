//! The menu bar from the keyboard (#4988): Alt / F10 focus the bar, Alt+letter opens a menu,
//! letters run rows, and the shortcuts wait while the bar has the keyboard.
use crate::PhotocraftApp;
use crate::menu_keys::Keys;
use crate::menu_nav::{Nav, is_open};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

/// The menu bar alone, as `menu_nav_tests` drives it.
fn bar() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

fn key(h: &mut Harness<'static, PhotocraftApp>, k: egui::Key) {
    h.key_press(k);
    h.run_steps(3);
}

fn alt_key(h: &mut Harness<'static, PhotocraftApp>, k: egui::Key) {
    h.event_modifiers(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT }, egui::Modifiers::ALT);
    h.run_steps(3);
}

/// A tap on Alt: down, then up with nothing else pressed between.
fn alt_tap(h: &mut Harness<'static, PhotocraftApp>) {
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
}

fn rows(h: &Harness<'static, PhotocraftApp>, level: usize) -> Vec<crate::menu_nav::Row> {
    Nav::current(&h.ctx).rows.get(level).cloned().unwrap_or_default()
}

#[test]
fn alt_focuses_the_bar_arrows_move_and_down_opens_the_lit_menu() {
    let mut h = bar();
    assert_eq!(Keys::current(&h.ctx).focus, None);
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "Alt puts the keyboard on File");
    assert!(crate::menu_keys::is_focused(&h.ctx), "the shortcuts wait for the bar");
    assert!(!is_open(&h.ctx), "no menu opens yet");
    key(&mut h, egui::Key::ArrowRight);
    assert_eq!(Keys::current(&h.ctx).focus, Some(1), "→ moves to Edit");
    key(&mut h, egui::Key::ArrowLeft);
    key(&mut h, egui::Key::ArrowLeft);
    assert_eq!(Keys::current(&h.ctx).focus, Some(9), "← wraps round to Help");
    key(&mut h, egui::Key::ArrowRight);
    key(&mut h, egui::Key::ArrowRight);
    assert_eq!(Keys::current(&h.ctx).focus, Some(1));
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(2);
    assert!(is_open(&h.ctx), "↓ opens the lit menu");
    assert!(h.query_by_label_contains("Undo").is_some(), "Edit is open");
    assert_eq!(Keys::current(&h.ctx).focus, None, "the focus moves into the menu");
    let first = rows(&h, 0).iter().position(|r| r.enabled).unwrap();
    assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(first), "its first row is lit, as on Windows");
    assert!(Keys::current(&h.ctx).underline, "a menu opened from the keyboard shows its mnemonics");
    key(&mut h, egui::Key::Escape);
    assert!(!is_open(&h.ctx) && Keys::current(&h.ctx).focus.is_none(), "Esc leaves the menu and the bar");
}

/// Alt+wheel zooms; letting go of Alt after it is no tap, so Space (the hand tool) can't then
/// open File and run New….
#[test]
fn alt_with_a_wheel_scroll_or_a_lost_focus_is_no_tap() {
    let mut h = bar();
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.event_modifiers(
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::ALT,
        },
        egui::Modifiers::ALT,
    );
    h.run_steps(2);
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "Alt+scroll leaves the bar alone");
    key(&mut h, egui::Key::Space);
    assert!(!is_open(&h.ctx), "Space opens no menu");
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.event(egui::Event::WindowFocused(false));
    h.event(egui::Event::WindowFocused(true));
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "Alt that went to the desktop leaves the bar alone");
}

#[test]
fn a_second_alt_or_esc_or_a_click_takes_the_keyboard_off_the_bar() {
    let mut h = bar();
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0));
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, None, "a second tap leaves");
    key(&mut h, egui::Key::F10);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "F10 focuses the bar too");
    key(&mut h, egui::Key::Escape);
    assert_eq!(Keys::current(&h.ctx).focus, None);
    assert!(!crate::menu_keys::is_focused(&h.ctx));
    key(&mut h, egui::Key::F10);
    let at = egui::pos2(600.0, 300.0);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "a click anywhere leaves");
}

#[test]
fn alt_with_a_letter_opens_that_menu_and_enter_opens_the_lit_one() {
    let mut h = bar();
    alt_key(&mut h, egui::Key::E);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Undo").is_some(), "Alt+E opens Edit");
    alt_key(&mut h, egui::Key::F);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_some(), "Alt+F switches to File while a menu is open");
    key(&mut h, egui::Key::Escape);
    assert!(!is_open(&h.ctx));
    key(&mut h, egui::Key::F10);
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_some(), "Enter opens the lit File menu");
    key(&mut h, egui::Key::Escape);
    key(&mut h, egui::Key::F10);
    key(&mut h, egui::Key::V);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Zoom In").is_some(), "a letter on the focused bar opens that menu (V = View)");
}

#[test]
fn a_letter_in_an_open_menu_runs_the_row_with_that_mnemonic_or_opens_its_submenu() {
    let mut h = bar();
    alt_key(&mut h, egui::Key::V);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Zoom In").is_some(), "View is open");
    let level = rows(&h, 0);
    let zoom_in = level.iter().find(|r| r.command.as_deref() == Some("view.zoomIn")).expect("Zoom In row");
    let letter = zoom_in.mnemonic.expect("Zoom In has a mnemonic");
    assert!(level.iter().filter(|r| r.mnemonic == Some(letter)).count() == 1, "mnemonics are unique within a level");
    let zoom = h.state().ui.views[0].zoom;
    key(&mut h, egui::Key::from_name(&letter.to_string()).unwrap());
    h.run_steps(3);
    assert!(h.state().ui.views[0].zoom > zoom, "the letter ran Zoom In ({letter})");
    assert!(!is_open(&h.ctx), "and closed the menu");
    // A submenu's letter opens it, with its first row lit.
    alt_key(&mut h, egui::Key::V);
    h.run_steps(2);
    let level = rows(&h, 0);
    let sub = level.iter().find(|r| r.command.is_none() && r.enabled && r.mnemonic.is_some()).expect("an enabled submenu with a mnemonic");
    let letter = sub.mnemonic.unwrap();
    key(&mut h, egui::Key::from_name(&letter.to_string()).unwrap());
    h.run_steps(3);
    assert!(!rows(&h, 1).is_empty(), "the submenu opened ({letter})");
    assert_eq!(Nav::current(&h.ctx).level, 1);
}

#[test]
fn mnemonics_are_underlined_only_with_alt_or_a_keyboard_opened_menu() {
    let mut h = bar();
    h.get_by_label("File").click();
    h.run_steps(4);
    assert!(is_open(&h.ctx));
    assert!(!Nav::current(&h.ctx).underline, "a menu opened with the mouse shows no underlines");
    h.key_down(egui::Key::AltLeft);
    h.run_steps(3);
    assert!(Nav::current(&h.ctx).underline, "while Alt is held they show");
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    // The tap closed the menu (as on Windows); opening from the keyboard underlines.
    assert!(!is_open(&h.ctx), "an Alt tap closes an open menu");
    alt_key(&mut h, egui::Key::F);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && Nav::current(&h.ctx).underline);
    assert!(rows(&h, 0).iter().any(|r| r.mnemonic.is_some()), "rows carry their letters");
}

#[test]
fn underlined_text_marks_the_mnemonic_once_and_keeps_the_prefix() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let t = crate::menu_keys::mnemonic_text(ui, "✔ ", "Save As…", Some('A'), None);
        let egui::WidgetText::LayoutJob(job) = t else { panic!("a mnemonic makes a layout job") };
        assert_eq!(job.text, "✔ Save As…");
        let underlined: Vec<&str> =
            job.sections.iter().filter(|s| s.format.underline != egui::Stroke::NONE).map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect();
        assert_eq!(underlined, ["a"], "the first occurrence, case-insensitively");
        let t = crate::menu_keys::mnemonic_text(ui, "", "Edit", None, None);
        assert!(matches!(t, egui::WidgetText::RichText(_)), "without a mnemonic, plain text");
        let t = crate::menu_keys::mnemonic_text(ui, "", "Édition", Some('D'), None);
        let egui::WidgetText::LayoutJob(job) = t else { panic!() };
        assert_eq!(job.sections.len(), 3, "multi-byte text before the letter is kept whole");
    });
    out.textures_delta.clear();
}

/// The whole app: with the bar focused a letter opens a menu instead of picking a tool, and the
/// arrows don't nudge the layer.
#[test]
fn the_shortcuts_wait_while_the_keyboard_is_on_the_bar() {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app();
        app.services.native_menu = None;
        app
    });
    h.run_steps(4);
    let tool = h.state().ui.tool;
    key(&mut h, egui::Key::F10);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "F10 focuses the bar");
    key(&mut h, egui::Key::E);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, tool, "E did not pick the Eraser");
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Undo").is_some(), "E opened Edit");
    key(&mut h, egui::Key::Escape);
    h.run_steps(2);
    assert!(!is_open(&h.ctx));
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert_ne!(h.state().ui.tool, tool, "off the bar, E picks the Eraser again");
}
