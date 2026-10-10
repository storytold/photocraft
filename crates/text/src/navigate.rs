//! Caret movement, caret geometry and selection shapes for laid-out type (stage 1b).
//!
//! **Approach C** (spec §3): parley decides the *order* of the caret stops on a line, through its
//! bidi-aware visual cursor, and PhotoCraft's [`crate::ClusterInfo`] decides their *geometry*:
//! the layout moves glyphs after parley (alignment, indents, kerning, "Justify all") without
//! changing their visual order. Lines are crossed here rather than by parley, whose cursor leaves
//! a line left to right and top to bottom whatever the paragraph direction (backwards in RTL).
//!
//! Positions are byte offsets in the layer text; geometry is in line space (see
//! [`crate::TextLayout::to_text`]). Pure functions of a layout and its text, shared by the Type
//! tool and the engine's `type.*` caret commands. Nothing here panics: indices go through `get`,
//! and every loop is bounded.

use parley::Layout;

use crate::layout::{ClusterInfo, FORCED_LINE_BREAK, RunBrush, TextLayout};

/// A paragraph's parley layout, kept for caret movement (hidden lines of an overflowing box
/// included). parley's text equals the layer's `start..end` after a `prefix`-byte LRM/RLM (0 or 3
/// bytes, from the paragraph direction), so `parley offset = layer offset − start + prefix`.
#[derive(Clone, Debug)]
pub(crate) struct ParagraphNav {
    pub(crate) layout: Layout<RunBrush>,
    /// Layer byte range of the paragraph's text, without its break.
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Length of the direction mark parley's text starts with.
    pub(crate) prefix: usize,
}

/// A caret: a byte offset in the layer text and the side it belongs to.
///
/// Where an Arabic and a Latin run meet, or at the end of a wrapped line, one offset has two
/// places on screen. `upstream` is the place after the character before `byte`, where a caret
/// moving forward arrives and where a click on a character's trailing half lands. Otherwise the
/// caret is before the character at `byte`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caret {
    pub byte: usize,
    pub upstream: bool,
}

impl Caret {
    pub fn new(byte: usize, upstream: bool) -> Self {
        Self { byte, upstream }
    }
}

/// Where a caret is drawn, in line space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaretGeom {
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
    /// Index into `TextLayout::lines`.
    pub line: usize,
}

/// Where caret `c` is drawn: after the cluster ending at its byte (upstream), or before the
/// cluster starting there (downstream), on the side the cluster's direction puts that edge. It
/// falls back to the other side, then to [`TextLayout::caret`] (inside a grapheme, an empty line,
/// hidden text). A forced line break is never the cluster a caret comes after.
pub fn caret_geometry(l: &TextLayout, text: &str, c: Caret) -> CaretGeom {
    let before = || l.clusters.iter().find(|k| k.range.end == c.byte && !is_hard_break(text, k)).map(|k| (k, true));
    let after = || l.clusters.iter().find(|k| k.range.start == c.byte).map(|k| (k, false));
    let pick = if c.upstream { before().or_else(after) } else { after().or_else(before) };
    if let Some((k, trailing)) = pick
        && let Some(ln) = l.lines.get(k.line)
    {
        // An RTL cluster's trailing edge is its left side.
        let x = if trailing == k.rtl { k.x } else { k.x + k.advance };
        return CaretGeom { x, top: ln.baseline - ln.ascent, bottom: ln.baseline + ln.descent, line: k.line };
    }
    let (x, top, bottom) = l.caret(c.byte);
    CaretGeom { x, top, bottom, line: crate::layout::line_index(l, c.byte) }
}

/// [`caret_geometry`] as a text-space segment (top end, bottom end).
pub fn caret_segment(l: &TextLayout, text: &str, c: Caret) -> [(f32, f32); 2] {
    let g = caret_geometry(l, text, c);
    [l.to_text(g.x, g.top), l.to_text(g.x, g.bottom)]
}

/// The caret nearest to a text-space point, with the side clicked: the existing direction-aware
/// hit test ([`TextLayout::hit_test`]) plus affinity.
pub fn hit(l: &TextLayout, text: &str, x: f32, y: f32) -> Caret {
    let (x, y) = l.to_line(x, y);
    let Some(li) = l.nearest_line(y) else { return Caret::default() };
    let (byte, upstream) = l.hit_in_line(li, x);
    not_after_break(l, text, li, Caret::new(byte, upstream))
}

/// A caret on line `li` after its forced line break would be drawn on the next line: put it
/// before the break instead.
fn not_after_break(l: &TextLayout, text: &str, li: usize, c: Caret) -> Caret {
    match l.clusters.iter().find(|k| c.upstream && k.line == li && k.range.end == c.byte && is_hard_break(text, k)) {
        Some(k) => Caret::new(k.range.start, false),
        None => c,
    }
}

/// A cluster that is a line break (a forced line break inside a paragraph).
fn is_hard_break(text: &str, c: &ClusterInfo) -> bool {
    text.get(c.range.clone()).is_some_and(|s| !s.is_empty() && s.chars().all(|ch| ch == FORCED_LINE_BREAK || ch == '\n' || ch == '\r'))
}
