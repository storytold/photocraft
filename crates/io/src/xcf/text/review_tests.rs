use super::*;

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
    for (source, expected) in [(0, 100), (40, 200), (50, 300), (55, 350), (75, 380), (80, 400), (100, 500), (180, 600), (200, 700), (205, 800), (210, 900), (215, 900)] {
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
