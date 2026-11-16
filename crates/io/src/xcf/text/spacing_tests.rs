use super::*;

/// Public GIMP3 API observation (installed3.2.4): line spacing follows the font
/// unit, while letter spacing stays in pixels. The public manual's pixel wording
/// does not describe the observed physical-font-unit case.
#[test]
fn added_line_spacing_uses_font_units_while_tracking_stays_in_pixels() {
    for (dpi, pixels, tracking) in [(72., 18., 333.33334), (144., 36., 166.66667), (300., 75., 80.)] {
        for (unit, size, six_points) in [
            ("pixels", pixels, 6. * f64::from(dpi) / 72.),
            ("points", 18., 6.),
            ("inches", 0.25, 6. / 72.),
            ("millimeters", 6.35, 6. * 25.4 / 72.),
            ("centimeters", 0.635, 6. * 2.54 / 72.),
            ("picas", 1.5, 0.5),
        ] {
            for (spacing, expected) in [(six_points, 27.6), (-six_points / 2., 18.6)] {
                let config =
                    format!("(text \"A\\nA\") (font \"Inter\") (font-size {size}) (font-size-unit {unit}) (line-spacing {spacing}) (letter-spacing 6)");
                let layer = decode(config.as_bytes(), dpi, (0, 0)).unwrap().layer;
                let style = &layer.runs[0].style;
                assert!((style.size_pt - 18.).abs() < 0.00001, "{unit} at {dpi} DPI");
                assert!((style.leading_pt.unwrap() - expected).abs() < 0.00001, "{unit} at {dpi} DPI: {:?}", style.leading_pt);
                assert!((style.tracking - tracking).abs() < 0.0001, "line spacing must not change device-pixel tracking");
            }
        }
    }
}

fn glyph_y(layout: &photocraft_text::layout::TextLayout, cluster: std::ops::Range<usize>) -> f32 {
    let glyphs: Vec<_> = layout.glyphs.iter().zip(&layout.glyph_ranges).filter(|(_, source)| **source == cluster).map(|(glyph, _)| glyph).collect();
    assert_eq!(glyphs.len(), 1, "one actual Latin glyph for {cluster:?}");
    glyphs[0].y
}

#[test]
fn physical_line_spacing_adds_six_native_points_at_144_dpi_instead_of_three() {
    let layer =
        decode(br#"(text "A\nA") (font "Inter") (font-size 18) (font-size-unit points) (line-spacing 6) (letter-spacing 6)"#, 144., (0, 0)).unwrap().layer;
    let mut expected = layer.clone();
    expected.runs[0].style.leading_pt = Some(27.6); // 18pt *1.2 plus6 source points.
    let mut old_pixels = layer.clone();
    old_pixels.runs[0].style.leading_pt = Some(24.6); // The incorrect6px at144DPI added only3pt.
    let mut engine = photocraft_text::TextEngine::new();
    let actual = engine.layout(&layer, 144.);
    let oracle = engine.layout(&expected, 144.);
    let old = engine.layout(&old_pixels, 144.);
    let gap = |layout: &photocraft_text::layout::TextLayout| glyph_y(layout, 2..3) - glyph_y(layout, 0..1);
    assert!((gap(&actual) - gap(&oracle)).abs() < 0.0001, "rendered line baselines use physical spacing");
    assert!((gap(&actual) - gap(&old) - 6.).abs() < 0.001, "three native points become six device pixels at144DPI");
    assert_eq!(layer.runs[0].style.leading_pt, Some(27.6));
    assert!((layer.runs[0].style.tracking - 166.66667).abs() < 0.0001, "letter spacing still means six device pixels");
}

#[test]
fn physical_line_spacing_applies_to_each_markup_size_and_survives_native_save() {
    for dpi in [144., 300.] {
        let layer = decode(
            br#"(markup "<markup><span size='36pt'>A\nA</span><span size='18pt'>\nA</span></markup>") (font "Inter") (font-size 0.25) (font-size-unit inches) (line-spacing 0.08333333333333333) (letter-spacing 6)"#,
            dpi,
            (0, 0),
        )
        .unwrap()
        .layer;
        assert_eq!(layer.text, "A\nA\nA");
        assert_eq!(layer.runs.len(), 2);
        for (run, size, expected) in [(&layer.runs[0], 36., 49.2), (&layer.runs[1], 18., 27.6)] {
            assert_eq!(run.style.size_pt, size);
            assert!((run.style.leading_pt.unwrap() - expected).abs() < 0.00001);
        }
        let mut document = photocraft_doc::Document::new(
            "Physical spacing",
            photocraft_geom::Size::new(256, 256),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        );
        document.resolution_dpi = dpi;
        document.layers.push(photocraft_doc::Layer::new("Editable text", photocraft_doc::LayerContent::Text(layer.clone())));
        let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.resolution_dpi, dpi);
        let photocraft_doc::LayerContent::Text(restored) = &reopened.layers[0].content else { panic!("editable native text") };
        assert_eq!(restored, &layer);
        let mut engine = photocraft_text::TextEngine::new();
        assert_eq!(engine.layout(restored, dpi).glyphs, engine.layout(&layer, dpi).glyphs);
    }
}

#[test]
fn modern_pixel_size_and_legacy_font_keep_device_spacing_but_modern_points_use_physical_spacing() {
    let modern = decode(br#"(text "A\nA") (font "Inter") (font-size 10) (font-size-unit points) (line-spacing 2)"#, 144., (0, 0)).unwrap().layer;
    let pixels = decode(br#"(text "A\nA") (font "Inter") (font-size 20) (font-size-unit pixels) (line-spacing 2)"#, 144., (0, 0)).unwrap().layer;
    let legacy =
        decode_gdyntext(br"GDT10{A\nA}{1}{0}{0}{2}{ffffff}{0}{-misc-fixed-medium-r-normal--20-120-75-75-p-100-iso8859-1}", 144., (0, 0)).unwrap().layer;
    assert_eq!(modern.size_pt, 10.);
    assert_eq!(pixels.size_pt, 10.);
    assert_eq!(legacy.size_pt, 10.);
    assert_eq!(modern.runs[0].style.leading_pt, Some(14.));
    assert_eq!(pixels.runs[0].style.leading_pt, Some(13.));
    assert_eq!(pixels.runs[0].style.leading_pt, legacy.runs[0].style.leading_pt);
}
