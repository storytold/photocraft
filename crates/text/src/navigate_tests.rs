//! Caret navigation over real layouts (`navigate.rs`). Bundled fonts only: Arabic glyphs are
//! .notdef, but bidi levels, clusters, line breaks and offsets are real.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{ParagraphRun, ParagraphStyle, TextDirection, TextShape};

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
