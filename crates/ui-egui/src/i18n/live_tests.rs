//! Behavioural checks for switching languages in a running shell.

use super::*;
use crate::prefs_ui::{apply_window, open_preferences, set_draft, window_open};
use crate::{PhotocraftApp, Services};
use craft::settings::testing::MemoryFiles;
use craft::settings::{Location, OpenOptions, SettingsStore};
use craft::ui::patterns::preferences::PrefValue;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// The folder of the scratch settings store; its file lives in memory.
const DIR: &str = "/memory/photocraft";

/// Services whose preferences store reads and writes `files`. A zero debounce makes the first frame after a
/// change write it.
fn services_on(files: &Arc<MemoryFiles>) -> Services {
    let options = OpenOptions::new(Location::Dir(DIR.into())).debounce(Duration::ZERO).files(files.clone());
    Services { settings: Some(SettingsStore::<photocraft_engine::prefs::SavedPrefs>::open_with(options)), ..Default::default() }
}

/// The saved `interface.language`.
fn saved_language(files: &MemoryFiles) -> Value {
    let text = files.text(&std::path::Path::new(DIR).join("config.toml")).expect("the shell saved preferences");
    toml::from_str::<Value>(&text).expect("saved TOML")["interface"]["language"].clone()
}

/// The File menu's title as the menu bar draws it in `lang`: the translation, plus the English access key where it
/// has no Latin letter of its own ("文件(F)", #5863). Read from the drawn bar, whose rule spans all its titles.
fn file_title(lang: Lang) -> String {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    with_language(lang, || {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 80.0))
            .with_context(app.ctx().clone())
            .build_ui_state(|ui, app: &mut PhotocraftApp| crate::menus::menu_bar(app, ui), app);
        h.run_steps(2);
        h.query_all_by_role(Role::MenuItem).next().and_then(|node| node.accesskit_node().label()).expect("the bar's first title")
    })
}

fn lang(code: &str) -> Lang {
    Lang::from_code(code).expect("registered language")
}

/// Regional locale tags (`fr-CA`, `ko-KR`, `bs-BA` …) resolve to their registered catalog, as
/// a preference and as a system tag.
#[test]
fn requested_languages_resolve_regional_preferences() {
    if !needs_catalogs() {
        return;
    }
    for (tag, code) in [
        ("en-US", "en"),
        ("zh-CN", "zh-hans"),
        ("ja-JP", "ja"),
        ("ko-KR", "ko"),
        ("ru-RU", "ru"),
        ("uk-UA", "uk"),
        ("fr-CA", "fr"),
        ("id-ID", "id"),
        ("vi-VN", "vi"),
        ("sr-RS", "sr-latn"),
        ("az-Latn-AZ", "az"),
        ("bs-BA", "bs"),
    ] {
        assert_eq!(Lang::from_pref(tag), lang(code), "{tag}");
    }
    assert_eq!(lang_from_tag("ko_KR.UTF-8"), Some(lang("ko")));
    assert_eq!(lang_from_tag("FR-fr"), Some(lang("fr")));
    assert_eq!(lang_from_tag("ko"), Some(lang("ko")));
}

/// Switching redraws the menu and persists through the settings store on restart.
#[test]
fn azerbaijani_draw_after_switching_and_restore_after_restart() {
    if !needs_catalogs() {
        return;
    }
    let code = "az";
    let file = "Fayl";
    with_language(Lang::EN, || {
        let files = Arc::new(MemoryFiles::new());
        let mut h = harness_with_services(services_on(&files));
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("choose language");
        h.run_steps(4);
        assert_eq!(current(), lang(code));
        assert_eq!(tr(current(), "File"), file);
        assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang(code))));
        assert_eq!(saved_language(&files), code);
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "en"})).expect("switch back");
        h.run_steps(4);
        assert_eq!(current(), Lang::EN);
        assert_eq!(saved_language(&files), "en");
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("choose language again");
        h.run_steps(4);
        assert_eq!(saved_language(&files), code);
        drop(h);
        // The restarted shell, not the old one's drawing language, must bring Azerbaijani back.
        set_current(Lang::EN);
        let restarted = harness_with_services(services_on(&files));
        assert_eq!(restarted.state().session.prefs().interface.language, code);
        assert_eq!(current(), lang(code));
        assert!(drawn_text(&restarted).iter().any(|text| *text == file_title(lang(code))));
    });
}

#[test]
fn scoped_preview_restores_language_even_after_a_panic() {
    if !needs_catalogs() {
        return;
    }
    with_language(Lang::EN, || {
        with_language(lang("fr"), || {
            assert_eq!(current(), lang("fr"));
            assert_eq!(t("Layer"), tr(lang("fr"), "Layer"));
            with_language(lang("ko"), || assert_eq!(current(), lang("ko")));
            assert_eq!(current(), lang("fr"));
        });
        let result = std::panic::catch_unwind(|| with_language(lang("ja"), || panic!("test unwind")));
        assert!(result.is_err());
        assert_eq!(current(), Lang::EN);
    });
}

#[test]
fn drawing_languages_are_isolated_between_threads() {
    if !needs_catalogs() {
        return;
    }
    with_language(lang("fr"), || {
        let thread = std::thread::spawn(|| {
            assert_eq!(current(), Lang::EN);
            with_language(lang("ko"), || assert_eq!(t("Layer"), tr(lang("ko"), "Layer")));
        });
        thread.join().expect("language thread");
        assert_eq!(current(), lang("fr"));
    });
}

#[test]
fn cjk_font_order_follows_the_selected_language() {
    if !needs_catalogs() {
        return;
    }
    for (code, han) in [("zh-hans", "Simplified"), ("ja", "Japanese"), ("ko", "Korean")] {
        with_language(lang(code), || {
            // The font stack takes the Han preference from the UI language, whatever the OS locale.
            let preferred = craft::ui::fonts::han_for_locale(crate::i18n::current().code(), crate::cjk_fonts::os_locale());
            assert_eq!(format!("{preferred:?}"), format!("Some({han})"), "{code}");
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn korean_glyphs_remain_available_after_repeated_language_switches() {
    if !needs_catalogs() {
        return;
    }
    use photocraft_text::cjk::{self, CjkChar, CjkScript};
    #[cfg(target_os = "linux")]
    assert!(crate::cjk_fonts::wait_for_fontconfig_cjk_scan(), "Fontconfig CJK lookup did not finish within 10 seconds");
    if !cjk::font_files(CjkScript::Korean).iter().any(|font| font.path.is_file()) {
        eprintln!("skipping: no installed Korean font");
        return;
    }
    with_language(Lang::EN, || {
        let mut h = harness();
        let glyphs: std::collections::BTreeSet<_> =
            lang("ko").catalog_texts().flat_map(str::chars).filter(|c| cjk::classify(*c) == Some(CjkChar::Hangul)).collect();
        for code in ["zh-hans", "ja", "ko", "fr", "ko"] {
            h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("language");
            h.run_steps(12);
            if code == "ko" {
                h.ctx.fonts_mut(|fonts| {
                    for family in [egui::FontFamily::Proportional, egui::FontFamily::Name("medium".into()), egui::FontFamily::Name("semibold".into())] {
                        let font = egui::FontId::new(12.0, family);
                        for c in &glyphs {
                            assert!(fonts.has_glyph(&font, *c), "missing Korean glyph {c} after a hot language change");
                        }
                    }
                });
            }
        }
    });
}

fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    harness_with_services(Services::default())
}

fn harness_with_services(services: Services) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"name": "User layer name", "width": 64, "height": 64})).expect("new document");
        app
    });
    harness.run_steps(4);
    harness
}

/// Auto draws the detected system language on first launch while keeping the Auto preference; no or
/// unsupported system languages draw English.
#[test]
fn first_launch_renders_the_system_language_without_overwriting_auto() {
    if !needs_catalogs() {
        return;
    }
    let cases: [(&[&str], &str); 14] = [
        (&["en-US"], "en"),
        (&["zh-CN"], "zh-hans"),
        (&["ja-JP"], "ja"),
        (&["ko-KR"], "ko"),
        (&["ru-RU"], "ru"),
        (&["uk-UA"], "uk"),
        (&["fr-CA"], "fr"),
        (&["sv-SE"], "sv"),
        (&["fi-FI"], "fi"),
        (&["da-DK"], "da"),
        (&["nb-NO"], "nb"),
        (&["az-AZ"], "az"),
        (&[], "en"),
        (&["sw-KE", "is-IS"], "en"),
    ];
    for (tags, code) in cases {
        system::with_system_tags(tags, || {
            with_language(Lang::EN, || {
                let h = harness();
                assert_eq!(h.state().session.prefs().interface.language, "auto");
                assert_eq!(current(), lang(code), "{tags:?}");
                assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang(code))), "first-launch menu: {tags:?}");
            });
        });
    }
}

#[test]
fn an_explicit_language_survives_restart_on_a_different_system_language() {
    if !needs_catalogs() {
        return;
    }
    let files = Arc::new(MemoryFiles::new());
    system::with_system_tags(&["ja-JP"], || {
        with_language(Lang::EN, || {
            let mut h = harness_with_services(services_on(&files));
            assert_eq!(current(), lang("ja"));
            h.state_mut().run("prefs.set", json!({"path":"interface.language","value":"ko"})).expect("choose language");
            h.step();
            assert_eq!(current(), lang("ko"));
            h.step();
            assert_eq!(saved_language(&files), "ko");
        })
    });
    system::with_system_tags(&["fr-FR"], || {
        with_language(Lang::EN, || {
            let h = harness_with_services(services_on(&files));
            assert_eq!(h.state().session.prefs().interface.language, "ko");
            assert_eq!(current(), lang("ko"));
            assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang("ko"))));
        });
    });
}

#[test]
fn a_control_request_changes_the_drawing_language_in_the_same_frame() {
    if !needs_catalogs() {
        return;
    }
    with_language(Lang::EN, || {
        let mut h = harness();
        let document = h.state().session.active().expect("document").doc.clone();
        for code in ["zh-hans", "ja", "ko", "ru", "uk", "fr", "id", "en"] {
            let (tx, rx) = std::sync::mpsc::channel();
            h.state_mut().control = Some(craft::automation::ControlChannel::new(rx));
            let (request, reply) =
                crate::control::ControlRequest::new("engine.execute", json!({"command": "prefs.set", "params": {"path": "interface.language", "value": code}}));
            tx.send(request).expect("request");
            h.step();
            assert_eq!(reply.try_recv().expect("response")["ok"], true);
            assert_eq!(current(), lang(code));
            assert!(drawn_text(&h).iter().any(|text| *text == file_title(lang(code))), "the menu must change in this frame: {code}");
            assert!(std::sync::Arc::ptr_eq(&document, &h.state().session.active().expect("document").doc), "switching language must preserve the document");
        }
        assert!(h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": []})).is_err());
        h.step();
        assert_eq!(current(), Lang::EN);
    });
}

/// The draft language previews inside the Preferences window only; Apply commits it.
#[test]
fn preferences_preview_does_not_commit_until_applied_and_survives_reload() {
    if !needs_catalogs() {
        return;
    }
    use egui_kittest::kittest::Queryable;
    with_language(Lang::EN, || {
        let mut h = harness();
        open_preferences(h.state_mut(), "interface");
        assert!(set_draft(h.state_mut(), "interface.language", PrefValue::Choice("fr".into())), "the window took the draft language");
        h.run_steps(3);
        let text = drawn_text(&h);
        let fr = lang("fr");
        assert!(text.iter().any(|text| text == tr(fr, "Appearance")), "the draft language must translate the visible window");
        assert!(text.iter().any(|text| text == tr(fr, "Cancel")), "the window buttons must preview the language too");
        assert!(text.iter().any(|text| text == "File"), "the main window keeps its committed language");
        assert_eq!(h.state().session.prefs().interface.language, "auto");
        assert_eq!(current(), Lang::EN, "preview must not leak to the main window");
        // Cancel drops the draft and the preview.
        h.get_by_label(tr(fr, "Cancel")).click();
        h.run_steps(3);
        assert!(!window_open(h.state()));
        assert_eq!(h.state().session.prefs().interface.language, "auto");
        assert_eq!(current(), Lang::EN);
        open_preferences(h.state_mut(), "interface");
        assert!(set_draft(h.state_mut(), "interface.language", PrefValue::Text("ko".into())));
        apply_window(h.state_mut()).expect("apply");
        h.step();
        assert_eq!(current(), lang("ko"));
        let saved = h.state().session.prefs_to_json();
        let mut restored = photocraft_engine::Session::new();
        restored.load_prefs_json(&saved).expect("reload preferences");
        assert_eq!(restored.prefs().interface.language, "ko");
    });
}

fn drawn_text(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}
