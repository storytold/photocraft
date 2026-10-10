//! Kashida justification: a justified line of Arabic stretches the connections between letters
//! before it stretches the gaps between words (spec 6.2).
//!
//! 1. **Candidates** ([`candidates`]): `raqim-kashida` (arabic-naskh rules) lists the joins of a
//!    paragraph with priorities; one join is kept per word, the highest priority, ties going to
//!    the join nearest the word's end.
//! 2. **Reshape probe** ([`Layouter::plan_joins`]): a tatweel inserted after shaping is only safe
//!    where the font would not have reshaped the word. The word is shaped alone and with one
//!    U+0640 at the join in a scratch layout; the join is accepted only if the two shapings are
//!    identical apart from the tatweel (glyph count, clusters, ids, advances, offsets), the word
//!    alone matches the word as shaped in its line, and the font maps U+0640. The tatweel glyph
//!    and its metrics come from the probe, never from the cmap, because a font may substitute a
//!    contextual tatweel. Words with no accepted join keep word-gap justification.
//! 3. **Lengths and rendering** ([`justify_line`]): the slack is split evenly over the accepted
//!    joins, each capped at 0.5 em and at [`MAX_COPIES`] tatweels; the rest goes to the word gaps.
//!    A join whose share would be under one tatweel advance is dropped (widest tatweel first) and
//!    its word keeps word-gap justification, because a copy narrower than the gap would start left
//!    of it, over the next letter. Copies of the probe's tatweel fill each gap (the last
//!    overlapping the one before to hit the exact width). Underlines and strikethroughs grow with
//!    the gaps. The added width
//!    belongs to the logically preceding letter's cluster, so carets, hit tests and selections
//!    stay consistent. The text itself never changes.
//!
//! Limits: a probe validates one tatweel, so copies are assumed to tile flat; a join at a style
//! run boundary is never a candidate (parley has no shaping context across it). Not built: reshaping
//! the justified line itself with U+0640 in the shaping text, which would let fonts pick
//! kashida-aware forms.

use std::collections::HashMap;
use std::ops::Range;

use parley::{FontData, Line, StyleProperty};
use photocraft_doc::text::CharStyle;
use skrifa::MetadataProvider;

use super::{ClusterInfo, JustifyLine, Layouter, PlacedGlyph, RunBrush, SmallCapsMode, justify_all_line, shift_decorations, style_props, word_gap_indices};
use crate::fonts::FontDb;

const TATWEEL: char = '\u{640}';
/// A kashida grows by at most this many em.
const MAX_EM: f32 = 0.5;
/// Tatweel copies per join, a guard against a degenerate (tiny) advance.
const MAX_COPIES: usize = 64;
/// Probe results kept; the cache is cleared when it fills.
const PROBE_CACHE_CAP: usize = 4096;
const EPS: f32 = 1e-3;

/// A kept kashida join of a paragraph: a byte offset in the paragraph text, inside a cursive word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub word: Range<usize>,
    pub join: usize,
}

/// One candidate per cursive word of `text`: the join with the highest priority, ties to the join
/// nearest the word's end. `words` are the cursive words and `graphemes` the grapheme boundaries
/// of `text` (see `segment`).
pub(crate) fn candidates(text: &str, words: &[Range<usize>], graphemes: &[usize]) -> Vec<Candidate> {
    // Never `compile_pattern_text`: its pattern slicing is unchecked. The built-in sets only.
    let Some(set) = ::kashida::builtin_pattern_set("arabic-naskh") else {
        return Vec::new();
    };
    let (_, points) = ::kashida::find_kashida_points(text, set, false);
    let mut best: Vec<Option<(u8, usize)>> = vec![None; words.len()];
    for p in points {
        let Ok(i) = usize::try_from(p.index) else { continue };
        let (Some(&start), Some(&join)) = (graphemes.get(i), graphemes.get(i + 1)) else {
            continue;
        };
        let Some(w) = crate::segment::word_at(words, start) else { continue };
        let (Some(word), Some(slot)) = (words.get(w), best.get_mut(w)) else { continue };
        if join >= word.end {
            continue;
        }
        if slot.is_none_or(|(priority, at)| (p.priority, join) > (priority, at)) {
            *slot = Some((p.priority, join));
        }
    }
    words.iter().zip(best).filter_map(|(word, b)| b.map(|(_, join)| Candidate { word: word.clone(), join })).collect()
}

#[derive(Clone, Copy, Debug)]
struct Glyph {
    id: u32,
    /// Byte offset of the glyph's cluster in the shaped word.
    cluster: usize,
    advance: f32,
    x: f32,
    y: f32,
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

fn same_glyphs(a: &[Glyph], b: &[Glyph]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| p.id == q.id && p.cluster == q.cluster && close(p.advance, q.advance) && close(p.x, q.x) && close(p.y, q.y))
}

/// The glyphs of a word in visual order, with the font instance that shaped them.
#[derive(Clone, Debug)]
struct Shaped {
    font: FontData,
    coords: Vec<i16>,
    size: f32,
    glyphs: Vec<Glyph>,
}

impl Shaped {
    fn same_font(&self, other: &Self) -> bool {
        self.font.data.id() == other.font.data.id() && self.font.index == other.font.index && self.coords == other.coords && close(self.size, other.size)
    }
}

/// The glyphs of the clusters inside `range` (paragraph text offsets) in `runs`; `None` when
/// there are none or when more than one font instance shaped them.
fn collect<'a>(runs: impl Iterator<Item = parley::Run<'a, RunBrush>>, range: &Range<usize>) -> Option<Shaped> {
    let mut out: Option<Shaped> = None;
    for run in runs {
        for c in run.visual_clusters() {
            let r = c.text_range();
            if r.start < range.start || r.end > range.end {
                continue;
            }
            let shaped = out.get_or_insert_with(|| Shaped {
                font: run.font().clone(),
                coords: run.normalized_coords().to_vec(),
                size: run.font_size(),
                glyphs: Vec::new(),
            });
            if shaped.font.data.id() != run.font().data.id() || shaped.font.index != run.font().index || shaped.coords != run.normalized_coords() {
                return None;
            }
            shaped.glyphs.extend(c.glyphs().map(|g| Glyph { id: g.id, cluster: r.start - range.start, advance: g.advance, x: g.x, y: g.y }));
        }
    }
    out.filter(|s| !s.glyphs.is_empty())
}

/// The tatweel as the probe shaped it (px at the run's size, before horizontal scale).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tatweel {
    pub id: u32,
    pub advance: f32,
    pub x: f32,
    pub y: f32,
}

/// What the probes remember: results by (style, font instance, word, join), and the plain
/// shaping of each word by (style, word, font generation).
#[derive(Default)]
pub(super) struct ProbeCache {
    probes: HashMap<String, Option<Tatweel>>,
    plain: HashMap<String, Option<Shaped>>,
}

/// An accepted join of a line.
#[derive(Clone, Copy, Debug)]
pub(super) struct Join {
    /// Layer byte offset of the join (the end of the logically preceding letter).
    pub offset: usize,
    pub tatweel: Tatweel,
    /// The most this join may grow (px).
    pub cap: f32,
}

/// What [`Layouter::plan_joins`] needs to know about the paragraph being laid out.
pub(super) struct LineText<'a> {
    pub ptext: &'a str,
    /// Layer offset of the paragraph start.
    pub prange_start: usize,
    /// Length of the direction mark prefixed to `ptext`.
    pub prefix: usize,
    pub k: f32,
    pub fallback: &'a [String],
    pub small_caps: &'a [SmallCapsMode],
    pub styles: &'a [CharStyle],
    pub run_starts: &'a [usize],
}

impl LineText<'_> {
    fn layer_offset(&self, o: usize) -> usize {
        self.prange_start + o.saturating_sub(self.prefix)
    }

    fn style_at(&self, layer_offset: usize) -> usize {
        self.run_starts.iter().rposition(|&s| s <= layer_offset).unwrap_or(0)
    }
}

impl Layouter {
    /// The candidates on `line` whose join the font would not reshape (the probe in the module
    /// docs).
    pub(super) fn plan_joins(&mut self, fonts: &mut FontDb, cx: &LineText<'_>, line: &Line<'_, RunBrush>, candidates: &[Candidate]) -> Vec<Join> {
        let lr = line.text_range();
        let mut joins = Vec::new();
        for cand in candidates.iter().filter(|c| c.word.start >= lr.start && c.word.end <= lr.end) {
            let Some(word) = cx.ptext.get(cand.word.clone()) else { continue };
            let (first, last) = (cx.layer_offset(cand.word.start), cx.layer_offset(cand.word.end.saturating_sub(1)));
            let si = cx.style_at(first);
            // A join never straddles a style change: parley has no shaping context across one.
            if si != cx.style_at(last) {
                continue;
            }
            let (Some(style), Some(&small_caps)) = (cx.styles.get(si), cx.small_caps.get(si)) else { continue };
            let mut props = style_props(style, cx.k, cx.fallback, u32::try_from(si).unwrap_or(0), small_caps);
            // No tracking inside cursive words (the paragraph shaping sets this too).
            props.push(StyleProperty::LetterSpacing(0.0));
            let Some(in_line) = collect(line.runs(), &cand.word) else { continue };
            let Some(tatweel) = self.probe_join(fonts, &props, word, cand.join - cand.word.start, &in_line) else {
                continue;
            };
            let hs = if style.horizontal_scale > 0.0 { style.horizontal_scale } else { 1.0 };
            let scaled = Tatweel { advance: tatweel.advance * hs, x: tatweel.x * hs, ..tatweel };
            joins.push(Join { offset: cx.layer_offset(cand.join), tatweel: scaled, cap: MAX_EM * in_line.size * hs });
        }
        joins
    }

    fn shape_scratch(&mut self, fonts: &mut FontDb, text: &str, props: &[StyleProperty<'static, RunBrush>]) -> Option<Shaped> {
        let mut b = self.lcx.ranged_builder(&mut fonts.fcx, text, 1.0, false);
        for p in props {
            b.push_default(p.clone());
        }
        let mut layout = b.build(text);
        layout.break_all_lines(None);
        collect(layout.lines().flat_map(|l| l.runs()), &(0..text.len()))
    }

    /// The word shaped alone, remembered per style and word (until the font set changes).
    fn plain_shaping(&mut self, fonts: &mut FontDb, props: &[StyleProperty<'static, RunBrush>], word: &str) -> Option<Shaped> {
        let key = format!("{props:?}|{word}|{}", crate::fonts::generation());
        if let Some(plain) = self.kashida_probes.plain.get(&key) {
            return plain.clone();
        }
        let plain = self.shape_scratch(fonts, word, props);
        if self.kashida_probes.plain.len() >= PROBE_CACHE_CAP {
            self.kashida_probes.plain.clear();
        }
        self.kashida_probes.plain.insert(key, plain.clone());
        plain
    }

    /// The tatweel to place at byte `join` of `word`, or `None` when the join is unsafe.
    fn probe_join(&mut self, fonts: &mut FontDb, props: &[StyleProperty<'static, RunBrush>], word: &str, join: usize, in_line: &Shaped) -> Option<Tatweel> {
        let plain = self.plain_shaping(fonts, props, word)?;
        if !plain.same_font(in_line) || !same_glyphs(&plain.glyphs, &in_line.glyphs) {
            return None;
        }
        let key = format!("{props:?}|{word}|{join}|{}|{}|{:?}|{}", plain.font.data.id(), plain.font.index, plain.coords, plain.size.to_bits());
        if let Some(t) = self.kashida_probes.probes.get(&key) {
            return *t;
        }
        let t = self.probe_tatweel(fonts, props, word, join, &plain);
        if self.kashida_probes.probes.len() >= PROBE_CACHE_CAP {
            self.kashida_probes.probes.clear();
        }
        self.kashida_probes.probes.insert(key, t);
        t
    }

    fn probe_tatweel(&mut self, fonts: &mut FontDb, props: &[StyleProperty<'static, RunBrush>], word: &str, join: usize, plain: &Shaped) -> Option<Tatweel> {
        let (head, tail) = (word.get(..join)?, word.get(join..)?);
        let with = format!("{head}{TATWEEL}{tail}");
        let shaped = self.shape_scratch(fonts, &with, props)?;
        if !shaped.same_font(plain) {
            return None;
        }
        let mut tatweels = shaped.glyphs.iter().filter(|g| g.cluster == join);
        let (Some(&t), None) = (tatweels.next(), tatweels.next()) else {
            return None;
        };
        let shift = TATWEEL.len_utf8();
        let rest: Vec<Glyph> = shaped
            .glyphs
            .iter()
            .filter(|g| g.cluster != join)
            .map(|g| Glyph { cluster: if g.cluster > join { g.cluster - shift } else { g.cluster }, ..*g })
            .collect();
        let font = skrifa::FontRef::from_index(plain.font.data.as_ref(), plain.font.index).ok()?;
        (same_glyphs(&rest, &plain.glyphs) && font.charmap().map(TATWEEL).is_some() && t.advance > EPS).then_some(Tatweel {
            id: t.id,
            advance: t.advance,
            x: t.x,
            y: t.y,
        })
    }
}

/// A join resolved against the line's clusters and glyphs.
struct Placed {
    /// Index of the preceding letter's cluster.
    cluster: usize,
    /// Left edge of that cluster: where the gap opens.
    gap_x: f32,
    /// Glyphs from this x on are right of the gap.
    glyph_from: f32,
    cap: f32,
    tatweel: Tatweel,
    face: u32,
    style: u32,
    width: f32,
}

/// Resolves `join` on a line (clusters and glyphs in line space): the letters on both sides of it
/// must be right-to-left clusters, the following one visually to the left.
fn place(join: &Join, clusters: &[ClusterInfo], glyphs: &[PlacedGlyph]) -> Option<Placed> {
    let p = clusters.iter().position(|c| c.range.end == join.offset && c.range.start < join.offset)?;
    let (pc, fc) = (clusters.get(p)?, clusters.iter().find(|c| c.range.start == join.offset)?);
    if !pc.rtl || !fc.rtl || fc.x + fc.advance > pc.x + 0.01 {
        return None;
    }
    let anchor = glyphs.iter().find(|g| g.x >= pc.x - EPS && g.x <= pc.x + pc.advance)?;
    Some(Placed {
        cluster: p,
        gap_x: pc.x,
        // Marks sit a little left of their base's pen position; they move with it.
        glyph_from: pc.x - (0.25 * pc.advance).min(0.5 * fc.advance),
        cap: join.cap,
        tatweel: join.tatweel,
        face: anchor.face,
        style: anchor.style,
        width: 0.0,
    })
}

/// How a kashida line is justified.
pub(super) struct Apply<'a> {
    /// The layer text (cluster ranges are offsets in it).
    pub text: &'a str,
    pub rtl: bool,
    pub baseline: f32,
    /// A "Justify all" last line: with no word gaps it is letter-spaced, as without kashida.
    pub spread_all: bool,
}

/// Keeps the joins that can each take an even share of `slack` as at least one whole tatweel: a
/// narrower gap would put the copy over the next letter. Joins that cannot (their cap is under
/// one tatweel) are dropped first, then the widest tatweels until the rest fit; their words keep
/// word-gap justification.
fn viable(mut placed: Vec<Placed>, slack: f32) -> Vec<Placed> {
    placed.retain(|p| p.cap >= p.tatweel.advance);
    while let Some((i, widest)) = placed.iter().enumerate().map(|(i, p)| (i, p.tatweel.advance)).max_by(|a, b| a.1.total_cmp(&b.1)) {
        if slack / placed.len() as f32 >= widest {
            break;
        }
        placed.remove(i);
    }
    placed
}

/// Spreads `slack` px over a justified line: kashida at the accepted `joins` first, the word gaps
/// second. `line` holds the line's clusters, glyphs and decorations (line space); the tatweel
/// copies are appended to its glyphs and its underlines grow with it. The line is anchored at its
/// left edge. Returns (width added to the line, shift of the whole line).
pub(super) fn justify_line(line: &mut JustifyLine<'_>, joins: &[Join], slack: f32, ap: &Apply<'_>) -> (f32, f32) {
    let placed: Vec<Placed> = joins.iter().filter_map(|j| line.glyphs.get(line.g0..).and_then(|g| place(j, line.clusters, g))).collect();
    let mut placed = viable(placed, slack);
    placed.sort_by(|a, b| a.gap_x.total_cmp(&b.gap_x));
    let mut added = 0.0;
    if !placed.is_empty() {
        let share = slack / placed.len() as f32;
        for p in &mut placed {
            p.width = share.min(p.cap).min(MAX_COPIES as f32 * p.tatweel.advance);
        }
        added = widen(line, &placed, ap.baseline);
    }
    let leftover = slack - added;
    if leftover <= EPS {
        return (added, 0.0);
    }
    if ap.spread_all || !word_gap_indices(line.clusters, ap.text).is_empty() {
        let (a, shift) = justify_all_line(line, ap.text, leftover, ap.rtl);
        return (added + a, shift);
    }
    if ap.rtl {
        // No gap to take the rest: the line stays short on its start (right) side.
        for c in line.clusters.iter_mut() {
            c.x += leftover;
        }
        for g in line.glyphs.iter_mut().skip(line.g0) {
            g.x += leftover;
        }
        shift_decorations(line.decorations, line.vertical, |_, _| leftover);
        return (added, leftover);
    }
    (added, 0.0)
}

/// Opens the gaps and fills them with tatweel copies. Returns the width added.
fn widen(line: &mut JustifyLine<'_>, placed: &[Placed], baseline: f32) -> f32 {
    let opened_before = |x: f32| placed.iter().filter(|p| p.gap_x < x - EPS).map(|p| p.width).sum::<f32>();
    for (ci, c) in line.clusters.iter_mut().enumerate() {
        c.x += opened_before(c.x);
        if let Some(p) = placed.iter().find(|p| p.cluster == ci) {
            c.advance += p.width;
        }
    }
    for g in line.glyphs.iter_mut().skip(line.g0) {
        g.x += placed.iter().filter(|p| p.glyph_from <= g.x).map(|p| p.width).sum::<f32>();
    }
    shift_decorations(line.decorations, line.vertical, |x, _| opened_before(x));
    let mut before = 0.0;
    for p in placed {
        let start = p.gap_x + before;
        before += p.width;
        if p.width <= EPS {
            continue;
        }
        let copies = ((p.width / p.tatweel.advance).ceil() as usize).clamp(1, MAX_COPIES);
        for i in 0..copies {
            let at = (i as f32 * p.tatweel.advance).min(p.width - p.tatweel.advance);
            line.glyphs.push(PlacedGlyph {
                face: p.face,
                id: p.tatweel.id,
                x: start + at + p.tatweel.x,
                y: baseline + p.tatweel.y,
                style: p.style,
                orient: super::GlyphOrient::Horizontal,
            });
        }
    }
    before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{cursive_words, grapheme_boundaries};

    fn picks(text: &str) -> Vec<(String, usize)> {
        let words = cursive_words(text);
        candidates(text, &words, &grapheme_boundaries(text)).into_iter().map(|c| (text[c.word.clone()].to_string(), c.join)).collect()
    }

    #[test]
    fn one_candidate_per_word_ties_go_to_the_end() {
        let text = "سلام مدرسة عربي";
        let found = picks(text);
        assert_eq!(found.len(), 3, "{found:?}");
        let words = cursive_words(text);
        for (c, w) in found.iter().zip(&words) {
            assert!(w.start < c.1 && c.1 < w.end, "{c:?} inside {w:?}");
        }
        // Every join sits on a letter boundary.
        let bounds = grapheme_boundaries(text);
        assert!(found.iter().all(|c| bounds.contains(&c.1)));
        // The kept join has the word's top priority and, among those, the greatest offset.
        let set = ::kashida::builtin_pattern_set("arabic-naskh").unwrap();
        let (_, pts) = ::kashida::find_kashida_points(text, set, false);
        for (w, c) in words.iter().zip(&found) {
            let mine: Vec<(u8, usize)> =
                pts.iter().filter_map(|p| Some((p.priority, *bounds.get(p.index as usize + 1)?))).filter(|&(_, at)| at > w.start && at < w.end).collect();
            assert_eq!(mine.iter().max().map(|m| m.1), Some(c.1), "{mine:?} vs {c:?}");
        }
    }

    #[test]
    fn no_join_in_allah_or_lam_alef() {
        assert!(picks("الله").is_empty(), "{:?}", picks("الله"));
        assert!(picks("لا").is_empty(), "{:?}", picks("لا"));
        assert!(picks("abc 123").is_empty());
        assert!(picks("").is_empty());
    }

    /// The probe's verdict for `word` with the join after its first `letters` letters.
    fn probe(e: &mut crate::TextEngine, family: &str, word: &str, letters: usize) -> Option<Tatweel> {
        let st = CharStyle { font_family: family.into(), size_pt: 40.0, ..Default::default() };
        let mut props = style_props(&st, 1.0, &[], 0, SmallCapsMode::None);
        props.push(StyleProperty::LetterSpacing(0.0));
        let join = word.char_indices().nth(letters).map(|(i, _)| i)?;
        let in_line = e.layouter.shape_scratch(&mut e.fonts, word, &props)?;
        e.layouter.probe_join(&mut e.fonts, &props, word, join, &in_line)
    }

    #[test]
    fn the_probe_rejects_stacked_joins_and_accepts_flat_ones() {
        let mut e = crate::TextEngine::with_system_fonts();
        let Some(amiri) = ["Amiri"].into_iter().find(|f| e.fonts.has_family(f)) else {
            eprintln!("skipped: Amiri is not registered (craft-fonts request pending)");
            return;
        };
        // Reviewer's harfrust probe: before the alef of كتاب and after the beh of بحر the font
        // reshapes when a tatweel is present; سلام and مدرسة are identical.
        assert_eq!(probe(&mut e, amiri, "كتاب", 2), None, "كتاب before the alef");
        assert_eq!(probe(&mut e, amiri, "بحر", 1), None, "بحر after the beh");
        let t = probe(&mut e, amiri, "سلام", 1).expect("سلام accepts a kashida after the seen");
        assert!(t.advance > 0.0 && t.id != 0, "{t:?}");
        assert!(probe(&mut e, amiri, "مدرسة", 4).is_some(), "مدرسة before the teh marbuta");
        // Cached: the same answer again.
        let shaped = e.layouter.kashida_probes.plain.len();
        assert_eq!(probe(&mut e, amiri, "كتاب", 2), None);
        assert_eq!(probe(&mut e, amiri, "سلام", 1), Some(t));
        assert_eq!(e.layouter.kashida_probes.plain.len(), shaped, "the plain shaping of a word is kept, not redone");
    }

    use super::super::{DecorationRect, GlyphOrient};

    const WORD: &str = "سلمم";

    /// WORD laid out right-to-left, 10 px per letter: visually م م ل س, the first letter س at 30.
    fn rtl_word() -> (Vec<ClusterInfo>, Vec<PlacedGlyph>) {
        let letters = [(6..8, 0.0), (4..6, 10.0), (2..4, 20.0), (0..2, 30.0)];
        let clusters = letters.iter().map(|(r, x)| ClusterInfo { range: r.clone(), x: *x, advance: 10.0, line: 0, rtl: true }).collect();
        let glyphs = letters.iter().map(|(_, x)| PlacedGlyph { face: 0, id: 1, x: *x, y: 0.0, style: 0, orient: GlyphOrient::Horizontal }).collect();
        (clusters, glyphs)
    }

    fn join(offset: usize, advance: f32, cap: f32) -> Join {
        Join { offset, tatweel: Tatweel { id: 99, advance, x: 0.0, y: 0.0 }, cap }
    }

    /// Justifies `rtl_word` with `joins`; returns the result, the tatweel copies and the line.
    fn run(joins: &[Join], slack: f32) -> ((f32, f32), Vec<PlacedGlyph>, Vec<ClusterInfo>, DecorationRect) {
        let (mut clusters, mut glyphs) = rtl_word();
        let mut under = [DecorationRect { x0: 0.0, y0: 0.0, x1: 40.0, y1: 1.0, style: 0 }];
        let ap = Apply { text: WORD, rtl: true, baseline: 0.0, spread_all: false };
        let mut line = JustifyLine { clusters: &mut clusters, glyphs: &mut glyphs, g0: 0, decorations: &mut under, vertical: false };
        let out = justify_line(&mut line, joins, slack, &ap);
        let copies = glyphs.iter().filter(|g| g.id == 99).cloned().collect();
        (out, copies, clusters, under[0])
    }

    #[test]
    fn a_share_under_one_tatweel_drops_the_widest_join_to_the_rest() {
        // Joins after the seen (offset 2) and after the second meem (offset 6); 10 px for two
        // joins is 5 each, under the 12 px tatweel of the second: it is dropped, the first (4 px)
        // takes all 10.
        let ((added, shift), copies, clusters, under) = run(&[join(2, 4.0, 20.0), join(6, 12.0, 20.0)], 10.0);
        assert_eq!((added, shift), (10.0, 0.0));
        assert_eq!(copies.len(), 3, "10 px of 4 px tatweels");
        for t in &copies {
            assert!(t.x >= 30.0 - 1e-3 && t.x + 4.0 <= 40.0 + 1e-3, "inside the gap 30..40: {}", t.x);
        }
        let span = |start: usize| clusters.iter().find(|c| c.range.start == start).map(|c| (c.x, c.advance));
        assert_eq!((span(0), span(4)), (Some((30.0, 20.0)), Some((10.0, 10.0))), "the first join opened, the dropped one did not");
        assert_eq!((under.x0, under.x1), (0.0, 50.0), "the underline grew with the gap");
    }

    #[test]
    fn no_join_is_kept_when_none_fits_a_tatweel() {
        let ((added, shift), copies, clusters, under) = run(&[join(2, 12.0, 20.0)], 10.0);
        assert!(copies.is_empty(), "{copies:?}");
        // No word gap either: the right-to-left line stays short on its start side.
        assert_eq!((added, shift), (0.0, 10.0));
        assert_eq!(clusters.iter().map(|c| c.x).collect::<Vec<_>>(), [10.0, 20.0, 30.0, 40.0]);
        assert_eq!((under.x0, under.x1), (10.0, 50.0));
        // A cap under one tatweel can never be filled.
        assert!(run(&[join(2, 4.0, 3.0)], 10.0).1.is_empty());
    }

    #[test]
    fn a_kashida_never_grows_past_the_copy_limit() {
        let ((added, shift), copies, ..) = run(&[join(2, 1.0, 1000.0)], 500.0);
        assert_eq!(copies.len(), MAX_COPIES);
        assert_eq!((added, shift), (MAX_COPIES as f32, 500.0 - MAX_COPIES as f32));
        let mut xs: Vec<f32> = copies.iter().map(|t| t.x).collect();
        xs.sort_by(f32::total_cmp);
        assert!(xs.windows(2).all(|w| (w[1] - w[0] - 1.0).abs() < 1e-3), "the copies tile without overlap");
    }

    #[test]
    fn the_probe_rejects_a_font_without_tatweel() {
        let mut e = crate::TextEngine::new();
        assert_eq!(probe(&mut e, "Inter", "سلام", 1), None);
    }
}
