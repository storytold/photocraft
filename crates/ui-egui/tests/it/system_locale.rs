//! Use the actual platform UI-language API and the production (non-cfg(test)) Auto path.
#![cfg(not(target_arch = "wasm32"))]

use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, i18n, menu_keys};

#[test]
fn native_first_launch_follows_os_ui_preferences() {
    let tags: Vec<String> = match std::env::var("PHOTOCRAFT_LOCALE").ok().filter(|tag| !tag.trim().is_empty()) {
        Some(tag) => vec![tag],
        None => sys_locale::get_locales().take(64).collect(),
    };
    let expected = tags
        .iter()
        .filter(|tag| tag.len() <= 128 && tag.trim().bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'@')))
        .find_map(|tag| i18n::lang_from_tag(tag.trim()))
        .unwrap_or(i18n::Lang::EN);
    eprintln!("native UI-language preferences: {tags:?}; resolved: {}", expected.code());
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    h.run_steps(4);
    assert_eq!(h.state().session.prefs().interface.language, "auto");
    assert_eq!(i18n::current(), expected, "native language preferences: {tags:?}");
    // Non-Latin menus include their access key, e.g. "Файл(F)"; the locale check
    // must look for the rendered title rather than the translation alone.
    let (titles, _) = menu_keys::localized(&[i18n::tr(expected, "File").to_owned()], &["File"], &[Some('F')]);
    let title = titles.first().expect("one menu title");
    assert!(h.query_by_label(title).is_some(), "the first-launch menu must use the OS UI language");
}
