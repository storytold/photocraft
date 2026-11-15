//! Shaping and layout of a text layer's model into positioned glyphs (text-space pixels, y down,
//! first baseline of point text at y = 0).
//!
//! Each paragraph is shaped and line-broken by parley (HarfRust shaping, Unicode bidi and line
//! breaking); paragraphs are then stacked with Photoshop's rules: baseline-to-baseline distance =
//! the largest leading on the line (auto leading = paragraph factor × size), space before/after,
//! indents, and point-text alignment around the anchor.
//!
//! **Vertical type** (tategaki) is laid out in the same *line space* and then turned 90° clockwise
//! into text space: a line becomes a column running top to bottom, and successive columns advance
//! right to left. Within a column, CJK characters stay upright (centred on the column, placed with
//! the font's vertical metrics: `VORG`, `vhea`/`vmtx`, else the ideographic em box) and use the
//! font's vertical alternates (`vert`) for punctuation, brackets and the long-vowel mark. Roman
//! letters stand upright one per em unless the run sets `rotate_roman`. Lines, clusters, carets
//! and hit tests stay in line space; [`TextLayout::to_text`] / [`TextLayout::to_line`] convert.

use std::borrow::Cow;
use std::ops::Range;

use parley::{FontData, FontFamily, FontFeatures, FontStyle, FontVariations, FontWeight, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{Caps, CharStyle, FitLine, FontBaseline, Kerning, Orientation, ParagraphStyle, TextFit, TextShape};
use skrifa::raw::types::Tag;

use crate::fonts::FontDb;

mod area;
mod lines;
mod navigate;
mod paragraphs;
mod shaping;
mod style;
mod vertical;

#[cfg(test)]
pub(crate) use area::AREA_PASSES;
pub use navigate::{byte_index, char_index, hit_char, line_edge, line_index, line_step, text_point_inside, word_boundary};
pub use paragraphs::{DEFAULT_TAB_STOP_PT, split_paragraphs};
use paragraphs::{next_tab_stop, strip_break};
use shaping::{Para, resolve_run_styles, running_starts, small_caps_modes};
use style::SmallCapsMode;
#[cfg(test)]
use style::{SMCP_PROBES, feature_list, small_caps_mode, variation_list};
#[cfg(test)]
pub(crate) use style::{hangul_runs, squeeze_punctuation, synthetic_bold};
pub use vertical::{VClass, vertical_class};

/// Index of the run containing byte offset `off`, given each run's start offset (non-decreasing):
/// the last run starting at or before `off`, or 0 when none does. Binary search, so a layer with
/// many runs doesn't make per-character lookups quadratic.
fn run_index_at(run_starts: &[usize], off: usize) -> usize {
    run_starts.partition_point(|&s| s <= off).saturating_sub(1)
}

/// Index of the character run whose style a glyph uses, plus the vertical-type class of its
/// characters ([`VClass`] as `u8`; always 0 in horizontal type).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunBrush(pub u32, pub u8);

/// How a placed glyph's outline is oriented in text space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphOrient {
    /// Horizontal type: origin on the baseline, outline upright.
    #[default]
    Horizontal,
    /// Vertical type, upright: the origin is the glyph's horizontal origin (left end of its
    /// advance, on its baseline); baseline shift moves it right.
    Upright,
    /// Vertical type, rotated 90° clockwise about its origin (the baseline runs downwards).
    Rotated,
}

/// A font instance used by some glyphs.
#[derive(Clone, Debug)]
pub struct GlyphFace {
    pub font: FontData,
    /// Normalised variation coordinates (F2Dot14 bits).
    pub coords: Vec<i16>,
    pub size_px: f32,
    /// Synthetic bold requested by font matching (face lacks the weight).
    pub embolden: bool,
    /// Synthetic oblique angle in degrees (face lacks italics).
    pub skew_deg: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub face: u32,
    pub id: u32,
    /// Pen position (text space px): x along the baseline, y = baseline (before baseline shift).
    pub x: f32,
    pub y: f32,
    /// Character run index (into [`TextLayout::styles`]).
    pub style: u32,
    /// Orientation of the outline (vertical type).
    pub orient: GlyphOrient,
    /// Optional line-space glyph center before rotation into text space.
    pub line_center: Option<f32>,
}

/// Underline/strikethrough rectangle (text space px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub style: u32,
}

/// A line (a column in vertical type) in line space: `x` runs along the line and `baseline` is the
/// cross-line position. In vertical type `baseline` is the column's centre line and
/// `ascent`/`descent` are the column's half widths on either side.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    /// Byte range in the layer text (without the paragraph break).
    pub range: Range<usize>,
    pub baseline: f32,
    /// Visual extent along the baseline.
    pub x0: f32,
    pub x1: f32,
    pub ascent: f32,
    pub descent: f32,
    pub paragraph: usize,
}

/// A grapheme cluster (caret stops, hit testing, selection), in line space.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterInfo {
    pub range: Range<usize>,
    pub x: f32,
    pub advance: f32,
    pub line: usize,
    pub rtl: bool,
}

/// Result of laying out a text layer.
#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub faces: Vec<GlyphFace>,
    pub glyphs: Vec<PlacedGlyph>,
    /// Layer-text byte range represented by each glyph, in the same order as `glyphs`.
    pub glyph_ranges: Vec<Range<usize>>,
    pub decorations: Vec<DecorationRect>,
    pub lines: Vec<LineInfo>,
    pub clusters: Vec<ClusterInfo>,
    /// Resolved character styles: one per run of the layer, then for solved Dynamic Text lines not
    /// at scale 1, scaled copies the glyphs of those lines use.
    pub styles: Vec<CharStyle>,
    /// Pixels per point used (dpi / 72).
    pub px_per_pt: f32,
    /// Vertical type: line space is turned 90° clockwise into text space.
    pub vertical: bool,
    /// Box or area text that doesn't fit: [`Self::hidden`], or a line too wide for its width.
    /// A line is too wide when its shaped advance, without trailing whitespace, is more than
    /// 0.5 px wider than the width it was broken at (box or span less indents, and on a
    /// paragraph's first line less the first-line indent). Horizontal scale and manual or
    /// optical kerning are applied after line breaking and never count: lines they push past
    /// the edge are drawn as before but not flagged. A solved Dynamic Text line counts by
    /// its visible width (`x1 - x0`, scale and kerning included), as the solver sizes it, so
    /// one that clamping left wider than its box or span counts as too wide. Always false for
    /// point and path text.
    pub overflowed: bool,
    /// Text other than line and paragraph breaks is hidden: the box height or area shape ran out
    /// (an empty last paragraph below the box doesn't count). Implies `overflowed`. Always false
    /// for point and path text. The Dynamic Text box solver uses it as its height test.
    pub hidden: bool,
}

impl TextLayout {
    /// Line space → text space: the identity for horizontal type; for vertical type the line
    /// direction points down and the cross-line direction (towards the next line) points left.
    pub fn to_text(&self, x: f32, y: f32) -> (f32, f32) {
        if self.vertical { (-y, x) } else { (x, y) }
    }

    /// Text space → line space (inverse of [`Self::to_text`]).
    pub fn to_line(&self, x: f32, y: f32) -> (f32, f32) {
        if self.vertical { (y, -x) } else { (x, y) }
    }

    /// Logical bounds (x0, y0, x1, y1) of all lines, text space.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let b = self.line_bounds()?;
        Some(if self.vertical { [-b[3], b[0], -b[1], b[2]] } else { b })
    }

    /// Logical bounds of all lines in line space (equal to [`Self::bounds`] for horizontal type).
    pub fn line_bounds(&self) -> Option<[f32; 4]> {
        self.lines.iter().fold(None, |acc, l| {
            let r = [l.x0, l.baseline - l.ascent, l.x1, l.baseline + l.descent];
            Some(match acc {
                None => r,
                Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
            })
        })
    }

    /// Whether boxed text doesn't all fit its shape: a box's layout drops the lines that
    /// overflow (`layout` breaks out of the line loop), so any character the laid-out lines
    /// don't cover — the last line's range ends short of the whole text — points at dropped
    /// content. Text that fits completely covers its end: paragraph breaks are inside their
    /// lines, and a trailing empty paragraph that fits gains its own (empty) line. Empty text
    /// has nothing to drop. Callers gate this on the shape being a box; point text has no box
    /// to overflow.
    pub fn overflows(&self, text: &str) -> bool {
        !text.is_empty() && self.lines.last().is_none_or(|ln| ln.range.end < text.len())
    }

    /// Byte offset of the caret nearest to a text-space point.
    pub fn hit_test(&self, x: f32, y: f32) -> usize {
        let (x, y) = self.to_line(x, y);
        self.hit_test_line(x, y)
    }

    /// The visual line nearest to a line-space ordinate, including its ascent and descent.
    /// Pointer hit testing uses this even where adjacent lines share a caret byte offset.
    fn closest_line(&self, y: f32) -> Option<(usize, &LineInfo)> {
        self.lines.iter().enumerate().min_by(|a, b| {
            let d = |l: &LineInfo| {
                if y < l.baseline - l.ascent {
                    l.baseline - l.ascent - y
                } else if y > l.baseline + l.descent {
                    y - l.baseline - l.descent
                } else {
                    0.0
                }
            };
            d(a.1).total_cmp(&d(b.1))
        })
    }

    /// Byte offset of the caret nearest to a line-space point.
    pub fn hit_test_line(&self, x: f32, y: f32) -> usize {
        let Some((li, _)) = self.closest_line(y) else {
            return 0;
        };
        self.hit_test_x_on_line(li, x)
    }

    fn hit_test_x_on_line(&self, line_index: usize, x: f32) -> usize {
        let Some(line) = self.lines.get(line_index) else { return 0 };
        let mut best = (f32::MAX, line.range.end);
        for c in self.clusters.iter().filter(|c| c.line == line_index) {
            let mid = c.x + c.advance / 2.0;
            let (before, after) = if c.rtl { (c.range.end, c.range.start) } else { (c.range.start, c.range.end) };
            let (dist, off) = if x < mid { ((x - c.x).abs(), before) } else { ((x - c.x - c.advance).abs(), after) };
            if dist < best.0 {
                best = (dist, off);
            }
        }
        best.1
    }

    /// The caret for a byte offset as a text-space segment (its two end points).
    pub fn caret_segment(&self, offset: usize) -> [(f32, f32); 2] {
        let (x, top, bottom) = self.caret(offset);
        [self.to_text(x, top), self.to_text(x, bottom)]
    }

    /// Caret geometry for a byte offset: (x, top, bottom) in line space (the same as text space
    /// for horizontal type; see [`Self::caret_segment`]).
    pub fn caret(&self, offset: usize) -> (f32, f32, f32) {
        for c in &self.clusters {
            if c.range.start == offset
                && let Some(l) = self.lines.get(c.line)
            {
                let x = if c.rtl { c.x + c.advance } else { c.x };
                return (x, l.baseline - l.ascent, l.baseline + l.descent);
            }
        }
        // End of a line (or empty line): after the last cluster of the line containing it.
        let li = self.lines.iter().position(|l| offset >= l.range.start && offset <= l.range.end).unwrap_or(self.lines.len().saturating_sub(1));
        match self.lines.get(li) {
            Some(l) => {
                let x = self
                    .clusters
                    .iter()
                    .filter(|c| c.line == li && c.range.end == offset)
                    .map(|c| if c.rtl { c.x } else { c.x + c.advance })
                    .next()
                    .unwrap_or(l.x1);
                (x, l.baseline - l.ascent, l.baseline + l.descent)
            }
            None => (0.0, 0.0, 0.0),
        }
    }
}

const LRM: &str = "\u{200E}";
const RLM: &str = "\u{200F}";
/// A line break inside a paragraph (Shift+Return), stored as U+0003 in PSD type.
pub(crate) const FORCED_LINE_BREAK: char = '\u{3}';

pub(crate) struct Layouter {
    lcx: LayoutContext<RunBrush>,
    optical: crate::optical::Cache,
}

impl Layouter {
    pub fn new() -> Self {
        Self { lcx: LayoutContext::new(), optical: crate::optical::Cache::default() }
    }

    /// Lays out text runs into positioned glyphs, shaped lines, and decoration rectangles at target DPI.
    pub fn layout(&mut self, fonts: &mut FontDb, t: &TextLayer, dpi: f32) -> TextLayout {
        let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
        let runs = t.char_runs();
        let paras = t.paragraph_runs();
        let vertical = t.orientation == Orientation::Vertical;
        // Dynamic Text: a word gap widens each space (percent of its own width) and a line gap
        // replaces the leading (percent of the font size). None lays out exactly as before.
        let gaps = t.fit.as_ref().map(TextFit::gaps);
        let line_gap = gaps.map(|(_, line)| line);
        let mut out = TextLayout { px_per_pt: k, vertical, ..Default::default() };
        let styles = resolve_run_styles(fonts, &runs);
        let run_starts = running_starts(runs.iter().map(|r| r.len));
        let para_starts = running_starts(paras.iter().map(|r| r.len));
        let (box_rect, is_box) = match t.shape {
            TextShape::Box { x, y, width, height } => ((x, y, width, height), true),
            // Path text is laid out as one point-text line, then bent onto the path. Area text
            // places its own lines (`area` below).
            TextShape::Point | TextShape::Path { .. } | TextShape::Area { .. } => ((0.0, 0.0, 0.0, 0.0), false),
        };
        // Area type: the shape its lines wrap inside. A path that encloses nothing (only from a
        // hostile file; `type.fillPath` refuses one) lays out as point text.
        let area = match &t.shape {
            TextShape::Area { path } => crate::area::AreaShape::new(path, vertical),
            _ => None,
        };
        // Fixed for the whole layout and indexed by run (never by a scaled copy below).
        let small_caps = small_caps_modes(fonts, &styles);
        let text = t.text.as_str();
        // Dynamic Text lines (the fit solver's output): each line starts where the solver chose
        // and is laid out at its own scale, without wrapping. Empty until the first refresh
        // solves them; point and path text don't use them.
        let fit_lines: &[FitLine] = match &t.fit {
            Some(f) if is_box || area.is_some() => f.valid_lines(text),
            _ => &[],
        };
        let units = break_units(text, fit_lines);
        // Each unit's style for each run: its own, or for a solved line not at scale 1, a copy at
        // that line's scale (appended to the styles, so render reads the scaled baseline shift).
        // Only runs the unit touches are copied; `unit_styles[u][i]` is run `first_run + i`.
        let mut styles = styles;
        let unit_styles: Vec<Vec<usize>> = units
            .iter()
            .map(|unit| {
                if unit.scale == 1.0 {
                    return Vec::new();
                }
                let first_run = run_index_at(&run_starts, unit.range.start);
                let end = run_starts.partition_point(|&s| s < unit.content_end).max(first_run + 1).min(runs.len());
                (first_run..end)
                    .map(|ri| {
                        let (rs, re) = (run_starts[ri], run_starts[ri] + runs[ri].len);
                        let touches = ri == first_run || (rs < unit.content_end && re > unit.range.start);
                        match styles.get(ri) {
                            Some(st) if touches => {
                                let scaled = scaled_style(st, unit.scale);
                                styles.push(scaled);
                                styles.len() - 1
                            }
                            _ => ri,
                        }
                    })
                    .collect()
            })
            .collect();
        let env = Env {
            text,
            runs: &runs,
            styles: &styles,
            run_starts: &run_starts,
            small_caps: &small_caps,
            k,
            vertical,
            is_box,
            // Box and area text wrap, align within their lines and hide what doesn't fit.
            wraps: is_box || area.is_some(),
            box_rect,
            path_range: crate::warp::path_range_length(&t.shape).map(|n| n.min(f32::MAX as f64) as f32),
            area,
            solved: !fit_lines.is_empty(),
            line_gap,
            word_gap: gaps.map(|(word, _)| word),
        };
        let mut flow = Flow::default();
        // Per paragraph, shared by its units: its index, the font fallback stack and, for a
        // paragraph split into solved lines, its first strong direction.
        let mut para_cache: Option<(usize, Vec<String>, Option<bool>)> = None;
        for (unit, unit_style) in units.iter().zip(unit_styles) {
            if flow.stop {
                break;
            }
            let pi = unit.paragraph;
            // One style per paragraph, from its start: a run edge inside a paragraph (hand-edited or
            // imported `paragraphs`) must not restyle its later solved lines.
            let ps = paras[run_index_at(&para_starts, unit.paragraph_text.start)].style.clone();
            if para_cache.as_ref().is_none_or(|c| c.0 != pi) {
                let ptext_all = text.get(unit.paragraph_text.clone()).unwrap_or("");
                // Scripts the loaded fonts may lack (Arabic, Japanese, …): a host serving a font
                // for them fetches it (`served`); it joins the fallback stack once it arrives.
                crate::served::request_for_text(ptext_all);
                let fallback = fonts.fallback_stack().map(str::to_string).collect();
                let split = !(unit.first && unit.last);
                let rtl = if split && ps.direction == photocraft_doc::text::TextDirection::Auto { first_strong_rtl(ptext_all) } else { None };
                para_cache = Some((pi, fallback, rtl));
            }
            let Some((_, fallback, para_rtl)) = para_cache.as_ref() else { continue };
            let para = Para::new(&env, shaping::UnitInput { unit, unit_style, fallback: fallback.clone(), para_rtl: *para_rtl }, ps);
            self.layout_paragraph(fonts, &mut out, &mut flow, para);
        }
        out.styles = styles;
        out
    }

    /// Breaks one paragraph (or solved Dynamic Text line) and stacks its lines below the previous one's.
    fn layout_paragraph(&mut self, fonts: &mut FontDb, out: &mut TextLayout, flow: &mut Flow, para: Para<'_>) {
        let env = para.env;
        // Whether hiding the text from `from` on hides anything but line and paragraph breaks
        // (an empty last paragraph below the box is not overflow).
        let hides_text = |from: usize| env.text.get(from..).is_none_or(|r| r.chars().any(|c| !matches!(c, '\r' | '\n' | FORCED_LINE_BREAK)));
        let hide = |out: &mut TextLayout, from: usize| {
            let h = hides_text(from);
            out.hidden |= h;
            out.overflowed |= h;
        };
        let (mut layout, area_slots, area_full) = para.break_paragraph(&mut self.lcx, &mut fonts.fcx, flow);
        layout.align(para.alignment(), para.align_options());
        if flow.prev_baseline.is_some() && para.first {
            flow.pending_space += para.ps.space_before_pt * env.k;
        }
        let nlines = layout.len();
        // End of the text this paragraph has shown so far.
        let mut shown_end = para.start;
        for (li, line) in layout.lines().enumerate() {
            // Area type: the slot the breaker gave this line.
            let slot = area_slots.get(li).copied();
            if env.area.is_some() && slot.is_none() {
                hide(out, para.map_offset(line.text_range().start));
                flow.stop = true;
                break;
            }
            let Some(frame) = para.frame_line(&line, li, nlines, slot, flow) else {
                hide(out, para.map_offset(line.text_range().start));
                flow.stop = true;
                break;
            };
            para.place_line(&mut self.optical, out, &line, &frame);
            shown_end = shown_end.max(para.map_offset(line.text_range().end));
        }
        if area_full {
            // The shape is full: the rest of this paragraph and later ones are hidden.
            hide(out, shown_end);
            flow.stop = true;
        }
        if para.last {
            flow.pending_space += para.ps.space_after_pt * env.k;
        }
    }
}

/// What a layout run shares between its paragraphs.
struct Env<'a> {
    text: &'a str,
    runs: &'a [photocraft_doc::text::TextRun],
    styles: &'a [CharStyle],
    /// Start offset of each run.
    run_starts: &'a [usize],
    small_caps: &'a [SmallCapsMode],
    k: f32,
    vertical: bool,
    is_box: bool,
    /// Box and area text wrap, align within their lines and hide what doesn't fit.
    wraps: bool,
    box_rect: (f32, f32, f32, f32),
    /// Available explicit path span in text-space pixels, capped by the curve's capacity.
    /// `None` keeps legacy alignment around the start anchor (an alignment span of zero).
    path_range: Option<f32>,
    area: Option<crate::area::AreaShape>,
    /// Dynamic Text lines are solved: each unit is one line, laid out without wrapping.
    solved: bool,
    /// Dynamic Text's line gap (percent of the font size), replacing the leading.
    line_gap: Option<f32>,
    /// Dynamic Text's word gap (percent of each space's own width).
    word_gap: Option<f32>,
}

impl Env<'_> {
    /// Index of the run (and style) containing byte offset `off`.
    fn style_at(&self, off: usize) -> usize {
        run_index_at(self.run_starts, off)
    }
}

/// Where the previous paragraph left off.
#[derive(Default)]
struct Flow {
    prev_baseline: Option<f32>,
    /// The previous line's descent and largest size (px): a solved Dynamic Text line is
    /// stacked from it, so a big line's descenders never reach the smaller line below.
    prev_line: Option<(f32, f32)>,
    pending_space: f32,
    stop: bool,
}

/// Text laid out as one parley paragraph: a whole paragraph, or with solved Dynamic Text
/// lines, one line of it.
struct Unit {
    paragraph: usize,
    /// Byte range in the layer text; the paragraph's last unit includes its break.
    range: Range<usize>,
    /// End of the text shown (without a paragraph break, or a forced line break the unit ends at).
    content_end: usize,
    scale: f32,
    /// The paragraph's text without its break.
    paragraph_text: Range<usize>,
    /// The paragraph's first unit (first-line indent, space before) and last (space after).
    first: bool,
    last: bool,
}

/// Paragraphs, each split at the starts of `lines` (valid solved Dynamic Text lines, else empty)
/// that fall inside its text. A unit takes the scale of the line it starts in.
fn break_units(text: &str, lines: &[FitLine]) -> Vec<Unit> {
    let scale_at = |off: usize| {
        let i = lines.partition_point(|l| l.start as usize <= off);
        i.checked_sub(1).and_then(|i| lines.get(i)).map_or(1.0, |l| l.scale)
    };
    let mut out = Vec::new();
    for (paragraph, prange) in split_paragraphs(text).into_iter().enumerate() {
        let content_end = strip_break(text, &prange);
        let mut cuts = vec![prange.start];
        cuts.extend(lines.iter().map(|l| l.start as usize).filter(|&s| s > prange.start && s < content_end));
        for (j, &a) in cuts.iter().enumerate() {
            let (range, end) = match cuts.get(j + 1) {
                None => (a..prange.end, content_end),
                // The solved break replaces a forced line break it follows.
                Some(&z) => (a..z, if text.get(a..z).is_some_and(|s| s.ends_with(FORCED_LINE_BREAK)) { z - FORCED_LINE_BREAK.len_utf8() } else { z }),
            };
            out.push(Unit {
                paragraph,
                range,
                content_end: end,
                scale: scale_at(a),
                paragraph_text: prange.start..content_end,
                first: j == 0,
                last: j + 1 == cuts.len(),
            });
        }
    }
    out
}

/// Whether a paragraph of `t` lays out right to left: its paragraph style (the one at its start,
/// as layout reads it) sets Right to Left, or Auto with a right-to-left first strong character
/// in that paragraph. Dynamic Text refuses such text; the engine's Convert and PSD export both
/// ask this, so they agree on which layers the solver may fit.
pub fn has_rtl_paragraph(t: &TextLayer) -> bool {
    use photocraft_doc::text::TextDirection;
    let paras = t.paragraph_runs();
    let starts = running_starts(paras.iter().map(|r| r.len));
    split_paragraphs(&t.text).into_iter().any(|r| {
        let Some(p) = paras.get(run_index_at(&starts, r.start)) else { return false };
        match p.style.direction {
            TextDirection::Rtl => true,
            TextDirection::Ltr => false,
            TextDirection::Auto => t.text.get(r.start..strip_break(&t.text, &r)).and_then(first_strong_rtl) == Some(true),
        }
    })
}

/// Whether the first strong character of `s` is right-to-left (UAX #9 P2), or `None` when it has
/// none. Dynamic Text uses it to refuse Auto-direction paragraphs that lay out right to left.
/// As P2 says, characters between an isolate initiator (LRI, RLI, FSI) and its matching PDI are
/// skipped, and an unmatched initiator skips to the end: the layout's own bidi pass detects the
/// paragraph's direction that way, so a paragraph opening with an isolated Latin name before its
/// Hebrew text is right-to-left here too.
pub fn first_strong_rtl(s: &str) -> Option<bool> {
    use icu_properties::{CodePointMapData, props::BidiClass};
    let classes = CodePointMapData::<BidiClass>::new();
    let mut isolates = 0usize;
    for c in s.chars() {
        match classes.get(c) {
            BidiClass::LeftToRightIsolate | BidiClass::RightToLeftIsolate | BidiClass::FirstStrongIsolate => isolates = isolates.saturating_add(1),
            BidiClass::PopDirectionalIsolate => isolates = isolates.saturating_sub(1),
            _ if isolates > 0 => {}
            BidiClass::LeftToRight => return Some(false),
            BidiClass::RightToLeft | BidiClass::ArabicLetter => return Some(true),
            _ => {}
        }
    }
    None
}

/// `st` at `scale` times its size, clamped to the type size range; explicit leading and baseline
/// shift follow the size.
pub(crate) fn scaled_style(st: &CharStyle, scale: f32) -> CharStyle {
    let mut s = st.clone();
    if st.size_pt.is_finite() && st.size_pt > 0.0 {
        s.size_pt = (st.size_pt * scale).clamp(crate::fit::MIN_PT, crate::fit::MAX_PT);
        let r = s.size_pt / st.size_pt;
        s.leading_pt = st.leading_pt.map(|l| l * r);
        s.baseline_shift_pt = st.baseline_shift_pt * r;
    }
    s
}

/// The spaces parley applies word spacing to (U+0020 and no-break space, tabs excluded) in a
/// broken layout of `ptext`: each one's range and its shaped advance (px, its glyphs' own
/// advances, without letter spacing).
fn space_advances(layout: &Layout<RunBrush>, ptext: &str, tabs: &[usize]) -> Vec<(Range<usize>, f32)> {
    let mut out: Vec<(Range<usize>, f32)> = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            for c in gr.run().visual_clusters() {
                let r = c.text_range();
                if !matches!(ptext.get(r.clone()), Some(" " | "\u{a0}")) || tabs.binary_search(&r.start).is_ok() {
                    continue;
                }
                out.push((r, c.glyphs().map(|g| g.advance).sum()));
            }
        }
    }
    // A run split over several items lists its clusters more than once.
    out.sort_by_key(|(r, _)| r.start);
    out.dedup_by_key(|(r, _)| r.start);
    out
}

/// Where one line of area type goes: its baseline, the left edge of its span, and the width
/// it may fill (the span less the paragraph's indents), plus the row and span it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    baseline: f32,
    x: f32,
    width: f32,
    row: usize,
    span: usize,
}

#[cfg(test)]
mod tests {
    use super::{SmallCapsMode, feature_list, small_caps_mode, variation_list};
    use photocraft_doc::TextLayer;
    use photocraft_doc::text::{Caps, CharStyle, FontVariation, TextRun};

    /// Reference: the lookup the layout used before it became a binary search.
    fn run_index_reference(starts: &[usize], off: usize) -> usize {
        starts.iter().rposition(|&s| s <= off).unwrap_or(0)
    }

    #[test]
    fn run_lookup_matches_the_linear_reference() {
        // Deterministic pseudo-random run layouts with empty runs, equal adjacent styles and
        // offsets past the end.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % m) as usize
        };
        for _ in 0..300 {
            let n = next(12);
            let mut acc = 0;
            let starts: Vec<usize> = (0..n)
                .map(|_| {
                    let s = acc;
                    acc += next(4); // 0 = empty run
                    s
                })
                .collect();
            for off in 0..acc + 4 {
                assert_eq!(super::run_index_at(&starts, off), run_index_reference(&starts, off), "{starts:?} @ {off}");
            }
        }
        assert_eq!(super::run_index_at(&[], 5), 0);
    }

    /// Many runs and paragraphs: the font is probed once per small-caps style per layout, not
    /// once per style per paragraph, and styles that don't use small caps are never probed.
    #[test]
    fn small_caps_probe_runs_once_per_small_caps_style() {
        let plain = CharStyle { font_family: "Inter".into(), ..Default::default() };
        let small = CharStyle { caps: Caps::SmallCaps, ..plain.clone() };
        let text: String = "a\n".repeat(300);
        let runs = |small_every: usize| -> Vec<TextRun> {
            (0..text.len()).map(|i| TextRun { len: 1, style: if small_every > 0 && i % small_every == 0 { small.clone() } else { plain.clone() } }).collect()
        };
        let mut engine = crate::TextEngine::new();
        let mut probes = |small_every: usize| {
            super::SMCP_PROBES.with(|c| c.set(0));
            let layer = TextLayer { text: text.clone(), runs: runs(small_every), ..Default::default() };
            let _ = engine.layout(&layer, 72.0);
            super::SMCP_PROBES.with(|c| c.get())
        };
        assert_eq!(probes(0), 0);
        let styles = text.len().div_ceil(2); // every other index is a small-caps run
        assert_eq!(probes(2), styles);
    }

    /// A variable font's optical size follows the type size unless the style sets `opsz` (#2985).
    #[test]
    fn optical_size_follows_the_type_size_unless_set() {
        let style = CharStyle { size_pt: 36.0, variations: vec![FontVariation { axis: "wght".into(), value: 650.0 }], ..Default::default() };
        assert_eq!(variation_list(&style), ["\"wght\" 650", "\"opsz\" 36"]);
        let set = CharStyle { size_pt: 36.0, variations: vec![FontVariation { axis: "opsz".into(), value: 12.0 }], ..Default::default() };
        assert_eq!(variation_list(&set), ["\"opsz\" 12"]);
    }

    #[test]
    fn uses_real_small_caps_only_when_the_selected_face_supports_smcp() {
        let style = CharStyle { caps: Caps::SmallCaps, ..Default::default() };
        let real = small_caps_mode(style.caps, true);
        let synthetic = small_caps_mode(style.caps, false);

        assert_eq!(real, SmallCapsMode::OpenType);
        assert!(feature_list(&style, real).iter().any(|feature| feature == "\"smcp\" 1"));
        assert_eq!(synthetic, SmallCapsMode::Synthetic);
        assert!(!feature_list(&style, synthetic).iter().any(|feature| feature == "\"smcp\" 1"));
    }
}
