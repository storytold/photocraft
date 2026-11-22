//! #14240: the document tab marks the mode and depth with "*" ("RGB/8*") while the document's profile differs in
//! colour from the working space, also while the Assign Profile dialog previews such a profile; the mark goes away
//! when the working space is assigned again or the preview is cancelled. Untagged and 32-bit documents keep the tab as
//! it was.

use serde_json::json;

use super::tabs::tab_title_at;
use crate::PhotocraftApp;

/// A document tagged with the working RGB space (sRGB by default) at `depth` bits.
fn app(depth: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "name": "mark", "depth": depth})).unwrap();
    app.run("edit.assignProfile", json!({"profile": "working"})).unwrap();
    app
}

fn title(app: &PhotocraftApp) -> String {
    tab_title_at(app, 0, 100.0)
}

fn assign(app: &mut PhotocraftApp, profile: &str) {
    app.run("edit.assignProfile", json!({"profile": profile})).unwrap();
}

#[test]
fn another_profile_marks_the_mode_and_the_working_space_clears_it() {
    let mut app = app(8);
    assert!(title(&app).ends_with("(RGB/8)"), "{}", title(&app));
    assign(&mut app, "adobe-rgb-compat");
    assert!(title(&app).ends_with("(RGB/8*)"), "{}", title(&app));
    assign(&mut app, "working");
    assert!(title(&app).ends_with("(RGB/8)"), "{}", title(&app));
}

#[test]
fn the_same_colours_under_other_bytes_are_not_marked() {
    let mut app = app(16);
    // The built-in sRGB is the default working space: the same colours, whatever the bytes.
    assign(&mut app, "srgb");
    assert!(title(&app).ends_with("(RGB/16)"), "{}", title(&app));
}

#[test]
fn untagged_and_32_bit_documents_keep_the_tab_as_it_was() {
    let mut app = app(8);
    assign(&mut app, "none");
    assert!(title(&app).ends_with("(RGB/8)"), "untagged: {}", title(&app));
    let mut app = self::app(32);
    assign(&mut app, "adobe-rgb-compat");
    assert!(!title(&app).contains('*'), "32-bit: {}", title(&app));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_assign_profile_preview_shows_the_mark_and_cancel_takes_it_away() {
    let mut app = app(8);
    let ctx = app.ctx().clone();
    crate::filter_dialog::open(&mut app, "edit.assignProfile").unwrap();
    crate::dialogs::testing::show(&mut app);
    let id = app.framework.dialogs.stack().top().unwrap();
    app.framework.dialogs.set_json_field(&ctx, id, "profile", json!("adobe-rgb-compat")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !app.filter_preview.as_ref().is_some_and(|p| p.result.is_some() && p.key.params["profile"] == json!("adobe-rgb-compat")) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the preview");
        super::filter_preview::ensure_filter_preview(&mut app, 0, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(title(&app).ends_with("(RGB/8*)"), "previewing: {}", title(&app));
    assert!(
        app.session.active().unwrap().doc.icc_profile.as_ref().is_some_and(|t| !app.session.color.differs_from_working(photocraft_color::ColorMode::Rgb, t)),
        "the document itself is unchanged"
    );
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(title(&app).ends_with("(RGB/8)"), "after Cancel: {}", title(&app));
}
