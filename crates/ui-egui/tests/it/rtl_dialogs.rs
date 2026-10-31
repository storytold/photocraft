//! Dialogs mirror through the real language path: the shell syncs the drawing language from the
//! preference, and the dialog host follows its direction with no test-side flag.

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, filter_dialog, i18n};
use serde_json::json;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app
    });
    h.run_steps(2);
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "ar"})).expect("Arabic is registered");
    filter_dialog::open(h.state_mut(), "filter.blur.gaussianBlur").expect("the filter dialog opens");
    h.run_steps(4);
    h
}

/// The Arabic text for `s` as a dialog shows it: the catalog carries the "…", the title trims it.
fn ar(s: &str) -> String {
    let lang = i18n::Lang::from_code("ar").expect("registered language");
    i18n::tr(lang, s).trim_end_matches('…').to_string()
}

fn window(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let id = h.state().ui.dialogs.last().expect("the dialog is open").id;
    h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog window")
}

/// In an Arabic UI the dialog mirrors — title, close box and footer swap ends — and the same
/// open dialog returns to the English layout when the language switches back.
#[test]
fn an_arabic_dialog_mirrors_and_follows_a_live_language_switch() {
    let mut h = harness();
    let title = h.get_by_label(&ar("Gaussian Blur…")).rect();
    let x = h.get_by_label(&ar("Close")).rect();
    let ok = h.get_by_label(&ar("OK")).rect();
    let cancel = h.get_by_label(&ar("Cancel")).rect();
    let win = window(&h);
    assert!(title.right() > win.center().x, "the title reads from the right: {title:?} in {win:?}");
    assert!(x.right() < title.left(), "the × sits left of the title: {x:?}, {title:?}");
    assert!(x.left() - win.left() < 40.0, "at the band's left end: {x:?} in {win:?}");
    assert!(ok.right() > cancel.right(), "mirrored footer order: {ok:?} vs {cancel:?}");
    assert!(cancel.left() - win.left() < 40.0, "the footer group hugs the reading end: {cancel:?} in {win:?}");

    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "en"})).expect("English is registered");
    h.run_steps(4);
    let title = h.get_by_label("Gaussian Blur").rect();
    let x = h.get_by_label("Close").rect();
    let ok = h.get_by_label("OK").rect();
    let cancel = h.get_by_label("Cancel").rect();
    let win = window(&h);
    assert!(title.left() < win.center().x, "English keeps the title at the left: {title:?} in {win:?}");
    assert!(x.left() > title.right(), "the × stays right of the title: {x:?}, {title:?}");
    assert!(ok.right() < cancel.right(), "English footer order: {ok:?} vs {cancel:?}");
    assert!(win.right() - cancel.right() < 40.0, "the footer group hugs the right edge: {cancel:?} in {win:?}");
}
