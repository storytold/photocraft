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

use craft::ui::fonts::{EmbeddedFace, FontCandidates, OnDemandFonts, SystemCandidates, han_for_locale};
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

/// A face for each script the interface can show that no craft face draws (Tamil, say): the
/// language list writes every offered language in its own script, and the on-demand loader
/// registers a face only after a pass drew a missing glyph. One pass too late: a release build
/// shows a box for that pass, and a build that panics on a missing glyph (egui's
/// `MissingGlyphPolicy::Panic`, which the tests use) dies on it.
///
/// Faces come from the installed fonts, one per script, read once per process; a script this
/// machine has no font for is left to the loader (and to tofu). Empty on the web, which has no
/// installed fonts. Inter, the interface face, already draws Latin, Cyrillic and Greek.
pub fn uncovered_script_faces() -> &'static [EmbeddedFace] {
    static FACES: std::sync::OnceLock<Vec<EmbeddedFace>> = std::sync::OnceLock::new();
    FACES.get_or_init(uncovered_scripts).as_slice()
}

/// The scripts of the offered languages that no craft face and no interface face draws, with an
/// installed face for each; empty on the web.
fn uncovered_scripts() -> Vec<EmbeddedFace> {
    #[cfg(target_arch = "wasm32")]
    return Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut drawn: Vec<&str> = craft_fonts::CRAFT_FONTS.iter().flat_map(|face| face.scripts.iter().copied()).collect();
        drawn.extend_from_slice(INTERFACE_SCRIPTS);
        let mut scripts: Vec<&'static str> = Vec::new();
        for language in crate::i18n::LANGUAGES.iter().map(|lang| lang.code) {
            for script in craft::i18n::scripts(language) {
                let code = script.code();
                if !drawn.iter().any(|drawn| drawn.eq_ignore_ascii_case(code)) && !scripts.contains(&code) {
                    scripts.push(code);
                }
            }
        }
        scripts.into_iter().filter_map(installed_face).collect()
    }
}

/// The scripts Inter draws, so no system face is read for them.
const INTERFACE_SCRIPTS: &[&str] = &["latn", "cyrl", "grek"];

/// The largest installed face read for a script: a font collection of several CJK-sized faces
/// buys nothing here.
const MAX_SCRIPT_FACE_BYTES: u64 = 8 << 20;

/// The first installed face that may draw `script`, as an embedded face of the UI stack.
fn installed_face(script: &'static str) -> Option<EmbeddedFace> {
    for path in SystemCandidates.files(script, None) {
        let Some(size) = std::fs::metadata(&path).ok().map(|meta| meta.len()) else { continue };
        if size == 0 || size > MAX_SCRIPT_FACE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(family) = path.file_stem().and_then(|stem| stem.to_str()) else { continue };
        let family: &'static str = Box::leak(family.to_owned().into_boxed_str());
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let scripts: &'static [&'static str] = Box::leak(vec![script].into_boxed_slice());
        return Some(EmbeddedFace { family, style: "Regular", scripts, index: 0, bytes, role: None });
    }
    None
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
