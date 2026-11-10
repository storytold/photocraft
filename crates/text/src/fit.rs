//! Dynamic Text: chooses a box or area layer's lines and sizes each one to span its width
//! (Photoshop's Dynamic Text), and bakes the result into ordinary type.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{FitLine, TextFit, TextRun, TextShape};

pub const MIN_PT: f32 = 0.1;
pub const MAX_PT: f32 = 1296.0;
pub const MAX_PROBES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitResult {
    pub scale: f32,
    pub fits: bool,
    pub probes: usize,
}

const UNTOUCHED: FitResult = FitResult { scale: 1.0, fits: true, probes: 0 };

fn is_fitted(layer: &TextLayer) -> bool {
    layer.fit.is_some() && !layer.text.is_empty() && matches!(layer.shape, TextShape::Box { .. } | TextShape::Area { .. })
}

/// The layer's own size when finite, else MIN_PT: what a non-finite size is replaced with.
fn sane_fallback(layer: &TextLayer) -> f32 {
    if layer.size_pt.is_finite() { layer.size_pt.clamp(MIN_PT, MAX_PT) } else { MIN_PT }
}

fn sane(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}

/// Replaces only invalid (non-finite) sizes and lengths, as [`apply_scale`] does: sizes become
/// the layer's size (or MIN_PT), a leading becomes auto, shifts, indents and spacing become 0.
/// Every finite value, the user's base sizes included, is left as it is.
fn replace_non_finite(layer: &mut TextLayer) {
    let fallback = sane_fallback(layer);
    let length = |v: &mut f32| {
        if !v.is_finite() {
            *v = 0.0;
        }
    };
    layer.size_pt = sane(layer.size_pt, fallback);
    for run in &mut layer.runs {
        let s = &mut run.style;
        s.size_pt = sane(s.size_pt, fallback);
        s.leading_pt = s.leading_pt.filter(|v| v.is_finite());
        length(&mut s.baseline_shift_pt);
    }
    for para in &mut layer.paragraphs {
        let p = &mut para.style;
        for v in [&mut p.first_line_indent_pt, &mut p.start_indent_pt, &mut p.end_indent_pt, &mut p.space_before_pt, &mut p.space_after_pt] {
            length(v);
        }
    }
}

/// Multiplies `size_pt` and every run's `size_pt` (and explicit `leading_pt`, `baseline_shift_pt`)
/// by `scale`, clamping each size to MIN_PT..=MAX_PT. Keeps run ratios except where clamped.
/// Paragraph indents and spacing scale too (signs kept, so hanging indents stay negative).
/// Non-finite sizes become the layer's size (or MIN_PT) first, so NaN never reaches layout.
pub fn apply_scale(layer: &mut TextLayer, scale: f32) {
    let scale = if scale.is_finite() { scale } else { 1.0 };
    let fallback = sane_fallback(layer);
    let size = |v: f32| (sane(v, fallback) * scale).clamp(MIN_PT, MAX_PT);
    let length = |v: f32| if v.is_finite() { (v * scale).clamp(-MAX_PT, MAX_PT) } else { 0.0 };
    layer.size_pt = size(layer.size_pt);
    for run in &mut layer.runs {
        let s = &mut run.style;
        s.size_pt = size(s.size_pt);
        s.leading_pt = s.leading_pt.filter(|v| v.is_finite()).map(size);
        s.baseline_shift_pt = length(s.baseline_shift_pt);
    }
    for para in &mut layer.paragraphs {
        let p = &mut para.style;
        p.first_line_indent_pt = length(p.first_line_indent_pt);
        p.start_indent_pt = length(p.start_indent_pt);
        p.end_indent_pt = length(p.end_indent_pt);
        p.space_before_pt = length(p.space_before_pt);
        p.space_after_pt = length(p.space_after_pt);
    }
}

/// Box text: [`solve_box`] written to `fit.lines`; Area text: [`solve_area`]. The sizes are left
/// alone (scale 1), and an old file's `area_scale` is kept so toggling off still reverts it.
/// Point/Path, fit None or empty text: untouched.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before calling.
pub fn refit(engine: &mut crate::TextEngine, layer: &mut TextLayer, dpi: f32) -> FitResult {
    if !is_fitted(layer) {
        return UNTOUCHED;
    }
    replace_non_finite(layer);
    let solved = match layer.shape {
        TextShape::Box { .. } => solve(engine, layer, dpi),
        TextShape::Area { .. } => solve_shape(engine, layer, dpi),
        TextShape::Point | TextShape::Path { .. } => return UNTOUCHED,
    };
    if let Some(f) = layer.fit.as_mut() {
        f.lines = solved.lines;
    }
    FitResult { scale: 1.0, fits: solved.fits, probes: solved.probes }
}

/// Whether `layer` is Dynamic Text box or area type with solved lines: what [`bake`] turns into
/// plain type.
pub fn is_fitted_lines(layer: &TextLayer) -> bool {
    layer.fit.as_ref().is_some_and(|f| !f.valid_lines(&layer.text).is_empty()) && matches!(layer.shape, TextShape::Box { .. } | TextShape::Area { .. })
}

/// Whether every glyph of `b` is within 0.5 px of the same glyph of `a`, in order. A fitted
/// layout shapes each solved line alone and has an invisible direction mark per line (and the
/// spaces a break replaced) that the baked layout lacks: those are skipped, at most two per line
/// of `a`, so a baked text that lost a word does not pass.
fn same_glyphs(a: &crate::TextLayout, b: &crate::TextLayout) -> bool {
    if a.glyphs.len().saturating_sub(b.glyphs.len()) > 2 * a.lines.len().max(1) {
        return false;
    }
    let mut from = 0;
    for q in &b.glyphs {
        let found = a.glyphs.get(from..).and_then(|rest| rest.iter().position(|p| p.id == q.id && (p.x - q.x).abs() <= 0.5 && (p.y - q.y).abs() <= 0.5));
        let Some(off) = found else { return false };
        from += off + 1;
    }
    true
}

/// Line shrink factors `bake` tries in turn. A larger shrink moves a wide line's far glyphs
/// more than half a pixel, which the verification refuses anyway.
const MARGINS: [f32; 2] = [1.0 - 1e-5, 0.999];

/// Why [`bake`] cannot keep the fitted look as ordinary type.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// A run is condensed (Horizontal Scale below 100%). Box wrapping measures the unscaled
    /// width, so the baked lines would wrap early.
    Condensed,
    /// The baked text lays out in a different number of lines than the fitted look.
    LineCount { baked: usize, fitted: usize },
    /// The baked text keeps the lines, but glyphs move more than 0.5 px.
    Drift,
    /// The text cannot be cut at a byte range.
    Cut { start: usize, end: usize },
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Condensed => f.write_str("condensed type (Horizontal Scale below 100%) can't be kept yet"),
            Self::LineCount { baked, fitted } => write!(f, "the baked text lays out in {baked} lines, not the fitted {fitted}"),
            Self::Drift => f.write_str("the baked text lays out with glyphs more than 0.5 px from the fitted look"),
            Self::Cut { start, end } => write!(f, "the text cannot be cut at byte {start}..{end}"),
        }
    }
}

impl std::error::Error for BakeError {}

/// Whether a run of `layer`'s text is condensed (Horizontal Scale below 100%), which [`bake`]
/// refuses. Non-positive and non-finite scales lay out at 100%, so they don't count.
pub fn has_condensed_type(layer: &TextLayer) -> bool {
    layer.char_runs().iter().any(|r| r.len > 0 && r.style.horizontal_scale > 0.0 && r.style.horizontal_scale < 1.0)
}

/// Bakes a solved Dynamic Text box into ordinary type: each line's runs get their size times the
/// line's scale (leading and baseline shift follow, as layout does), and a forced line break
/// takes the place of the space each line ends with (or is inserted where the line follows no
/// space). Wrapped text counts a trailing space against the box, and the solved lines span it
/// exactly, so keeping the space would wrap a word early; the break replaces it and the runs
/// stay plain. The line gap becomes each line's explicit leading.
///
/// Area text is baked the same way, except that every solved line becomes its own paragraph (a
/// paragraph break, not a forced line break, ends each line; a forced line break a line follows
/// becomes one too). Area type plans all the rows of a paragraph with its tallest line, so lines
/// of different sizes keep their own spacing only as separate paragraphs. Each piece keeps the
/// paragraph's style, with the first-line indent and space before only on its first piece and
/// the space after only on its last.
///
/// Known limitation: the word gap becomes extra tracking on each space inside a line (read from
/// the difference in that space's measured advance), an approximation of word spacing: it also
/// shows if text is typed in front of that space.
///
/// The result has no `fit` and lays out like the fitted layer (glyphs within 0.5 px); this is
/// what Release and PSD export use. It is verified: when the baked layout does not have the
/// fitted layout's lines, or the text cannot be cut cleanly, the result is an error naming the
/// cause and nothing is returned. When every margin fails and one of them changed the line
/// count, that is the cause reported: it is what a later margin's drift came from. Area text
/// whose first line was moved down past the shape's top row is baked behind an empty first
/// paragraph at the smallest size, with the first line's leading carrying the distance.
///
/// Condensed type (a run with Horizontal Scale below 100%) is refused before baking with
/// [`BakeError::Condensed`]: box wrapping measures its unscaled width.
///
/// Anything that is not fitted box or area text with solved lines (no lines yet) only loses its
/// `fit`. `engine` measures the layout.
pub fn bake(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Result<TextLayer, BakeError> {
    use crate::layout::{FORCED_LINE_BREAK, scaled_style};
    use std::collections::{BTreeMap, BTreeSet};

    let mut out = layer.clone();
    let lines: Vec<FitLine> = match &layer.fit {
        Some(f) if matches!(layer.shape, TextShape::Box { .. } | TextShape::Area { .. }) => f.valid_lines(&layer.text).to_vec(),
        _ => Vec::new(),
    };
    out.fit = None;
    if lines.is_empty() || layer.text.is_empty() {
        return Ok(out);
    }
    if has_condensed_type(layer) {
        return Err(BakeError::Condensed);
    }
    replace_non_finite(&mut out);
    let text = &layer.text;
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let (word_gap, line_gap) = layer.fit.as_ref().map_or((0.0, 0.0), TextFit::gaps);

    // Where each line break goes: the spaces before a line start (possibly none) become one
    // forced break, unless the line follows a break already (or nothing). Area text: a paragraph
    // break, which also replaces a forced line break the line follows.
    let area = matches!(layer.shape, TextShape::Area { .. });
    let break_char = if area { '\r' } else { FORCED_LINE_BREAK };
    let mut breaks: BTreeMap<usize, usize> = BTreeMap::new();
    for l in lines.iter().skip(1) {
        let p = l.start as usize;
        let mut q = p;
        while q > 0 && text.as_bytes().get(q - 1) == Some(&b' ') {
            q -= 1;
        }
        if area && text.get(..q).is_some_and(|s| s.ends_with(FORCED_LINE_BREAK)) && p < text.len() {
            breaks.insert(q - FORCED_LINE_BREAK.len_utf8(), p);
            continue;
        }
        if p == 0 || p >= text.len() || text.get(..q).is_none_or(|s| s.is_empty() || s.ends_with(['\r', '\n', FORCED_LINE_BREAK])) {
            continue;
        }
        breaks.insert(q, p);
    }

    // The fitted layout, and the same without the word gap: the difference of a space advance is
    // what the gap added.
    let fitted = engine.layout(layer, dpi);
    let mut space_extra: BTreeMap<usize, (usize, f32)> = BTreeMap::new();
    if word_gap != 0.0 {
        let mut plain_layer = layer.clone();
        plain_layer.cache = None;
        plain_layer.psd_raw = None;
        if let Some(f) = plain_layer.fit.as_mut() {
            f.word_gap = 0.0;
        }
        let plain = engine.layout(&plain_layer, dpi);
        let by_start: std::collections::HashMap<usize, f32> = plain.clusters.iter().map(|c| (c.range.start, c.advance)).collect();
        for c in fitted.clusters.iter().filter(|c| matches!(text.get(c.range.clone()), Some(" " | "\u{a0}"))) {
            // A space that a break replaces is gone: no tracking for it.
            if breaks.iter().any(|(&q, &p)| c.range.start >= q && c.range.start < p) {
                continue;
            }
            if let Some(delta) = by_start.get(&c.range.start).map(|p| c.advance - p).filter(|d| d.is_finite() && d.abs() > 1e-4) {
                space_extra.insert(c.range.start, (c.range.end, delta));
            }
        }
    }

    // Pieces: cut at run edges, line starts, break ranges and the spaces that get extra tracking.
    let runs = out.char_runs();
    let mut cuts: BTreeSet<usize> = BTreeSet::from([0, text.len()]);
    let mut at = 0usize;
    for r in &runs {
        at = at.saturating_add(r.len).min(text.len());
        cuts.insert(at);
    }
    cuts.extend(lines.iter().map(|l| l.start as usize).filter(|&p| p <= text.len()));
    for (&q, &p) in &breaks {
        cuts.extend([q, p]);
    }
    for (&a, &(z, _)) in &space_extra {
        cuts.extend([a, z]);
    }
    // A last empty paragraph (the text ends with a paragraph break) has no character of its own
    // and takes the style of the break before it: that break gets the empty line's own style.
    let break_len = if text.ends_with("\r\n") { 2 } else { 1 };
    let terminal = (lines.last().is_some_and(|l| l.start as usize >= text.len()) && text.ends_with(['\r', '\n'])).then(|| text.len().saturating_sub(break_len));
    cuts.extend(terminal);
    let cuts: Vec<usize> = cuts.into_iter().collect();
    let mut run_ends = Vec::with_capacity(runs.len());
    let mut at = 0usize;
    for r in &runs {
        at = at.saturating_add(r.len);
        run_ends.push(at);
    }
    let run_at = |off: usize| run_ends.partition_point(|&e| e <= off).min(runs.len().saturating_sub(1));
    let line_at = |off: usize| match terminal {
        Some(t) if off >= t => lines.len().saturating_sub(1),
        _ => lines.partition_point(|l| l.start as usize <= off).saturating_sub(1),
    };
    let removed = |off: usize| breaks.iter().any(|(&q, &p)| off >= q && off < p);

    // Each line's leading, as layout stacks solved lines: the previous line's descent, its own
    // ascent and the gap, a percent of the larger of the two lines' largest sizes. The first
    // line: its own height plus the gap (it only matters if text is typed above it).
    let mut line_size = vec![0.0f32; lines.len()];
    for w in cuts.windows(2) {
        let Some(run) = runs.get(run_at(w[0])) else { continue };
        let li = line_at(w[0]);
        if let Some(sz) = line_size.get_mut(li) {
            let scale = lines.get(li).map_or(1.0, |l| l.scale);
            *sz = sz.max(scaled_style(&run.style, scale).size_pt);
        }
    }
    let info_of = |l: &FitLine| fitted.lines.iter().find(|i| i.range.start == l.start as usize);
    let leading: Vec<Option<f32>> = lines
        .iter()
        .zip(&line_size)
        .enumerate()
        .map(|(i, (l, &size))| {
            let info = info_of(l)?;
            let prev = i.checked_sub(1).and_then(|p| Some((info_of(&lines[p])?, line_size[p])));
            let v = match prev {
                Some((p, prev_size)) => (p.descent + info.ascent) / k + line_gap / 100.0 * size.max(prev_size),
                None => (info.ascent + info.descent) / k + line_gap / 100.0 * size,
            };
            Some(v.clamp(-MAX_PT, MAX_PT * 4.0))
        })
        .collect();

    // Area text: how far (pt) a line sits below where it stacks on the line above (the solver
    // skipped rows too narrow for it). Its paragraph gets that as extra space before.
    let para_space = |off: usize, after: bool| {
        let mut at = 0usize;
        for p in layer.paragraph_runs() {
            let end = at.saturating_add(p.len);
            if off < end || end >= text.len() {
                let v = if after { p.style.space_after_pt } else { p.style.space_before_pt };
                return if v.is_finite() { v } else { 0.0 };
            }
            at = end;
        }
        0.0
    };
    let para_starts: BTreeSet<usize> = crate::layout::split_paragraphs(text).iter().map(|r| r.start).collect();
    let skip: Vec<f32> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if !area || i == 0 {
                return 0.0;
            }
            let (Some(info), Some(prev), Some(Some(lead))) = (info_of(l), info_of(&lines[i - 1]), leading.get(i)) else { return 0.0 };
            let start = l.start as usize;
            let spacing = if para_starts.contains(&start) { para_space(lines[i - 1].start as usize, true) + para_space(start, false) } else { 0.0 };
            let dy = (info.baseline - prev.baseline) / k - lead - spacing;
            if dy > 1e-3 { dy } else { 0.0 }
        })
        .collect();

    // The solved lines span the box exactly, and wrapped text breaks a line that is a hair wider
    // than its width: every line shrinks by a margin, the smallest of MARGINS that keeps the
    // fitted line count. Each try is verified; none fitting is an error.
    //
    // Area text whose first line sits below the row area type would give it (the solver moved
    // it down past a narrow top): `lead_in` is Some(leading, pt) and the text starts with an
    // empty paragraph at the smallest size, which takes the top row; the first line, with that
    // leading, stacks on it where it was fitted.
    let build = |margin: f32, lead_in: Option<f32>| -> Result<TextLayer, BakeError> {
        let mut out = out.clone();
        let mut new_text = String::with_capacity(text.len() + 1);
        let mut new_runs: Vec<(usize, photocraft_doc::text::CharStyle)> = Vec::new();
        let mut new_paras = Vec::new();
        if lead_in.is_some() {
            let mut style = runs.first().map(|r| r.style.clone()).unwrap_or_default();
            style.size_pt = MIN_PT;
            style.leading_pt = None;
            let mut ps = layer.paragraph_runs().first().map(|p| p.style.clone()).unwrap_or_default();
            ps.first_line_indent_pt = 0.0;
            ps.space_before_pt = 0.0;
            ps.space_after_pt = 0.0;
            new_text.push('\r');
            new_runs.push((1, style));
            new_paras.push(photocraft_doc::text::ParagraphRun { len: 1, style: ps });
        }
        let lead = new_text.len();
        for w in cuts.windows(2) {
            let (a, z) = (w[0], w[1]);
            if a == z || removed(a) {
                continue;
            }
            let piece = text.get(a..z).ok_or(BakeError::Cut { start: a, end: z })?;
            let li = line_at(a);
            let base = runs.get(run_at(a)).map(|r| r.style.clone()).unwrap_or_default();
            let mut style = scaled_style(&base, lines.get(li).map_or(1.0, |l| l.scale) * margin);
            if let Some(lead) = lead_in.filter(|_| li == 0).or_else(|| leading.get(li).copied().flatten()) {
                style.leading_pt = Some(lead);
            }
            if let Some(&(_, delta)) = space_extra.get(&a) {
                // Word spacing is added to the shaped advance, before the horizontal scale stretches
                // it; tracking is added after shaping and scaled with it. Divide the scale back out.
                let px = style.size_pt * k * if style.horizontal_scale > 0.0 { style.horizontal_scale } else { 1.0 };
                if px > 0.0 {
                    style.tracking += delta / px * 1000.0;
                }
            }
            new_text.push_str(piece);
            let mut len = piece.len();
            if breaks.contains_key(&z) {
                new_text.push(break_char);
                len += break_char.len_utf8();
            }
            match new_runs.last_mut() {
                Some(last) if last.1 == style => last.0 += len,
                _ => new_runs.push((len, style)),
            }
        }
        // An original offset in the new text: each break before it replaces its spaces by one char.
        let moved = |off: usize| {
            let shift: isize = breaks.iter().filter(|&(_, &p)| p <= off).map(|(&q, &p)| 1 - (p - q) as isize).sum();
            off.saturating_add_signed(shift) + lead
        };
        let mut at = 0usize;
        for p in layer.paragraph_runs() {
            let end = at.saturating_add(p.len);
            // Area text: a piece per paragraph break inserted inside this paragraph.
            let mut starts = vec![moved(at)];
            if area {
                starts.extend(breaks.iter().filter(|&(&q, _)| q >= at && q < end).map(|(_, &p)| moved(p)));
            }
            let n = starts.len();
            for (i, &a) in starts.iter().enumerate() {
                let z = starts.get(i + 1).copied().unwrap_or_else(|| moved(end));
                let mut style = p.style.clone();
                if i > 0 {
                    style.first_line_indent_pt = 0.0;
                    style.space_before_pt = 0.0;
                }
                if i + 1 < n {
                    style.space_after_pt = 0.0;
                }
                // The line this piece holds: the first solved line starting in it.
                if area && let Some(li) = lines.iter().position(|l| moved(l.start as usize) >= a && moved(l.start as usize) < z.max(a + 1)) {
                    style.space_before_pt += skip.get(li).copied().unwrap_or(0.0);
                }
                new_paras.push(photocraft_doc::text::ParagraphRun { len: z.saturating_sub(a), style });
            }
            at = end;
        }
        out.text = new_text;
        out.runs = new_runs.into_iter().map(|(len, style)| TextRun { len, style }).collect();
        out.paragraphs = new_paras;
        out.sync_summary();
        Ok(out)
    };
    let assemble = |engine: &mut crate::TextEngine, margin: f32, lead_in: Option<f32>| -> Result<TextLayer, BakeError> {
        let out = build(margin, lead_in)?;
        let baked = engine.layout(&out, dpi);
        // The empty lead-in paragraph lays out as a line of its own.
        let lines = baked.lines.len() - usize::from(lead_in.is_some() && !baked.lines.is_empty());
        if lines != fitted.lines.len() {
            return Err(BakeError::LineCount { baked: lines, fitted: fitted.lines.len() });
        }
        if !same_glyphs(&fitted, &baked) {
            return Err(BakeError::Drift);
        }
        Ok(out)
    };
    // The cause reported: the first line-count failure, else the last failure.
    let mut cause = BakeError::Drift;
    let mut keep = |r: Result<TextLayer, BakeError>| match r {
        Ok(b) => Some(b),
        Err(e) => {
            if !matches!(cause, BakeError::LineCount { .. }) {
                cause = e;
            }
            None
        }
    };
    for margin in MARGINS {
        if let Some(b) = keep(assemble(engine, margin, None)) {
            return Ok(b);
        }
    }
    // Area text: the first line's leading is its distance below the lead-in paragraph's row,
    // read from where the empty paragraph lays out (any leading puts it there).
    let first = lines.first().and_then(info_of).map(|i| i.baseline);
    if area && let Some(b1) = first {
        let probe = build(MARGINS[0], Some(1.0))?;
        let at = engine.layout(&probe, dpi).lines.first().map(|l| l.baseline);
        if let Some(lead) = at.map(|b0| (b1 - b0) / k).filter(|v| v.is_finite() && *v > 0.0) {
            for margin in MARGINS {
                if let Some(b) = keep(assemble(engine, margin, Some(lead))) {
                    return Ok(b);
                }
            }
        }
    }
    Err(cause)
}

/// Exact layouts after the search: each lays out the chosen lines and corrects every scale by
/// its measured width (optical-size fonts don't scale linearly) and the block by its height.
const REFINE_PASSES: usize = 4;
/// Widths the search scans before it bisects (part of MAX_PROBES).
const SCAN_PROBES: usize = 10;
/// How far (px) the last line's descent may hang below the box and the block still fit.
const FIT_EPS: f32 = 0.5;
/// A line is done once its measured width is this close to its target (px).
const WIDTH_EPS: f32 = 0.05;

/// One word of a segment at base size: where it starts, the width of the spaces before it (since
/// the previous word, or the segment start) and its own width, in px.
struct Word {
    start: usize,
    lead: f32,
    width: f32,
    /// Largest type size among its characters (pt).
    pt: f32,
    /// Size of its first character (pt): area type finds a paragraph's first row with it.
    first_pt: f32,
}

/// Text between mandatory breaks (paragraph breaks and forced line breaks): every segment
/// starts a line.
struct Segment {
    start: usize,
    words: Vec<Word>,
    /// Starts its paragraph (first-line indent, space before).
    para_first: bool,
    /// Ascent and descent per px of type size, from the base layout.
    ascent: f32,
    descent: f32,
    /// Size (pt) at the segment start: an empty segment's line height and gap.
    pt: f32,
    /// Paragraph metrics (px).
    avail: f32,
    first_indent: f32,
    space_before: f32,
    space_after: f32,
}

/// A candidate line: its segment, its words, and its natural width at base size (px).
#[derive(Clone)]
struct Cand {
    seg: usize,
    first: usize,
    end: usize,
    natural: f32,
}

struct Solved {
    lines: Vec<FitLine>,
    fits: bool,
    /// Height evaluations of the search (≤ MAX_PROBES).
    probes: usize,
    /// Full layouts: one to measure, then the refinement passes.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the bound tests"))]
    layouts: usize,
    /// Base layout lines visited while splitting into segments.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the bound tests"))]
    visited: usize,
    /// Area text: each placed line's planned baseline (px), for the tests that check layout
    /// puts it there.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the placement tests"))]
    planned: Vec<f32>,
}

/// Photoshop Block: lines and per-line scales so each line spans the box width (spec: Behaviour
/// 1–3). Empty text → empty Vec. Bounded: ≤ MAX_PROBES search iterations and at most
/// 1 + REFINE_PASSES layouts.
///
/// Measures every word once (one point-text layout), then searches the target natural width `w`
/// (a geometric scan, then bisection) over at most MAX_PROBES height estimates: the smallest `w`
/// whose [`contrast_breaks`] lines, each scaled to the box width, stack no taller than the box.
/// The chosen lines are laid out exactly (at most REFINE_PASSES layouts of the whole layer) and
/// each scale is corrected by the width the layout measures. A line starts only at a paragraph, a
/// forced line break or a word after a space; an empty one keeps scale 1. Each effective size
/// stays within MIN_PT..=MAX_PT.
///
/// Sizing is by width only: nothing is shrunk to fit the height. `w` is searched up to
/// `min(hi_w, MAX_SHRINK_W · box width)`; when even that is too tall, its lines are kept and layout
/// hides the ones past the box bottom (as Photoshop cuts them off), flagging the layer overflowed.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before solving.
pub fn solve_box(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Vec<FitLine> {
    solve(engine, layer, dpi).lines
}

fn solve(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Solved {
    let none = Solved { lines: Vec::new(), fits: true, probes: 0, layouts: 0, visited: 0, planned: Vec::new() };
    let TextShape::Box { y: box_y, width, height, .. } = layer.shape else { return none };
    if layer.text.is_empty() || u32::try_from(layer.text.len()).is_err() || !width.is_finite() || !height.is_finite() || !box_y.is_finite() {
        return none;
    }
    let (box_w, box_h) = (width.max(1.0), height.max(1.0));
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let line_gap = layer.fit.as_ref().map_or(0.0, |f| f.gaps().1) / 100.0;

    // The layer as the solver lays it out: non-finite sizes replaced (as `apply_scale` does), no
    // cached pixels, gaps kept.
    let mut base = layer.clone();
    base.cache = None;
    base.psd_raw = None;
    replace_non_finite(&mut base);
    let fit = base.fit.get_or_insert_with(TextFit::default);
    fit.lines.clear();

    // 1–2. Paragraphs, segments and words, measured at base size in one point-text layout.
    let mut point = base.clone();
    point.shape = TextShape::Point;
    let measured = engine.layout(&point, dpi);
    let Some((segs, visited)) = segments(&base, &measured, k, box_w) else {
        // Nothing to measure the lines with: no solved lines, and the text is not known to fit.
        return Solved { fits: false, ..none };
    };
    if segs.is_empty() {
        return none;
    }
    let mut layouts = 1;

    // 3. Search the target natural width `w`: the smallest whose contrast lines, each scaled to
    // the box width, stack no taller than the box.
    let lo_w = segs.iter().flat_map(|s| s.words.iter().enumerate().map(|(i, w)| if i == 0 { w.lead + w.width } else { w.width })).fold(0.0f32, f32::max);
    let hi_w = segs.iter().map(|s| natural(s, 0, s.words.len())).fold(lo_w, f32::max);
    let measure = |lines: Vec<Cand>| {
        let scales: Vec<f32> = lines.iter().map(|c| width_scale(&segs, c)).collect();
        let h = stack(&segs, &lines, &scales, line_gap, k);
        (lines, scales, h)
    };
    let eval = |w: f32| measure(contrast_breaks(&segs, w));
    // The widest `w` searched: lines may not shrink on average below about 1/MAX_SHRINK_W of the
    // base size. At `hi_w` every segment is one line, in pieces of at most MAX_LINE_WORDS words.
    let w_max = hi_w.min(MAX_SHRINK_W * box_w);
    let (mut lines, mut scales, h_top) = if w_max >= hi_w { measure(whole_lines(&segs)) } else { eval(w_max) };
    let mut probes = 1;
    // Too tall even at `w_max`: those lines are kept and layout hides the ones past the bottom.
    if h_top <= box_h && lo_w > 0.0 && lo_w < w_max {
        // The breaks make the height jump about with `w` (a word moving up can leave a
        // short last line that scales up a lot), so a plain bisection can miss smaller widths
        // that fit. Scan SCAN_PROBES widths from the longest word up (geometric steps), then
        // bisect between the last one too tall and the first that fits. Of every probe that
        // fits, the tallest is kept (the smaller `w` on a tie): the block ends as close to the
        // box bottom as the breaks allow.
        let mut best_h = h_top;
        let step = (w_max / lo_w).powf(1.0 / SCAN_PROBES as f32);
        let (mut lo, mut hi) = (lo_w, w_max);
        let mut first = true;
        for i in 0..SCAN_PROBES {
            let w = lo_w * step.powi(i as i32);
            let m = eval(w);
            probes += 1;
            if m.2 <= box_h {
                hi = w;
                if m.2 >= best_h {
                    (lines, scales, best_h) = m;
                }
                break;
            }
            lo = w;
            first = false;
        }
        if !first {
            while probes < MAX_PROBES && hi - lo > 0.25 {
                let mid = 0.5 * (lo + hi);
                let m = eval(mid);
                probes += 1;
                if m.2 <= box_h {
                    hi = mid;
                    if m.2 >= best_h {
                        (lines, scales, best_h) = m;
                    }
                } else {
                    lo = mid;
                }
            }
        }
    }

    // Each line's target width: the box less its indents.
    let target: Vec<f32> = lines.iter().map(|c| line_target(&segs, c)).collect();

    // 4. Exact widths: lay the chosen lines out and correct each scale by the visible width
    // layout measures (kerning, horizontal scale, optical sizes). The scales returned are the
    // ones last laid out.
    let starts: Vec<u32> = lines.iter().map(|c| line_start(&segs, c) as u32).collect();
    let mut fits = true;
    for pass in 0..REFINE_PASSES {
        if let Some(f) = base.fit.as_mut() {
            f.lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
        }
        let out = engine.layout(&base, dpi);
        layouts += 1;
        let bottom = out.lines.last().map_or(0.0, |l| l.baseline + l.descent) - box_y;
        // Fits: nothing hidden and the last line's descent inside the box too.
        fits = !out.hidden && bottom <= box_h + FIT_EPS;
        let mut next = scales.clone();
        let mut changed = false;
        let mut at = 0;
        for (i, c) in lines.iter().enumerate() {
            // Layout lines come in the order of the solved lines; a hidden one is missing.
            let Some(off) = out.lines.get(at..).and_then(|ls| ls.iter().position(|l| l.range.start == starts[i] as usize)) else { continue };
            at += off;
            // The visible width: horizontal scale and manual or optical kerning included.
            let advance = out.lines[at].x1 - out.lines[at].x0;
            if c.first >= c.end || advance <= 0.0 || (advance - target[i]).abs() <= WIDTH_EPS {
                continue;
            }
            let s = clamp_scale(&segs, c, scales[i] * target[i] / advance);
            changed |= s != scales[i];
            next[i] = s;
        }
        if !changed || pass + 1 == REFINE_PASSES {
            break;
        }
        scales = next;
    }
    let lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
    Solved { lines, fits, probes, layouts, visited, planned: Vec::new() }
}

/// Every segment as one line, in pieces of at most MAX_LINE_WORDS words: the lines at `hi_w`.
fn whole_lines(segs: &[Segment]) -> Vec<Cand> {
    segs.iter()
        .enumerate()
        .flat_map(|(seg, s)| {
            let n = s.words.len();
            let starts: Vec<usize> = if n == 0 { vec![0] } else { (0..n).step_by(MAX_LINE_WORDS).collect() };
            starts.into_iter().map(move |first| {
                let end = (first + MAX_LINE_WORDS).min(n);
                Cand { seg, first, end, natural: natural(s, first, end) }
            })
        })
        .collect()
}

/// Most lines one area solve places (layout's area row cap).
const MAX_AREA_LINES: usize = 4096;
/// Steps of a line's scale bisection in a shape (spec: Solver bounds).
const AREA_BISECT_STEPS: usize = 12;
/// Share of its band's widest span a line in a shape is sized to: a hair inside, so layout always
/// keeps it on the band the solver placed it on.
pub(crate) const AREA_MARGIN: f32 = 0.999;
/// Smallest scale a line in a shape may take, as a share of the largest line's scale (see
/// [`place_lines`]): rows too narrow for that (tips, tails) are skipped and left empty. 1/MAX_CONTRAST, so the
/// size steps stay within the contrast the line choice aims for.
pub(crate) const MIN_ROW_SHARE: f32 = 1.0 / MAX_CONTRAST;
/// How far a line in a shape moves down per step when its band is too narrow, as a share of
/// its height at its floor scale.
pub(crate) const ROW_SKIP_STEP: f32 = 0.05;
/// Most steps a line in a shape moves down looking for a band that holds it.
pub(crate) const ROW_SKIP_STEPS: usize = 256;
/// Most extra placements of a shape's lines with the largest line's size as every line's floor
/// (see [`place_lines`]).
const FLOOR_PASSES: usize = 3;
/// How far below MIN_ROW_SHARE of the largest line a placed line may stay without another pass.
const FLOOR_SLACK: f32 = 0.98;

/// Per-line sizing in a shape (spec: Behaviour 4): lines and per-line scales so each line spans
/// the widest span of the band it sits on. Empty text, or a path that encloses nothing → empty
/// Vec. Bounded: ≤ MAX_PROBES placements of the lines, each line's scale bisected in at most
/// AREA_BISECT_STEPS steps, and at most 1 + REFINE_PASSES layouts.
///
/// The line choice is [`solve_box`]'s ([`contrast_breaks`] over the target natural width `w`),
/// searched up to `min(hi_w, MAX_SHRINK_W · shape width)`, keeping of the probes whose lines all
/// fit the one that ends lowest. Lines are placed top-down from the shape's top (GUESS) by
/// [`place_area`]: each at the largest scale whose width fits the widest span of its band
/// (GUESS: a split row uses its widest span), never below MIN_ROW_SHARE of the largest line
/// (narrower rows are skipped and left empty, the first line's included; see [`place_lines`]). The chosen lines are then laid out, each
/// line's width corrected by what layout measures, and the lines placed again.
/// Nothing is shrunk to fit the height: when even the widest `w` leaves lines below the shape,
/// layout hides them and flags the layer overflowed.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before solving.
pub fn solve_area(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Vec<FitLine> {
    solve_shape(engine, layer, dpi).lines
}

fn solve_shape(engine: &mut crate::TextEngine, layer: &TextLayer, dpi: f32) -> Solved {
    let none = Solved { lines: Vec::new(), fits: true, probes: 0, layouts: 0, visited: 0, planned: Vec::new() };
    let TextShape::Area { path } = &layer.shape else { return none };
    if layer.text.is_empty() || u32::try_from(layer.text.len()).is_err() {
        return none;
    }
    // A path that encloses nothing lays out as point text: there is nothing to fit.
    let Some(shape) = crate::area::AreaShape::new(path, false) else { return none };
    let shape_w = (shape.bounds[2] - shape.bounds[0]).max(1.0);
    let k = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let line_gap = layer.fit.as_ref().map_or(0.0, |f| f.gaps().1) / 100.0;

    // As in `solve`: the layer as the solver lays it out, and its words measured as point text.
    let mut base = layer.clone();
    base.cache = None;
    base.psd_raw = None;
    replace_non_finite(&mut base);
    let fit = base.fit.get_or_insert_with(TextFit::default);
    fit.lines.clear();
    let mut point = base.clone();
    point.shape = TextShape::Point;
    let measured = engine.layout(&point, dpi);
    let Some((segs, visited)) = segments(&base, &measured, k, shape_w) else {
        return Solved { fits: false, ..none };
    };
    if segs.is_empty() {
        return none;
    }
    let mut layouts = 1;

    // Search `w` as `solve` does, with the shape for the box: a geometric scan from the longest
    // word up, then a bisection between the last width that doesn't fit and the first that does.
    // Of every probe whose lines all fit, the one ending lowest is kept (the smaller `w` on a tie).
    let lo_w = segs.iter().flat_map(|s| s.words.iter().enumerate().map(|(i, w)| if i == 0 { w.lead + w.width } else { w.width })).fold(0.0f32, f32::max);
    let hi_w = segs.iter().map(|s| natural(s, 0, s.words.len())).fold(lo_w, f32::max);
    let w_max = hi_w.min(MAX_SHRINK_W * shape_w);
    let eval = |w: f32| {
        let lines = if w >= hi_w { whole_lines(&segs) } else { contrast_breaks(&segs, w) };
        let (scales, fits, bottom, planned) = place_lines(&segs, &lines, &shape, line_gap, k, shape_w);
        (lines, scales, fits, bottom, planned)
    };
    let (mut lines, mut scales, fits_top, mut best, mut planned) = eval(w_max);
    let mut probes = 1;
    // Not even `w_max` fits: its lines are kept and layout hides the ones past the shape.
    if fits_top && lo_w > 0.0 && lo_w < w_max {
        let step = (w_max / lo_w).powf(1.0 / SCAN_PROBES as f32);
        let (mut lo, mut hi) = (lo_w, w_max);
        let mut first = true;
        for i in 0..SCAN_PROBES {
            let w = lo_w * step.powi(i as i32);
            let m = eval(w);
            probes += 1;
            if m.2 {
                hi = w;
                if m.3 >= best {
                    (lines, scales, _, best, planned) = m;
                }
                break;
            }
            lo = w;
            first = false;
        }
        if !first {
            while probes < MAX_PROBES && hi - lo > 0.25 {
                let mid = 0.5 * (lo + hi);
                let m = eval(mid);
                probes += 1;
                if m.2 {
                    hi = mid;
                    if m.3 >= best {
                        (lines, scales, _, best, planned) = m;
                    }
                } else {
                    lo = mid;
                }
            }
        }
    }

    // Exact widths: lay the lines out, correct each line's natural width by the visible width
    // layout measures (kerning, horizontal scale, optical sizes), and place the lines again. A
    // line's scale moves the rows of every line below it, so the lines are re-placed together
    // rather than each scaled to its old row. The scales returned are the ones last laid out.
    let starts: Vec<u32> = lines.iter().map(|c| line_start(&segs, c) as u32).collect();
    let mut fits = true;
    for pass in 0..REFINE_PASSES {
        if let Some(f) = base.fit.as_mut() {
            f.lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
        }
        let out = engine.layout(&base, dpi);
        layouts += 1;
        fits = !out.hidden;
        let mut changed = false;
        let mut at = 0;
        for (i, c) in lines.iter_mut().enumerate() {
            // Layout lines come in the order of the solved lines; a hidden one is missing.
            let Some(off) = out.lines.get(at..).and_then(|ls| ls.iter().position(|l| l.range.start == starts[i] as usize)) else { continue };
            at += off;
            let l = &out.lines[at];
            let (advance, want) = (l.x1 - l.x0, c.natural * scales[i]);
            if c.first >= c.end || advance <= 0.0 || want <= 0.0 || (advance - want).abs() <= WIDTH_EPS {
                continue;
            }
            c.natural *= advance / want;
            changed = true;
        }
        if !changed || pass + 1 == REFINE_PASSES {
            break;
        }
        (scales, _, _, planned) = place_lines(&segs, &lines, &shape, line_gap, k, shape_w);
    }
    let lines = starts.iter().zip(&scales).map(|(&start, &scale)| FitLine { start, scale }).collect();
    Solved { lines, fits, probes, layouts, visited, planned }
}

/// What a line's indents take off its row (px): the paragraph's indents, and on its first line
/// the first-line indent.
fn line_inset(segs: &[Segment], c: &Cand, shape_w: f32) -> f32 {
    (shape_w - line_target(segs, c)).max(0.0)
}

/// Places `lines` in `shape` with [`place_area`] so no placed line is smaller than
/// MIN_ROW_SHARE of the largest one, the first line included: a first pass floors each line
/// by the lines above it only, and while a placed line is still below MIN_ROW_SHARE of the
/// largest, the lines are placed again with the largest seen so far as every line's floor (at
/// most FLOOR_PASSES more passes; the last one is kept).
fn place_lines(segs: &[Segment], lines: &[Cand], shape: &crate::area::AreaShape, line_gap: f32, k: f32, shape_w: f32) -> (Vec<f32>, bool, f32, Vec<f32>) {
    let mut out = place_area(segs, lines, shape, line_gap, k, shape_w, 0.0);
    let mut target = 0.0f32;
    for _ in 0..FLOOR_PASSES {
        let placed = || lines.iter().zip(&out.0).take(out.3.len()).filter(|(c, _)| c.first < c.end).map(|(_, &s)| s);
        let largest = placed().fold(0.0f32, f32::max);
        // Within FLOOR_SLACK: a pass that only nudges the largest line isn't worth another.
        if placed().all(|s| s >= MIN_ROW_SHARE * largest * FLOOR_SLACK) {
            break;
        }
        target = target.max(largest);
        out = place_area(segs, lines, shape, line_gap, k, shape_w, target);
    }
    out
}

/// Places `lines` top-down in `shape` as layout places solved area lines (spec: Behaviour 4).
///
/// The first line starts at the first row from the shape's top that has a span at least its
/// first character's size wide (area type's own row search); each later line starts at the band
/// stacked on the previous line. A line's floor scale is MIN_ROW_SHARE of the larger of `target`
/// and the largest scale placed so far, clamped to MIN_PT..=MAX_PT. A line at scale `s` goes
/// from its start down in steps of ROW_SKIP_STEP of its height at `s`, at most ROW_SKIP_STEPS
/// steps, to the first band whose widest span holds it (layout's search), so narrow tips and
/// tails stay empty instead of filling with tiny lines. Its scale is the largest (a log
/// bisection of at most AREA_BISECT_STEPS steps, from the floor up) whose first band is no lower
/// than the floor's.
///
/// Returns the scales, whether every line fits, the bottom of the last placed line and each
/// placed line's baseline. A line
/// with no band left and every line after it keep the last placed line's scale (clamped);
/// layout hides them.
fn place_area(
    segs: &[Segment],
    lines: &[Cand],
    shape: &crate::area::AreaShape,
    line_gap: f32,
    k: f32,
    shape_w: f32,
    target: f32,
) -> (Vec<f32>, bool, f32, Vec<f32>) {
    let mut scales: Vec<f32> = Vec::with_capacity(lines.len());
    let mut baselines: Vec<f32> = Vec::with_capacity(lines.len());
    // The previous line's baseline, descent and largest size (px).
    let mut prev: Option<(f32, f32, f32)> = None;
    let mut largest = 0.0f32;
    let mut bottom = shape.bounds[1];
    for (i, c) in lines.iter().enumerate() {
        let seg = &segs[c.seg];
        let empty = c.first >= c.end;
        let pt = line_pt(segs, c).max(MIN_PT);
        let spacing = if i > 0 && seg.para_first && c.first == 0 { segs[lines[i - 1].seg].space_after + seg.space_before } else { 0.0 };
        let inset = line_inset(segs, c, shape_w);
        // The line at scale `s`, `dy` below its stacked band, when that band is inside the shape
        // and its widest span holds the line: its baseline, descent and size (px).
        // The first line goes on the first row from the shape's top with a span at least its
        // first character's size wide (area type's own row search, so baked text finds it too).
        let first_pt = line_first_pt(segs, c).max(MIN_PT);
        let place = |s: f32, dy: f32| -> Option<(f32, f32, f32)> {
            let px = pt * k * s;
            let (asc, desc) = (seg.ascent * px, seg.descent * px);
            let (b, widest) = match prev {
                None => {
                    let (b, spans) = shape.next_row(shape.bounds[1] + asc, asc, desc, first_pt * k * s)?;
                    if dy == 0.0 {
                        (b, spans.iter().map(|sp| sp.1 - sp.0).fold(0.0f32, f32::max))
                    } else {
                        let b = b + dy;
                        if b + desc > shape.bounds[3] || b.is_nan() {
                            return None;
                        }
                        (b, shape.spans(b - asc, b + desc, 0.0).iter().map(|sp| sp.1 - sp.0).fold(0.0f32, f32::max))
                    }
                }
                Some((b, prev_desc, prev_px)) => {
                    let b = dy + b + prev_desc + asc + line_gap * prev_px.max(px) + spacing;
                    if b + desc > shape.bounds[3] || b.is_nan() {
                        return None;
                    }
                    (b, shape.spans(b - asc, b + desc, 0.0).iter().map(|sp| sp.1 - sp.0).fold(0.0f32, f32::max))
                }
            };
            (widest > 0.0 && (empty || c.natural * s <= AREA_MARGIN * (widest - inset))).then_some((b, desc, px))
        };
        let (lo, hi) = (MIN_PT / pt, MAX_PT / pt);
        let floor = if empty { 1.0 } else { (MIN_ROW_SHARE * largest.max(target)).clamp(lo, hi) };
        // The line at scale `s` on the first band, from its stacked one down in steps of
        // ROW_SKIP_STEP of its height at `s` (layout's search), at most `steps` steps down.
        let first_fit = |s: f32, steps: usize| {
            let step = ROW_SKIP_STEP * (seg.ascent + seg.descent) * pt * k * s;
            (0..=steps.min(ROW_SKIP_STEPS)).map(|j| j as f32 * step).find_map(|dy| place(s, dy).map(|p| (dy, step, p)))
        };
        let placed = if i >= MAX_AREA_LINES {
            None
        } else if empty {
            first_fit(1.0, ROW_SKIP_STEPS).map(|(_, _, p)| (1.0, p))
        } else {
            // The band the line reaches at its floor; a larger scale must find its own first
            // band no lower than that, so growing never drops the line further down the shape.
            first_fit(floor, ROW_SKIP_STEPS).map(|(dy, _, p)| {
                let within = |s: f32| {
                    let step = ROW_SKIP_STEP * (seg.ascent + seg.descent) * pt * k * s;
                    first_fit(s, (dy / step).ceil() as usize)
                };
                if let Some((_, _, q)) = within(hi) {
                    return (hi, q);
                }
                let (mut a, mut z, mut p, mut s) = (floor.ln(), hi.ln(), p, floor);
                for _ in 0..AREA_BISECT_STEPS {
                    let mid = 0.5 * (a + z);
                    match within(mid.exp()) {
                        Some((_, _, q)) => {
                            a = mid;
                            p = q;
                            s = mid.exp();
                        }
                        None => z = mid,
                    }
                }
                (s, p)
            })
        };
        let Some((s, p)) = placed else {
            let last = scales.last().copied();
            for c in &lines[i..] {
                scales.push(if c.first >= c.end { 1.0 } else { last.map_or_else(|| width_scale(segs, c), |s| clamp_scale(segs, c, s)) });
            }
            return (scales, false, bottom, baselines);
        };
        scales.push(s);
        if !empty {
            largest = largest.max(s);
        }
        bottom = p.0 + p.1;
        baselines.push(p.0);
        prev = Some(p);
    }
    (scales, true, bottom, baselines)
}

/// Splits the layer into segments of words measured in `m` (the layer laid out as point text).
/// Also returns how many base layout lines it visited (tests check it stays linear).
/// None when a segment has no base layout line to measure.
fn segments(t: &TextLayer, m: &crate::TextLayout, k: f32, box_w: f32) -> Option<(Vec<Segment>, usize)> {
    use crate::layout::{FORCED_LINE_BREAK, split_paragraphs};
    let text = &t.text;
    let runs = t.char_runs();
    let mut run_ends = Vec::with_capacity(runs.len());
    let mut acc = 0usize;
    for r in &runs {
        acc = acc.saturating_add(r.len);
        run_ends.push(acc);
    }
    let pt_at = |off: usize| {
        let i = run_ends.partition_point(|&e| e <= off).min(runs.len().saturating_sub(1));
        runs.get(i).map_or(t.size_pt, |r| r.style.size_pt).clamp(MIN_PT, MAX_PT)
    };
    let paras = t.paragraph_runs();
    let mut para_ends = Vec::with_capacity(paras.len());
    let mut acc = 0usize;
    for p in &paras {
        acc = acc.saturating_add(p.len);
        para_ends.push(acc);
    }
    let para_at =
        |off: usize| paras.get(para_ends.partition_point(|&e| e <= off).min(paras.len().saturating_sub(1))).map(|p| p.style.clone()).unwrap_or_default();
    let len = |v: f32| if v.is_finite() { v * k } else { 0.0 };

    let mut clusters: Vec<&crate::ClusterInfo> = m.clusters.iter().collect();
    clusters.sort_by_key(|c| c.range.start);
    let mut lines: Vec<&crate::LineInfo> = m.lines.iter().collect();
    lines.sort_by_key(|l| l.range.start);
    let mut out = Vec::new();
    let mut visited = 0usize;
    for prange in split_paragraphs(text) {
        let content_end = prange.start + text.get(prange.clone()).unwrap_or("").trim_end_matches(['\r', '\n']).len();
        let ps = para_at(prange.start);
        let avail = (box_w - len(ps.start_indent_pt) - len(ps.end_indent_pt)).max(1.0);
        let mut bounds = vec![prange.start];
        if let Some(content) = text.get(prange.start..content_end) {
            bounds.extend(content.match_indices(FORCED_LINE_BREAK).map(|(i, b)| prange.start + i + b.len()));
        }
        for (j, &a) in bounds.iter().enumerate() {
            let z = bounds.get(j + 1).map_or(content_end, |&n| n - FORCED_LINE_BREAK.len_utf8());
            let mut words: Vec<Word> = Vec::new();
            let (mut pending, mut in_word) = (0.0f32, false);
            let first = clusters.partition_point(|c| c.range.start < a);
            for c in clusters[first..].iter().take_while(|c| c.range.start < z) {
                let piece = text.get(c.range.clone()).unwrap_or("");
                let adv = if c.advance.is_finite() { c.advance } else { 0.0 };
                if !piece.is_empty() && piece.chars().all(breaking_space) {
                    pending += adv;
                    in_word = false;
                    continue;
                }
                let pt = piece.char_indices().map(|(i, _)| pt_at(c.range.start + i)).fold(MIN_PT, f32::max);
                // A combining mark (or joiner) the shaper split from a space can't start a line:
                // it joins the word before, spaces and all.
                let glued = piece.chars().next().is_some_and(extends);
                match words.last_mut() {
                    Some(w) if in_word || glued => {
                        w.width += pending + adv;
                        w.pt = w.pt.max(pt);
                    }
                    _ => words.push(Word { start: c.range.start, lead: pending, width: adv, pt, first_pt: pt_at(c.range.start) }),
                }
                pending = 0.0;
                in_word = true;
            }
            // Ascent and descent per px from the base line(s) of this segment: the font's real
            // metrics as layout measured them. Point text gives every segment a line; if one
            // has none, there is nothing true to stack it with, so the solver gives up.
            let (mut asc, mut desc, mut found) = (0.0f32, 0.0f32, false);
            let from = lines.partition_point(|l| l.range.start < a);
            for l in lines[from..].iter().take_while(|l| l.range.start <= z) {
                visited += 1;
                found = true;
                asc = asc.max(l.ascent);
                desc = desc.max(l.descent);
            }
            if !found {
                return None;
            }
            let seg_pt = words.iter().map(|w| w.pt).fold(0.0f32, f32::max);
            let pt = pt_at(a);
            let px = if seg_pt > 0.0 { seg_pt } else { pt } * k;
            let (ascent, descent) = (asc / px, desc / px);
            out.push(Segment {
                start: a,
                words,
                para_first: j == 0,
                ascent,
                descent,
                pt,
                avail,
                first_indent: len(ps.first_line_indent_pt),
                space_before: len(ps.space_before_pt),
                space_after: len(ps.space_after_pt),
            });
        }
    }
    Some((out, visited))
}

/// Whitespace a line may break after (UAX #14 classes SP and BA). No-break spaces (GL, such
/// as U+00A0 and U+202F) and word joiners stay inside their word.
fn breaking_space(c: char) -> bool {
    use icu_properties::{CodePointMapData, props::LineBreak as L};
    c.is_whitespace() && matches!(CodePointMapData::<L>::new().get(c), L::Space | L::BreakAfter)
}

/// Whether `c` extends the grapheme before it (combining marks, spacing marks, ZWJ).
fn extends(c: char) -> bool {
    use icu_properties::{CodePointMapData, props::GraphemeClusterBreak as G};
    matches!(CodePointMapData::<G>::new().get(c), G::Extend | G::SpacingMark | G::ZWJ)
}

/// Natural width (px, base size) of words `first..end` of `s` as one line. The segment's first
/// line keeps the spaces before its first word; later lines start at their word.
fn natural(s: &Segment, first: usize, end: usize) -> f32 {
    s.words.get(first..end).map_or(0.0, |ws| ws.iter().enumerate().map(|(i, w)| if i > 0 || first == 0 { w.lead + w.width } else { w.width }).sum())
}

/// Shortest line a break may make, as a share of the target width `w` (a paragraph's last line
/// is exempt, and a lone word only when no other chain of lines exists). Fitted to Photoshop's
/// examples (spec: Line choice).
const MIN_LINE_FRAC: f32 = 0.35;
/// Reward for a size change between neighbouring lines (per unit of |ln| width ratio).
const CONTRAST_WEIGHT: f32 = 1.0;
/// Cost of a line's natural width straying from `w` (per unit of ln² ratio).
const SIZE_WEIGHT: f32 = 1.0;
/// Largest width ratio between neighbouring lines that still earns contrast: the zigzag gains
/// nothing past about 3×, Photoshop's widest step in its examples (GUESS).
const MAX_CONTRAST: f32 = 3.0;
/// Widest target natural width the search tries, as a multiple of the box width: lines average
/// no smaller than about half the layer's base size, and text that still doesn't fit overflows
/// (layout hides it). GUESS fitted to Photoshop's overflow in its examples (cafe-08).
const MAX_SHRINK_W: f32 = 2.0;
/// Most words one line may take (bounds the DP).
const MAX_LINE_WORDS: usize = 64;
/// Most words one DP pass takes (spec: Solver bounds); a longer segment is broken in passes of
/// this many words, each pass's last line ending at the pass end.
const MAX_DP_WORDS: usize = 4096;

/// Lines no wider than `w` at base size whose widths alternate long and short, as Photoshop's
/// Dynamic Text lockups do (GUESS fitted to its examples). Per segment, a DP over (line start,
/// line end) states minimises SIZE_WEIGHT·ln²(width/w) per line minus
/// CONTRAST_WEIGHT·min(|ln(w_i/w_{i+1})|, ln MAX_CONTRAST) between neighbours; a lone word may
/// exceed `w`; ties take the earlier break.
///
/// A line other than the segment's last must be at least MIN_LINE_FRAC·w wide, unless it already
/// holds MAX_LINE_WORDS words (the most a line may take). A lone word narrower than that is
/// allowed only when a pass has no chain of lines without one; one-word lines always chain, so the
/// retry always has one. The DP is O(words × MAX_LINE_WORDS²) time and
/// O(MAX_DP_WORDS × MAX_LINE_WORDS) memory.
fn contrast_breaks(segs: &[Segment], w: f32) -> Vec<Cand> {
    const D: usize = MAX_LINE_WORDS;
    let max_gain = MAX_CONTRAST.ln();
    let mut out = Vec::new();
    for (si, s) in segs.iter().enumerate() {
        let n = s.words.len();
        if n == 0 {
            out.push(Cand { seg: si, first: 0, end: 0, natural: 0.0 });
            continue;
        }
        // `natural(s, i, j)` in O(1): a prefix sum of lead + width, less the lead of the line's
        // first word unless it starts the segment.
        let mut prefix = Vec::with_capacity(n + 1);
        let mut acc = 0.0f32;
        prefix.push(acc);
        for wd in &s.words {
            acc += wd.lead + wd.width;
            prefix.push(acc);
        }
        let nat = |i: usize, j: usize| prefix[j] - prefix[i] - if i > 0 { s.words[i].lead } else { 0.0 };
        let too_wide = |v: f32| v > w * 1.0001;
        // Line [i, j) is allowed when it ends the segment, holds MAX_LINE_WORDS words or is at
        // least MIN_LINE_FRAC·w wide, and is no wider than `w` unless it is one word. With
        // `lone`, any one-word line is allowed too.
        let allowed =
            |i: usize, j: usize, v: f32, lone: bool| (j == i + 1 || !too_wide(v)) && ((lone && j == i + 1) || j == n || j - i == D || v >= MIN_LINE_FRAC * w);
        let own = |v: f32| SIZE_WEIGHT * (v.max(1e-3) / w.max(1e-3)).ln().powi(2);
        let gain = |a: f32, b: f32| CONTRAST_WEIGHT * (a.max(1e-3) / b.max(1e-3)).ln().abs().min(max_gain);
        // The lines of words base..end, or None when no chain of allowed lines covers them.
        let pass = |base: usize, end: usize, lone: bool| -> Option<Vec<Cand>> {
            // best[at(j, d)]: the cheapest cost of everything from `base` up to a line
            // [j - d, j), and the previous line's start (None: the pass's first line).
            let at = |j: usize, d: usize| (j - base) * D + (d - 1);
            let mut best: Vec<Option<(f32, Option<usize>)>> = vec![None; (end - base + 1) * D];
            for j in base + 1..=end {
                for d in 1..=D.min(j - base) {
                    let i = j - d;
                    let v = nat(i, j);
                    if d > 1 && too_wide(v) {
                        // Lines only widen as they start earlier: no earlier start is allowed either.
                        break;
                    }
                    if !allowed(i, j, v, lone) {
                        continue;
                    }
                    best[at(j, d)] = if i == base {
                        Some((own(v), None))
                    } else {
                        // Earliest previous start first, so a tie keeps the earlier break.
                        let mut pick: Option<(f32, Option<usize>)> = None;
                        for pd in (1..=D.min(i - base)).rev() {
                            let Some((pc, _)) = best[at(i, pd)] else { continue };
                            let p = i - pd;
                            let c = pc + own(v) - gain(nat(p, i), v);
                            if pick.is_none_or(|(b, _)| c < b) {
                                pick = Some((c, Some(p)));
                            }
                        }
                        pick
                    };
                }
            }
            // The cheapest last line (the earliest start on a tie), then walk back.
            let mut last: Option<(usize, f32)> = None;
            for d in (1..=D.min(end - base)).rev() {
                if let Some((c, _)) = best[at(end, d)]
                    && last.is_none_or(|(_, b)| c < b)
                {
                    last = Some((d, c));
                }
            }
            let (d, _) = last?;
            let mut lines = Vec::new();
            let (mut i, mut j) = (end - d, end);
            loop {
                lines.push(Cand { seg: si, first: i, end: j, natural: nat(i, j) });
                match best[at(j, j - i)].and_then(|(_, p)| p) {
                    Some(p) => (i, j) = (p, i),
                    None => break,
                }
            }
            lines.reverse();
            Some(lines)
        };
        for base in (0..n).step_by(MAX_DP_WORDS) {
            let end = (base + MAX_DP_WORDS).min(n);
            // One-word lines chain on their own, so the retry with `lone` always has lines.
            out.extend(pass(base, end, false).or_else(|| pass(base, end, true)).unwrap_or_default());
        }
    }
    out
}

fn line_start(segs: &[Segment], c: &Cand) -> usize {
    let s = &segs[c.seg];
    if c.first == 0 { s.start } else { s.words.get(c.first).map_or(s.start, |w| w.start) }
}

/// Width the line must span: the box less the paragraph's indents, and on its first line the
/// first-line indent (as layout breaks it).
fn line_target(segs: &[Segment], c: &Cand) -> f32 {
    let s = &segs[c.seg];
    let first_line = s.para_first && c.first == 0;
    (s.avail - if first_line { s.first_indent } else { 0.0 }).max(1.0)
}

/// Largest type size on the line (pt); an empty line's is its segment's size.
fn line_pt(segs: &[Segment], c: &Cand) -> f32 {
    let s = &segs[c.seg];
    match s.words.get(c.first..c.end) {
        Some(ws) if !ws.is_empty() => ws.iter().map(|w| w.pt).fold(MIN_PT, f32::max),
        _ => s.pt,
    }
}

/// Size of the line's first character (pt): the segment's at its start (its leading spaces), else
/// its first word's.
fn line_first_pt(segs: &[Segment], c: &Cand) -> f32 {
    let s = &segs[c.seg];
    if c.first == 0 { s.pt } else { s.words.get(c.first).map_or(s.pt, |w| w.first_pt) }
}

/// `scale` limited so the line's largest size stays within MIN_PT..=MAX_PT; empty lines keep 1.
fn clamp_scale(segs: &[Segment], c: &Cand, scale: f32) -> f32 {
    if c.first >= c.end {
        return 1.0;
    }
    let pt = line_pt(segs, c).max(MIN_PT);
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    s.clamp(MIN_PT / pt, MAX_PT / pt)
}

/// The scale that makes the line span its target width (clamped).
fn width_scale(segs: &[Segment], c: &Cand) -> f32 {
    if c.first >= c.end || c.natural <= 0.0 {
        return clamp_scale(segs, c, 1.0);
    }
    clamp_scale(segs, c, line_target(segs, c) / c.natural)
}

/// Height (px) from the box top to the last line's descent, stacked as layout stacks solved
/// lines: the first line's ascent, then for each later line the previous line's descent, its own
/// ascent and the line gap (percent of the larger of the two lines' sizes), plus paragraph
/// spacing, then the last descent.
fn stack(segs: &[Segment], lines: &[Cand], scales: &[f32], line_gap: f32, k: f32) -> f32 {
    let mut h = 0.0f32;
    let n = lines.len();
    let mut prev: Option<(f32, f32)> = None;
    for (i, (c, &s)) in lines.iter().zip(scales).enumerate() {
        let seg = &segs[c.seg];
        let empty = c.first >= c.end;
        let px = line_pt(segs, c) * k * if empty { 1.0 } else { s };
        let (asc, desc) = (seg.ascent * px, seg.descent * px);
        h += match prev {
            None => asc,
            Some((prev_desc, prev_px)) => prev_desc + asc + line_gap * prev_px.max(px),
        };
        prev = Some((desc, px));
        if i + 1 == n {
            h += desc;
        }
        if i > 0 && seg.para_first && c.first == 0 {
            let prev = &segs[lines[i - 1].seg];
            h += prev.space_after + seg.space_before;
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextEngine;
    use crate::tests::{area_ellipse_layer, box_layer};
    use photocraft_doc::text::TextFit;
    const FORCED_BREAK: &str = "\u{3}";

    fn fit_box(text: &str, size: f32, w: f32, h: f32) -> TextLayer {
        let mut t = box_layer(text, size, (0.0, 0.0, w, h));
        t.fit = Some(TextFit::default());
        t
    }

    /// `box_layer` with Dynamic Text on (default gaps).
    fn fitted_box(text: &str, pt: f32, rect: (f32, f32, f32, f32)) -> TextLayer {
        let mut t = box_layer(text, pt, rect);
        t.fit = Some(TextFit::default());
        t
    }

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    /// Splits the layer's text into runs of the given (byte length, size) pairs.
    fn set_run_sizes(t: &mut TextLayer, parts: &[(usize, f32)]) {
        t.runs = parts
            .iter()
            .map(|&(len, size_pt)| {
                let mut style = t.char_runs()[0].style.clone();
                style.size_pt = size_pt;
                TextRun { len, style }
            })
            .collect();
    }

    #[test]
    fn area_ellipse_lines_fill_and_fit() {
        // Was `area_ellipse_fits_tightly` (uniform fit): the text now fits as per-line sizes, the
        // base size kept, and every line is as big as its row allows.
        let mut e = TextEngine::new();
        let mut t = area_ellipse_layer(&"talk ".repeat(12), 12.0, 260.0, 160.0);
        t.fit = Some(TextFit::default());
        let r = refit(&mut e, &mut t, 72.0);
        assert!(r.fits && r.scale == 1.0 && t.size_pt == 12.0, "{r:?}");
        let out = e.layout(&t, 72.0);
        assert!(!out.hidden && !out.lines.is_empty());
        // Any line 2% bigger no longer fits where it is.
        let mut bigger = t.clone();
        if let Some(f) = bigger.fit.as_mut() {
            f.lines.iter_mut().for_each(|l| l.scale *= 1.02);
        }
        let b = e.layout(&bigger, 72.0);
        assert!(b.overflowed || b.hidden || b.lines.len() < out.lines.len() || b.lines.iter().zip(&out.lines).any(|(x, y)| x.baseline > y.baseline + 0.5));
    }

    #[test]
    fn area_lines_keep_run_ratios_at_the_top_end() {
        // Was `area_fit_keeps_run_ratios_at_the_top_end` (uniform fit). A shape far bigger than the
        // type can grow: the line's largest run stops at MAX_PT and the smaller run on it keeps
        // its 1:10 ratio instead of catching up at the clamp. One word, so one line.
        let mut e = TextEngine::new();
        let mut t = area_ellipse_layer("bigsmall", 12.0, 60_000.0, 60_000.0);
        set_run_sizes(&mut t, &[(3, 10.0), (5, 100.0)]); // "big" 10pt, "small" 100pt
        t.fit = Some(TextFit::default());
        refit(&mut e, &mut t, 72.0);
        let s: Vec<f32> = t.runs.iter().map(|r| r.style.size_pt).collect();
        assert_eq!(s, [10.0, 100.0], "base sizes kept");
        let lines = t.fit.as_ref().unwrap().lines.clone();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!((lines[0].scale * 100.0 - MAX_PT).abs() < 1.0, "{lines:?}");
        let out = e.layout(&t, 72.0);
        let sizes: Vec<f32> = out.glyphs.iter().map(|g| out.faces[g.face as usize].size_px).collect();
        let (lo, hi) = sizes.iter().fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!((hi / lo - 10.0).abs() < 1e-2, "{sizes:?}");
    }

    #[test]
    fn not_fitted_point_or_empty_is_untouched() {
        let mut e = TextEngine::new();
        let mut t = fit_box("", 12.0, 100.0, 100.0);
        assert_eq!(refit(&mut e, &mut t, 72.0), FitResult { scale: 1.0, fits: true, probes: 0 });
        let mut p = fit_box("hello", 12.0, 100.0, 100.0);
        p.shape = TextShape::Point;
        refit(&mut e, &mut p, 72.0);
        assert_eq!(p.size_pt, 12.0);
        let mut n = fit_box("hello", 12.0, 100.0, 100.0);
        n.fit = None;
        refit(&mut e, &mut n, 72.0);
        assert_eq!(n.size_pt, 12.0);
    }

    #[test]
    fn apply_scale_scales_leading_and_shift_and_clamps() {
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        set_run_sizes(&mut t, &[(1, 10.0), (1, 1000.0)]);
        t.runs[0].style.leading_pt = Some(20.0);
        t.runs[0].style.baseline_shift_pt = 4.0;
        apply_scale(&mut t, 2.0);
        assert_eq!(t.runs[0].style.size_pt, 20.0);
        assert_eq!(t.runs[0].style.leading_pt, Some(40.0));
        assert_eq!(t.runs[0].style.baseline_shift_pt, 8.0);
        assert_eq!(t.runs[1].style.size_pt, MAX_PT);
        apply_scale(&mut t, 1e-9);
        assert_eq!(t.runs[0].style.size_pt, MIN_PT);
    }

    fn with_para(t: &mut TextLayer, p: photocraft_doc::text::ParagraphStyle) {
        t.paragraphs = vec![photocraft_doc::text::ParagraphRun { len: t.text.len(), style: p }];
    }

    #[test]
    fn apply_scale_halves_paragraph_fields_and_keeps_hanging_negative() {
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        with_para(
            &mut t,
            photocraft_doc::text::ParagraphStyle {
                first_line_indent_pt: -8.0,
                start_indent_pt: 20.0,
                end_indent_pt: 10.0,
                space_before_pt: 6.0,
                space_after_pt: 4.0,
                ..Default::default()
            },
        );
        apply_scale(&mut t, 0.5);
        let p = &t.paragraphs[0].style;
        assert_eq!((p.first_line_indent_pt, p.start_indent_pt, p.end_indent_pt, p.space_before_pt, p.space_after_pt), (-4.0, 10.0, 5.0, 3.0, 2.0));
    }

    #[test]
    fn non_finite_sizes_never_reach_layout() {
        let mut e = TextEngine::new();
        let mut t = fit_box("ab", 10.0, 100.0, 100.0);
        set_run_sizes(&mut t, &[(1, f32::NAN), (1, f32::INFINITY)]);
        apply_scale(&mut t, 2.0);
        assert!(t.runs.iter().all(|r| r.style.size_pt.is_finite() && r.style.size_pt >= MIN_PT));
        assert_eq!(t.runs[0].style.size_pt, 20.0);
        // Neither does the Area solver.
        let mut u = ellipse_area("ab", f32::NAN);
        set_run_sizes(&mut u, &[(1, f32::NAN), (1, 12.0)]);
        let lines = solve_area(&mut e, &u, 72.0);
        assert!(!lines.is_empty() && lines.iter().all(|l| l.scale.is_finite() && l.scale > 0.0), "{lines:?}");
        // The Box solver never writes sizes, and its scales stay finite on non-finite sizes.
        let mut v = fit_box("ab cd", f32::NAN, 100.0, 100.0);
        set_run_sizes(&mut v, &[(2, f32::NAN), (3, f32::INFINITY)]);
        let lines = solve_box(&mut e, &v, 72.0);
        assert!(!lines.is_empty() && lines.iter().all(|l| l.scale.is_finite() && l.scale > 0.0), "{lines:?}");
    }

    const FOX: &str = "The quick brown fox jumps over the lazy dog";

    /// `t` with its solved lines written, and its layout.
    fn solved(e: &mut TextEngine, mut t: TextLayer) -> (TextLayer, crate::TextLayout) {
        let lines = solve_box(e, &t, 72.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = lines;
        }
        let l = e.layout(&t, 72.0);
        (t, l)
    }

    fn scales(t: &TextLayer) -> Vec<f32> {
        t.fit.as_ref().map(|f| f.lines.iter().map(|l| l.scale).collect()).unwrap_or_default()
    }

    fn widths(l: &crate::TextLayout) -> Vec<f32> {
        l.lines.iter().map(|li| li.x1 - li.x0).collect()
    }

    fn bottom(l: &crate::TextLayout) -> f32 {
        l.lines.last().map_or(0.0, |li| li.baseline + li.descent)
    }

    fn assert_spans(l: &crate::TextLayout, w: f32) {
        for (i, got) in widths(l).into_iter().enumerate() {
            assert!((got - w).abs() <= 1.5, "line {i} is {got} px wide, box {w}: {:?}", widths(l));
        }
    }

    #[test]
    fn every_line_spans_the_box_width() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box(FOX, 12.0, 400.0, 300.0));
        let s = scales(&t);
        assert!(s.len() >= 2 && l.lines.len() == s.len(), "{s:?}");
        assert_spans(&l, 400.0);
        let (lo, hi) = s.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 1e-3, "lines of different lengths get different sizes: {s:?}");
    }

    #[test]
    fn justified_lines_still_span_the_box_width() {
        use photocraft_doc::text::{ParagraphStyle, TextAlign};
        for align in [TextAlign::JustifyLeft, TextAlign::JustifyCenter, TextAlign::JustifyRight, TextAlign::JustifyAll] {
            let mut e = TextEngine::new();
            let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
            with_para(&mut t, ParagraphStyle { align, ..Default::default() });
            let (t, l) = solved(&mut e, t);
            assert!(scales(&t).len() >= 2, "{align:?}: {:?}", scales(&t));
            assert_spans(&l, 400.0);
            // Every line starts at the box's left edge: justification adds no space.
            assert!(l.lines.iter().all(|ln| ln.x0.abs() <= 1.5), "{align:?}: {:?}", l.lines.iter().map(|ln| ln.x0).collect::<Vec<_>>());
        }
    }

    #[test]
    fn short_line_is_bigger_and_both_span_width() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box("I AM A\nEXPANDABLE", 12.0, 400.0, 300.0));
        let s = scales(&t);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[0] > s[1], "{s:?}");
        assert_spans(&l, 400.0);
    }

    #[test]
    fn narrower_box_gives_more_lines() {
        let mut e = TextEngine::new();
        let (wide, _) = solved(&mut e, fit_box(FOX, 12.0, 600.0, 300.0));
        let (narrow, _) = solved(&mut e, fit_box(FOX, 12.0, 200.0, 300.0));
        assert!(scales(&narrow).len() > scales(&wide).len(), "{:?} vs {:?}", scales(&narrow), scales(&wide));
    }

    #[test]
    fn block_fits_box_height_when_width_sizing_allows() {
        // Width-only sizing: the `w` search still picks lines whose stack fits the box, so when
        // such lines exist nothing is hidden and the block ends inside the box.
        let mut e = TextEngine::new();
        for (text, w, h, line_gap) in [(FOX, 400.0, 300.0, 0.0), (FOX, 300.0, 500.0, 50.0), (FOX, 250.0, 90.0, 200.0)] {
            let mut t = fit_box(text, 12.0, w, h);
            t.fit = Some(TextFit { line_gap, ..Default::default() });
            let (_, l) = solved(&mut e, t);
            assert!(bottom(&l) <= h + 0.5, "{text:?} {w}x{h} gap {line_gap}: bottom {}", bottom(&l));
            assert!(!l.overflowed && !l.hidden, "{text:?} {w}x{h} gap {line_gap}");
        }
        // One line per paragraph is already taller than the box: the lines keep spanning its
        // width (no height shrink), the last one hangs past the bottom and the fit reports it.
        let mut t = fit_box("I AM A\nEXPANDABLE", 12.0, 400.0, 120.0);
        t.fit = Some(TextFit { line_gap: -50.0, ..Default::default() });
        let r = refit(&mut e, &mut t, 72.0);
        let l = e.layout(&t, 72.0);
        assert_spans(&l, 400.0);
        assert!(!r.fits && bottom(&l) > 120.5, "bottom {}", bottom(&l));
    }

    #[test]
    fn too_short_box_hides_lines_instead_of_shrinking() {
        // Even one line per paragraph is too tall: the lines keep spanning the box width and the
        // ones below the bottom are hidden (Photoshop cuts them off), the layer flagged overflowed.
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box("ONE\nTWO WORDS\nTHREE", 12.0, 400.0, 20.0));
        assert_eq!(scales(&t).len(), 3, "one line per paragraph: {:?}", scales(&t));
        assert_spans(&l, 400.0);
        assert!(l.hidden && l.overflowed && l.lines.len() < 3, "{} visible", l.lines.len());
    }

    const CAFE: &str = "Typography is considered as an art form in itself. People have spent their entire lives in the study and art of typography. If you think this is new? Remember that the Romans created the Romanized fonts that we use today over 2,000 years ago. The Chinese and Middle Eastern cultures have been designing with type for thousands of years";

    #[test]
    fn lines_mix_long_and_short_for_contrast() {
        // Photoshop's examples: natural widths vary 2-3x within one block.
        let mut eng = engine();
        let t = fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 400.0));
        let lines = solve_box(&mut eng, &t, 72.0);
        let scales: Vec<f32> = lines.iter().map(|l| l.scale).collect();
        let (hi, lo) = scales.iter().fold((f32::MIN, f32::MAX), |(h, l), &s| (h.max(s), l.min(s)));
        assert!(lines.len() >= 5, "{lines:?}");
        assert!(hi / lo >= 2.5, "size contrast {hi}/{lo}: {scales:?}");
        // Short lines of two or three words sit between the long ones.
        let words: Vec<usize> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| CAFE[l.start as usize..lines.get(i + 1).map_or(CAFE.len(), |n| n.start as usize)].split_whitespace().count())
            .collect();
        assert!(words.iter().filter(|&&n| n <= 3).count() >= 2, "words per line {words:?}");
        // Neighbours differ by at most about MAX_CONTRAST.
        for p in scales.windows(2) {
            let r = p[0].max(p[1]) / p[0].min(p[1]);
            assert!(r <= 3.05, "neighbour ratio {r}: {scales:?}");
        }
    }

    #[test]
    fn every_visible_line_spans_the_box() {
        let mut eng = engine();
        let mut t = fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 400.0));
        refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        for l in &out.lines {
            assert!((l.x1 - l.x0 - 500.0).abs() <= 1.0, "{} wide", l.x1 - l.x0);
        }
    }

    #[test]
    fn too_long_text_overflows_instead_of_shrinking() {
        let mut eng = engine();
        let mut t = fitted_box(&CAFE.repeat(6), 24.0, (0.0, 0.0, 300.0, 120.0));
        refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        assert!(out.hidden && out.overflowed, "lines past the bottom are hidden");
        // Width-only: visible lines still span the box (not shrunk to fit the height).
        assert!(out.lines.iter().all(|l| (l.x1 - l.x0 - 300.0).abs() <= 1.0));
        // Not shrunk to fit: the visible lines stay near the base size (MAX_SHRINK_W bounds `w`).
        let lines = &t.fit.as_ref().unwrap().lines;
        for l in &out.lines {
            let scale = lines.iter().find(|f| f.start as usize == l.range.start).map(|f| f.scale).unwrap();
            assert!(scale >= 0.3, "visible line at {} has scale {scale}", l.range.start);
        }
    }

    #[test]
    fn block_ends_near_the_box_bottom() {
        // The search keeps the tallest fitting probe: the last line ends within 20% of the box
        // height above its bottom edge.
        let mut eng = engine();
        let mut t = fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 400.0));
        refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        let b = bottom(&out);
        assert!(!out.hidden && (400.0 * 0.8..=400.5).contains(&b), "block bottom {b}");
    }

    #[test]
    fn a_big_line_never_collides_with_the_line_below() {
        // Each line is stacked from the previous line's descent: a big line's descenders stay
        // clear of the smaller line under it, even at a 0% line gap.
        let mut eng = engine();
        let mut t = fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 400.0));
        refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        assert!(out.lines.len() >= 5);
        for p in out.lines.windows(2) {
            let (prev, next) = (&p[0], &p[1]);
            assert!(next.baseline - next.ascent >= prev.baseline + prev.descent - 0.5, "{prev:?} overlaps {next:?}");
        }
    }

    #[test]
    fn taller_box_gives_more_lines() {
        let mut eng = engine();
        let short = solve_box(&mut eng, &fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 250.0)), 72.0).len();
        let tall = solve_box(&mut eng, &fitted_box(CAFE, 24.0, (0.0, 0.0, 500.0, 900.0)), 72.0).len();
        assert!(tall > short, "{short} -> {tall}");
    }

    #[test]
    fn paragraph_longer_than_one_dp_pass_keeps_every_word_in_order() {
        // MAX_DP_WORDS bounds one DP pass; a longer paragraph is broken in passes, still one
        // line after another with no word lost.
        let words = MAX_DP_WORDS + 900;
        let segs = vec![Segment {
            start: 0,
            words: (0..words)
                .map(|i| Word { start: i * 5, lead: if i == 0 { 0.0 } else { 4.0 }, width: 20.0 + (i % 7) as f32 * 6.0, pt: 12.0, first_pt: 12.0 })
                .collect(),
            para_first: true,
            ascent: 0.8,
            descent: 0.2,
            pt: 12.0,
            avail: 400.0,
            first_indent: 0.0,
            space_before: 0.0,
            space_after: 0.0,
        }];
        let lines = contrast_breaks(&segs, 300.0);
        assert_eq!(lines.first().map(|c| c.first), Some(0));
        assert_eq!(lines.last().map(|c| c.end), Some(words));
        assert!(lines.windows(2).all(|p| p[0].end == p[1].first && p[0].first < p[0].end), "lines chain");
        assert!(lines.iter().any(|c| c.end == MAX_DP_WORDS), "a pass ends a line");
        assert!(lines.iter().all(|c| c.end - c.first == 1 || c.natural <= 300.0 * 1.0001));
    }

    #[test]
    fn one_huge_word_gets_its_own_clamped_line() {
        let mut eng = engine();
        let lines = solve_box(&mut eng, &fitted_box(&"x".repeat(500), 24.0, (0.0, 0.0, 300.0, 300.0)), 72.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].scale * 24.0 >= MIN_PT - 1e-4);
    }

    #[test]
    fn one_long_word_is_bounded() {
        let mut e = TextEngine::new();
        let (t, l) = solved(&mut e, fit_box(&"a".repeat(500), 12.0, 50.0, 50.0));
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0), "{s:?}");
        assert!(!l.hidden);
    }

    #[test]
    fn word_gap_minus_100_removes_spaces() {
        let mut e = TextEngine::new();
        let natural = |e: &mut TextEngine, text: &str| {
            let mut t = fit_box(text, 12.0, 400.0, 400.0);
            t.fit = Some(TextFit { word_gap: -100.0, ..Default::default() });
            let s = scales(&solved(e, t).0);
            assert_eq!(s.len(), 1, "{text:?}: {s:?}");
            400.0 / s[0]
        };
        let (spaced, joined) = (natural(&mut e, "aa bb"), natural(&mut e, "aabb"));
        assert!((spaced - joined).abs() <= 1.0, "aa bb {spaced} vs aabb {joined}");
    }

    #[test]
    fn refit_box_keeps_base_sizes() {
        let mut e = TextEngine::new();
        let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
        set_run_sizes(&mut t, &[(10, 20.0), (FOX.len() - 10, 12.0)]);
        let runs = t.runs.clone();
        let r = refit(&mut e, &mut t, 72.0);
        assert_eq!(t.size_pt, 12.0);
        assert_eq!(t.runs, runs);
        assert!(r.fits && r.scale == 1.0 && r.probes <= MAX_PROBES, "{r:?}");
        assert!(!t.fit.as_ref().is_none_or(|f| f.lines.is_empty()));
        // Mixed sizes keep their ratio on a line: the solved layout spans the width all the same.
        assert_spans(&e.layout(&t, 72.0), 400.0);
    }

    #[test]
    fn empty_paragraphs_and_breaks() {
        let mut e = TextEngine::new();
        let text = "ONE TWO\n\nTHREE\u{3}FOUR FIVE\n";
        let (t, l) = solved(&mut e, fit_box(text, 12.0, 300.0, 600.0));
        let lines = &t.fit.as_ref().unwrap().lines;
        let starts: Vec<u32> = lines.iter().map(|l| l.start).collect();
        // Every paragraph and forced line break starts a line; empty paragraphs keep scale 1.
        for must in [0, 8, 9, 15, 25] {
            assert!(starts.contains(&must), "{starts:?}");
        }
        for (i, line) in lines.iter().enumerate() {
            let piece = &text[line.start as usize..lines.get(i + 1).map_or(text.len(), |n| n.start as usize)];
            if piece.trim_end_matches(['\n', '\u{3}']).is_empty() {
                assert_eq!(line.scale, 1.0, "empty line {i} {starts:?}");
            }
        }
        assert!(!l.hidden && bottom(&l) <= 600.5);
    }

    #[test]
    fn lines_start_at_word_boundaries_not_inside_clusters() {
        let mut e = TextEngine::new();
        // Combining acute accents (U+0301) after base letters and after a space.
        // No-break spaces (U+00A0, U+202F) glue their neighbours: never a line start after one.
        let text = "cafe\u{301} re\u{301}sume\u{301} \u{301}x na\u{ef}ve words 10\u{a0}km here 5\u{202f}kg and 20\u{a0}km there 7\u{202f}kg";
        let (t, _) = solved(&mut e, fit_box(text, 12.0, 120.0, 400.0));
        let lines = &t.fit.as_ref().unwrap().lines;
        assert!(lines.len() >= 2, "{lines:?}");
        for l in lines.iter().skip(1) {
            let at = l.start as usize;
            assert!(text.is_char_boundary(at));
            let next = text[at..].chars().next().unwrap();
            assert!(next != '\u{301}' && !next.is_whitespace(), "line at {at} starts with {next:?}");
            assert!(text[..at].ends_with(' '), "line at {at} doesn't follow a space");
        }
    }

    #[test]
    fn hostile_input_is_bounded() {
        let mut e = TextEngine::new();
        for t in [
            fit_box(&"word ".repeat(3000), 12.0, 400.0, 300.0),
            fit_box(FOX, 12.0, 0.0, 0.0),
            fit_box(FOX, 12.0, f32::NAN, f32::INFINITY),
            fit_box(FOX, 1296.0, 1e6, 1e6),
            fit_box(FOX, 0.1, 1.0, 1e6),
            fit_box("   ", 12.0, 400.0, 300.0),
        ] {
            let lines = solve_box(&mut e, &t, 72.0);
            assert!(lines.iter().all(|l| l.scale.is_finite() && l.scale > 0.0));
            let mut f = t.fit.clone().unwrap();
            f.lines = lines.clone();
            assert_eq!(f.clone().sanitized(&t.text).lines, lines, "valid lines");
        }
        assert!(solve_box(&mut e, &fit_box("", 12.0, 400.0, 300.0), 72.0).is_empty());
    }

    #[test]
    fn search_and_layouts_are_bounded() {
        let mut e = TextEngine::new();
        let long = "lorem ipsum dolor sit amet ".repeat(40);
        let cases: [(&str, f32, f32, f32); 8] = [
            (FOX, 400.0, 300.0, 12.0),
            (FOX, 600.0, 300.0, 12.0),
            (FOX, 200.0, 300.0, 12.0),
            (
                "I AM A
EXPANDABLE",
                400.0,
                300.0,
                12.0,
            ),
            (
                "ONE
TWO WORDS
THREE",
                400.0,
                20.0,
                12.0,
            ),
            (&long, 500.0, 700.0, 12.0),
            (&long, 300.0, 2000.0, 72.0),
            ("Hi", 200.0, 80.0, 12.0),
        ];
        for (text, w, h, size) in cases {
            let t = fit_box(text, size, w, h);
            let r = solve(&mut e, &t, 72.0);
            assert!(r.probes <= MAX_PROBES && r.layouts <= 1 + REFINE_PASSES, "{text:?}: {} probes, {} layouts", r.probes, r.layouts);
            let mut t2 = t.clone();
            t2.fit.as_mut().unwrap().lines = r.lines.clone();
            let l = e.layout(&t2, 72.0);
            let err = l.lines.iter().map(|li| (li.x1 - li.x0 - w).abs()).fold(0.0f32, f32::max);
            eprintln!(
                "{:.24?} {w}x{h} {size}pt: {} lines, {} probes, {} layouts, max width error {err:.3} px, bottom {:.2}, hidden {}",
                text,
                r.lines.len(),
                r.probes,
                r.layouts,
                bottom(&l),
                l.hidden
            );
        }
    }

    #[test]
    fn indents_shorten_each_lines_target() {
        let mut e = TextEngine::new();
        let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
        with_para(
            &mut t,
            photocraft_doc::text::ParagraphStyle { start_indent_pt: 20.0, end_indent_pt: 10.0, first_line_indent_pt: 30.0, ..Default::default() },
        );
        let (t, l) = solved(&mut e, t);
        assert!(scales(&t).len() >= 2, "{:?}", scales(&t));
        let w = widths(&l);
        assert!((w[0] - 340.0).abs() <= 1.5, "first line takes the first-line indent: {w:?}");
        assert!(w[1..].iter().all(|v| (v - 370.0).abs() <= 1.5), "{w:?}");
        assert!(!l.overflowed);
    }

    #[test]
    fn horizontal_scale_and_kerning_lines_span_the_visible_width() {
        let mut e = TextEngine::new();
        for (hs, kern) in [(1.5, 40.0), (0.8, -30.0)] {
            let mut t = fit_box(FOX, 12.0, 400.0, 300.0);
            set_run_sizes(&mut t, &[(FOX.len(), 12.0)]);
            t.runs[0].style.horizontal_scale = hs;
            t.runs[0].style.kern = kern;
            let (t, l) = solved(&mut e, t);
            assert!(scales(&t).len() >= 2, "{:?}", scales(&t));
            assert_spans(&l, 400.0);
            assert!(!l.overflowed, "hs {hs} kern {kern}: solved lines aren't flagged too wide");
        }
    }

    #[test]
    fn many_short_lines_stay_linear() {
        let mut e = TextEngine::new();
        let text = "ab cd
"
        .repeat(5000);
        let t = fit_box(&text, 12.0, 400.0, 300.0);
        let r = solve(&mut e, &t, 72.0);
        assert_eq!(r.lines.len(), 5001, "one line per paragraph (the box is too short)");
        assert!(r.probes <= MAX_PROBES && r.layouts <= 1 + REFINE_PASSES, "{} probes, {} layouts", r.probes, r.layouts);
        assert!(r.visited <= 2 * r.lines.len(), "{} base lines visited for {} segments", r.visited, r.lines.len());
    }

    #[test]
    fn refit_box_replaces_only_non_finite_sizes() {
        let mut e = TextEngine::new();
        let mut t = fit_box("ab cd ef", f32::NAN, 300.0, 200.0);
        set_run_sizes(&mut t, &[(2, f32::NAN), (3, 30.0), (3, f32::NEG_INFINITY)]);
        t.runs[1].style.leading_pt = Some(f32::NAN);
        t.runs[1].style.baseline_shift_pt = f32::INFINITY;
        refit(&mut e, &mut t, 72.0);
        assert!(t.size_pt.is_finite() && t.runs.iter().all(|r| r.style.size_pt.is_finite()), "{:?}", t.runs);
        assert_eq!(t.runs[1].style.size_pt, 30.0, "a valid base size is untouched");
        assert!(t.runs[1].style.leading_pt.is_none_or(f32::is_finite) && t.runs[1].style.baseline_shift_pt.is_finite());
        // Valid sizes, even outside the type range, are the user's base: left alone.
        let mut u = fit_box("ab cd", 12.0, 300.0, 200.0);
        set_run_sizes(&mut u, &[(2, 2000.0), (3, 0.05)]);
        let runs = u.runs.clone();
        refit(&mut e, &mut u, 72.0);
        assert_eq!(u.runs, runs);
    }

    #[test]
    fn long_text_in_a_tiny_box_overflows_at_its_width_size() {
        // Width-only: each line is sized to span the 2 px box (never below MIN_PT) and not
        // shrunk further to fit the height; what doesn't fit is hidden.
        let mut e = TextEngine::new();
        let mut t = fit_box(&"word ".repeat(5000), 12.0, 2.0, 2.0);
        let r = refit(&mut e, &mut t, 72.0);
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0) && r.probes <= MAX_PROBES);
        let smallest = s.iter().map(|v| v * t.size_pt).fold(f32::MAX, f32::min);
        assert!(smallest > 2.0 * MIN_PT, "sized by width, not bottomed out at MIN_PT: {smallest}");
        let l = e.layout(&t, 72.0);
        assert!(!r.fits && l.hidden && l.overflowed);
    }

    #[test]
    fn box_indents_wider_than_the_box_stay_finite_and_fit() {
        let mut e = TextEngine::new();
        let mut t = fit_box("one two three four", 12.0, 100.0, 200.0);
        with_para(&mut t, photocraft_doc::text::ParagraphStyle { start_indent_pt: 60.0, end_indent_pt: 60.0, ..Default::default() });
        let (t, l) = solved(&mut e, t);
        let s = scales(&t);
        assert!(!s.is_empty() && s.iter().all(|v| v.is_finite() && *v > 0.0), "{s:?}");
        assert!(!l.hidden && bottom(&l) <= 200.5, "bottom {}", bottom(&l));
    }

    #[test]
    fn fits_means_the_whole_block_is_inside_the_box() {
        let mut e = TextEngine::new();
        // Two empty paragraphs (scale 1, nothing to shrink): the second baseline is inside the
        // box but its descent hangs below, so nothing is hidden and the block still doesn't fit.
        let mut big = fit_box(
            "
", 12.0, 400.0, 400.0,
        );
        refit(&mut e, &mut big, 72.0);
        let full = bottom(&e.layout(&big, 72.0));
        let mut t = fit_box(
            "
",
            12.0,
            400.0,
            full - 1.5,
        );
        let r = refit(&mut e, &mut t, 72.0);
        let l = e.layout(&t, 72.0);
        assert!(!l.hidden && bottom(&l) > full - 1.0, "{} vs box {}", bottom(&l), full - 1.5);
        assert!(!r.fits, "{r:?}");
        assert!(refit(&mut e, &mut big, 72.0).fits);
    }

    /// Every glyph of `b` sits within 0.5 px of the same glyph of `a`, in order. A fitted layout
    /// shapes each solved line on its own and so has an invisible direction mark glyph per line
    /// that the baked paragraph does not: those are skipped.
    fn assert_same_look(a: &crate::TextLayout, b: &crate::TextLayout) {
        assert!(!b.glyphs.is_empty());
        let mut from = 0;
        for (i, q) in b.glyphs.iter().enumerate() {
            let found = a.glyphs.get(from..).and_then(|rest| rest.iter().position(|p| p.id == q.id && (p.x - q.x).abs() <= 0.5 && (p.y - q.y).abs() <= 0.5));
            let Some(off) = found else { panic!("glyph {i} at ({}, {}) has no match in the fitted layout", q.x, q.y) };
            from += off + 1;
        }
        // A direction mark per fitted line, and the space each baked break replaced.
        assert!(a.glyphs.len() - b.glyphs.len() <= 2 * a.lines.len(), "{} vs {} glyphs", a.glyphs.len(), b.glyphs.len());
    }

    fn baked_matches(t: TextLayer) -> TextLayer {
        let mut e = TextEngine::new();
        let (t, fitted) = solved(&mut e, t);
        let baked = bake(&mut e, &t, 72.0).unwrap();
        assert!(baked.fit.is_none());
        assert_same_look(&fitted, &e.layout(&baked, 72.0));
        baked
    }

    fn sizes(t: &TextLayer) -> Vec<f32> {
        let mut v: Vec<f32> = t.char_runs().iter().map(|r| r.style.size_pt).collect();
        v.dedup();
        v
    }

    #[test]
    fn bake_keeps_the_look_and_gives_each_line_its_size() {
        let baked = baked_matches(fit_box(FOX, 12.0, 400.0, 300.0));
        assert!(sizes(&baked).len() >= 2, "{:?}", sizes(&baked));
        assert!(baked.text.contains(FORCED_BREAK), "{:?}", baked.text);
        assert_eq!(baked.text.replace(FORCED_BREAK, " "), FOX, "each break replaces a space");
        let total: usize = baked.char_runs().iter().map(|r| r.len).sum();
        assert_eq!(total, baked.text.len());
        let para: usize = baked.paragraph_runs().iter().map(|r| r.len).sum();
        assert_eq!(para, baked.text.len());
    }

    #[test]
    fn bake_keeps_the_look_with_gaps() {
        for (word, line) in [(50.0, 0.0), (-100.0, 25.0), (0.0, -30.0), (120.0, 80.0)] {
            let mut t = fit_box(FOX, 12.0, 400.0, 400.0);
            if let Some(f) = t.fit.as_mut() {
                f.word_gap = word;
                f.line_gap = line;
            }
            baked_matches(t);
        }
    }

    #[test]
    fn bake_does_not_double_existing_breaks() {
        let text = "ONE TWO\n\nTHREE\u{3}FOUR FIVE\n";
        let baked = baked_matches(fit_box(text, 12.0, 300.0, 600.0));
        assert!(!baked.text.contains("\u{3}\u{3}") && !baked.text.contains("\n\u{3}") && !baked.text.contains("\u{3}\n"), "{:?}", baked.text);
        assert!(baked.text.contains("THREE\u{3}FOUR"));
    }

    #[test]
    fn bake_keeps_mixed_runs_and_paragraph_styles() {
        let mut t = fit_box(FOX, 12.0, 300.0, 400.0);
        set_run_sizes(&mut t, &[(10, 20.0), (FOX.len() - 10, 12.0)]);
        let style = photocraft_doc::text::ParagraphStyle { first_line_indent_pt: 20.0, space_after_pt: 6.0, ..Default::default() };
        t.paragraphs = vec![photocraft_doc::text::ParagraphRun { len: FOX.len(), style }];
        let baked = baked_matches(t);
        assert_eq!(baked.paragraph_runs().len(), 1);
    }

    #[test]
    fn bake_of_text_without_solved_lines_only_drops_the_fit() {
        let mut e = TextEngine::new();
        let mut t = box_layer(FOX, 12.0, (0.0, 0.0, 300.0, 100.0));
        assert_eq!(bake(&mut e, &t, 72.0).unwrap(), t);
        t.fit = Some(TextFit::default());
        let b = bake(&mut e, &t, 72.0).unwrap();
        assert!(b.fit.is_none() && b.text == t.text, "no solved lines: nothing to bake");
        // Area text with no solved lines yet: the same.
        let mut a = area_ellipse_layer("Hello world", 12.0, 200.0, 100.0);
        a.fit = Some(TextFit::default());
        let b = bake(&mut e, &a, 72.0).unwrap();
        assert!(b.fit.is_none() && b.text == a.text);
    }

    #[test]
    fn bake_sizes_are_clamped_and_finite() {
        let mut e = TextEngine::new();
        let mut t = fit_box("hi", 1000.0, 5000.0, 5000.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = vec![FitLine { start: 0, scale: 50.0 }];
        }
        let b = bake(&mut e, &t, 72.0).unwrap();
        assert!(b.char_runs().iter().all(|r| r.style.size_pt <= MAX_PT && r.style.size_pt >= MIN_PT));
    }

    #[test]
    fn bake_replaces_line_ending_spaces_with_the_break() {
        let baked = baked_matches(fit_box(FOX, 12.0, 400.0, 300.0));
        assert!(!baked.text.contains(" \u{3}"), "{:?}", baked.text);
        assert!(
            baked.char_runs().iter().all(|r| r.style.tracking == 0.0),
            "plain runs: {:?}",
            baked.char_runs().iter().map(|r| r.style.tracking).collect::<Vec<_>>()
        );
        // Typing at a former line end after a release: type_fit_cmds' tests type there.
    }

    #[test]
    fn bake_keeps_the_look_with_horizontal_scale_and_word_gap() {
        let make = |hs: f32| {
            let mut t = fit_box(FOX, 12.0, 400.0, 400.0);
            t.runs = vec![TextRun { len: FOX.len(), style: photocraft_doc::text::CharStyle { horizontal_scale: hs, ..t.char_runs()[0].style.clone() } }];
            if let Some(f) = t.fit.as_mut() {
                f.word_gap = 60.0;
            }
            t
        };
        // Wider type: the word gap tracking is not scaled twice.
        baked_matches(make(1.3));
        // Condensed type: box wrapping measures the unscaled width, which is wider than the box,
        // so baked lines would wrap. It is refused before baking, naming the cause.
        let mut e = TextEngine::new();
        let (t, _) = solved(&mut e, make(0.8));
        let m = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(m.contains("condensed type (Horizontal Scale below 100%)"), "{m}");
    }

    #[test]
    fn same_glyphs_allows_only_a_few_unmatched_glyphs_per_line() {
        let mut e = TextEngine::new();
        let full = e.layout(&box_layer("Hello world and more words", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert_eq!(full.lines.len(), 1);
        // Dropping a whole word leaves many fitted glyphs unmatched: not the same look.
        let short = e.layout(&box_layer("Hello world", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert!(!same_glyphs(&full, &short));
        // Dropping one trailing space (as a baked break does) still is.
        let spaced = e.layout(&box_layer("Hello world ", 12.0, (0.0, 0.0, 400.0, 100.0)), 72.0);
        assert!(same_glyphs(&spaced, &short));
    }

    #[test]
    fn bake_reports_the_line_count_when_every_margin_fails() {
        let mut e = TextEngine::new();
        // A line 0.05 % wider than a wide box: the first margin leaves it wrapping, the second
        // keeps the line count but moves the far glyphs more than half a pixel.
        let (mut t, _) = solved(&mut e, fit_box(FOX, 12.0, 1500.0, 300.0));
        if let Some(f) = t.fit.as_mut() {
            f.lines.iter_mut().for_each(|l| l.scale *= 1.0005);
        }
        let m = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(m.contains("lays out in"), "{m}");
    }

    #[test]
    fn bake_refuses_a_look_it_cannot_keep() {
        let mut e = TextEngine::new();
        // One solved line for text far wider than the box: the fitted layout never wraps, baked text would.
        let mut t = fit_box(FOX, 12.0, 100.0, 300.0);
        if let Some(f) = t.fit.as_mut() {
            f.lines = vec![FitLine { start: 0, scale: 1.0 }];
        }
        let err = bake(&mut e, &t, 72.0).unwrap_err().to_string();
        assert!(err.contains("lays out in"), "{err}");
    }

    #[test]
    fn bake_goes_past_the_first_margin_and_keeps_half_a_pixel() {
        let mut e = TextEngine::new();
        let (mut t, _) = solved(&mut e, fit_box(FOX, 12.0, 400.0, 400.0));
        // Lines 0.05 % wider than the box: the first margin (0.001 %) leaves them wrapping.
        if let Some(f) = t.fit.as_mut() {
            f.lines.iter_mut().for_each(|l| l.scale *= 1.0005);
        }
        let fitted = e.layout(&t, 72.0);
        let baked = bake(&mut e, &t, 72.0).expect("a later margin keeps the lines");
        let bl = e.layout(&baked, 72.0);
        assert_same_look(&fitted, &bl);
    }

    // Dynamic Text in shapes: per-line sizing (spec: Behaviour 4).

    /// Area type in the polygon `pts`.
    fn area_layer(text: &str, pt: f32, pts: &[(f64, f64)]) -> TextLayer {
        crate::tests::area_layer(text, pt, pts)
    }

    /// A 400×300 ellipse as a 64-point polygon, Dynamic Text on.
    fn ellipse_area(text: &str, pt: f32) -> TextLayer {
        let pts: Vec<(f64, f64)> = (0..64)
            .map(|i| {
                let a = i as f64 / 64.0 * std::f64::consts::TAU;
                (200.0 + 200.0 * a.cos(), 150.0 + 150.0 * a.sin())
            })
            .collect();
        let mut t = area_layer(text, pt, &pts);
        t.fit = Some(TextFit::default());
        t
    }

    /// A 400×400 square minus a centred 160×160 square (Combine Shapes: Subtract), Dynamic Text on.
    fn ring_area(text: &str, pt: f32) -> TextLayer {
        use photocraft_doc::vector::{Path, PathOp, Subpath};
        let path = Path::new(vec![
            Subpath::polygon(&[(0.0, 0.0), (400.0, 0.0), (400.0, 400.0), (0.0, 400.0)]),
            Subpath::polygon(&[(120.0, 120.0), (280.0, 120.0), (280.0, 280.0), (120.0, 280.0)]).with_op(PathOp::Subtract),
        ]);
        let mut t = crate::tests::area_layer(text, pt, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]);
        t.shape = TextShape::Area { path };
        t.fit = Some(TextFit::default());
        t
    }

    fn area_shape(t: &TextLayer) -> crate::area::AreaShape {
        let TextShape::Area { path } = &t.shape else { panic!("not area type") };
        crate::area::AreaShape::new(path, false).unwrap()
    }

    fn widest_span(shape: &crate::area::AreaShape, l: &crate::LineInfo) -> f32 {
        shape.spans(l.baseline - l.ascent, l.baseline + l.descent, 1.0).iter().map(|s| s.1 - s.0).fold(0.0f32, f32::max)
    }

    #[test]
    fn area_lines_span_their_rows_and_middle_is_biggest() {
        let mut eng = engine();
        let mut t = ellipse_area(CAFE, 12.0);
        refit(&mut eng, &mut t, 72.0);
        assert!(t.runs.iter().all(|r| r.style.size_pt == 12.0), "base sizes kept");
        let lines = t.fit.as_ref().unwrap().lines.clone();
        assert!(lines.len() >= 4, "{lines:?}");
        let out = eng.layout(&t, 72.0);
        let shape = area_shape(&t);
        for l in &out.lines {
            let widest = widest_span(&shape, l);
            assert!((l.x1 - l.x0) <= widest + 1.0 && (l.x1 - l.x0) >= widest * 0.97 - 1.0, "line {} vs row {widest}", l.x1 - l.x0);
        }
        // The middle rows are the widest, so the middle lines' rows are taller than the edges'.
        let h = |l: &crate::LineInfo| l.ascent + l.descent;
        let n = out.lines.len();
        let mid = out.lines[n / 3..n - n / 3].iter().map(h).fold(0.0f32, f32::max);
        assert!(mid > h(&out.lines[0]) && mid > h(&out.lines[n - 1]), "{:?}", out.lines);
        for g in &out.glyphs {
            assert!(shape.contains(g.x, g.y), "{g:?}");
        }
    }

    #[test]
    fn area_rectangle_matches_box() {
        let mut eng = engine();
        let mut a = area_layer(CAFE, 12.0, &[(0.0, 0.0), (500.0, 0.0), (500.0, 400.0), (0.0, 400.0)]);
        a.fit = Some(TextFit::default());
        refit(&mut eng, &mut a, 72.0);
        let b = solve_box(&mut eng, &fitted_box(CAFE, 12.0, (0.0, 0.0, 500.0, 400.0)), 72.0);
        let al = &a.fit.as_ref().unwrap().lines;
        assert_eq!(al.iter().map(|l| l.start).collect::<Vec<_>>(), b.iter().map(|l| l.start).collect::<Vec<_>>());
        for (x, y) in al.iter().zip(&b) {
            assert!((x.scale / y.scale - 1.0).abs() < 0.01, "{al:?} vs {b:?}");
        }
    }

    #[test]
    fn area_too_long_hides_rather_than_shrinks() {
        let mut eng = engine();
        let mut t = ellipse_area(&CAFE.repeat(30), 12.0);
        let r = refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        assert!(out.hidden && out.overflowed && !r.fits);
        // Sizes come from each line's row, not from shrinking the block: the visible lines in the
        // middle of the ellipse stay near or above the base size.
        let shown = out.lines.len();
        assert!(t.fit.as_ref().unwrap().lines[..shown].iter().any(|l| l.scale >= 0.9), "{:?}", t.fit);
    }

    #[test]
    fn ring_shape_keeps_glyphs_inside() {
        // Rows through the hole are split; each line takes the widest span of its band and no
        // glyph lands in the hole.
        let mut eng = engine();
        let mut t = ring_area(CAFE, 12.0);
        refit(&mut eng, &mut t, 72.0);
        let out = eng.layout(&t, 72.0);
        assert!(!out.lines.is_empty());
        let shape = area_shape(&t);
        for g in &out.glyphs {
            assert!(shape.contains(g.x, g.y), "{g:?}");
        }
        // No line is wider than its band's widest span (a line can be held smaller by the hole
        // below it); bands through the hole whose sides can't hold a line at a third of the
        // largest size are left empty.
        for l in &out.lines {
            let widest = widest_span(&shape, l);
            assert!((l.x1 - l.x0) <= widest + 1.0, "{l:?} vs {widest}");
        }
    }

    #[test]
    fn area_release_bakes_within_half_a_pixel() {
        let mut eng = engine();
        let mut t = ellipse_area(CAFE, 12.0);
        refit(&mut eng, &mut t, 72.0);
        let baked = bake(&mut eng, &t, 72.0).expect("area bake");
        assert!(baked.fit.is_none() && matches!(baked.shape, TextShape::Area { .. }));
        assert!(baked.runs.iter().any(|r| r.style.size_pt != 12.0), "explicit per-line sizes");
    }

    #[test]
    fn area_hostile_shapes_finish_within_bounds() {
        let mut eng = engine();
        // A 1×1 shape, a 500-character word, empty text and a 4096-anchor path.
        let mut tiny = area_layer("hello world", 12.0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        tiny.fit = Some(TextFit::default());
        let s = solve_shape(&mut eng, &tiny, 72.0);
        assert!(s.probes <= MAX_PROBES && s.layouts <= 1 + REFINE_PASSES);
        let mut word = ellipse_area(&"w".repeat(500), 12.0);
        let r = refit(&mut eng, &mut word, 72.0);
        assert!(r.probes <= MAX_PROBES && word.fit.as_ref().unwrap().lines.iter().all(|l| l.scale.is_finite()));
        let empty = ellipse_area("", 12.0);
        assert!(solve_area(&mut eng, &empty, 72.0).is_empty());
        let pts: Vec<(f64, f64)> = (0..4096)
            .map(|i| {
                let a = i as f64 / 4096.0 * std::f64::consts::TAU;
                (200.0 + 200.0 * a.cos(), 200.0 + 200.0 * (1.0 + 0.1 * (a * 50.0).sin()) * a.sin())
            })
            .collect();
        let mut many = area_layer(CAFE, 12.0, &pts);
        many.fit = Some(TextFit::default());
        let s = solve_shape(&mut eng, &many, 72.0);
        assert!(s.probes <= MAX_PROBES && s.layouts <= 1 + REFINE_PASSES && !s.lines.is_empty());
    }

    /// The speech bubble of the earlier renders: a rounded 500×350 body with a tail below.
    fn bubble_area(text: &str, pt: f32) -> TextLayer {
        use photocraft_doc::vector::{Knot, Path, Subpath};
        use photocraft_geom::Point;
        let p = Point::new;
        let k = |a: (f64, f64), i: (f64, f64), o: (f64, f64)| Knot { anchor: p(a.0, a.1), in_ctrl: p(i.0, i.1), out_ctrl: p(o.0, o.1), smooth: false };
        let c = |a: (f64, f64)| k(a, a, a);
        let knots = vec![
            k((225.0, 75.0), (185.0, 75.0), (225.0, 75.0)),
            k((575.0, 75.0), (575.0, 75.0), (615.0, 75.0)),
            k((650.0, 150.0), (650.0, 110.0), (650.0, 150.0)),
            k((650.0, 350.0), (650.0, 350.0), (650.0, 390.0)),
            k((575.0, 425.0), (615.0, 425.0), (575.0, 425.0)),
            c((375.0, 425.0)),
            c((250.0, 540.0)),
            c((275.0, 425.0)),
            k((225.0, 425.0), (225.0, 425.0), (185.0, 425.0)),
            k((150.0, 350.0), (150.0, 390.0), (150.0, 350.0)),
            k((150.0, 150.0), (150.0, 150.0), (150.0, 110.0)),
        ];
        let mut t = crate::tests::area_layer(text, pt, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]);
        t.shape = TextShape::Area { path: Path::new(vec![Subpath { closed: true, knots, op: Default::default() }]) };
        t.fit = Some(TextFit::default());
        t
    }

    const BUBBLE: &str = "Hey! Did you see what Dynamic Text does to a speech bubble?";

    #[test]
    fn bubble_tail_stays_empty_and_no_line_is_tiny() {
        let mut eng = engine();
        for text in [BUBBLE, CAFE] {
            let mut t = bubble_area(text, 18.0);
            refit(&mut eng, &mut t, 72.0);
            let out = eng.layout(&t, 72.0);
            assert!(!out.lines.is_empty());
            assert_no_line_is_tiny(&t, &out);
            // The body ends at y = 425; the tail below it holds no line.
            for l in &out.lines {
                assert!(l.baseline + l.descent <= 425.5, "{l:?}");
            }
        }
    }

    /// Every shown line is at least MIN_ROW_SHARE of the largest shown line (within FLOOR_SLACK).
    fn assert_no_line_is_tiny(t: &TextLayer, out: &crate::TextLayout) {
        let lines = &t.fit.as_ref().unwrap().lines;
        let shown: Vec<f32> = out.lines.iter().filter_map(|l| lines.iter().find(|f| f.start as usize == l.range.start)).map(|f| f.scale).collect();
        let largest = shown.iter().copied().fold(0.0f32, f32::max);
        assert!(shown.iter().all(|&v| v >= MIN_ROW_SHARE * largest * FLOOR_SLACK), "{shown:?}");
    }

    #[test]
    fn oval_top_line_is_not_tiny_and_release_keeps_it() {
        // An oval's narrow top used to take the first line at a sixteenth of the largest size:
        // the line moves down to where it is at least MIN_ROW_SHARE of the largest instead.
        let mut eng = engine();
        let mut lead_in = false;
        for text in [BUBBLE, CAFE, "Wow! Look at this amazing thing we made today", "Hi there, this is a short one"] {
            let mut t = ellipse_area(text, 18.0);
            refit(&mut eng, &mut t, 72.0);
            let out = eng.layout(&t, 72.0);
            assert!(!out.lines.is_empty() && !out.hidden, "{text}");
            assert_no_line_is_tiny(&t, &out);
            let shape = area_shape(&t);
            for g in &out.glyphs {
                assert!(shape.contains(g.x, g.y), "{g:?}");
            }
            // Release keeps the look; a first line below the top row gets an empty paragraph
            // above it.
            let baked = bake(&mut eng, &t, 72.0).unwrap_or_else(|e| panic!("{text}: {e}"));
            lead_in |= baked.text.starts_with('\r') && !t.text.starts_with('\r');
        }
        assert!(lead_in, "some first line sits below the top row");
    }

    #[test]
    fn layout_puts_each_area_line_where_the_solver_planned_it() {
        let mut eng = engine();
        for t in [ellipse_area(CAFE, 12.0), bubble_area(BUBBLE, 18.0), bubble_area(CAFE, 18.0), ring_area(CAFE, 12.0), ellipse_area(&CAFE.repeat(30), 12.0)] {
            let s = solve_shape(&mut eng, &t, 72.0);
            let mut t = t;
            t.fit.as_mut().unwrap().lines = s.lines.clone();
            let out = eng.layout(&t, 72.0);
            assert!(!out.lines.is_empty());
            for l in &out.lines {
                let i = s.lines.iter().position(|f| f.start as usize == l.range.start).unwrap();
                let planned = s.planned.get(i).copied().unwrap_or(f32::NAN);
                assert!((l.baseline - planned).abs() <= 1.0, "line {i}: laid out at {} planned {planned}", l.baseline);
            }
        }
    }

    #[test]
    fn area_bubble_release_bakes_within_half_a_pixel() {
        let mut eng = engine();
        let mut t = bubble_area(CAFE, 18.0);
        refit(&mut eng, &mut t, 72.0);
        let baked = bake(&mut eng, &t, 72.0).expect("area bake");
        assert!(baked.fit.is_none() && matches!(baked.shape, TextShape::Area { .. }));
        assert!(baked.paragraph_runs().len() > 1, "a paragraph per line");
    }
}
