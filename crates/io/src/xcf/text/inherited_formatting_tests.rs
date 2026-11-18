use super::*;

fn parse_markup(markup: &str) -> Result<(String, Vec<TextRun>)> {
    markup_runs(markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new())
}

#[test]
fn overwritten_inherited_family_and_face_copies_consume_the_formatting_allowance() {
    for (ancestor, leaf) in [("font_family", "face"), ("face", "font_family")] {
        let markup = format!("<markup><span {ancestor}='{}'>{}</span></markup>", "A".repeat(16_384), format!("<span {leaf}='Tiny'>x</span>").repeat(64));
        assert!(matches!(parse_markup(&markup), Err(crate::IoError::Unsupported(message)) if message.contains("bounded formatting allowance")));
    }
}

#[test]
fn bounded_overridden_font_copies_keep_the_final_family_and_text() {
    for (ancestor, leaf) in [("font_family", "face"), ("face", "font_family")] {
        let markup = format!("<markup><span {ancestor}='{}'><span {leaf}='Tiny'>x</span><span {leaf}='Other'>é</span></span></markup>", "A".repeat(16_384));
        let (text, runs) = parse_markup(&markup).unwrap();
        assert_eq!(text, "xé");
        assert_eq!(runs.iter().map(|r| r.len).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(runs.iter().map(|r| r.style.font_family.as_str()).collect::<Vec<_>>(), ["Tiny", "Other"]);
    }
}

#[test]
fn letter_spacing_disables_liga_and_clig_after_explicit_features_in_either_order() {
    for spacing in ["1024", "-1024", "0"] {
        for attributes in [
            format!("letter_spacing='{spacing}' font_features='liga=1,clig=1,dlig=1,kern=0'"),
            format!("font_features='liga=1,clig=1,dlig=1,kern=0' letter_spacing='{spacing}'"),
        ] {
            let (text, runs) = parse_markup(&format!("<markup><span {attributes}>ffi</span></markup>")).unwrap();
            assert_eq!(text, "ffi");
            assert_eq!(runs.len(), 1);
            let style = &runs[0].style;
            assert!(!style.ligatures);
            assert_eq!(
                style.features,
                [
                    FontFeature { tag: "liga".into(), value: 0 },
                    FontFeature { tag: "clig".into(), value: 0 },
                    FontFeature { tag: "dlig".into(), value: 1 },
                    FontFeature { tag: "kern".into(), value: 0 },
                ]
            );
            let expected = spacing.parse::<f32>().unwrap() / 1024. * 1000. / style.size_pt;
            assert!((style.tracking - expected).abs() < 0.001);
        }
    }
}

#[test]
fn nested_tracking_overrides_later_features_without_changing_unspaced_siblings() {
    let (text, runs) = parse_markup(
        "<markup><span letter_spacing='1024'><span font_features='liga=1,clig=1'>ffi</span></span><span font_features='liga=1,clig=1'>ffi</span></markup>",
    )
    .unwrap();
    assert_eq!(text, "ffiffi");
    assert_eq!(runs.len(), 2);
    assert!(!runs[0].style.ligatures);
    assert!(runs[0].style.features.iter().all(|f| f.value == 0));
    assert!(runs[1].style.ligatures);
    assert!(runs[1].style.features.iter().all(|f| f.value == 1));
    assert_eq!(runs[1].style.tracking, 0.);
    let (_, runs) = parse_markup("<markup><span letter_spacing='0'>ffi</span></markup>").unwrap();
    assert!(!runs[0].style.ligatures);
    assert!(runs[0].style.features.is_empty(), "default liga/clig suppression uses the native flag without adding feature tags");
}

#[test]
fn decoded_letter_spacing_ligature_policy_survives_native_save() {
    let layer =
        decode(br#"(markup "<markup><span letter_spacing='1024' font_features='liga=1,clig=1,dlig=1'>ffi</span>plain</markup>") (font-size 12)"#, 72., (0, 0))
            .unwrap()
            .layer;
    assert_eq!(layer.text, "ffiplain");
    assert!(!layer.runs[0].style.ligatures);
    assert!(layer.runs[1].style.ligatures);
    assert_eq!(layer.runs[0].style.features.iter().map(|f| (f.tag.as_str(), f.value)).collect::<Vec<_>>(), [("liga", 0), ("clig", 0), ("dlig", 1)]);
    let mut document =
        photocraft_doc::Document::new("Tracked markup", photocraft_geom::Size::new(64, 64), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    document.layers.push(photocraft_doc::Layer::new("Editable text", photocraft_doc::LayerContent::Text(layer.clone())));
    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    let photocraft_doc::LayerContent::Text(restored) = &reopened.layers[0].content else { panic!("editable native text") };
    assert_eq!(restored, &layer);
}

#[test]
fn font_variant_and_text_transform_resolve_independently_in_either_attribute_order() {
    for attributes in ["variant='small-caps' text_transform='none'", "text_transform='none' variant='small-caps'"] {
        let (_, runs) = parse_markup(&format!("<markup><span {attributes}>abc</span></markup>")).unwrap();
        assert_eq!(runs[0].style.caps, Caps::SmallCaps);
    }
    let (_, runs) = parse_markup(concat!(
        "<markup><span variant='small-caps'><span text_transform='none'>a</span></span>",
        "<span text_transform='uppercase'><span variant='normal'>b</span><span text_transform='none'>c</span></span></markup>"
    ))
    .unwrap();
    assert_eq!(runs.iter().map(|run| run.style.caps).collect::<Vec<_>>(), [Caps::SmallCaps, Caps::AllCaps, Caps::Normal]);
    for source in [
        "<markup><span variant='small-caps'><span text_transform='uppercase'>abc</span></span></markup>",
        "<markup><span text_transform='uppercase'><span variant='small-caps'>abc</span></span></markup>",
        "<markup><span variant='small-caps' text_transform='uppercase'>abc</span></markup>",
        "<markup><span text_transform='uppercase' variant='small-caps'>abc</span></markup>",
    ] {
        assert!(matches!(parse_markup(source), Err(crate::IoError::Unsupported(message)) if message.contains("combined small-caps and uppercase")));
    }
}

#[test]
fn opaque_foregrounds_inherit_alpha_and_explicit_span_alpha_still_wins() {
    let (_, runs) = parse_markup(concat!(
        "<markup><span alpha='25%'>",
        "<span foreground='#ff0000ff'>a</span>",
        "<span foreground='#ff000080'>b</span>",
        "<span alpha='50%' foreground='#ff000080'>c</span>",
        "<span foreground='#ff0000ff' alpha='50%'>d</span>",
        "</span></markup>",
    ))
    .unwrap();
    assert_eq!(runs.iter().map(|run| run.style.color.alpha).collect::<Vec<_>>(), [0.25, 128. / 255., 0.5, 0.5]);
    assert!(runs.iter().all(|run| run.style.color.c == Color::rgb(1., 0., 0.).c));
}
