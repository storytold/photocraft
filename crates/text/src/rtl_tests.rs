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
