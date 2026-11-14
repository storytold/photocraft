use super::{button_fill, button_label_color};
use crate::theme::{ThemeKind, Tokens};
use egui::Color32;

/// Imported colours without a classic swatch use the theme's complete neutral foreground/background pair.
#[test]
fn metadata_colours_use_readable_neutral_button_tokens() {
    for kind in [ThemeKind::Pro, ThemeKind::ProLight] {
        let t = Tokens::for_kind(kind);
        for color in ["", "teal", "blue2", "magenta", "purple", "@atn-color:12", "@atn-color:65535"] {
            assert_eq!(button_fill(color, &t), t.card, "{kind:?}/{color}");
            assert_eq!(button_label_color(color, &t), t.text, "{kind:?}/{color}");
        }
    }
}

/// Metadata preservation does not alter the classic seven named swatches or their existing foreground.
#[test]
fn classic_button_swatches_keep_their_original_colours() {
    for kind in [ThemeKind::Pro, ThemeKind::ProLight] {
        let t = Tokens::for_kind(kind);
        for (color, fill) in [
            ("red", Color32::from_rgb(196, 64, 64)),
            ("orange", Color32::from_rgb(204, 120, 40)),
            ("yellow", Color32::from_rgb(196, 176, 48)),
            ("green", Color32::from_rgb(56, 140, 72)),
            ("blue", Color32::from_rgb(48, 104, 176)),
            ("violet", Color32::from_rgb(120, 72, 168)),
            ("gray", t.text_dim),
        ] {
            assert_eq!(button_fill(color, &t), fill, "{kind:?}/{color}");
            assert_eq!(button_label_color(color, &t), Color32::from_rgb(20, 20, 20), "{kind:?}/{color}");
        }
    }
}
