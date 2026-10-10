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

use parley::{Affinity, Cursor, Layout};

pub use crate::segment::first_strong_rtl;

use crate::layout::{ClusterInfo, FORCED_LINE_BREAK, LineInfo, RunBrush, TextLayout};

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

impl ParagraphNav {
    /// parley's cursor for a layer caret in this paragraph.
    fn cursor(&self, c: Caret) -> Cursor {
        let i = c.byte.saturating_sub(self.start).saturating_add(self.prefix);
        // At the paragraph start the character before is the prefix or the previous paragraph's
        // break, not a layer character: downstream.
        let affinity = if c.upstream && c.byte > self.start { Affinity::Upstream } else { Affinity::Downstream };
        Cursor::from_byte_index(&self.layout, i, affinity)
    }

    /// The layer caret for a parley cursor; `None` inside the direction prefix.
    fn to_layer(&self, c: Cursor) -> Option<Caret> {
        let i = c.index().checked_sub(self.prefix)?;
        let byte = self.start.saturating_add(i).min(self.end);
        Some(Caret::new(byte, c.affinity() == Affinity::Upstream && byte > self.start))
    }
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
    // An empty line owns its offset (a forced break ending the text leaves one after it).
    if let Some((line, ln)) = l.lines.iter().enumerate().find(|(_, ln)| ln.range.is_empty() && ln.range.start == c.byte) {
        return CaretGeom { x: ln.x0, top: ln.baseline - ln.ascent, bottom: ln.baseline + ln.descent, line };
    }
    let (x, top, bottom) = l.caret(c.byte);
    CaretGeom { x, top, bottom, line: crate::layout::line_index(l, c.byte) }
}

/// [`caret_geometry`] as a text-space segment (top end, bottom end).
pub fn caret_segment(l: &TextLayout, text: &str, c: Caret) -> [(f32, f32); 2] {
    let g = caret_geometry(l, text, c);
    [l.to_text(g.x, g.top), l.to_text(g.x, g.bottom)]
}

/// The caret nearest to a text-space point, with the side clicked. A cluster under the point
/// beats a neighbour whose edge is as near, so a click where directions meet keeps its side.
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

/// Carets closer than this (line-space px) are one visual stop.
const SAME_X: f32 = 0.5;

/// Visual direction of an arrow key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
}

/// How far an arrow key moves: one caret stop (a grapheme), or one word (⌘/Ctrl).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Grapheme,
    Word,
}

/// The caret after one arrow press (spec 5.1.3). Visual: parley orders the stops of the line, and
/// the result skips stops inside the direction prefix, inside a grapheme (parley stops between a
/// letter and its haraka), and at the same x (zero-width bidi controls). At a line's edge it goes
/// to the neighbouring drawn line: with the paragraph's flow (→ in LTR, ← in RTL) to the next
/// line's start edge, against it to the previous line's end edge. This crosses paragraphs, and
/// never reaches the hidden lines of an overflowing box. Vertical type moves in text order.
pub fn step(l: &TextLayout, text: &str, from: Caret, dir: Dir, unit: Unit) -> Caret {
    let from = clamp(l, text, from);
    if l.vertical {
        return clamp(l, text, logical_step(text, from, dir, unit));
    }
    let start = caret_geometry(l, text, from);
    let Some(p) = l.paragraphs.iter().find(|p| (p.start..=p.end).contains(&from.byte)) else { return from };
    let content = text.get(p.start..p.end).unwrap_or("");
    let stops = crate::segment::grapheme_boundaries(content);
    let mut cur = p.cursor(from);
    // Each move passes at least one parley cluster; the bound only guards against a cycle.
    for _ in 0..content.len().saturating_add(p.prefix).saturating_add(8) {
        let next = match (dir, unit) {
            (Dir::Right, Unit::Grapheme) => cur.next_visual(&p.layout),
            (Dir::Left, Unit::Grapheme) => cur.previous_visual(&p.layout),
            (Dir::Right, Unit::Word) => cur.next_visual_word(&p.layout),
            (Dir::Left, Unit::Word) => cur.previous_visual_word(&p.layout),
        };
        if next == cur {
            break;
        }
        cur = next;
        let Some(c) = p.to_layer(cur) else { continue };
        match classify(l, text, p, &stops, start, c) {
            Visit::Stop => return c,
            Visit::Skip => {}
            Visit::Leave => break,
        }
    }
    cross(l, text, from, start, dir)
}

/// What [`step`] does with the caret parley's cursor reached.
enum Visit {
    /// A caret stop: the step ends here.
    Stop,
    /// Not a stop for the user: keep moving.
    Skip,
    /// Past the line (or the drawn text): stop moving and cross the line instead.
    Leave,
}

/// Classifies layer caret `c`, reached from `start` on the same paragraph `p` (whose grapheme
/// boundaries are `stops`): hidden text and other lines leave; a position between a letter and its
/// marks, or at the same x as `start` (a zero-width bidi control), is skipped.
fn classify(l: &TextLayout, text: &str, p: &ParagraphNav, stops: &[usize], start: CaretGeom, c: Caret) -> Visit {
    if c.byte > drawn_end(l) {
        return Visit::Leave;
    }
    let g = caret_geometry(l, text, c);
    if g.line != start.line {
        return Visit::Leave;
    }
    let inside_grapheme = stops.binary_search(&c.byte.saturating_sub(p.start)).is_err();
    if inside_grapheme || (g.x - start.x).abs() < SAME_X {
        return Visit::Skip;
    }
    Visit::Stop
}

/// Vertical type keeps moving in text order (RTL in vertical type is a non-goal): → / ↓ forward.
fn logical_step(text: &str, from: Caret, dir: Dir, unit: Unit) -> Caret {
    let idx = crate::layout::char_index(text, from.byte);
    let forward = dir == Dir::Right;
    let to = match unit {
        Unit::Grapheme => crate::layout::grapheme_step(text, idx, forward),
        Unit::Word => crate::layout::word_boundary(text, idx, forward),
    };
    Caret::new(crate::layout::byte_index(text, to), false)
}

/// Leaves the caret's line in `dir`. A word step that stopped short goes to the line's edge
/// first; at the edge the caret goes to the neighbouring drawn line (see [`step`]).
fn cross(l: &TextLayout, text: &str, from: Caret, start: CaretGeom, dir: Dir) -> Caret {
    let Some(line) = l.lines.get(start.line) else { return from };
    let right = dir == Dir::Right;
    if let Some(edge) = edge_caret(l, text, start.line, right)
        && (caret_geometry(l, text, edge).x - start.x).abs() >= SAME_X
    {
        return snap_to_grapheme(text, edge);
    }
    let forward = right != line.rtl;
    let target = if forward { start.line.checked_add(1) } else { start.line.checked_sub(1) };
    let Some((li, next)) = target.and_then(|i| l.lines.get(i).map(|n| (i, n))) else { return from };
    // Forward: the next line's start edge (its right side in RTL). Backward: the previous line's
    // end edge (its left side in RTL).
    let at_right = forward == next.rtl;
    snap_to_grapheme(text, edge_caret(l, text, li, at_right).unwrap_or(Caret::new(next.range.start, false)))
}

/// The caret on the left or right visual edge of line `li`: outside its outermost cluster
/// (trailing whitespace included, forced line breaks not). An empty line: its start.
fn edge_caret(l: &TextLayout, text: &str, li: usize, right: bool) -> Option<Caret> {
    let on_line = || l.clusters.iter().filter(|c| c.line == li && !is_hard_break(text, c));
    let outer = if right { on_line().max_by(|a, b| (a.x + a.advance).total_cmp(&(b.x + b.advance))) } else { on_line().min_by(|a, b| a.x.total_cmp(&b.x)) };
    let Some(c) = outer else { return l.lines.get(li).map(|ln| Caret::new(ln.range.start, false)) };
    // The outer side is the cluster's trailing edge when it is its end.
    let trailing = right != c.rtl;
    Some(Caret::new(if trailing { c.range.end } else { c.range.start }, trailing))
}

/// The grapheme boundary at or after `c`: parley 0.11 can wrap a line inside an RTL grapheme
/// (1a follow-up), leaving a line edge between a letter and its mark.
fn snap_to_grapheme(text: &str, c: Caret) -> Caret {
    let b = crate::segment::grapheme_boundaries(text);
    match b.binary_search(&c.byte) {
        Ok(_) => c,
        Err(i) => b.get(i).map_or(c, |&byte| Caret::new(byte, false)),
    }
}

/// End of the drawn text: an overflowing box draws fewer lines than parley keeps.
fn drawn_end(l: &TextLayout) -> usize {
    l.lines.last().map_or(0, |ln| ln.range.end)
}

/// `c` normalised: on a char boundary, not between the CR and LF of a CR LF (no paragraph
/// contains that offset), inside the drawn text, and not upstream right after a forced line break
/// (the break is never the cluster a caret comes after, see [`caret_geometry`]).
fn clamp(l: &TextLayout, text: &str, c: Caret) -> Caret {
    let mut b = c.byte.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    let between_cr_lf = text.get(..b).is_some_and(|s| s.ends_with('\r')) && text.get(b..).is_some_and(|s| s.starts_with('\n'));
    if between_cr_lf {
        b -= 1;
    }
    let end = drawn_end(l);
    if b > end {
        return Caret::new(end, end > 0);
    }
    let after_break = text.get(..b).is_some_and(|s| s.ends_with(FORCED_LINE_BREAK));
    Caret::new(b, c.upstream && !after_break)
}

/// Home (`end` false) or End of the caret's line: its logical start, or its logical end with the
/// caret kept on this line (upstream). A forced line break ends a line before itself.
pub fn home_end(l: &TextLayout, text: &str, from: Caret, end: bool) -> Caret {
    let from = clamp(l, text, from);
    let Some(ln) = l.lines.get(caret_geometry(l, text, from).line) else { return from };
    if !end {
        return Caret::new(ln.range.start, false);
    }
    if let Some(b) = break_after(text, ln).filter(|&b| b < ln.range.end) {
        return Caret::new(b, false);
    }
    Caret::new(ln.range.end, ln.range.end > ln.range.start)
}

/// ↑ (`dir` < 0) or ↓: the caret at line-space `x` on the neighbouring drawn line, with the side
/// it lands on. Above the first line: the text start; below the last drawn line: its end.
pub fn adjacent_line(l: &TextLayout, text: &str, from: Caret, x: f32, dir: i32) -> Caret {
    let from = clamp(l, text, from);
    let line = caret_geometry(l, text, from).line;
    let target = if dir < 0 { line.checked_sub(1) } else { line.checked_add(1).filter(|&i| i < l.lines.len()) };
    match target {
        Some(li) => {
            let (byte, upstream) = l.hit_in_line(li, x);
            not_after_break(l, text, li, Caret::new(byte, upstream))
        }
        None if dir < 0 => Caret::new(0, false),
        None => {
            let end = drawn_end(l);
            Caret::new(end, end > 0)
        }
    }
}

/// Where ←/→ without Shift put the caret when text is selected from `anchor` to `focus`: the end
/// lying further in `dir`. On one line, the one further left or right; across lines, the later
/// line is further in its paragraph's flow (→ in LTR, ← in RTL).
pub fn collapse(l: &TextLayout, text: &str, anchor: Caret, focus: Caret, dir: Dir) -> Caret {
    let (ga, gf) = (caret_geometry(l, text, anchor), caret_geometry(l, text, focus));
    let right = dir == Dir::Right;
    if ga.line == gf.line {
        return if (gf.x > ga.x) == right { focus } else { anchor };
    }
    let rtl = l.lines.get(gf.line).is_some_and(|ln| ln.rtl);
    let (earlier, later) = if ga.line < gf.line { (anchor, focus) } else { (focus, anchor) };
    if right != rtl { later } else { earlier }
}

/// One rectangle of a selection highlight, in line space: `x0..x1` across line `line`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionSegment {
    pub line: usize,
    pub x0: f32,
    pub x1: f32,
    /// The marker for a selected line break after the line's text, not text itself.
    pub line_break: bool,
}

/// The highlight of the selection between byte offsets `a` and `b` (either order). Per line, its
/// selected clusters are sorted by x and merged where they touch, so a selection across a
/// direction change shows as one rectangle per visual piece. A selected hard line break
/// (paragraph or forced) adds a marker on the paragraph's end side: right of the line in LTR,
/// left of it in RTL (spec 5.1.6).
pub fn selection_segments(l: &TextLayout, text: &str, a: usize, b: usize) -> Vec<SelectionSegment> {
    const TOUCH: f32 = 0.5;
    let (lo, hi) = (a.min(b), a.max(b));
    let mut out = Vec::new();
    if lo == hi {
        return out;
    }
    for (li, ln) in l.lines.iter().enumerate() {
        let mut xs: Vec<(f32, f32)> = l
            .clusters
            .iter()
            .filter(|c| c.line == li && c.range.start >= lo && c.range.end <= hi && !is_hard_break(text, c))
            .map(|c| (c.x, c.x + c.advance))
            .collect();
        xs.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (x0, x1) in xs {
            match merged.last_mut() {
                Some(m) if x0 <= m.1 + TOUCH => m.1 = m.1.max(x1),
                _ => merged.push((x0, x1)),
            }
        }
        out.extend(merged.into_iter().filter(|(x0, x1)| x1 > x0).map(|(x0, x1)| SelectionSegment { line: li, x0, x1, line_break: false }));
        if let Some(brk) = break_after(text, ln)
            && lo <= brk
            && brk < hi
        {
            let w = (ln.ascent + ln.descent) * 0.25;
            let on_line = l.clusters.iter().filter(|c| c.line == li);
            if ln.rtl {
                let edge = on_line.map(|c| c.x).fold(ln.x0, f32::min);
                out.push(SelectionSegment { line: li, x0: edge - w, x1: edge, line_break: true });
            } else {
                let edge = on_line.map(|c| c.x + c.advance).fold(ln.x1, f32::max);
                out.push(SelectionSegment { line: li, x0: edge, x1: edge + w, line_break: true });
            }
        }
    }
    out
}

/// Byte of the hard line break ending line `ln`: a forced line break as its last character, or
/// the paragraph break after it. `None` for a soft wrap or the end of the text.
fn break_after(text: &str, ln: &LineInfo) -> Option<usize> {
    if text.get(ln.range.clone()).and_then(|s| s.chars().next_back()) == Some(FORCED_LINE_BREAK) {
        return ln.range.end.checked_sub(FORCED_LINE_BREAK.len_utf8());
    }
    matches!(text.get(ln.range.end..).and_then(|s| s.chars().next()), Some('\n' | '\r')).then_some(ln.range.end)
}
