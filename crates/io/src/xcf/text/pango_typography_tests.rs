use super::*;

fn glyph_at(layout: &photocraft_text::layout::TextLayout, range: std::ops::Range<usize>) -> photocraft_text::layout::PlacedGlyph {
    // Native LTR paragraphs also shape an injected direction mark, whose range
    // is empty. Select the actual source cluster rather than a glyph index.
    assert_eq!(layout.glyphs.len(), layout.glyph_ranges.len());
    let matching: Vec<_> = layout.glyphs.iter().zip(&layout.glyph_ranges).filter(|(_, source)| **source == range).map(|(glyph, _)| *glyph).collect();
    assert_eq!(matching.len(), 1, "one visible Latin glyph for source cluster {range:?}");
    assert_ne!(matching[0].id, 0, "source cluster uses a real font glyph");
    matching[0]
}

fn roundtrip(layer: &TextLayer) {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    let mut document = Document::new("Source typography", photocraft_geom::Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
    document.layers.push(Layer::new("Editable", LayerContent::Text(layer.clone())));
    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    let LayerContent::Text(reopened) = &reopened.layers[0].content else { panic!("editable native text") };
    assert_eq!(reopened, layer);
    let mut engine = photocraft_text::TextEngine::new();
    assert_eq!(engine.layout(reopened, 72.).glyphs, engine.layout(layer, 72.).glyphs);
}

#[test]
fn inactive_source_kerning_flag_preserves_metrics_glyph_positions_and_explicit_features() {
    for flag in ["", " (kerning no)", " (kerning yes)"] {
        let config = format!("(text \"AV\") (font \"Inter\") (font-size 72){flag}");
        let parsed = decode(config.as_bytes(), 72., (0, 0)).unwrap();
        let layer = parsed.layer;
        let mut oracle = layer.clone();
        oracle.runs[0].style.kerning = Kerning::Metrics;
        let mut engine = photocraft_text::TextEngine::new();
        let actual = engine.layout(&layer, 72.);
        assert_eq!(actual.glyphs, engine.layout(&oracle, 72.).glyphs);
        let a = glyph_at(&actual, 0..1);
        let v = glyph_at(&actual, 1..2);
        assert!(v.x > a.x);
        oracle.runs[0].style.kerning = Kerning::Off;
        let off = engine.layout(&oracle, 72.);
        assert_ne!(v.x - a.x, glyph_at(&off, 1..2).x - glyph_at(&off, 0..1).x, "AV exercises actual font kerning");
        assert_eq!(layer.runs[0].style.kerning, Kerning::Metrics);
        if !flag.is_empty() {
            assert!(parsed.native_policy.iter().any(|note| note.contains("kerning") && note.contains("inactive")));
        }
        roundtrip(&layer);
    }
    let parsed =
        decode(br#"(markup "<markup><span font_features='kern=0'>AV</span></markup>") (font "Inter") (font-size 72) (kerning no)"#, 72., (0, 0)).unwrap();
    assert_eq!(parsed.layer.runs[0].style.features, vec![FontFeature { tag: "kern".into(), value: 0 }]);
    let mut oracle = parsed.layer.clone();
    oracle.runs[0].style.features.clear();
    oracle.runs[0].style.kerning = Kerning::Off;
    let mut engine = photocraft_text::TextEngine::new();
    assert_eq!(engine.layout(&parsed.layer, 72.).glyphs, engine.layout(&oracle, 72.).glyphs, "explicit kern=0 remains authoritative");
    roundtrip(&parsed.layer);
}

#[test]
fn additional_line_spacing_uses_resolved_markup_sizes_for_multiline_layout() {
    let parsed = decode(
        br#"(markup "<markup><span size='72pt'>A\nA</span><span size='24pt'>\nA</span></markup>") (font "Inter") (font-size 12) (line-spacing 1)"#,
        72.,
        (0, 0),
    )
    .unwrap();
    let layer = parsed.layer;
    let layout = photocraft_text::TextEngine::new().layout(&layer, 72.);
    assert_eq!(layer.text, "A\nA\nA");
    let first = glyph_at(&layout, 0..1);
    let second = glyph_at(&layout, 2..3);
    let third = glyph_at(&layout, 4..5);
    assert!(third.y > second.y);
    assert!((second.y - first.y - 87.4).abs() < 0.001, "72pt lines receive additional spacing rather than overlapping");
    for (run, size) in layer.runs.iter().zip([72., 24.]) {
        assert_eq!(run.style.size_pt, size);
        assert!((run.style.leading_pt.unwrap() - (size * 1.2 + 1.)).abs() < 0.0001);
    }
    roundtrip(&layer);
}

#[test]
fn nested_markup_percentages_replace_scale_independently_of_absolute_font_size() {
    for (markup, expected) in [
        ("<span size='200%'><span size='50%'>AV</span></span>", 12.),
        ("<span size='30pt'><span size='50%'>AV</span></span>", 15.),
        ("<span size='200%'><span size='30pt'>AV</span></span>", 60.),
        ("<span size='30pt'><span size='50%'><big>AV</big></span></span>", 18.),
        ("<span size='200%'><big>AV</big></span>", 57.6),
        ("<span size='200%'><span size='smaller'>AV</span></span>", 40.),
        ("<span size='30pt'><span size='medium'>AV</span></span>", 30.),
        ("<span size='200%'><span font='Inter 30'><span size='50%'>AV</span></span></span>", 15.),
    ] {
        let config = format!("(markup \"<markup>{markup}</markup>\") (font \"Inter\") (font-size 24) (letter-spacing 2) (line-spacing 1)");
        let layer = decode(config.as_bytes(), 72., (0, 0)).unwrap().layer;
        assert_eq!(layer.runs.len(), 1);
        let style = &layer.runs[0].style;
        assert!((style.size_pt - expected).abs() < 0.0001, "{markup}: {}", style.size_pt);
        assert!((style.tracking * style.size_pt / 1000. - 2.).abs() < 0.0001);
        assert!((style.leading_pt.unwrap() - (expected * 1.2 + 1.)).abs() < 0.0001);
        let mut oracle = layer.clone();
        oracle.runs[0].style.size_pt = expected;
        let mut engine = photocraft_text::TextEngine::new();
        let actual = engine.layout(&layer, 72.);
        let expected_layout = engine.layout(&oracle, 72.);
        assert_eq!(actual.glyphs.len(), expected_layout.glyphs.len());
        assert!(!actual.glyphs.is_empty());
        for (actual, expected) in actual.glyphs.iter().zip(&expected_layout.glyphs) {
            assert_eq!(actual.id, expected.id);
            assert!((actual.x - expected.x).abs() < 0.0001 && (actual.y - expected.y).abs() < 0.0001);
        }
        roundtrip(&layer);
    }
}
