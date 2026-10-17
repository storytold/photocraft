//! Typed style model for type (text) layers: character runs, paragraph runs, point/box shape and
//! warp. Pure data; layout and rendering live in `photocraft-text`.
//!
//! Units follow Photoshop's Character/Paragraph panels: sizes, leading, indents and spacing are in
//! **points** (converted to pixels with the document resolution: `px = pt * dpi / 72`), tracking
//! is in 1/1000 em, and box bounds are in text-space **pixels** (before the layer transform).
//!
//! Runs cover the text by **UTF-8 byte length**, in order. A run list whose lengths don't add up
//! to `text.len()` is normalised by [`crate::TextLayer::char_runs`] (the last run is stretched or
//! truncated), so edits never produce unstyled text.

use photocraft_color::Color;
use serde::{Deserialize, Serialize};

/// Kerning mode (Photoshop: Metrics, Optical, 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kerning {
    /// Font kerning (`kern` feature).
    #[default]
    Metrics,
    /// Photoshop's optical kerning: pair spacing computed from the glyph outlines.
    Optical,
    /// No automatic kerning (Photoshop's "0" / manual kerning).
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Caps {
    #[default]
    Normal,
    SmallCaps,
    AllCaps,
}

/// An OpenType feature setting, e.g. `{ tag: "ss01", value: 1 }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontFeature {
    pub tag: String,
    pub value: u16,
}

/// A variable-font axis setting, e.g. `{ axis: "wght", value: 650.0 }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FontVariation {
    pub axis: String,
    pub value: f32,
}

/// Character style (Photoshop's Character panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharStyle {
    /// Family name ("Inter"). Empty = the default family.
    pub font_family: String,
    /// Face style name as shown in the UI ("Bold Italic"); informational, `weight`/`italic` rule.
    pub font_style: String,
    /// PostScript name from a PSD (`ArialMT`), used to find the exact face when present.
    pub postscript_name: Option<String>,
    /// CSS-style weight, 100–900.
    pub weight: u16,
    pub italic: bool,
    pub size_pt: f32,
    pub color: Color,
    /// Letter spacing in 1/1000 em.
    pub tracking: f32,
    /// Baseline-to-baseline distance in points; `None` = auto (paragraph `auto_leading` × size).
    pub leading_pt: Option<f32>,
    /// Positive raises the text.
    pub baseline_shift_pt: f32,
    pub horizontal_scale: f32,
    pub vertical_scale: f32,
    pub underline: bool,
    pub strikethrough: bool,
    pub faux_bold: bool,
    pub faux_italic: bool,
    /// Automatic (pair) kerning mode.
    pub kerning: Kerning,
    /// Manual kerning in 1/1000 em between this character and the next one (Photoshop stores it
    /// per character run as `Kerning`). Added to the automatic kerning of the pair.
    pub kern: f32,
    pub caps: Caps,
    /// Standard ligatures (`liga`/`clig`).
    pub ligatures: bool,
    /// Discretionary ligatures (`dlig`).
    pub discretionary_ligatures: bool,
    /// Extra OpenType features (`ss01`, `onum`, `tnum`, …).
    pub features: Vec<FontFeature>,
    /// Variable-font axis values.
    pub variations: Vec<FontVariation>,
    /// BCP-47 language tag (affects shaping and line breaking).
    pub language: Option<String>,
    /// Applied character style (id in [`crate::TextStyles`]); `None` = "None". Attributes above
    /// stay fully resolved; differences from the style are local overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_sheet: Option<u32>,
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_style: String::new(),
            postscript_name: None,
            weight: 400,
            italic: false,
            size_pt: 12.0,
            color: Color::BLACK,
            tracking: 0.0,
            leading_pt: None,
            baseline_shift_pt: 0.0,
            horizontal_scale: 1.0,
            vertical_scale: 1.0,
            underline: false,
            strikethrough: false,
            faux_bold: false,
            faux_italic: false,
            kerning: Kerning::Metrics,
            kern: 0.0,
            caps: Caps::Normal,
            ligatures: true,
            discretionary_ligatures: false,
            features: Vec::new(),
            variations: Vec::new(),
            language: None,
            style_sheet: None,
        }
    }
}

/// A character style applied to `len` UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub len: usize,
    pub style: CharStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    /// Justified, last line left.
    JustifyLeft,
    /// Justified, last line centered.
    JustifyCenter,
    /// Justified, last line right.
    JustifyRight,
    /// Justified including the last line.
    JustifyAll,
}

impl TextAlign {
    pub fn is_justified(self) -> bool {
        matches!(self, Self::JustifyLeft | Self::JustifyCenter | Self::JustifyRight | Self::JustifyAll)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextDirection {
    /// From the first strong character (Unicode bidi rules).
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// Paragraph style (Photoshop's Paragraph panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParagraphStyle {
    pub align: TextAlign,
    pub first_line_indent_pt: f32,
    pub start_indent_pt: f32,
    pub end_indent_pt: f32,
    pub space_before_pt: f32,
    pub space_after_pt: f32,
    /// Auto leading as a multiple of the font size (Photoshop default 1.2).
    pub auto_leading: f32,
    pub direction: TextDirection,
    pub hyphenate: bool,
    /// Applied paragraph style (id in [`crate::TextStyles`]); `None` = Basic Paragraph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_sheet: Option<u32>,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: TextAlign::Left,
            first_line_indent_pt: 0.0,
            start_indent_pt: 0.0,
            end_indent_pt: 0.0,
            space_before_pt: 0.0,
            space_after_pt: 0.0,
            auto_leading: 1.2,
            direction: TextDirection::Auto,
            hyphenate: false,
            style_sheet: None,
        }
    }
}

/// A paragraph style applied to `len` UTF-8 bytes (whole paragraphs, including the break).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParagraphRun {
    pub len: usize,
    pub style: ParagraphStyle,
}

/// Point text grows from its anchor (text-space origin = first baseline at the anchor);
/// paragraph (box) text wraps inside a rectangle; path text runs along a curve.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum TextShape {
    #[default]
    Point,
    /// Text-space pixels, before the layer transform.
    Box { x: f32, y: f32, width: f32, height: f32 },
    /// Type on a path: one line laid out like point text, then each glyph is placed on `path`
    /// (text-space pixels, before the layer transform) by arc length and turned to follow the
    /// curve. `start` is the arc-length offset (px) of the text origin along the path; `flip`
    /// puts the text on the other side of the path.
    Path { path: super::vector::Subpath, start: f32, flip: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Horizontal,
    /// Vertical type (tategaki): columns top to bottom, advancing right to left.
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AntiAlias {
    None,
    Sharp,
    Crisp,
    Strong,
    #[default]
    Smooth,
    /// Photoshop's "Windows" (platform grayscale) anti-aliasing; rendered like `Sharp`.
    Windows,
    /// Photoshop's "Windows LCD" (platform subpixel) anti-aliasing; rendered like `Sharp` (we
    /// never produce colour-fringed subpixel text in document pixels).
    WindowsLcd,
}

/// Warp text settings (Photoshop `warp` descriptor), applied to the glyph outlines by
/// `photocraft-text` (see its `warp` module) and round-tripped through PSD.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextWarp {
    /// `warpNone`, `warpArc`, `warpFlag`, …
    pub style: String,
    /// Bend in percent (-100..100).
    pub value: f32,
    pub horizontal_distortion: f32,
    pub vertical_distortion: f32,
    /// Horizontal (true) or vertical warp axis.
    pub horizontal: bool,
}

/// Most free transforms one type layer keeps (each one costs a pass over the outlines).
pub const MAX_TYPE_DISTORTIONS: usize = 16;

/// A free transform of live type that Photoshop can't keep editable ("Better Than PS"): a custom
/// warp mesh (Edit › Transform › Warp) or a projective map (Distort, Perspective), applied to the
/// glyph outlines after Warp Text, so the text stays editable and re-renders under it.
///
/// It is defined in the document space of the moment it was made: `frame` is the layer's
/// `transform` then (text space → that space). The outlines go through `frame`, the distortion
/// and `frame`'s inverse, then the current `transform`, so moving, scaling or rotating the type
/// afterwards carries the distortion along.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeDistortion {
    pub frame: photocraft_geom::Affine,
    pub kind: TypeDistortionKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeDistortionKind {
    /// A custom warp over its box (`bounds`), in the frame's space.
    Warp(photocraft_geom::warp::Warp),
    /// Distort / Perspective: a row-major 3×3 projective map in the frame's space.
    Projective([f64; 9]),
}

impl TypeDistortionKind {
    /// Maps a point of the distortion's own space. A custom warp continues outside its box (type
    /// edited after the warp can grow past it): a point outside follows the nearest edge point,
    /// shifted by its distance from the box. A point past a perspective's horizon stays put.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (qx, qy) = match self {
            TypeDistortionKind::Warp(w) => {
                let b = w.bounds;
                let (cx, cy) = (x.clamp(b[0], b[2].max(b[0])), y.clamp(b[1], b[3].max(b[1])));
                let (mx, my) = w.map(cx, cy);
                (mx + (x - cx), my + (y - cy))
            }
            TypeDistortionKind::Projective(h) => {
                let d = h[6] * x + h[7] * y + h[8];
                if d.abs() <= 1e-9 {
                    return (x, y);
                }
                ((h[0] * x + h[1] * y + h[2]) / d, (h[3] * x + h[4] * y + h[5]) / d)
            }
        };
        if qx.is_finite() && qy.is_finite() { (qx, qy) } else { (x, y) }
    }
}

/// `a · b` for row-major 3×3 matrices (apply `b` first).
pub fn mat3_mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut r = [0.0; 9];
    for i in 0..3 {
        for j in 0..3 {
            r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
        }
    }
    r
}

fn affine3(a: &photocraft_geom::Affine) -> [f64; 9] {
    let [a0, b0, c0, d0, e0, f0] = a.m;
    [a0, c0, e0, b0, d0, f0, 0.0, 0.0, 1.0]
}

impl TypeDistortion {
    /// `transform · frame⁻¹`: the distortion's space → the document space of a layer whose
    /// transform is now `transform`.
    pub fn to_document(&self, transform: &photocraft_geom::Affine) -> Option<photocraft_geom::Affine> {
        Some(transform.mul(&self.frame.inverse()?))
    }

    /// Maps a document point the way this distortion moves the type today (layer transform
    /// `transform`): exactly what the renderer does to the outlines.
    pub fn map_in_document(&self, transform: &photocraft_geom::Affine, x: f64, y: f64) -> (f64, f64) {
        let Some(m) = self.to_document(transform) else { return (x, y) };
        let Some(mi) = m.inverse() else { return (x, y) };
        let q = mi.apply(photocraft_geom::Point::new(x, y));
        let (qx, qy) = self.kind.apply(q.x, q.y);
        let r = m.apply(photocraft_geom::Point::new(qx, qy));
        (r.x, r.y)
    }

    /// This distortion in today's document space (layer transform `transform`), for editing it
    /// and for PSD export. Exact for a projective map and a custom mesh (Bezier patches are
    /// affine-invariant) and for a preset under a move and scale; a preset under a rotation or
    /// skew becomes its fitted mesh.
    pub fn in_document(&self, transform: &photocraft_geom::Affine) -> Option<TypeDistortionKind> {
        let m = self.to_document(transform)?;
        match &self.kind {
            TypeDistortionKind::Projective(h) => {
                let mi = m.inverse()?;
                Some(TypeDistortionKind::Projective(mat3_mul(&affine3(&m), &mat3_mul(h, &affine3(&mi)))))
            }
            TypeDistortionKind::Warp(w) => {
                use photocraft_geom::warp::{Warp, WarpStyle};
                let ap = |p: [f64; 2]| {
                    let q = m.apply(photocraft_geom::Point::new(p[0], p[1]));
                    [q.x, q.y]
                };
                let b = w.bounds;
                let c = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(ap);
                let bb = c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |r, p| [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[0]), r[3].max(p[1])]);
                let axis = m.m[1] == 0.0 && m.m[2] == 0.0 && m.m[0] > 0.0 && m.m[3] > 0.0;
                Some(TypeDistortionKind::Warp(if w.style == WarpStyle::None {
                    Warp::none(bb)
                } else if w.style.is_preset() && axis {
                    Warp { bounds: bb, mesh: None, ..w.clone() }
                } else if w.style.is_preset() {
                    Warp::custom(w.to_mesh(4, 4).map_points(ap), bb)
                } else {
                    Warp::custom(w.to_mesh(1, 1).map_points(ap), bb)
                }))
            }
        }
    }

    /// Finite, invertible and well-formed (data from files and commands is checked with this).
    pub fn is_valid(&self) -> bool {
        let frame_ok = self.frame.m.iter().all(|v| v.is_finite()) && self.frame.inverse().is_some();
        frame_ok
            && match &self.kind {
                TypeDistortionKind::Warp(w) => {
                    w.bounds.iter().all(|v| v.is_finite())
                        && w.bounds[2] > w.bounds[0]
                        && w.bounds[3] > w.bounds[1]
                        && w.mesh.as_ref().is_none_or(|m| m.is_valid() && m.points.len() <= 193 * 193)
                }
                TypeDistortionKind::Projective(h) => {
                    let det = h[0] * (h[4] * h[8] - h[5] * h[7]) - h[1] * (h[3] * h[8] - h[5] * h[6]) + h[2] * (h[3] * h[7] - h[4] * h[6]);
                    h.iter().all(|v| v.is_finite()) && det.abs() > 1e-12
                }
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextLayer;

    fn run(len: usize, size: f32) -> TextRun {
        TextRun { len, style: CharStyle { size_pt: size, ..Default::default() } }
    }

    #[test]
    fn runs_are_normalized_to_the_text() {
        let base = TextLayer { text: "héllo".into(), size_pt: 9.0, ..Default::default() };
        // No runs: one run from the summary fields.
        let r = base.char_runs();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].len, r[0].style.size_pt), (6, 9.0));
        // Too short: last run stretched. Split inside 'é' snaps to the char boundary.
        let t = TextLayer { runs: vec![run(2, 10.0), run(1, 20.0)], ..base.clone() };
        let r = t.char_runs();
        assert_eq!(r.iter().map(|r| r.len).collect::<Vec<_>>(), vec![3, 3]);
        // Too long: truncated; empty runs dropped.
        let t = TextLayer { runs: vec![run(0, 1.0), run(100, 10.0), run(5, 20.0)], ..base.clone() };
        let r = t.char_runs();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].len, r[0].style.size_pt), (6, 10.0));
        // Empty text keeps the first run's style.
        let t = TextLayer { text: String::new(), runs: vec![run(0, 33.0)], ..base.clone() };
        assert_eq!(t.char_runs()[0].style.size_pt, 33.0);
        assert_eq!(t.paragraph_runs().len(), 1);
    }

    #[test]
    fn summary_follows_first_run() {
        let mut t = TextLayer {
            text: "ab".into(),
            runs: vec![TextRun { len: 2, style: CharStyle { font_family: "X".into(), size_pt: 7.0, ..Default::default() } }],
            ..Default::default()
        };
        t.sync_summary();
        assert_eq!((t.font_family.as_str(), t.size_pt), ("X", 7.0));
    }

    #[test]
    fn styles_serialize() {
        let s = CharStyle { features: vec![FontFeature { tag: "ss01".into(), value: 1 }], ..Default::default() };
        let v = serde_json::to_value(&s).unwrap();
        let back: CharStyle = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
        // Missing keys take defaults.
        let p: ParagraphStyle = serde_json::from_str(r#"{"align":"Center"}"#).unwrap();
        assert_eq!((p.align, p.auto_leading), (TextAlign::Center, 1.2));
    }
}
