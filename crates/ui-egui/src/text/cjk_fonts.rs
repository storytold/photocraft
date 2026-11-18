//! PhotoCraft's inputs to the framework's UI font stack (`craft::ui::fonts::UiFonts`).
//!
//! The stack orders the craft-fonts faces by UI language, and `OnDemandFonts` reads an installed
//! system font the first time a visible character of a script has no glyph. What stays here is
//! what only PhotoCraft knows: which craft-fonts faces this build embeds
//! ([`photocraft_text::craft_fonts`], empty without `CRAFT_FONTS_DIR`; the web build embeds none),
//! and the OS locale.
//!
//! PhotoCraft's baseline for shared Han and kana is the craft manifest's first Japanese face
//! (Shippori Mincho): [`craft_faces`] passes the manifest in order, and the stack keeps that order
//! while neither the UI language nor the OS locale names a Han script. An explicit ja/ko/zh
//! preference puts its own script's face first.

use std::sync::Arc;

use craft::ui::fonts::{EmbeddedFace, OnDemandFonts, SystemCandidates, han_for_locale};
use photocraft_text::craft_fonts::{self, CraftFont};

/// The craft-fonts faces `faces` holds, as the stack's embedded faces, in manifest order. A face
/// without bytes is left out. Each is installed as face 0, as a font collection's first face.
fn embedded(faces: &[CraftFont]) -> Vec<EmbeddedFace> {
    faces
        .iter()
        .filter(|face| !face.bytes.is_empty())
        .map(|face| EmbeddedFace { family: face.family, style: face.style, scripts: face.scripts, index: 0, bytes: face.bytes, role: None })
        .collect()
}

/// The craft-fonts faces this build embeds, in manifest order; empty without craft-fonts.
pub fn craft_faces() -> Vec<EmbeddedFace> {
    embedded(craft_fonts::CRAFT_FONTS)
}

/// The OS locale (`ja_JP`), which decides the Han preference when the UI language names none. `None` on the web and
/// when nothing is set; `PHOTOCRAFT_LOCALE` overrides it.
pub fn os_locale() -> Option<&'static str> {
    photocraft_text::cjk::ui_locale()
}

/// The loader that registers an installed system font the first time text needs a script no installed face covers.
/// Nothing is read on the web, whose candidates are none.
pub fn on_demand_fonts(ui_language: &str) -> OnDemandFonts {
    OnDemandFonts::new(Arc::new(SystemCandidates), han_for_locale(ui_language, os_locale()))
}

/// Headless test frame pumps do not wait for `request_repaint_after`, so let the Fontconfig CJK
/// lookup finish before asserting on installed system fonts. `pub` (rather than `pub(crate)`) so
/// strict-policy integration tests can use the same wait as the unit tests.
///
/// Returns whether the lookup finished; it never panics in production. Test callers assert the
/// result (and choose their own message).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn wait_for_fontconfig_cjk_scan() -> bool {
    use craft::ui::fonts::FontCandidates as _;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    // `pending` starts the lookup when none is running.
    while SystemCandidates.pending() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    !SystemCandidates.pending()
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
