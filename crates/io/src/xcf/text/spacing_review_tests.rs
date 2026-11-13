use super::*;

/// Public GIMP text options define added line/letter spacing in pixels, independently of font units:
/// https://docs.gimp.org/2.10/en/gimp-tool-text.html
#[test]
fn added_line_spacing_is_a_device_distance_across_font_units_and_dpi() {
    for (dpi, pixels, positive_leading, negative_leading, tracking) in
        [(72., 18., 33.6, 15.6, 333.33334), (144., 36., 27.6, 18.6, 166.66667), (300., 75., 24.48, 20.16, 80.)]
    {
        for (unit, size) in [("pixels", pixels), ("points", 18.), ("inches", 0.25), ("millimeters", 6.35), ("centimeters", 0.635), ("picas", 1.5)] {
            for (spacing, expected) in [(12., positive_leading), (-6., negative_leading)] {
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

#[test]
fn device_line_spacing_applies_to_each_resolved_markup_size_and_survives_native_save() {
    for (dpi, large_leading, small_leading) in [(144., 49.2, 27.6), (300., 46.08, 24.48)] {
        let layer = decode(
            br#"(markup "<markup><span size='36pt'>A\nA</span><span size='18pt'>\nA</span></markup>") (font "Inter") (font-size 0.25) (font-size-unit inches) (line-spacing 12) (letter-spacing 6)"#,
            dpi,
            (0, 0),
        )
        .unwrap()
        .layer;
        assert_eq!(layer.text, "A\nA\nA");
        assert_eq!(layer.runs.len(), 2);
        for (run, size, expected) in [(&layer.runs[0], 36., large_leading), (&layer.runs[1], 18., small_leading)] {
            assert_eq!(run.style.size_pt, size);
            assert!((run.style.leading_pt.unwrap() - expected).abs() < 0.00001);
        }
        let mut document = photocraft_doc::Document::new(
            "Device spacing",
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
    }
}

#[test]
fn modern_point_size_and_legacy_pixel_font_agree_on_added_device_line_spacing() {
    let modern = decode(br#"(text "A\nA") (font "Inter") (font-size 10) (font-size-unit points) (line-spacing 2)"#, 144., (0, 0)).unwrap().layer;
    let legacy =
        decode_gdyntext(br"GDT10{A\nA}{1}{0}{0}{2}{ffffff}{0}{-misc-fixed-medium-r-normal--20-120-75-75-p-100-iso8859-1}", 144., (0, 0)).unwrap().layer;
    assert_eq!(modern.size_pt, 10.);
    assert_eq!(legacy.size_pt, 10.);
    assert_eq!(modern.runs[0].style.leading_pt, Some(13.));
    assert_eq!(modern.runs[0].style.leading_pt, legacy.runs[0].style.leading_pt);
}
