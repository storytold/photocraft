//! Display digits (Arabic-Indic, Persian): after shaping, an ASCII digit's glyph is swapped for the
//! run font's glyph of the shaped digit. The text keeps its ASCII digits and every byte offset.

use parley::FontData;
use photocraft_doc::text::Digits;
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};

/// A shaped digit's replacement: the new glyph and the change of its advance (px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DigitSwap {
    pub id: u32,
    pub delta: f32,
}

/// The font instance of a shaping run, for looking up digit glyphs.
pub(super) struct DigitFont<'a> {
    font: skrifa::FontRef<'a>,
    coords: Vec<NormalizedCoord>,
    size: f32,
}

impl<'a> DigitFont<'a> {
    pub(super) fn new(fd: &'a FontData, coords: &[i16], size: f32) -> Option<Self> {
        let font = skrifa::FontRef::from_index(fd.data.as_ref(), fd.index).ok()?;
        Some(Self { font, coords: coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect(), size })
    }

    /// The swap for the ASCII digit `ascii` shown as `digits`, whose shaped glyph advances
    /// `advance` px. `None` for Western digits, other characters, and fonts without the digit
    /// (the Western glyph stays).
    pub(super) fn swap(&self, digits: Digits, ascii: char, advance: f32) -> Option<DigitSwap> {
        let shown = digits.shape(ascii)?;
        let gid = self.font.charmap().map(shown)?;
        let new_advance = self.font.glyph_metrics(Size::new(self.size), LocationRef::new(&self.coords)).advance_width(gid)?;
        let delta = new_advance - advance;
        delta.is_finite().then_some(DigitSwap { id: gid.to_u32(), delta })
    }
}
