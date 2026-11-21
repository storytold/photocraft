//! Optional point-origin measurements are a bounded import preview, not a full
//! text-editor operation. Refusal keeps the complete source and saved pixels.
use super::{CharStyle, TextLayer, TextShape};

pub(super) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(super) const MAX_SCALARS: usize = 4096;
pub(super) const MAX_LINE_SCALARS: usize = 2048;
pub(super) const MAX_RUNS: usize = 256;
pub(super) const MAX_BREAKS: usize = 128;
const MAX_FONT_REQUEST_BYTES: usize = 16 * 1024;
const MAX_EXTENT: f64 = super::super::MAX_SIDE as f64;

/// Deterministic admission units bound source shaping/font requests, not elapsed
/// time. Scalar counting includes combining marks; byte/grapheme counts alone
/// would not bound them. Geometry is measured in effective device pixels.
pub(super) fn cost(layer: &TextLayer, dpi: f32) -> Option<usize> {
    if !matches!(layer.shape, TextShape::Point) || layer.text.is_empty() {
        return Some(0);
    }
    if layer.text.len() > MAX_TEXT_BYTES || layer.runs.is_empty() || layer.runs.len() > MAX_RUNS || layer.paragraphs.len() > MAX_BREAKS + 1 {
        return None;
    }
    let k = f64::from(dpi) / 72.;
    if !k.is_finite() || k <= 0. {
        return None;
    }
    let (mut scalars, mut breaks, mut line, mut longest, mut cr) = (0usize, 0usize, 0usize, 0usize, false);
    for character in layer.text.chars() {
        scalars = scalars.checked_add(1)?;
        if scalars > MAX_SCALARS {
            return None;
        }
        if matches!(character, '\r' | '\n' | '\u{3}' | '\u{2028}' | '\u{2029}') {
            if !(character == '\n' && cr) {
                breaks = breaks.checked_add(1)?;
            }
            longest = longest.max(line);
            line = 0;
        } else {
            line = line.checked_add(1)?;
        }
        cr = character == '\r';
    }
    longest = longest.max(line);
    if breaks > MAX_BREAKS || longest > MAX_LINE_SCALARS {
        return None;
    }
    let (mut advance, mut leading, mut offset, mut font_bytes) = (0f64, 0f64, 0f64, 0usize);
    for run in &layer.runs {
        let style = &run.style;
        let request = style
            .font_family
            .len()
            .checked_add(style.font_style.len())?
            .checked_add(style.postscript_name.as_ref().map_or(0, String::len))?
            .checked_add(style.language.as_ref().map_or(0, String::len))?;
        font_bytes = font_bytes.checked_add(request)?;
        if request > 1024 || font_bytes > MAX_FONT_REQUEST_BYTES || style.features.len() > 64 || style.variations.len() > 64 {
            return None;
        }
        let (a, l, o) = style_extent(style, k)?;
        advance = advance.max(a);
        leading = leading.max(l);
        offset = offset.max(o);
    }
    let mut indent = 0f64;
    for paragraph in &layer.paragraphs {
        let p = &paragraph.style;
        for points in [p.first_line_indent_pt, p.start_indent_pt, p.end_indent_pt, p.space_before_pt, p.space_after_pt] {
            let pixels = (f64::from(points) * k).abs();
            if !pixels.is_finite() || pixels > MAX_EXTENT {
                return None;
            }
            indent = indent.max(pixels);
        }
        if !p.auto_leading.is_finite() || p.auto_leading <= 0. || p.auto_leading > 16. {
            return None;
        }
        leading = leading.max(advance * f64::from(p.auto_leading));
    }
    // This conservative logical-span estimate uses the existing XCF side limit;
    // it is a measurement policy, not a promise about a particular font's ink.
    if longest as f64 * advance + 2. * (offset + indent) > MAX_EXTENT || (breaks + 1) as f64 * leading + 2. * (offset + indent) > MAX_EXTENT {
        return None;
    }
    scalars.checked_add(layer.runs.len().checked_mul(16)?)?.checked_add((breaks + 1).checked_mul(32)?)
}

fn style_extent(style: &CharStyle, k: f64) -> Option<(f64, f64, f64)> {
    let size = f64::from(style.size_pt) * k;
    if !style.horizontal_scale.is_finite() || !style.vertical_scale.is_finite() {
        return None;
    }
    let scale = f64::from(style.horizontal_scale).abs().max(f64::from(style.vertical_scale).abs());
    let spacing = (f64::from(style.tracking) + f64::from(style.kern)).abs() * size / 1000.;
    let advance = size * 2. * scale + spacing;
    let leading = f64::from(style.leading_pt.unwrap_or(style.size_pt * 1.2)) * k;
    let offset = (f64::from(style.baseline_shift_pt) * k).abs();
    (size.is_finite()
        && size > 0.
        && scale.is_finite()
        && scale > 0.
        && advance.is_finite()
        && leading.is_finite()
        && leading > 0.
        && offset.is_finite()
        && advance <= MAX_EXTENT
        && leading <= MAX_EXTENT
        && offset <= MAX_EXTENT)
        .then_some((advance, leading, offset))
}
