//! Right-to-left shell layout: which languages read right to left, and where the mirrored dock
//! lands in a running shell when such a language is chosen.

use super::*;
use crate::PhotocraftApp;
use crate::i18n::{self, Lang};
use egui_kittest::Harness;
use photocraft_text::scripts;
use serde_json::json;

#[test]
fn right_to_left_scripts_by_language_code() {
    // Arabic, Hebrew and Urdu are the right-to-left catalogs; every other registered language stays
    // left to right.
    for lang in Lang::all() {
        let want = if matches!(lang.code(), "ar" | "he" | "ur") { Dir::Rtl } else { Dir::Ltr };
        assert_eq!(lang.dir(), want, "{}", lang.code());
    }
}

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    harness_with(Harness::builder())
}

fn harness_with(builder: egui_kittest::HarnessBuilder<Box<PhotocraftApp>>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = builder.with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro_medium());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// The middle of the Layers group across the window.
fn dock_x(h: &Harness<'static, Box<PhotocraftApp>>) -> f32 {
    crate::panel_docking::probe::group_rect(h, "layers").expect("Layers docked").center().x
}

/// Choosing a right-to-left catalog (not just forcing the direction) mirrors the shell, and the
/// shell's fonts draw its letters; English mirrors back. Panel interiors and dialogs stay
/// left-to-right for now (module docs), so only the dock side is checked.
#[test]
fn hebrew_and_urdu_mirror_the_shell_and_draw_their_letters() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let _lang = i18n::language_scope(Lang::EN);
    for (code, letter) in [("he", 'ש'), ("ur", 'ر')] {
        let lang = Lang::from_code(code).expect("registered language");
        // Builds with craft-fonts (CI: `CRAFT_FONTS_REQUIRED`) draw these letters from the first
        // frame and check them. Without it the host's fonts decide, so the harness allows tofu
        // and only the mirroring is checked.
        let fonts = craft_font_covers(letter);
        if !fonts {
            eprintln!("{code}: no craft-fonts face for {letter}; glyphs not checked (build with CRAFT_FONTS_DIR)");
        }
        let mut h = if fonts { harness() } else { harness_with(Harness::builder().allow_missing_glyphs()) };
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("choose language");
        h.run_steps(4);
        assert_eq!(i18n::current(), lang);
        assert!(is_rtl(), "{code} draws right-to-left");
        assert!(dock_x(&h) < 600.0, "{code}: the dock moves to the left");
        for label in ["File", "Help", "Layers"] {
            let text = i18n::tr(lang, label);
            assert!(!text.is_ascii(), "{code}: {label} is translated");
            assert!(!fonts || h.ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(12.0), text)), "{code}: {text} has glyphs");
        }
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "en"})).expect("back to English");
        h.run_steps(4);
        assert!(!is_rtl() && dock_x(&h) > 600.0, "{code} -> en: the shell mirrors back");
    }
}

/// Whether craft-fonts has a face for `letter`. Only those are served in the first frame (the
/// eager provider); installed fonts arrive a frame later through the lazy loader, which the
/// harness's strict missing-glyph policy doesn't wait for (`cjk_fonts` tests).
fn craft_font_covers(letter: char) -> bool {
    photocraft_text::craft_fonts::CRAFT_FONTS.iter().any(|f| !f.bytes.is_empty() && scripts::covering_face(f.bytes, letter).is_some())
}
