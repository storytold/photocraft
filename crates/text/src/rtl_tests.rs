//! Right-to-left and Arabic layout. The bundled fonts have no Arabic glyphs, so glyphs are
//! .notdef, but bidi levels, clusters and byte offsets are real: these tests don't need a font.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{CharStyle, ParagraphRun, ParagraphStyle, TextAlign, TextDirection, TextRun, TextShape};

use crate::{TextEngine, TextLayout};

fn point(text: &str, size_pt: f32) -> TextLayer {
    TextLayer { text: text.into(), font_family: "Inter".into(), size_pt, ..Default::default() }
}

fn with_para(mut t: TextLayer, p: ParagraphStyle) -> TextLayer {
    t.paragraphs = vec![ParagraphRun { len: t.text.len(), style: p }];
    t
}

/// Box text at x 100..100+width.
fn boxed(text: &str, size_pt: f32, width: f32, p: ParagraphStyle) -> TextLayer {
    let mut t = with_para(point(text, size_pt), p);
    t.shape = TextShape::Box { x: 100.0, y: 0.0, width, height: 1000.0 };
    t
}

fn styled(text: &str, style: CharStyle) -> TextLayer {
    TextLayer { text: text.into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() }
}

fn inter(size_pt: f32) -> CharStyle {
    CharStyle { font_family: "Inter".into(), size_pt, ..Default::default() }
}

/// Visual extent of the clusters whose text matches `keep`.
fn span(l: &TextLayout, text: &str, keep: impl Fn(&str) -> bool) -> f32 {
    let cs: Vec<_> = l.clusters.iter().filter(|c| keep(&text[c.range.clone()])).collect();
    let lo = cs.iter().map(|c| c.x).fold(f32::MAX, f32::min);
    let hi = cs.iter().map(|c| c.x + c.advance).fold(f32::MIN, f32::max);
    hi - lo
}

#[test]
fn lines_record_the_paragraph_direction() {
    let mut e = TextEngine::new();
    assert!(e.layout(&point("مرحبا abc", 20.0), 72.0).lines[0].rtl);
    assert!(!e.layout(&point("abc مرحبا", 20.0), 72.0).lines[0].rtl);
    let forced = with_para(point("abc", 20.0), ParagraphStyle { direction: TextDirection::Rtl, ..Default::default() });
    assert!(e.layout(&forced, 72.0).lines[0].rtl);
}

#[test]
fn a_letter_and_its_harakat_are_one_cluster() {
    let mut e = TextEngine::new();
    let t = "كَتَبَ";
    let l = e.layout(&point(t, 40.0), 72.0);
    let mut ranges: Vec<_> = l.clusters.iter().map(|c| c.range.clone()).collect();
    ranges.sort_by_key(|r| r.start);
    assert_eq!(ranges, vec![0..4, 4..8, 8..12]);
    // No click lands between a letter and its fatha.
    let b = l.bounds().unwrap();
    let mut x = b[0] - 5.0;
    while x < b[2] + 5.0 {
        let off = l.hit_test(x, -10.0);
        assert!([0, 4, 8, 12].contains(&off), "x {x} hit {off}");
        x += 0.5;
    }
    // The merged clusters tile the line: each right edge meets the next cluster's x.
    let mut by_x: Vec<_> = l.clusters.iter().collect();
    by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
    for w in by_x.windows(2) {
        assert!((w[0].x + w[0].advance - w[1].x).abs() < 1e-3, "{} + {} vs {}", w[0].x, w[0].advance, w[1].x);
    }
    // An offset inside a grapheme (from an older caller) draws the caret after the whole letter:
    // in RTL that is the cluster's left edge.
    let kaf = l.clusters.iter().find(|c| c.range == (0..4)).unwrap();
    let (cx, top, bottom) = l.caret(2);
    assert!((cx - kaf.x).abs() < 1e-3, "{cx} vs {}", kaf.x);
    assert!(top < bottom);
}

#[test]
fn lam_alef_stays_two_clusters() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("لا", 40.0), 72.0);
    let mut ranges: Vec<_> = l.clusters.iter().map(|c| c.range.clone()).collect();
    ranges.sort_by_key(|r| r.start);
    assert_eq!(ranges, vec![0..2, 2..4]);
}

#[test]
fn latin_combining_marks_fold_too() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("e\u{301}x", 40.0), 72.0);
    let mut ranges: Vec<_> = l.clusters.iter().map(|c| c.range.clone()).collect();
    ranges.sort_by_key(|r| r.start);
    assert_eq!(ranges, vec![0..3, 3..4]);
}

#[test]
fn a_letter_with_shadda_and_fatha_is_one_cluster() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("شَّدَّ", 40.0), 72.0);
    let mut ranges: Vec<_> = l.clusters.iter().map(|c| c.range.clone()).collect();
    ranges.sort_by_key(|r| r.start);
    assert_eq!(ranges, vec![0..6, 6..12]);
}

#[test]
fn ltr_caret_inside_a_grapheme_is_after_the_whole_cluster() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("e\u{301}x", 40.0), 72.0);
    let host = l.clusters.iter().find(|c| c.range == (0..3)).unwrap();
    let (cx, _, _) = l.caret(1);
    assert!((cx - (host.x + host.advance)).abs() < 1e-3, "{cx} vs {}", host.x + host.advance);
}

#[test]
#[ignore = "parley 0.11 can break a line inside an RTL grapheme (a haraka starts the next line); upstream bug"]
fn wrapped_lines_never_start_inside_a_grapheme() {
    let mut e = TextEngine::new();
    let text = "كَتَبَ الوَلَدُ ".repeat(12);
    let text = text.trim_end();
    let l = e.layout(&boxed(text, 20.0, 260.0, ParagraphStyle::default()), 72.0);
    let starts = crate::segment::grapheme_boundaries(text);
    for ln in &l.lines {
        assert!(starts.contains(&ln.range.start), "line starts inside a grapheme at byte {}", ln.range.start);
    }
}

#[test]
fn start_indent_is_on_the_right_in_rtl_paragraphs() {
    let mut e = TextEngine::new();
    // Box 100..500, right-aligned RTL: the start indent keeps the text 50 px from the right edge.
    let p = ParagraphStyle { align: TextAlign::Right, start_indent_pt: 50.0, ..Default::default() };
    let l = e.layout(&boxed("مرحبا", 20.0, 400.0, p), 72.0);
    assert!((l.lines[0].x1 - 450.0).abs() < 0.5, "{:?}", l.lines[0]);
    // Left-aligned RTL: the end indent is on the left.
    let p = ParagraphStyle { align: TextAlign::Left, end_indent_pt: 30.0, ..Default::default() };
    let l = e.layout(&boxed("مرحبا", 20.0, 400.0, p), 72.0);
    assert!((l.lines[0].x0 - 130.0).abs() < 0.5, "{:?}", l.lines[0]);
    // LTR is unchanged: the start indent is on the left.
    let p = ParagraphStyle { align: TextAlign::Left, start_indent_pt: 50.0, ..Default::default() };
    let l = e.layout(&boxed("hello", 20.0, 400.0, p), 72.0);
    assert!((l.lines[0].x0 - 150.0).abs() < 0.5, "{:?}", l.lines[0]);
    // Point text: right-aligned RTL ends the start indent before its anchor (x 0).
    let t = with_para(point("مرحبا", 20.0), ParagraphStyle { align: TextAlign::Right, start_indent_pt: 30.0, ..Default::default() });
    let l = e.layout(&t, 72.0);
    assert!((l.lines[0].x1 + 30.0).abs() < 0.5, "{:?}", l.lines[0]);
}

/// Visible extent (min x, max x) of the non-whitespace clusters on line `li`. Line bounds
/// include the whitespace parley hangs past the edge (on the left in RTL), so tests that check
/// placement measure ink instead.
fn ink(l: &TextLayout, text: &str, li: usize) -> (f32, f32) {
    l.clusters
        .iter()
        .filter(|c| c.line == li && !text[c.range.clone()].chars().all(char::is_whitespace))
        .fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(c.x), hi.max(c.x + c.advance)))
}

#[test]
fn rtl_justified_last_lines_stay_in_the_box() {
    let mut e = TextEngine::new();
    let text = "سلام عليكم ".repeat(12);
    let text = text.trim_end();
    for align in [TextAlign::JustifyLeft, TextAlign::JustifyAll] {
        let l = e.layout(&boxed(text, 20.0, 300.0, ParagraphStyle { align, ..Default::default() }), 72.0);
        assert!(l.lines.len() >= 2, "{align:?} wraps");
        for li in 0..l.lines.len() {
            let (lo, hi) = ink(&l, text, li);
            assert!(lo >= 99.5 && hi <= 400.5, "{align:?} line {li}: {lo}..{hi}");
        }
        let (lo, _) = ink(&l, text, l.lines.len() - 1);
        if align == TextAlign::JustifyLeft {
            assert!((lo - 100.0).abs() < 0.5, "last line on the left: {lo}");
        }
    }
}

#[test]
fn tracking_never_separates_joined_letters() {
    let mut e = TextEngine::new();
    let t = "سلام abc";
    let tracked = |tracking: f32| styled(t, CharStyle { tracking, ..inter(20.0) });
    let plain = e.layout(&tracked(0.0), 72.0);
    let wide = e.layout(&tracked(200.0), 72.0);
    let arabic_word = |s: &str| s.chars().all(crate::segment::is_cursive_letter);
    let latin = |s: &str| s.chars().all(|c| c.is_ascii_alphabetic());
    assert!((span(&plain, t, arabic_word) - span(&wide, t, arabic_word)).abs() < 1e-3);
    // Latin still tracks: 200/1000 em = 4 px after each of a, b and c at 20 px.
    assert!(span(&wide, t, latin) - span(&plain, t, latin) > 7.0);
}
