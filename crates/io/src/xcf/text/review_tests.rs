use super::*;

#[test]
fn source_top_left_becomes_the_native_point_text_baseline_anchor() {
    fn check(layer: &TextLayer, x: f64, y: f64) {
        let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let layout = engine.layout(layer, 72.);
        let [left, top, _, _] = layout.bounds().expect("real font metrics");
        let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(left), f64::from(top)));
        assert!((origin.x - x).abs() < 0.0001 && (origin.y - y).abs() < 0.0001, "source origin ({x}, {y}) became {origin:?}");
    }
    for matrix in ["", " (transformation (matrix 1.2 0.3 -0.2 0.9))"] {
        let config = format!("(text \"Ag\") (font \"Inter\") (font-size 24){matrix}");
        check(&decode(config.as_bytes(), 72., (3, 4)).unwrap().layer, 3., 4.);
    }
    let legacy = br"GDT10{Ag}{1}{0}{0}{0}{ff2200}{0}{-*-sans-medium-r-normal-*-20-*-*-*-*-*-iso8859-1}";
    check(&decode_gdyntext(legacy, 72., (3, 4)).unwrap().layer, 3., 4.);
}

#[test]
fn explicit_alpha_wins_over_rgba_foreground_in_either_attribute_order() {
    for attrs in [
        "alpha='25%' foreground='#ff000080'",
        "foreground='#ff000080' alpha='25%'",
        "foreground_alpha='25%' color='#ff000080'",
        "color='#ff000080' foreground_alpha='25%'",
    ] {
        let markup = format!("<markup><span {attrs}>x</span></markup>");
        let (_, runs) = markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()).unwrap();
        assert_eq!(runs[0].style.color, Color::rgba(1., 0., 0., 0.25), "{attrs}");
    }
}

#[test]
fn font_description_resets_omitted_face_options_but_preserves_other_attributes() {
    let mut style = CharStyle {
        weight: 700,
        italic: true,
        caps: Caps::SmallCaps,
        horizontal_scale: 0.75,
        font_style: "Bold Italic".into(),
        postscript_name: Some("Inter-Regular".into()),
        color: Color::rgba(0.2, 0.3, 0.4, 0.25),
        tracking: 120.,
        ..Default::default()
    };
    font_description("Inter 12", &mut style, 72., true, &mut Vec::new()).unwrap();
    assert_eq!((style.weight, style.italic, style.caps, style.horizontal_scale), (400, false, Caps::Normal, 1.));
    assert!(style.font_style.is_empty());
    assert!(style.postscript_name.is_none());
    assert_eq!(style.color, Color::rgba(0.2, 0.3, 0.4, 0.25));
    assert_eq!(style.tracking, 120.);
}

#[test]
fn markup_face_overrides_clear_inherited_postscript_identity_and_resolve_native_styles() {
    let base = CharStyle { font_family: "Inter".into(), postscript_name: Some("Inter-Regular".into()), ..Default::default() };
    for (markup, weight, italic) in [
        ("<markup><b>x</b></markup>", 700, false),
        ("<markup><i>x</i></markup>", 400, true),
        ("<markup><span weight='700'>x</span></markup>", 700, false),
        ("<markup><span style='italic'>x</span></markup>", 400, true),
        ("<markup><span font_family='serif'>x</span></markup>", 400, false),
    ] {
        let (text, runs) = markup_runs(markup, &base, &HashMap::new(), 72., &mut Vec::new()).unwrap();
        assert!(runs[0].style.postscript_name.is_none(), "{markup}");
        let mut layer = TextLayer { text, runs, ..Default::default() };
        layer.sync_summary();
        let layout = photocraft_text::TextEngine::new().layout(&layer, 72.);
        assert!(!layout.glyphs.is_empty(), "layout uses a real font: {markup}");
        assert_eq!((layout.styles[0].weight, layout.styles[0].italic), (weight, italic), "{markup}");
    }
}

#[test]
fn fontconfig_weight_values_have_native_opentype_meaning() {
    for (source, expected) in
        [(0, 100), (40, 200), (50, 300), (55, 350), (75, 380), (80, 400), (100, 500), (180, 600), (200, 700), (205, 800), (210, 900), (215, 900)]
    {
        let fields = vec![Value::List(vec![Value::Atom(b"weight".to_vec()), Value::Atom(source.to_string().into_bytes())])];
        let mut style = CharStyle::default();
        requested_font(&fields, &mut style).unwrap();
        assert_eq!(style.weight, expected, "Fontconfig weight {source}");
    }
}

#[test]
fn unsupported_left_to_right_vertical_columns_do_not_replace_saved_raster() {
    for direction in ["ttb-ltr", "ttb-ltr-upright"] {
        let config = format!("(text \"first\\nsecond\") (font \"Sans\") (font-size 24) (base-direction {direction})");
        assert!(matches!(decode(config.as_bytes(), 72., (3, 4)), Err(crate::IoError::Unsupported(_))), "{direction}");
    }
}

#[test]
fn mixed_and_upright_vertical_text_keep_run_and_glyph_orientation_after_native_roundtrip() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::Size;
    use photocraft_text::layout::GlyphOrient;

    for (direction, mixed) in [("ttb-rtl", true), ("ttb-rtl-upright", false)] {
        for content in [
            r#"(text "漢A字B")"#,
            r#"(markup "<markup>漢<b>A</b><span font_family='Inter'>字B</span></markup>")"#,
            r#"(markup "<markup>漢<span font='native'>A</span>字<i>B</i></markup>" (markupfont "native" (font (family "Inter") (weight 200))))"#,
        ] {
            let config = format!("{content} (font \"Inter\") (font-size 24) (base-direction {direction})");
            let layer = decode(config.as_bytes(), 72., (20, 20)).unwrap().layer;
            assert_eq!(layer.text, "漢A字B");
            assert_eq!(layer.orientation, Orientation::Vertical);
            assert!(layer.runs.iter().all(|run| run.style.rotate_roman == mixed), "{config}");
            let mut engine = photocraft_text::TextEngine::new();
            let layout = engine.layout(&layer, 72.);
            assert!(layout.vertical);
            assert!(!layout.glyphs.is_empty());
            let mut cjk = 0;
            let mut roman = 0;
            for (glyph, range) in layout.glyphs.iter().zip(&layout.glyph_ranges) {
                match layer.text.get(range.clone()).unwrap() {
                    "漢" | "字" => {
                        cjk += 1;
                        assert_eq!(glyph.orient, GlyphOrient::Upright, "CJK stays upright: {config}");
                    }
                    "A" | "B" => {
                        roman += 1;
                        assert_ne!(glyph.id, 0, "real Latin font glyph");
                        assert_eq!(glyph.orient, if mixed { GlyphOrient::Rotated } else { GlyphOrient::Upright }, "{config}");
                    }
                    other => panic!("unexpected glyph range {other:?}"),
                }
            }
            assert_eq!((cjk, roman), (2, 2), "both scripts reached native shaping");
            let (_, pixels) = engine.render(&layer, 72., PixelFormat::RGBA8);
            assert!(!pixels.surface.content_bounds().is_empty());
            let mut document = Document::new("Vertical", Size::new(128, 128), ColorMode::Rgb, SampleType::U8);
            document.layers.push(Layer::new("Vertical", LayerContent::Text(layer.clone())));
            let saved = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Text(reopened) = &reopened.layers[0].content else { panic!("editable text on reopen") };
            assert_eq!(reopened, &layer);
            let reopened_layout = engine.layout(reopened, 72.);
            assert_eq!(reopened_layout.glyph_ranges, layout.glyph_ranges);
            assert_eq!(reopened_layout.glyphs, layout.glyphs);
            let (_, reopened_pixels) = engine.render(reopened, 72., PixelFormat::RGBA8);
            assert_eq!(reopened_pixels.rect, pixels.rect);
            assert_eq!(reopened_pixels.surface, pixels.surface);
        }
    }
}
