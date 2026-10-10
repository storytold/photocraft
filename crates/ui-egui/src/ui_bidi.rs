//! Bidirectional reordering for egui UI labels.
//!
//! egui 0.36 shapes with HarfRust but still paints runs in logical order (no paragraph bidi),
//! so Arabic / Hebrew layer names appear with the first word on the left. Reorder to visual
//! order before layout so LTR painting matches RTL reading (first word on the right).

use std::borrow::Cow;
use unicode_bidi::BidiInfo;

/// True when `text` contains a strong right-to-left character (Arabic or Hebrew scripts).
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_char)
}

fn is_rtl_char(c: char) -> bool {
    matches!(
        c as u32,
        0x0590..=0x05FF
            | 0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x0870..=0x08FF
            | 0xFB1D..=0xFB4F
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
            | 0x1EE00..=0x1EEFF
    )
}

/// Text to feed egui so an LTR layout engine shows RTL scripts in visual order.
///
/// Latin-only strings are returned unchanged. Mixed and RTL strings are reordered with the
/// Unicode bidi algorithm (auto base direction).
pub fn for_egui(text: &str) -> Cow<'_, str> {
    if text.is_empty() || !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let info = BidiInfo::new(text, None);
    let mut out = String::with_capacity(text.len());
    for para in &info.paragraphs {
        out.push_str(&info.reorder_line(para, para.range.clone()));
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_unchanged() {
        assert!(matches!(for_egui("Layer 1"), Cow::Borrowed("Layer 1")));
    }

    #[test]
    fn kurdish_sentence_puts_first_word_at_visual_end() {
        let logical = "ئەمە تێستە بۆ ئیش";
        let visual = for_egui(logical);
        // Visual order (LTR string): last word's letters first, first word's last.
        // ئیش → شیئ, ئەمە → ەمەئ
        assert!(visual.as_ref().starts_with('ش'), "visual={visual}");
        assert!(visual.as_ref().ends_with('ئ'), "visual={visual}");
        assert_ne!(visual.as_ref(), logical);
    }
}
