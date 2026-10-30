//! egui's default fonts (Ubuntu Light, Hack, egui-icons, Noto Emoji and emoji-icon-font: the
//! fallback stack behind Inter and JetBrains Mono, for symbols and emoji), stored deflated.
//!
//! egui's `default_fonts` feature embeds them uncompressed (1.4 MB), which the web build's 24 MiB
//! size gate (packaging/web/package.sh) can't afford. The app builds egui without it; build.rs
//! deflates the same files from the `epaint_default_fonts` crate (0.78 MB) and they are inflated
//! once, the first time the fonts are installed. Test builds keep the feature (dev-dependencies),
//! so egui's own definitions are the oracle for [`definitions`].

use std::sync::{Arc, LazyLock};

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};
use photocraft_text::fonts::inflate_font;

/// A font deflated by build.rs.
macro_rules! deflated {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/egui_fonts/", $name, ".ttf.deflate"))
    };
}

static HACK: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(deflated!("Hack-Regular")));
static EGUI_ICONS: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(deflated!("egui-icons")));
static NOTO_EMOJI: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(deflated!("NotoEmoji-Regular")));
static UBUNTU_LIGHT: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(deflated!("Ubuntu-Light")));
static EMOJI_ICON: LazyLock<Vec<u8>> = LazyLock::new(|| inflate_font(deflated!("emoji-icon-font")));

/// `FontDefinitions::default()` with egui's default fonts, whether or not egui was built with
/// `default_fonts`: the same faces, tweaks and family order.
pub fn definitions() -> FontDefinitions {
    let fonts = FontDefinitions::default();
    if fonts.font_data.is_empty() { from_deflated() } else { fonts }
}

/// egui's default definitions, built from the deflated copies.
fn from_deflated() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    let faces: [(&str, &'static [u8], f32); 5] = [
        ("Hack", HACK.as_slice(), 1.0),
        ("Ubuntu-Light", UBUNTU_LIGHT.as_slice(), 1.0),
        // The handful of icons in `egui::special_emojis`, which no platform font has.
        ("egui-icons", EGUI_ICONS.as_slice(), 0.90),
        // egui draws its emoji fonts a little smaller.
        ("NotoEmoji-Regular", NOTO_EMOJI.as_slice(), 0.81),
        ("emoji-icon-font", EMOJI_ICON.as_slice(), 0.90),
    ];
    for (name, bytes, scale) in faces {
        if bytes.is_empty() {
            log::error!("egui default font {name} is unavailable");
            continue;
        }
        let data = FontData::from_static(bytes).tweak(FontTweak { scale, ..Default::default() });
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
    }
    let stack = |names: &[&str]| -> Vec<String> { names.iter().filter(|n| fonts.font_data.contains_key(**n)).map(|n| (*n).to_owned()).collect() };
    // 0.37's family order: egui-icons and the monochrome emoji fonts last, as fallbacks.
    let mono = stack(&["Hack", "Ubuntu-Light", "egui-icons", "NotoEmoji-Regular", "emoji-icon-font"]);
    let proportional = stack(&["Ubuntu-Light", "egui-icons", "NotoEmoji-Regular", "emoji-icon-font"]);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definitions built from the deflated copies are egui's own: the same bytes, tweaks and
    /// family order (test builds enable `default_fonts`, so `FontDefinitions::default()` is egui's).
    #[test]
    fn deflated_fonts_match_egui_defaults() {
        let egui = FontDefinitions::default();
        assert!(!egui.font_data.is_empty(), "test builds enable egui's default_fonts (dev-dependencies)");
        assert_eq!(from_deflated(), egui);
        assert_eq!(definitions(), egui);
    }

    /// The deflated stack carries egui 0.37's `egui-icons` face and the monochrome emoji faces,
    /// so `egui::special_emojis` and the UI's symbol glyphs do not render as tofu (kittest's
    /// `MissingGlyphPolicy::Panic` depends on this).
    #[test]
    fn the_deflated_stack_draws_egui_icons() {
        let ctx = egui::Context::default();
        ctx.set_fonts(from_deflated());
        let mut output = ctx.run_ui(Default::default(), |_| {});
        // The headless test never uploads the atlas; drop its delta deliberately.
        output.textures_delta.clear();
        let id = egui::FontId::proportional(14.0);
        let icons: String = [
            egui::special_emojis::OS_LINUX,
            egui::special_emojis::OS_WINDOWS,
            egui::special_emojis::OS_ANDROID,
            egui::special_emojis::OS_APPLE,
            egui::special_emojis::GITHUB,
            egui::special_emojis::GIT,
        ]
        .into_iter()
        .collect();
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&id, &icons)), "egui-icons covers every special emoji");
        let mono = egui::FontId::monospace(14.0);
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&mono, &icons)), "and the monospace family too");
    }

    #[test]
    fn corrupt_font_data_is_left_out_not_a_panic() {
        assert!(inflate_font(b"not deflate data").is_empty());
        assert!(inflate_font(&[]).is_empty());
    }
}
