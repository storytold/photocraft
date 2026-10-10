//! Caret navigation over real layouts (`navigate.rs`). Bundled fonts only: Arabic glyphs are
//! .notdef, but bidi levels, clusters, line breaks and offsets are real.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{ParagraphRun, ParagraphStyle, TextDirection, TextShape};

use crate::navigate::{Caret, caret_geometry, caret_segment, hit};
use crate::{TextEngine, TextLayout};

fn point(text: &str) -> TextLayer {
    TextLayer { text: text.into(), font_family: "Inter".into(), size_pt: 20.0, ..Default::default() }
}

fn directed(text: &str, direction: TextDirection) -> TextLayer {
    let mut t = point(text);
    t.paragraphs = vec![ParagraphRun { len: text.len(), style: ParagraphStyle { direction, ..Default::default() } }];
    t
}

/// Box text at x 100..100+width, y 0..height.
fn boxed(text: &str, width: f32, height: f32) -> TextLayer {
    let mut t = point(text);
    t.shape = TextShape::Box { x: 100.0, y: 0.0, width, height };
    t
}

/// Navigation relies on parley 0.11 splitting a shaping cluster into one cluster per character
/// (review findings 3 and 11): its cursor stops between a letter and its haraka, so `step` skips
/// those stops. If a parley upgrade changes this, revisit `navigate::step`.
#[test]
fn parley_cursor_stops_are_per_character() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("كَتَبَ"), 72.0);
    let mut starts = Vec::new();
    for line in l.paragraphs[0].layout.lines() {
        for run in line.runs() {
            for c in run.clusters() {
                starts.push(c.text_range().start);
            }
        }
    }
    starts.sort_unstable();
    assert_eq!(starts, vec![0, 2, 4, 6, 8, 10]);
}

#[test]
fn layouts_keep_one_parley_layout_per_paragraph() {
    let mut e = TextEngine::new();
    let l = e.layout(&point("abc\n\nمرحبا"), 72.0);
    let spans: Vec<(usize, usize, usize)> = l.paragraphs.iter().map(|p| (p.start, p.end, p.prefix)).collect();
    assert_eq!(spans, vec![(0, 3, 0), (4, 4, 0), (5, 15, 0)]);
    assert_eq!(e.layout(&directed("abc", TextDirection::Rtl), 72.0).paragraphs[0].prefix, 3);
    // An overflowing box keeps its hidden lines in parley but draws fewer.
    let l = e.layout(&boxed("aaa bbb ccc ddd eee fff ggg hhh", 90.0, 50.0), 72.0);
    let parley_lines = l.paragraphs[0].layout.len();
    assert!(l.lines.len() < parley_lines, "{} drawn of {parley_lines}", l.lines.len());
    fn send_sync<T: Send + Sync>() {}
    send_sync::<TextLayout>();
}

/// Visual edges (left, right) of line `li`'s clusters.
fn edges(l: &TextLayout, li: usize) -> (f32, f32) {
    l.clusters.iter().filter(|c| c.line == li).fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(c.x), hi.max(c.x + c.advance)))
}

#[test]
fn one_offset_has_two_carets_where_directions_meet() {
    let mut e = TextEngine::new();
    // "abc" then Arabic, no space: byte 3 is after c (upstream) and before م (downstream).
    let t = "abcمرحبا";
    let l = e.layout(&point(t), 72.0);
    let c = l.clusters.iter().find(|k| k.range == (2..3)).unwrap().clone();
    let alef = l.clusters.iter().find(|k| k.range == (11..13)).unwrap().clone();
    let (_, right) = edges(&l, 0);
    let up = caret_geometry(&l, t, Caret::new(3, true));
    let down = caret_geometry(&l, t, Caret::new(3, false));
    assert!((up.x - (c.x + c.advance)).abs() < 0.01, "after c: {up:?}");
    assert!((down.x - right).abs() < 0.01, "before م, at the right end: {down:?} vs {right}");
    assert_eq!((up.line, down.line), (0, 0));
    // A click keeps the side it lands on; the cluster under the pointer wins the tie at the boundary.
    let ln = &l.lines[0];
    let y = ln.baseline - ln.ascent * 0.5;
    assert_eq!(hit(&l, t, c.x + c.advance * 0.75, y), Caret::new(3, true));
    assert_eq!(hit(&l, t, alef.x + alef.advance * 0.25, y), Caret::new(13, true), "left half of ا: after it");
    assert_eq!(hit(&l, t, right - 0.5, y), Caret::new(3, false));
    let seg = caret_segment(&l, t, Caret::new(3, true));
    assert!((seg[0].0 - up.x).abs() < 1e-3 && (seg[0].1 - up.top).abs() < 1e-3);
    // Upstream means nothing at the text start: the caret falls back to the next character.
    assert_eq!(caret_geometry(&l, t, Caret::new(0, true)), caret_geometry(&l, t, Caret::new(0, false)));
}

#[test]
fn a_click_never_lands_between_a_letter_and_its_haraka() {
    let mut e = TextEngine::new();
    let t = "بَ";
    let l = e.layout(&point(t), 72.0);
    let b = l.bounds().unwrap();
    let mut x = b[0] - 5.0;
    while x < b[2] + 5.0 {
        let c = hit(&l, t, x, -10.0);
        assert!(c.byte == 0 || c.byte == 4, "x {x}: {c:?}");
        x += 0.25;
    }
}

#[test]
fn a_caret_after_a_forced_line_break_belongs_to_the_next_line() {
    let mut e = TextEngine::new();
    let t = "ab\u{3}cd";
    let l = e.layout(&point(t), 72.0);
    let ln = &l.lines[0];
    // Far right of line 0: before the break, never after it.
    assert_eq!(hit(&l, t, edges(&l, 0).1 + 50.0, ln.baseline - ln.ascent * 0.5), Caret::new(2, true));
    assert_eq!(caret_geometry(&l, t, Caret::new(3, true)).line, 1);
}
