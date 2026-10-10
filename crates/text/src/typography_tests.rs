//! Arabic typography: display digits and kashida justification (spec 6). The bundled fonts have
//! no Arabic glyphs, so the font-dependent assertions run against installed Arabic fonts and skip
//! with a message when none is registered (Amiri and Noto Sans Arabic are requested for
//! craft-fonts); offsets, bounds and no-crash assertions run everywhere.

use photocraft_doc::TextLayer;
use photocraft_doc::text::{CharStyle, Digits, ParagraphRun, ParagraphStyle, TextAlign, TextRun, TextShape};

use crate::{TextEngine, TextLayout};

const BOX_X: f32 = 100.0;

fn style(family: &str, digits: Digits) -> CharStyle {
    CharStyle { font_family: family.into(), size_pt: 24.0, digits, ..Default::default() }
}

fn layer(text: &str, style: CharStyle, para: ParagraphStyle, shape: TextShape) -> TextLayer {
    TextLayer {
        text: text.into(),
        runs: vec![TextRun { len: text.len(), style }],
        paragraphs: vec![ParagraphRun { len: text.len(), style: para }],
        shape,
        ..Default::default()
    }
}

fn boxed(text: &str, family: &str, width: f32, para: ParagraphStyle) -> TextLayer {
    layer(text, style(family, Digits::Western), para, TextShape::Box { x: BOX_X, y: 0.0, width, height: 2000.0 })
}

/// The first of `families` that the engine has, with a skip message otherwise.
fn pick_font(e: &mut TextEngine, families: &[&str]) -> Option<String> {
    let found = families.iter().find(|f| e.fonts.has_family(f)).map(|f| (*f).to_string());
    if found.is_none() {
        eprintln!("skipped: none of {families:?} is registered");
    }
    found
}

const ARABIC_FONTS: [&str; 3] = ["Amiri", "Dubai", "Noto Naskh Arabic"];

fn justified(kashida: bool) -> ParagraphStyle {
    ParagraphStyle { align: TextAlign::JustifyLeft, kashida, ..Default::default() }
}

fn clusters_tile_the_line(l: &TextLayout, line: usize) {
    let mut cs: Vec<_> = l.clusters.iter().filter(|c| c.line == line).collect();
    cs.sort_by(|a, b| a.x.total_cmp(&b.x));
    for w in cs.windows(2) {
        assert!((w[0].x + w[0].advance - w[1].x).abs() < 0.02, "line {line}: {} + {} vs {}", w[0].x, w[0].advance, w[1].x);
    }
}

#[test]
fn digits_without_the_glyphs_keep_the_western_ones_and_every_offset() {
    let mut e = TextEngine::new();
    let text = "سعر 123 ريال 45";
    let western = e.layout(&layer(text, style("Inter", Digits::Western), ParagraphStyle::default(), TextShape::Point), 72.0);
    for d in [Digits::ArabicIndic, Digits::Persian] {
        let l = e.layout(&layer(text, style("Inter", d), ParagraphStyle::default(), TextShape::Point), 72.0);
        assert_eq!(
            l.glyphs.iter().map(|g| g.id).collect::<Vec<_>>(),
            western.glyphs.iter().map(|g| g.id).collect::<Vec<_>>(),
            "{d:?}: Inter has no such digits"
        );
        assert_eq!(l.clusters.iter().map(|c| c.range.clone()).collect::<Vec<_>>(), western.clusters.iter().map(|c| c.range.clone()).collect::<Vec<_>>());
        assert_eq!((l.lines[0].x0, l.lines[0].x1), (western.lines[0].x0, western.lines[0].x1));
    }
}

#[test]
fn digits_render_with_the_run_fonts_glyphs_and_keep_the_right_edge_on_the_anchor() {
    let mut e = TextEngine::with_system_fonts();
    let Some(family) = pick_font(&mut e, &ARABIC_FONTS) else { return };
    let text = "الرقم 0551234567 فقط";
    let right = ParagraphStyle { align: TextAlign::Right, ..Default::default() };
    let western = e.layout(&layer(text, style(&family, Digits::Western), right.clone(), TextShape::Point), 72.0);
    for d in [Digits::ArabicIndic, Digits::Persian] {
        let l = e.layout(&layer(text, style(&family, d), right.clone(), TextShape::Point), 72.0);
        assert_eq!(l.clusters.len(), western.clusters.len());
        assert_eq!(
            l.clusters.iter().map(|c| c.range.clone()).collect::<Vec<_>>(),
            western.clusters.iter().map(|c| c.range.clone()).collect::<Vec<_>>(),
            "byte offsets don't move"
        );
        let changed = l.glyphs.iter().zip(&western.glyphs).filter(|(a, b)| a.id != b.id).count();
        assert_eq!(changed, 10, "{family} {d:?}: each of the ten digits has its own glyph");
        assert!(l.lines[0].x1.abs() < 0.01, "{family} {d:?}: right edge {} stays on the anchor", l.lines[0].x1);
        assert!(western.lines[0].x1.abs() < 0.01);
        clusters_tile_the_line(&l, 0);
    }
}

#[test]
fn digits_missing_from_a_font_fall_back_to_western_per_digit() {
    // Inter has no Arabic-Indic digits and an Arabic font has them: a run in Inter keeps its
    // Western glyphs whatever the setting says, even in a text that also has Arabic letters.
    let mut e = TextEngine::new();
    let l = e.layout(&layer("a 7", style("Inter", Digits::ArabicIndic), ParagraphStyle::default(), TextShape::Point), 72.0);
    let w = e.layout(&layer("a 7", style("Inter", Digits::Western), ParagraphStyle::default(), TextShape::Point), 72.0);
    assert_eq!(l.glyphs.iter().map(|g| g.id).collect::<Vec<_>>(), w.glyphs.iter().map(|g| g.id).collect::<Vec<_>>());
}

const PARAGRAPH: &str = "سلام مدرسة سلام مدرسة سلام مدرسة سلام مدرسة سلام مدرسة سلام مدرسة";

#[test]
fn kashida_off_or_unjustified_leaves_the_layout_unchanged() {
    let mut e = TextEngine::with_system_fonts();
    let family = pick_font(&mut e, &ARABIC_FONTS).unwrap_or_else(|| "Inter".into());
    let off = e.layout(&boxed(PARAGRAPH, &family, 300.0, justified(false)), 72.0);
    let on_but_left = e.layout(&boxed(PARAGRAPH, &family, 300.0, ParagraphStyle { kashida: true, ..Default::default() }), 72.0);
    let plain_left = e.layout(&boxed(PARAGRAPH, &family, 300.0, ParagraphStyle::default()), 72.0);
    assert_eq!(on_but_left.glyphs, plain_left.glyphs, "kashida needs a justified paragraph");
    assert_eq!(on_but_left.lines, plain_left.lines);
    assert!(off.lines.len() > 2, "the text wraps");
}

#[test]
fn kashida_lines_fill_the_box_with_or_without_a_font() {
    // The bundled fonts have no Arabic or tatweel, so no join is accepted and the gaps take it all.
    let mut e = TextEngine::new();
    for text in [PARAGRAPH, "alpha beta gamma delta alpha beta gamma delta alpha beta gamma delta"] {
        let on = e.layout(&boxed(text, "Inter", 260.0, justified(true)), 72.0);
        assert!(on.lines.len() > 1);
        for (i, ln) in on.lines.iter().enumerate().take(on.lines.len() - 1) {
            assert!((ln.x0 - BOX_X).abs() < 0.05 && (ln.x1 - (BOX_X + 260.0)).abs() < 0.05, "line {i}: {ln:?}");
            clusters_tile_the_line(&on, i);
        }
        let last = on.lines.last().unwrap();
        assert!(last.x1 - last.x0 < 260.0 - 1.0, "the last line is not justified: {last:?}");
    }
}

#[test]
fn a_kashida_line_fills_its_box_exactly_and_the_last_line_is_left_alone() {
    let mut e = TextEngine::with_system_fonts();
    let Some(family) = pick_font(&mut e, &ARABIC_FONTS) else { return };
    let width = 300.0;
    let on = e.layout(&boxed(PARAGRAPH, &family, width, justified(true)), 72.0);
    let off = e.layout(&boxed(PARAGRAPH, &family, width, justified(false)), 72.0);
    let n = on.lines.len();
    assert!(n >= 3 && n == off.lines.len(), "{n} lines");
    for (i, ln) in on.lines.iter().enumerate().take(n - 1) {
        assert!((ln.x0 - BOX_X).abs() < 0.05 && (ln.x1 - (BOX_X + width)).abs() < 0.05, "{family} line {i} fills the box: {ln:?}");
        clusters_tile_the_line(&on, i);
    }
    assert!(on.glyphs.len() > off.glyphs.len(), "{family}: tatweel copies were placed");
    let (a, b) = (&on.lines[n - 1], &off.lines[n - 1]);
    assert!(a.x1 - a.x0 < width - 1.0, "the last line is not justified");
    assert!((a.x0 - b.x0).abs() < 0.01 && (a.x1 - b.x1).abs() < 0.01, "and has no kashida: {a:?} vs {b:?}");
}

#[test]
fn justify_all_stretches_the_last_line_and_a_forced_break_ends_a_line() {
    let mut e = TextEngine::with_system_fonts();
    let Some(family) = pick_font(&mut e, &ARABIC_FONTS) else { return };
    let width = 300.0;
    let all = ParagraphStyle { align: TextAlign::JustifyAll, kashida: true, ..Default::default() };
    let l = e.layout(&boxed(PARAGRAPH, &family, width, all), 72.0);
    let last = l.lines.last().unwrap();
    assert!((last.x0 - BOX_X).abs() < 0.05 && (last.x1 - (BOX_X + width)).abs() < 0.05, "{last:?}");
    // "سلام مدرسة" + forced break + more: the line before the break is short, like a last line.
    let broken = format!("سلام مدرسة{}سلام مدرسة", crate::layout::FORCED_LINE_BREAK);
    let l = e.layout(&boxed(&broken, &family, width, justified(true)), 72.0);
    assert!(l.lines[0].x1 - l.lines[0].x0 < width - 1.0, "{:?}", l.lines[0]);
}

#[test]
fn kashida_leaves_marks_with_their_letters() {
    let mut e = TextEngine::with_system_fonts();
    let Some(family) = pick_font(&mut e, &ARABIC_FONTS) else { return };
    let text = "سَلَامٌ مَدْرَسَةٌ سَلَامٌ مَدْرَسَةٌ سَلَامٌ مَدْرَسَةٌ سَلَامٌ مَدْرَسَةٌ";
    let on = e.layout(&boxed(text, &family, 280.0, justified(true)), 72.0);
    for (i, ln) in on.lines.iter().enumerate().take(on.lines.len() - 1) {
        assert!(ln.x0 >= BOX_X - 0.05 && ln.x1 <= BOX_X + 280.0 + 0.05, "line {i}: {ln:?}");
        clusters_tile_the_line(&on, i);
    }
}

fn decorated(text: &str, family: &str, from: usize) -> TextLayer {
    let plain = CharStyle { font_family: family.into(), size_pt: 20.0, ..Default::default() };
    let lined = CharStyle { underline: true, strikethrough: true, ..plain.clone() };
    TextLayer { text: text.into(), runs: vec![TextRun { len: from, style: plain }, TextRun { len: text.len() - from, style: lined }], ..Default::default() }
}

/// The underline and the strikethrough of a layout (the only decorations of `decorated`).
fn decorations_span(l: &TextLayout) -> Vec<(f32, f32)> {
    assert_eq!(l.decorations.len(), 2, "{:?}", l.decorations);
    l.decorations.iter().map(|d| (d.x0, d.x1)).collect()
}

fn box_layer(mut t: TextLayer, width: f32, para: ParagraphStyle) -> TextLayer {
    t.paragraphs = vec![ParagraphRun { len: t.text.len(), style: para }];
    t.shape = TextShape::Box { x: BOX_X, y: 0.0, width, height: 2000.0 };
    t
}

#[test]
fn justify_all_stretches_underlines_and_strikethroughs_with_the_word_gaps() {
    let mut e = TextEngine::new();
    let all = ParagraphStyle { align: TextAlign::JustifyAll, ..Default::default() };
    // The whole line is lined; so is only its tail "bb cc", which starts where its cluster starts.
    let l = e.layout(&box_layer(decorated("aa bb cc", "Inter", 0), 400.0, all.clone()), 72.0);
    for (x0, x1) in decorations_span(&l) {
        assert!((x0 - BOX_X).abs() < 0.05 && (x1 - (BOX_X + 400.0)).abs() < 0.5, "full line: {x0}..{x1}");
    }
    let l = e.layout(&box_layer(decorated("aa bb cc", "Inter", 3), 400.0, all), 72.0);
    let from = l.clusters.iter().find(|c| c.range.start == 3).expect("the b cluster").x;
    for (x0, x1) in decorations_span(&l) {
        assert!((x0 - from).abs() < 0.05, "starts at the first lined letter: {x0} vs {from}");
        assert!((x1 - (BOX_X + 400.0)).abs() < 0.5, "ends at the line's ink: {x1}");
    }
}

#[test]
fn justify_all_letter_spacing_carries_the_underline_to_the_last_letter() {
    let mut e = TextEngine::new();
    let all = ParagraphStyle { align: TextAlign::JustifyAll, ..Default::default() };
    let l = e.layout(&box_layer(decorated("abc", "Inter", 0), 400.0, all), 72.0);
    let last = l.clusters.iter().find(|c| c.range.start == 2).expect("the c cluster");
    for (x0, x1) in decorations_span(&l) {
        assert!((x0 - BOX_X).abs() < 0.05, "{x0}");
        assert!((x1 - (last.x + last.advance)).abs() < 0.05 && (x1 - (BOX_X + 400.0)).abs() < 0.5, "{x1} vs {}", last.x + last.advance);
    }
}

#[test]
fn kashida_lines_keep_underlines_over_the_ink() {
    let mut e = TextEngine::with_system_fonts();
    let Some(family) = pick_font(&mut e, &ARABIC_FONTS) else { return };
    let l = e.layout(&box_layer(decorated(PARAGRAPH, &family, 0), 250.0, justified(true)), 72.0);
    assert!(l.lines.len() > 2);
    for d in l.decorations.iter().take(l.decorations.len().saturating_sub(2)) {
        let line = l.lines.iter().find(|ln| ln.baseline > d.y0 - 30.0 && ln.baseline < d.y1 + 30.0).expect("its line");
        assert!(d.x0 >= line.x0 - 0.05 && d.x1 <= line.x1 + 0.05, "{d:?} inside {line:?}");
        assert!((d.x0 - line.x0).abs() < 0.1 && (d.x1 - line.x1).abs() < 0.1, "{d:?} spans {line:?}");
    }
}
