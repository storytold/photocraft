//! Dynamic Text: chooses a box layer's lines and sizes each one to span its width
//! (Photoshop's Dynamic Text).

use photocraft_doc::TextLayer;
use photocraft_doc::text::{FitLine, TextFit, TextShape};

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

/// Box text: [`solve_box`] written to `fit.lines`. The sizes are left alone (scale 1).
/// Point, Path and (for now) Area text, fit None or empty text: untouched.
///
/// Callers must refuse vertical text, RTL text and multi-layer input before calling.
pub fn refit(engine: &mut crate::TextEngine, layer: &mut TextLayer, dpi: f32) -> FitResult {
    if !is_fitted(layer) {
        return UNTOUCHED;
    }
    replace_non_finite(layer);
    let solved = match layer.shape {
        TextShape::Box { .. } => solve(engine, layer, dpi),
        TextShape::Point | TextShape::Path { .. } | TextShape::Area { .. } => return UNTOUCHED,
    };
    if let Some(f) = layer.fit.as_mut() {
        f.lines = solved.lines;
    }
    FitResult { scale: 1.0, fits: solved.fits, probes: solved.probes }
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
    let none = Solved { lines: Vec::new(), fits: true, probes: 0, layouts: 0, visited: 0 };
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
    Solved { lines, fits, probes, layouts, visited }
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

/// Share of its band's widest span a line in a shape is sized to: a hair inside, so layout always
/// keeps it on the band the solver placed it on.
pub(crate) const AREA_MARGIN: f32 = 0.999;
/// How far a line in a shape moves down per step when its band is too narrow, as a share of
/// its height at its floor scale.
pub(crate) const ROW_SKIP_STEP: f32 = 0.05;
/// Most steps a line in a shape moves down looking for a band that holds it.
pub(crate) const ROW_SKIP_STEPS: usize = 256;
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
                    _ => words.push(Word { start: c.range.start, lead: pending, width: adv, pt }),
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
    use crate::tests::box_layer;
    use photocraft_doc::text::{TextFit, TextRun};

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
            words: (0..words).map(|i| Word { start: i * 5, lead: if i == 0 { 0.0 } else { 4.0 }, width: 20.0 + (i % 7) as f32 * 6.0, pt: 12.0 }).collect(),
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
}
