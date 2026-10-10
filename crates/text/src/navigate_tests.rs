//! Caret navigation over real layouts (`navigate.rs`). Bundled fonts only: Arabic glyphs are
//! .notdef, but bidi levels, clusters, line breaks and offsets are real.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextDirection, TextShape};

use crate::navigate::{Caret, Dir, Unit, adjacent_line, caret_geometry, caret_segment, collapse, hit, home_end, selection_segments, step};
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

/// Carets from `from`, stepping in `dir` until the caret stops moving (at most 200 steps).
fn walk(l: &TextLayout, text: &str, from: Caret, dir: Dir, unit: Unit) -> Vec<Caret> {
    let mut out = vec![from];
    let mut c = from;
    for _ in 0..200 {
        let n = step(l, text, c, dir, unit);
        if n == c {
            break;
        }
        out.push(n);
        c = n;
    }
    out
}

fn x_of(l: &TextLayout, text: &str, c: Caret) -> f32 {
    caret_geometry(l, text, c).x
}

/// A one-line walk in `dir` visits every cluster edge once, strictly in `dir` order, from one
/// visual edge to the other, and only stops on grapheme boundaries.
fn assert_visual_sweep(l: &TextLayout, text: &str, from: Caret, dir: Dir) -> Vec<Caret> {
    let w = walk(l, text, from, dir, Unit::Grapheme);
    let xs: Vec<f32> = w.iter().map(|c| x_of(l, text, *c)).collect();
    for p in xs.windows(2) {
        let moved = if dir == Dir::Right { p[1] - p[0] } else { p[0] - p[1] };
        assert!(moved > 0.5, "{text:?} {dir:?}: {xs:?} at {w:?}");
    }
    let clusters = l.clusters.iter().filter(|c| c.line == 0).count();
    assert_eq!(w.len(), clusters + 1, "{text:?} {dir:?}: one stop per cluster edge: {w:?}");
    let (lo, hi) = edges(l, 0);
    let (start, end) = if dir == Dir::Right { (lo, hi) } else { (hi, lo) };
    assert!((xs[0] - start).abs() < 0.01 && (xs[xs.len() - 1] - end).abs() < 0.01, "{text:?} {dir:?}: {xs:?} vs {start}..{end}");
    let graphemes = crate::segment::grapheme_boundaries(text);
    assert!(w.iter().all(|c| graphemes.contains(&c.byte)), "{w:?}");
    w
}

#[test]
fn arrows_move_visually_through_mixed_lines() {
    let mut e = TextEngine::new();
    // LTR paragraph with an Arabic word: → goes left to right across the reversed run.
    let t = "ab مرحبا cd";
    let l = e.layout(&point(t), 72.0);
    assert!(!l.lines[0].rtl);
    let right = assert_visual_sweep(&l, t, Caret::new(0, false), Dir::Right);
    let back = assert_visual_sweep(&l, t, *right.last().unwrap(), Dir::Left);
    let a: Vec<f32> = right.iter().map(|c| x_of(&l, t, *c)).collect();
    let b: Vec<f32> = back.iter().rev().map(|c| x_of(&l, t, *c)).collect();
    assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 0.01), "the same stops both ways: {a:?} vs {b:?}");
    // The exact stops of → from after "ab" (parley's order; see the ground rules).
    let w = walk(&l, t, Caret::new(1, true), Dir::Right, Unit::Grapheme);
    assert_eq!(&w[..5], &[Caret::new(1, true), Caret::new(2, true), Caret::new(3, true), Caret::new(11, false), Caret::new(9, false)]);
    // RTL paragraph with a Latin word: ← goes from the logical start on the right to the left end.
    let t = "مرحبا abc";
    let l = e.layout(&point(t), 72.0);
    assert!(l.lines[0].rtl);
    let left = assert_visual_sweep(&l, t, Caret::new(0, false), Dir::Left);
    assert_visual_sweep(&l, t, *left.last().unwrap(), Dir::Right);
}

#[test]
fn moving_into_a_run_of_the_other_direction_keeps_the_side_it_came_from() {
    let mut e = TextEngine::new();
    let t = "abcمرحبا";
    let l = e.layout(&point(t), 72.0);
    assert_eq!(step(&l, t, Caret::new(2, false), Dir::Right, Unit::Grapheme), Caret::new(3, true), "after c, not at the right end");
}

#[test]
fn an_rtl_paragraph_can_start_with_latin() {
    let mut e = TextEngine::new();
    let t = "Galaxy S24 شاشة";
    // Auto: the first strong letter is Latin, so the paragraph is LTR.
    assert!(!e.layout(&point(t), 72.0).lines[0].rtl);
    // Right-to-left: the Latin run sits on the right, G mid-line; the RLM prefix is never a stop.
    let l = e.layout(&directed(t, TextDirection::Rtl), 72.0);
    assert!(l.lines[0].rtl);
    let (lo, hi) = edges(&l, 0);
    let edge = l.clusters.iter().max_by(|a, b| (a.x + a.advance).total_cmp(&(b.x + b.advance))).map(|k| Caret::new(k.range.end, true)).unwrap();
    assert_eq!(t.get(edge.byte - 1..edge.byte), Some("4"));
    let sweep = assert_visual_sweep(&l, t, edge, Dir::Left);
    let g = sweep.iter().find(|c| c.byte == 0).map(|c| x_of(&l, t, *c)).unwrap();
    assert!(g > lo + 1.0 && g < hi - 1.0, "G starts mid-line: {g} in {lo}..{hi}");
    assert_eq!(step(&l, t, edge, Dir::Right, Unit::Grapheme), edge, "nothing right of the first paragraph");
}

#[test]
fn arrows_cross_paragraphs_on_their_visual_edges() {
    let mut e = TextEngine::new();
    // LTR then RTL: → at the end of "abc" goes to the start of the Arabic line, on its right.
    let t = "abc\nمرحبا";
    let l = e.layout(&point(t), 72.0);
    let next = step(&l, t, Caret::new(3, true), Dir::Right, Unit::Grapheme);
    assert_eq!(next, Caret::new(4, false));
    let g = caret_geometry(&l, t, next);
    assert_eq!(g.line, 1);
    assert!((g.x - edges(&l, 1).1).abs() < 0.01);
    // In the RTL paragraph → is backwards: from its start it returns to the end of "abc".
    assert_eq!(step(&l, t, next, Dir::Right, Unit::Grapheme), Caret::new(3, true));
    // RTL then LTR: ← at the end of the Arabic line (its left edge) goes to the start of "abc".
    let t = "مرحبا\nabc";
    let l = e.layout(&point(t), 72.0);
    let end = Caret::new(10, true);
    assert!((x_of(&l, t, end) - edges(&l, 0).0).abs() < 0.01);
    let next = step(&l, t, end, Dir::Left, Unit::Grapheme);
    assert_eq!(next, Caret::new(11, false));
    assert_eq!(caret_geometry(&l, t, next).line, 1);
    assert_eq!(step(&l, t, next, Dir::Left, Unit::Grapheme), end);
}

#[test]
fn an_empty_paragraph_is_one_stop() {
    let mut e = TextEngine::new();
    let t = "ab\n\ncd";
    let l = e.layout(&point(t), 72.0);
    let w = walk(&l, t, Caret::new(2, true), Dir::Right, Unit::Grapheme);
    assert_eq!(&w[..3], &[Caret::new(2, true), Caret::new(3, false), Caret::new(4, false)]);
    assert_eq!(caret_geometry(&l, t, Caret::new(3, false)).line, 1);
    assert_eq!(step(&l, t, Caret::new(4, false), Dir::Left, Unit::Grapheme), Caret::new(3, false));
    assert_eq!(step(&l, t, Caret::new(3, false), Dir::Left, Unit::Grapheme), Caret::new(2, true));
    // Empty text never moves.
    let l = e.layout(&point(""), 72.0);
    for dir in [Dir::Left, Dir::Right] {
        for unit in [Unit::Grapheme, Unit::Word] {
            assert_eq!(step(&l, "", Caret::default(), dir, unit), Caret::default());
        }
    }
}

#[test]
fn zero_width_stops_and_the_direction_prefix_are_one_step() {
    let mut e = TextEngine::new();
    // An RLM between b and c has no width: after b and after the RLM are one stop.
    let t = "ab\u{200F}cd";
    let l = e.layout(&point(t), 72.0);
    let w = walk(&l, t, Caret::new(0, false), Dir::Right, Unit::Grapheme);
    let xs: Vec<f32> = w.iter().map(|c| x_of(&l, t, *c)).collect();
    assert_eq!(w.len(), 5, "{w:?} {xs:?}");
    assert!(xs.windows(2).all(|p| p[1] - p[0] > 0.5), "{xs:?}");
    assert_eq!(walk(&l, t, *w.last().unwrap(), Dir::Left, Unit::Grapheme).len(), 5);
    // A forced direction adds an LRM/RLM before the text: never a stop of its own.
    for dir in [TextDirection::Ltr, TextDirection::Rtl] {
        let l = e.layout(&directed("abc", dir), 72.0);
        assert_eq!(walk(&l, "abc", Caret::new(0, false), Dir::Right, Unit::Grapheme).len(), 4, "{dir:?}");
        assert_eq!(step(&l, "abc", Caret::new(0, false), Dir::Left, Unit::Grapheme), Caret::new(0, false), "{dir:?}");
    }
}

#[test]
fn a_letter_and_its_harakat_are_one_stop() {
    let mut e = TextEngine::new();
    let t = "كَتَبَ";
    let l = e.layout(&point(t), 72.0);
    let bytes: Vec<usize> = walk(&l, t, Caret::new(0, false), Dir::Left, Unit::Grapheme).iter().map(|c| c.byte).collect();
    assert_eq!(bytes, vec![0, 4, 8, 12]);
    let back: Vec<usize> = walk(&l, t, Caret::new(12, true), Dir::Right, Unit::Grapheme).iter().map(|c| c.byte).collect();
    assert_eq!(back, vec![12, 8, 4, 0]);
}

#[test]
fn wrapped_lines_are_crossed_in_reading_order() {
    let mut e = TextEngine::new();
    // LTR: → at the end of the first line goes to the start of the second (same byte, downstream).
    let t = "aaa bbb ccc ddd";
    let l = e.layout(&boxed(t, 90.0, 1000.0), 72.0);
    assert!(l.lines.len() >= 2, "wraps");
    let end0 = Caret::new(l.lines[0].range.end, true);
    assert_eq!(caret_geometry(&l, t, end0).line, 0, "upstream: drawn at the end of line 0");
    let next = step(&l, t, end0, Dir::Right, Unit::Grapheme);
    assert_eq!(next, Caret::new(l.lines[1].range.start, false));
    assert_eq!(caret_geometry(&l, t, next).line, 1);
    assert_eq!(step(&l, t, next, Dir::Left, Unit::Grapheme), end0);
    // RTL: ← at the left end of line 0 goes on to the right end of line 1, not back up.
    let t = "سلام عليكم سلام عليكم سلام";
    let l = e.layout(&boxed(t, 120.0, 1000.0), 72.0);
    assert!(l.lines.len() >= 2 && l.lines[0].rtl, "wraps RTL");
    let at_left = walk(&l, t, Caret::new(0, false), Dir::Left, Unit::Grapheme).into_iter().take_while(|c| caret_geometry(&l, t, *c).line == 0).last().unwrap();
    assert!((x_of(&l, t, at_left) - edges(&l, 0).0).abs() < 0.01);
    let down = step(&l, t, at_left, Dir::Left, Unit::Grapheme);
    let g = caret_geometry(&l, t, down);
    assert_eq!(g.line, 1);
    assert!((g.x - edges(&l, 1).1).abs() < 0.01, "right end of line 1");
    assert_eq!(step(&l, t, down, Dir::Right, Unit::Grapheme), at_left, "→ goes back up");
}

#[test]
fn an_overflowing_box_stops_at_its_last_drawn_line() {
    let mut e = TextEngine::new();
    let t = "aaa bbb ccc ddd eee fff ggg hhh";
    let l = e.layout(&boxed(t, 90.0, 50.0), 72.0);
    assert!(l.paragraphs[0].layout.len() > l.lines.len(), "has hidden lines");
    let last = l.lines.len() - 1;
    let end = l.lines[last].range.end;
    let w = walk(&l, t, Caret::new(0, false), Dir::Right, Unit::Grapheme);
    assert_eq!(*w.last().unwrap(), Caret::new(end, true), "{w:?}");
    assert!(w.iter().all(|c| c.byte <= end && caret_geometry(&l, t, *c).line <= last));
    // A caret in hidden text (from an agent) is pulled back into the drawn text first.
    assert!(step(&l, t, Caret::new(t.len(), false), Dir::Left, Unit::Grapheme).byte < end);
}

#[test]
fn ctrl_arrows_move_by_visual_word() {
    let mut e = TextEngine::new();
    let t = "abc مرحبا def";
    let l = e.layout(&point(t), 72.0);
    let w = walk(&l, t, Caret::new(0, false), Dir::Right, Unit::Word);
    let xs: Vec<f32> = w.iter().map(|c| x_of(&l, t, *c)).collect();
    assert!(xs.windows(2).all(|p| p[1] - p[0] > 0.5), "{w:?} {xs:?}");
    assert!(w.len() >= 3 && w.len() < 14, "word stops, not one per letter: {w:?}");
    assert!((xs[xs.len() - 1] - edges(&l, 0).1).abs() < 0.01, "ends at the right edge");
    let graphemes = crate::segment::grapheme_boundaries(t);
    assert!(w.iter().all(|c| graphemes.contains(&c.byte)));
    let back = walk(&l, t, *w.last().unwrap(), Dir::Left, Unit::Word);
    assert!((x_of(&l, t, *back.last().unwrap()) - edges(&l, 0).0).abs() < 0.01);
}

#[test]
fn vertical_type_steps_in_text_order() {
    let mut e = TextEngine::new();
    let s = "ab cd";
    let mut t = point(s);
    t.orientation = Orientation::Vertical;
    let l = e.layout(&t, 72.0);
    assert!(l.vertical);
    assert_eq!(step(&l, s, Caret::new(0, false), Dir::Right, Unit::Grapheme), Caret::new(1, false));
    assert_eq!(step(&l, s, Caret::new(1, false), Dir::Left, Unit::Grapheme), Caret::new(0, false));
    assert_eq!(step(&l, s, Caret::new(0, false), Dir::Right, Unit::Word), Caret::new(2, false));
}

#[test]
fn home_and_end_stay_on_the_carets_line() {
    let mut e = TextEngine::new();
    let t = "aaa bbb ccc ddd";
    let l = e.layout(&boxed(t, 90.0, 1000.0), 72.0);
    let (r0, r1) = (l.lines[0].range.clone(), l.lines[1].range.clone());
    let end = home_end(&l, t, Caret::new(r0.start + 2, false), true);
    assert_eq!(end, Caret::new(r0.end, true));
    assert_eq!(caret_geometry(&l, t, end).line, 0, "End of a wrapped line stays on it");
    assert_eq!(home_end(&l, t, end, false), Caret::new(r0.start, false));
    // The same byte downstream is the next line's start: its End is that line's end.
    assert_eq!(home_end(&l, t, Caret::new(r1.start, false), true).byte, r1.end);
    // A line ended by a forced line break ends before it.
    let t = "ab\u{3}cd";
    let l = e.layout(&point(t), 72.0);
    assert_eq!(home_end(&l, t, Caret::new(1, false), true), Caret::new(2, false));
}

#[test]
fn up_and_down_keep_the_x_and_report_the_side() {
    let mut e = TextEngine::new();
    let t = "abcdef\nab";
    let l = e.layout(&point(t), 72.0);
    let from = Caret::new(6, true);
    assert_eq!(adjacent_line(&l, t, from, caret_geometry(&l, t, from).x, 1), Caret::new(9, true), "past the short line's end");
    let b = Caret::new(8, false);
    assert_eq!(adjacent_line(&l, t, b, caret_geometry(&l, t, b).x, -1), Caret::new(1, false), "the same x on line 0");
    assert_eq!(adjacent_line(&l, t, Caret::new(1, false), 0.0, -1), Caret::new(0, false), "above the first line: text start");
    assert_eq!(adjacent_line(&l, t, b, 0.0, 1), Caret::new(t.len(), true), "below the last: text end");
    // An overflowing box: never below its last drawn line.
    let t = "aaa bbb ccc ddd eee fff ggg hhh";
    let l = e.layout(&boxed(t, 90.0, 50.0), 72.0);
    let end = l.lines[l.lines.len() - 1].range.end;
    assert_eq!(adjacent_line(&l, t, Caret::new(end, true), 0.0, 1), Caret::new(end, true));
}

#[test]
fn collapsing_a_selection_goes_to_its_end_in_the_arrows_direction() {
    let mut e = TextEngine::new();
    // In RTL text the logical start (byte 0) is on the right.
    let t = "مرحبا";
    let l = e.layout(&point(t), 72.0);
    let (a, b) = (Caret::new(0, false), Caret::new(6, false));
    assert_eq!(collapse(&l, t, a, b, Dir::Right), a);
    assert_eq!(collapse(&l, t, a, b, Dir::Left), b);
    let t = "abc";
    let l = e.layout(&point(t), 72.0);
    let (a, b) = (Caret::new(0, false), Caret::new(2, false));
    assert_eq!(collapse(&l, t, a, b, Dir::Right), b);
    assert_eq!(collapse(&l, t, b, a, Dir::Left), a);
}

#[test]
fn a_selection_is_one_rectangle_per_visual_piece() {
    let mut e = TextEngine::new();
    let t = "abc مرحبا def";
    let l = e.layout(&point(t), 72.0);
    // "c ", then the first Arabic letter, which sits at the far end of the reversed run.
    let s = selection_segments(&l, t, 2, 6);
    assert_eq!(s.len(), 2, "{s:?}");
    assert!(s.iter().all(|g| !g.line_break && g.line == 0));
    let mim = l.clusters.iter().find(|k| k.range == (4..6)).unwrap();
    assert!(s.iter().any(|g| (g.x0 - mim.x).abs() < 0.01 && (g.x1 - mim.x - mim.advance).abs() < 0.01), "{s:?}");
    // The whole Arabic word is one piece; a reversed range is the same selection.
    assert_eq!(selection_segments(&l, t, 4, 14).len(), 1);
    assert_eq!(selection_segments(&l, t, 14, 4), selection_segments(&l, t, 4, 14));
    assert!(selection_segments(&l, t, 5, 5).is_empty());
}

#[test]
fn the_selected_line_break_shows_on_the_paragraphs_end_side() {
    let mut e = TextEngine::new();
    let t = "abc\nxyz";
    let l = e.layout(&point(t), 72.0);
    let s = selection_segments(&l, t, 1, 5);
    let mark = s.iter().find(|g| g.line_break).unwrap();
    assert_eq!(mark.line, 0);
    assert!(mark.x0 >= edges(&l, 0).1 - 0.01, "right of the LTR line: {s:?}");
    let t = "مرحبا\nxyz";
    let l = e.layout(&point(t), 72.0);
    let s = selection_segments(&l, t, 2, 12);
    let mark = s.iter().find(|g| g.line_break).unwrap();
    assert_eq!(mark.line, 0);
    assert!(mark.x1 <= edges(&l, 0).0 + 0.01, "left of the RTL line: {s:?}");
    // Neither a soft wrap nor the end of the text is a line break.
    let t = "aaa bbb ccc ddd";
    let l = e.layout(&boxed(t, 90.0, 1000.0), 72.0);
    assert!(selection_segments(&l, t, 0, t.len()).iter().all(|g| !g.line_break));
}

#[test]
fn an_upstream_caret_after_a_forced_break_steps_like_the_downstream_one() {
    let mut e = TextEngine::new();
    // RTL: the break is visually leftmost on line 0, so parley's upstream cluster for byte 7 is it.
    let t = "ابت\u{3}ثجح";
    let l = e.layout(&point(t), 72.0);
    assert!(l.lines.len() == 2 && l.lines[0].rtl, "{:?}", l.lines);
    for dir in [Dir::Left, Dir::Right] {
        let up = step(&l, t, Caret::new(7, true), dir, Unit::Grapheme);
        assert_eq!(up, step(&l, t, Caret::new(7, false), dir, Unit::Grapheme), "{dir:?}");
    }
    // ← lands on line 1, one cluster in from its right edge, not at the end of the line.
    let left = step(&l, t, Caret::new(7, true), Dir::Left, Unit::Grapheme);
    let g = caret_geometry(&l, t, left);
    let first = l.clusters.iter().find(|k| k.line == 1 && k.range.start == 7).unwrap();
    assert_eq!(g.line, 1);
    assert!((g.x - first.x).abs() < 0.01, "{g:?} {first:?}");
    assert_eq!(left, Caret::new(9, true));
}

#[test]
fn a_forced_break_at_the_end_of_a_paragraph_leaves_a_reachable_empty_line() {
    let mut e = TextEngine::new();
    let t = "ab\u{3}";
    let l = e.layout(&point(t), 72.0);
    assert_eq!(l.lines.len(), 2, "{:?}", l.lines);
    assert_eq!(caret_geometry(&l, t, Caret::new(3, false)).line, 1);
    assert_eq!(step(&l, t, Caret::new(2, true), Dir::Right, Unit::Grapheme), Caret::new(3, false));
    assert_eq!(step(&l, t, Caret::new(3, false), Dir::Left, Unit::Grapheme), Caret::new(2, true));
}

#[test]
fn a_caret_between_cr_and_lf_still_moves() {
    let mut e = TextEngine::new();
    let t = "ab\r\ncd";
    let l = e.layout(&point(t), 72.0);
    for dir in [Dir::Left, Dir::Right] {
        let n = step(&l, t, Caret::new(3, false), dir, Unit::Grapheme);
        assert_ne!(n.byte, 3, "{dir:?}: {n:?}");
    }
}

/// Attaches one paragraph style to the whole text.
fn styled(mut t: TextLayer, style: ParagraphStyle) -> TextLayer {
    t.paragraphs = vec![ParagraphRun { len: t.text.len(), style }];
    t
}

#[test]
fn an_indented_justified_rtl_box_sweeps_visually() {
    let mut e = TextEngine::new();
    let t = "سلام عليكم سلام عليكم سلام عليكم سلام عليكم";
    let style = ParagraphStyle { align: TextAlign::JustifyRight, start_indent_pt: 30.0, ..Default::default() };
    let l = e.layout(&styled(boxed(t, 200.0, 1000.0), style), 72.0);
    assert!(l.lines.len() >= 2 && l.lines[0].rtl, "wraps RTL: {:?}", l.lines);
    let (lo, hi) = edges(&l, 0);
    let start = l
        .clusters
        .iter()
        .filter(|c| c.line == 0)
        .max_by(|a, b| (a.x + a.advance).total_cmp(&(b.x + b.advance)))
        .map(|k| Caret::new(k.range.start, false))
        .unwrap();
    let w: Vec<Caret> = walk(&l, t, start, Dir::Left, Unit::Grapheme).into_iter().take_while(|c| caret_geometry(&l, t, *c).line == 0).collect();
    let xs: Vec<f32> = w.iter().map(|c| x_of(&l, t, *c)).collect();
    assert!(xs.windows(2).all(|p| p[0] - p[1] > 0.5), "strictly leftwards: {xs:?}");
    let clusters = l.clusters.iter().filter(|c| c.line == 0).count();
    assert_eq!(w.len(), clusters + 1, "one stop per cluster edge: {w:?}");
    assert!((xs[0] - hi).abs() < 0.01 && (xs[xs.len() - 1] - lo).abs() < 0.01, "{xs:?} vs {lo}..{hi}");
    assert!(hi < 300.0 - 29.0, "the start indent keeps the text off the right edge: {hi}");
}

#[test]
fn ctrl_arrows_cross_a_wrapped_rtl_line() {
    let mut e = TextEngine::new();
    let t = "سلام عليكم سلام عليكم سلام";
    let l = e.layout(&boxed(t, 120.0, 1000.0), 72.0);
    assert!(l.lines.len() >= 2 && l.lines[0].rtl, "wraps RTL");
    let w = walk(&l, t, Caret::new(0, false), Dir::Left, Unit::Word);
    let lines: Vec<usize> = w.iter().map(|c| caret_geometry(&l, t, *c).line).collect();
    assert!(lines.contains(&0) && lines.contains(&1), "{w:?} {lines:?}");
    let last = *w.last().unwrap();
    let last_line = l.lines.len() - 1;
    assert_eq!(caret_geometry(&l, t, last).line, last_line);
    assert!((x_of(&l, t, last) - edges(&l, last_line).0).abs() < 0.01, "far (left) edge of the last line");
}

/// Home and End are logical (Dhia's decision): Home is the line's first character and End its last,
/// wherever they sit on screen. In a mixed line they are not the visual edges.
#[test]
fn home_and_end_on_a_mixed_line_are_logical() {
    let mut e = TextEngine::new();
    let t = "Hello مرحبا";
    let l = e.layout(&point(t), 72.0);
    assert_eq!(home_end(&l, t, Caret::new(2, false), true).byte, t.len());
    assert_eq!(home_end(&l, t, Caret::new(2, false), false).byte, 0);
    let t = "Galaxy S24 شاشة";
    let l = e.layout(&directed(t, TextDirection::Rtl), 72.0);
    let home = home_end(&l, t, Caret::new(t.len(), true), false);
    assert_eq!(home.byte, 0);
    let (lo, hi) = edges(&l, 0);
    let x = x_of(&l, t, home);
    assert!(x > lo + 1.0 && x < hi - 1.0, "the G is mid-line: {x} in {lo}..{hi}");
}

#[test]
fn collapsing_across_lines_goes_to_the_end_further_in_the_arrows_direction() {
    let mut e = TextEngine::new();
    // LTR: the later line is further right in flow.
    let t = "abc\ndef";
    let l = e.layout(&point(t), 72.0);
    let (a, b) = (Caret::new(1, false), Caret::new(6, false));
    assert_eq!(collapse(&l, t, a, b, Dir::Right), b);
    assert_eq!(collapse(&l, t, a, b, Dir::Left), a);
    assert_eq!(collapse(&l, t, b, a, Dir::Right), b, "anchor and focus in either order");
    // RTL: the later line is further left.
    let t = "ابت\nثجح";
    let l = e.layout(&point(t), 72.0);
    let (a, b) = (Caret::new(2, false), Caret::new(9, false));
    assert_eq!(collapse(&l, t, a, b, Dir::Left), b);
    assert_eq!(collapse(&l, t, a, b, Dir::Right), a);
}

#[test]
fn vertical_type_never_steps_into_hidden_columns() {
    let mut e = TextEngine::new();
    let s = "aaa bbb ccc ddd eee fff ggg hhh";
    let mut t = boxed(s, 60.0, 50.0);
    t.orientation = Orientation::Vertical;
    let l = e.layout(&t, 72.0);
    assert!(l.vertical);
    let end = l.lines.last().map_or(0, |ln| ln.range.end);
    assert!(end < s.len(), "has hidden text: drawn to {end}");
    let w = walk(&l, s, Caret::new(0, false), Dir::Right, Unit::Grapheme);
    assert!(w.iter().all(|c| c.byte <= end), "{w:?}");
    assert_eq!(step(&l, s, Caret::new(end, true), Dir::Right, Unit::Grapheme).byte, end);
}
