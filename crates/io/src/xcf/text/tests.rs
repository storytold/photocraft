use super::*;

fn assert_source_origin(layer: &TextLayer, source: Affine) {
    assert_eq!(&layer.transform.m[..4], &source.m[..4]);
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let [left, top, _, _] = engine.layout(layer, 72.).bounds().unwrap();
    let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(left), f64::from(top)));
    assert!((origin.x - source.m[4]).abs() < 0.0001 && (origin.y - source.m[5]).abs() < 0.0001, "{origin:?} versus {source:?}");
}
/// Verify independently observed Pango color-name facts, including CSS overrides and numbered X11 variants, without broadening the accepted syntax.
#[test]
fn pango_named_markup_colors_use_css_precedence_and_numbered_x11_facts() {
    for (name, expected) in [
        ("green", [0u8, 128, 0]),
        ("gray", [128, 128, 128]),
        ("grey", [128, 128, 128]),
        ("maroon", [128, 0, 0]),
        ("purple", [128, 0, 128]),
        ("LightGoldenrodYellow", [250, 250, 210]),
        (" lIgHt  GoLdEnRoD yElLoW", [250, 250, 210]),
        ("gray42", [107, 107, 107]),
        ("grey42", [107, 107, 107]),
        ("gray50", [127, 127, 127]),
        ("steel blue4", [54, 100, 139]),
        ("RebeccaPurple", [102, 51, 153]),
        ("aqua", [0, 255, 255]),
        ("crimson", [220, 20, 60]),
        ("fuchsia", [255, 0, 255]),
        ("indigo", [75, 0, 130]),
        ("lime", [0, 255, 0]),
        ("olive", [128, 128, 0]),
        ("silver", [192, 192, 192]),
        ("teal", [0, 128, 128]),
    ] {
        let [r, g, b] = expected.map(|sample| f32::from(sample) / 255.);
        assert_eq!(markup_color(name).unwrap(), Color::rgb(r, g, b), "{name}");
    }
    for name in [
        "",
        " ",
        "gray ",
        "Light-GoldenrodYellow",
        "light_goldenrod_yellow",
        "gray01",
        "webgray",
        "x11gray",
        "debianred",
        "transparent",
        "rgb(255,0,0)",
        "currentColor",
        "not-a-pango-color",
        "réd",
        "\0red",
        "r\ted",
        "r\ned",
        "file:///red",
    ] {
        assert!(matches!(markup_color(name), Err(crate::IoError::Unsupported(_))), "{name:?}");
    }
    assert!(markup_color(&"r".repeat(4096)).is_err());
    for value in ["#ééé", "#🙂🙂", "#12345g"] {
        assert!(markup_color(value).is_err());
    }
    for (value, expected) in [
        ("#f00", Color::rgb(1., 0., 0.)),
        ("#00ff00", Color::rgb(0., 1., 0.)),
        ("#ffff00000000", Color::rgb(1., 0., 0.)),
        ("#ff000080", Color::rgba(1., 0., 0., 128. / 255.)),
    ] {
        assert_eq!(markup_color(value).unwrap(), expected);
    }
}

/// Check named-color aliases retain alpha and that XML namespaces cannot silently change the admitted formatting meaning.
#[test]
fn named_markup_aliases_preserve_alpha_and_reject_namespaced_reinterpretation() {
    let base = CharStyle { color: Color::rgba(0.2, 0.3, 0.4, 0.3), ..Default::default() };
    let mut notes = Vec::new();
    let (text,runs) = markup_runs("<markup><span color='green'>A</span><span fgcolor='gray'>B</span><span foreground_alpha='25%' foreground='slategray' data-foreground='red'>C</span><span foreground='#0000ff80'>D</span></markup>", &base, &HashMap::new(), 72., &mut notes).unwrap();
    assert_eq!(text, "ABCD");
    assert_eq!(runs[0].style.color, Color::rgba(0., 128. / 255., 0., 0.3));
    assert_eq!(runs[1].style.color, Color::rgba(128. / 255., 128. / 255., 128. / 255., 0.3));
    assert_eq!(runs[2].style.color, Color::rgba(112. / 255., 128. / 255., 144. / 255., 0.25));
    assert_eq!(runs[3].style.color, Color::rgba(0., 0., 1., 128. / 255.));
    assert!(notes.iter().any(|note| note.contains("data-foreground")));
    // XML normalizes a literal tab to an ordinary space; a character
    // reference preserves the tab and is not a Pango color-name space.
    assert!(markup_runs("<markup><span foreground='light\tblue'>X</span></markup>", &base, &HashMap::new(), 72., &mut Vec::new()).is_ok());
    for source in [
        "<markup><span foreground='light&#9;blue'>X</span></markup>",
        "<markup xmlns:x='urn:untrusted'><span x:foreground='red'>X</span></markup>",
        "<markup xmlns='urn:untrusted'><span foreground='red'>X</span></markup>",
        "<markup><span foreground='unknown'>X</span></markup>",
    ] {
        assert!(markup_runs(source, &base, &HashMap::new(), 72., &mut Vec::new()).is_err(), "{source}");
    }
}

/// Compare cached text-color conversion with an independent uncached CMS transform and native rendering after persistence at each depth.
#[test]
fn named_markup_native_cms_matches_uncached_transform_and_render_reopen_each_depth() {
    use craft_cms::{Builtin, Intent, Profile, Transform};
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    let config=br#"(markup "<markup><span foreground=\"green\" foreground_alpha=\"50%\">MMMM</span><span fgcolor=\"gray\">X</span><span color=\"steelblue4\">Y</span></markup>") (font "Sans") (font-size 20)"#;
    let original = decode(config, 72., (0, 0)).unwrap().layer;
    let literals = [[0., 128. / 255., 0., 0.5], [128. / 255., 128. / 255., 128. / 255., 1.], [54. / 255., 100. / 255., 139. / 255., 1.]];
    let mut engine = photocraft_text::TextEngine::new();
    for (profile, mode) in [(Builtin::DisplayP3, ColorMode::Rgb), (Builtin::SGray, ColorMode::Grayscale)] {
        let bytes = profile.profile().to_bytes();
        let destination = Profile::parse(&bytes).unwrap();
        let independent = Transform::new(Builtin::Srgb.profile(), &destination, Intent::RelativeColorimetric, false).unwrap();
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("Imported named text", photocraft_geom::Size::new(192, 80), mode, depth);
            doc.icc_profile = Some(bytes.clone());
            doc.layers.push(Layer::new("Editable named colors", LayerContent::Text(original.clone())));
            convert_colors(&mut doc, &bytes).unwrap();
            let mut expected = original.clone();
            for (run, literal) in expected.runs.iter_mut().zip(literals) {
                let mut converted = [0.; 4];
                let channels = mode.color_channels();
                independent.convert_f32(&literal[..3], 3, &mut converted[..channels], channels, false).unwrap();
                run.style.color = Color { mode, c: converted, alpha: literal[3] };
            }
            expected.sync_summary();
            if let Some([x, y, _, _]) = engine.layout(&expected, 72.).bounds() {
                expected.transform = Affine::translate(3. - f64::from(x), 3. - f64::from(y));
            }
            let format = doc.pixel_format();
            let LayerContent::Text(actual) = &mut doc.layers[0].content else { panic!("native editable text") };
            actual.transform = expected.transform;
            assert_eq!(actual.runs, expected.runs);
            engine.render_layer(actual, 72., format);
            engine.render_layer(&mut expected, 72., format);
            assert_eq!(actual.cache, expected.cache);
            let pixels = photocraft_compose::render(&doc, doc.bounds()).into_px();
            assert!(pixels.iter().any(|pixel| pixel[3] > 0.), "independent native text must render visible glyphs");
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let mut reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Text(text) = &mut reopened.layers[0].content else { panic!("persisted native text") };
            assert_eq!(text.runs, expected.runs);
            engine.render_layer(text, 72., format);
            assert_eq!(pixels, photocraft_compose::render(&reopened, reopened.bounds()).px);
        }
    }
}

/// Verify Unicode text, unit conversion and editable native box/style data can be prepared through the native text engine.
#[test]
fn unicode_units_styles_box_and_native_cache() {
    let config = br#"(markup "<markup>h&#233;<b>&#955;</b>\n<span style=\"italic\">&#28450;&#23383;</span></markup>")
            (font "Missing Font") (font-size 24) (font-size-unit pixels) (box-mode fixed) (box-width 120) (box-height 60)
            (color (color-rgba 0.25 0.5 0.75 1)) (letter-spacing 2) (base-direction rtl)"#;
    let t = decode(config, 96.0, (-3, 7)).expect("text").layer;
    assert_eq!(t.text, "héλ\n漢字");
    assert_eq!(t.size_pt, 18.0);
    assert!(t.cache.is_none());
    assert_eq!(t.transform.m, [1.0, 0.0, 0.0, 1.0, -3.0, 7.0]);
    assert_eq!(t.runs[1].style.weight, 700);
    assert!(t.runs.last().expect("run").style.italic);
    assert_eq!(t.runs.iter().map(|r| r.len).sum::<usize>(), t.text.len());
    let mut at = 0;
    for r in &t.runs {
        at += r.len;
        assert!(t.text.is_char_boundary(at));
    }
    assert_eq!(t.shape, TextShape::Box { x: 0.0, y: 0.0, width: 120.0, height: 60.0 });
}
/// Exercise malformed, deeply nested and expression-like text configuration as inert input that must fail safely.
#[test]
fn hostile_text_is_rejected_without_evaluation() {
    for config in [
        b"(text \"abc\") (font-size nan)".as_slice(),
        b"(text \"\\777\")",
        b"(text \"abc\") (font-size 24) (font-size 12)",
        b"(markup \"<!DOCTYPE markup [<!ENTITY x SYSTEM 'file:///private'>]><markup>&x;</markup>\")",
        b"(text \"abc\"",
        b"(text \"\xff\")",
    ] {
        assert!(decode(config, 72.0, (0, 0)).is_err(), "{config:?}");
    }
    let deep = format!("{}0{}", "(".repeat(40), ")".repeat(40));
    assert!(decode(deep.as_bytes(), 72.0, (0, 0)).is_err());
}
/// Check font requests, text placement and stored color bytes translate to native data without treating source resource identities as file access.
#[test]
fn markup_font_resources_matrix_and_byte_color_are_data_only() {
    let config = br##"(markup "<markup><span font=\"font7\" size=\"18432\" foreground=\"#c04020\">h&#233;</span></markup>"
            (markupfont "font7" (font (fullname "Requested Face") (family "Requested Family") (psname "RequestedFace"))))
            (font "Parent") (color (color "R'G'B'A u8" 4 "\100\200\377\377" 0))
            (transformation (matrix 2 0.5 -0.25 1))"##;
    let parsed = decode(config, 72.0, (3, 4)).expect("markup font");
    assert_eq!(parsed.layer.runs[0].style.font_family, "Requested Face");
    assert_eq!(parsed.layer.runs[0].style.postscript_name.as_deref(), Some("RequestedFace"));
    assert_eq!(parsed.layer.runs[0].style.size_pt, 18.0);
    assert_eq!(parsed.layer.runs[0].style.color, Color::rgb(192.0 / 255.0, 64.0 / 255.0, 32.0 / 255.0));
    assert_source_origin(&parsed.layer, Affine { m: [2.0, -0.25, 0.5, 1.0, 3.0, 4.0] });
    assert_eq!(parsed.layer.text, "hé");
    assert!(parsed.layer.cache.is_none());
    let base = decode(br#"(text "x") (color (color "R'G'B'A u8" 4 "\100\200\377\377" 0))"#, 72.0, (0, 0)).expect("byte color");
    assert_eq!(base.layer.color, Color::rgb(64.0 / 255.0, 128.0 / 255.0, 1.0));
}

/// Verify markup spacing, baselines, feature settings and sizes retain correct UTF-8 run boundaries in native text.
#[test]
fn native_markup_baseline_spacing_features_and_sizes_preserve_utf8_runs() {
    let parsed=decode(br##"(markup "<markup><span size=\"20pt\" rise=\"3pt\" letter_spacing=\"2048\" variant=\"small-caps\" stretch=\"condensed\" font_features=\"liga=0, dlig on, -kern\" foreground=\"#c0402080\">h&#233;<span size=\"200%\">&#955;</span></span><sub>x</sub><sup>y</sup></markup>") (font-size 20) (line-spacing 4)"##,144.,(0,0)).unwrap();
    assert_eq!(parsed.layer.text, "héλxy");
    let first = &parsed.layer.runs[0].style;
    assert_eq!(first.size_pt, 20.);
    assert_eq!(first.baseline_shift_pt, 1.5);
    assert_eq!(first.tracking, 50.);
    assert_eq!(first.leading_pt, Some(26.));
    assert_eq!(first.caps, Caps::SmallCaps);
    assert_eq!(first.horizontal_scale, 0.75);
    assert_eq!(first.color.alpha, 128. / 255.);
    assert_eq!(
        first.features,
        vec![FontFeature { tag: "liga".into(), value: 0 }, FontFeature { tag: "dlig".into(), value: 1 }, FontFeature { tag: "kern".into(), value: 0 }]
    );
    assert_eq!(parsed.layer.runs[1].style.size_pt, 40.);
    assert_eq!(parsed.layer.runs[1].style.tracking, 25.);
    assert_eq!(parsed.layer.runs[2].style.baseline, FontBaseline::Subscript);
    assert_eq!(parsed.layer.runs[3].style.baseline, FontBaseline::Superscript);
    assert_eq!(parsed.layer.runs.iter().map(|run| run.len).sum::<usize>(), parsed.layer.text.len());
    for run in &parsed.layer.runs {
        assert!(run.style.size_pt.is_finite());
    }
}
/// Device-distance markup uses pixel-to-point conversion while explicit and scaled font sizes keep point semantics, layout, and native persistence.
#[test]
fn non72_dpi_markup_distances_match_independent_native_text_and_reopen() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::Size;
    for (dpi, positive_rise, negative_rise, markup_tracking, inherited_tracking) in [(144., 1.5, -0.5, 50., 150.), (300., 0.72, -0.24, 24., 72.)] {
        for rise in ["3072", "3pt"] {
            let config = format!(
                r#"(markup "<markup><span size=\"20pt\" rise=\"{rise}\" letter_spacing=\"2048\">AA</span><span size=\"20480\" rise=\"-1024\">BB</span></markup>") (font "Sans") (font-size 24) (font-size-unit points) (letter-spacing 6)"#
            );
            let mut actual = decode(config.as_bytes(), dpi, (0, 0)).unwrap().layer;
            assert_eq!(actual.text, "AABB");
            assert_eq!(actual.runs.len(), 2);
            let mut expected = actual.clone();
            for (index, run) in expected.runs.iter_mut().enumerate() {
                run.style.size_pt = 20.;
                run.style.baseline_shift_pt = if index == 0 { positive_rise } else { negative_rise };
                run.style.tracking = if index == 0 { markup_tracking } else { inherited_tracking };
            }
            expected.sync_summary();
            for (actual, expected) in actual.runs.iter().zip(&expected.runs) {
                assert_eq!(actual.style.size_pt, expected.style.size_pt, "font sizes are points rather than device distances");
                assert!((actual.style.baseline_shift_pt - expected.style.baseline_shift_pt).abs() < 1e-6);
                assert!((actual.style.tracking - expected.style.tracking).abs() < 1e-5);
            }
            let mut engine = photocraft_text::TextEngine::new();
            let layout = engine.layout(&actual, dpi);
            let oracle = engine.layout(&expected, dpi);
            assert!(!oracle.glyphs.is_empty(), "independent native text must produce real glyphs");
            assert_eq!(layout.glyphs.len(), oracle.glyphs.len());
            for (actual, expected) in layout.glyphs.iter().zip(&oracle.glyphs) {
                assert_eq!((actual.id, actual.style), (expected.id, expected.style));
                assert!((actual.x - expected.x).abs() < 1e-4 && (actual.y - expected.y).abs() < 1e-4);
            }
            if dpi == 144. {
                // These half-point distances and tracking values are exactly
                // representable, allowing an exact independent raster oracle.
                let format = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
                engine.render_layer(&mut actual, dpi, format);
                engine.render_layer(&mut expected, dpi, format);
                assert_eq!(actual.cache, expected.cache, "independently authored native point distances render identically");
            }
            let mut doc = Document::new("Device-distance native text", Size::new(384, 160), ColorMode::Rgb, SampleType::U8);
            doc.resolution_dpi = dpi;
            doc.layers.push(Layer::new("Editable native text", LayerContent::Text(actual.clone())));
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Text(reopened) = &reopened.layers[0].content else { panic!("native editable text") };
            assert_eq!(reopened.runs, actual.runs);
        }
    }
}

/// Base letter spacing remains a device-pixel distance even when font sizes use points or another physical unit.
#[test]
fn base_letter_spacing_is_independent_of_font_size_unit_at_each_dpi() {
    for (dpi, tracking) in [(72., 300.), (144., 150.), (300., 72.)] {
        for (unit, size) in [("points", 20.), ("pixels", 20. * f64::from(dpi) / 72.), ("inches", 20. / 72.)] {
            let config = format!("(text \"AB\") (font-size {size}) (font-size-unit {unit}) (letter-spacing 6)");
            let text = decode(config.as_bytes(), dpi, (0, 0)).unwrap().layer;
            assert!((text.runs[0].style.size_pt - 20.).abs() < 1e-5);
            assert!((text.runs[0].style.tracking - tracking).abs() < 1e-4, "{dpi}DPI/{unit} must retain six device pixels of spacing");
        }
    }
    for dpi in [0., f64::NAN, f64::INFINITY] {
        assert!(device_points(2., dpi).is_err());
    }
    assert!(device_points(f64::MAX, 72.).is_err());
}

/// Compare portable typed/profiled colors with the shared native color conversion, including grayscale document output.
#[test]
fn typed_text_profile_and_gray_colors_use_shared_native_cms() {
    let profile = craft_cms::Builtin::DisplayP3.profile().to_bytes();
    let escaped: String = profile.iter().map(|byte| format!("\\{byte:03o}")).collect();
    let config = format!("(text \"paint\") (color (color \"R'G'B'A u8\" 4 \"\\377\\0\\0\\200\" {} \"{}\"))", profile.len(), escaped);
    let parsed = decode(config.as_bytes(), 72., (0, 0)).unwrap();
    let transform =
        craft_cms::Transform::new(craft_cms::Builtin::DisplayP3.profile(), craft_cms::Builtin::Srgb.profile(), craft_cms::Intent::RelativeColorimetric, false)
            .unwrap();
    let mut expected = [0.; 3];
    transform.convert_f32(&[1., 0., 0.], 3, &mut expected, 3, false).unwrap();
    for (actual, expected) in parsed.layer.color.c.iter().zip(expected) {
        assert!((actual - expected).abs() < 0.00001);
    }
    assert_eq!(parsed.layer.color.alpha, 128. / 255.);
    let gray = decode(br#"(text "gray") (color (color "Y'A u8" 2 "\200\100" 0))"#, 72., (0, 0)).unwrap();
    assert!((gray.layer.color.c[0] - gray.layer.color.c[1]).abs() < 0.00001);
    assert_eq!(gray.layer.color.alpha, 64. / 255.);
}
/// Verify legacy XLFD requests and Unicode become native editable typography that survives native save and reopen.
#[test]
fn historical_gdyntext_maps_xlfd_unicode_native_style_and_reopens() {
    let config = br"GDT10{h\303\251\316\273}{1}{1}{0}{2}{c04020}{0}{-misc-fixed-bold-i-normal--20-120-75-75-p-100-iso8859-1}";
    let parsed = decode_gdyntext(config, 144., (-3, 7)).unwrap();
    let text = &parsed.layer;
    assert_eq!(text.text, "héλ");
    assert_eq!(text.font_family, "fixed");
    assert_eq!(text.size_pt, 10.);
    assert_eq!(text.runs[0].style.weight, 700);
    assert!(text.runs[0].style.italic);
    assert_eq!(text.runs[0].style.leading_pt, Some(13.));
    assert_eq!(text.paragraphs[0].style.align, TextAlign::Center);
    assert!(text.cache.is_none());
    let mut doc =
        photocraft_doc::Document::new("Legacy", photocraft_geom::Size::new(32, 24), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    doc.layers.push(photocraft_doc::Layer::new("Editable legacy", photocraft_doc::LayerContent::Text(parsed.layer)));
    let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().layers, doc.layers);
    assert_eq!(
        decode_gdyntext(br"GDT10{\351}{1}{0}{0}{0}{ffffff}{0}{-misc-fixed-medium-r-normal--12-120-75-75-p-100-iso8859-1}", 72., (0, 0)).unwrap().layer.text,
        "é"
    );
}
/// Ensure malformed or unsupported typography is reported to the caller so source raster fallback remains available.
#[test]
fn invalid_extended_markup_and_legacy_fields_leave_fallback_to_caller() {
    for config in [
        br#"(markup "<markup><span rise=\"nan\">x</span></markup>")"#.as_slice(),
        br#"(markup "<markup><span size=\"0%\">x</span></markup>")"#,
        br#"(markup "<markup><span font_features=\"badtag=1\">x</span></markup>")"#,
    ] {
        assert!(decode(config, 72., (0, 0)).is_err());
    }
    for data in [b"GDT10{missing}".as_slice(), b"GDT10{x}{1}{0}{0}{0}{ffffff}{0}{not-xlfd}"] {
        assert!(decode_gdyntext(data, 72., (0, 0)).is_err());
    }
    assert!(matches!(
        decode_gdyntext(br"GDT10{\351}{1}{0}{0}{0}{ffffff}{0}{-misc-fixed-medium-r-normal--12-120-75-75-p-100-iso8859-7}", 72., (0, 0)),
        Err(crate::IoError::Unsupported(_))
    ));
    assert!(matches!(
        decode_gdyntext(b"GDT10{x}{1}{0}{NaN}{0}{ffffff}{0}{-misc-fixed-medium-r-normal--12-120-75-75-p-100-iso8859-1}", 72., (0, 0)),
        Err(crate::IoError::Xcf(_))
    ));
    assert!(matches!(
        decode(br#"(markup "<markup><span foreground=\"photocraft-unknown-color\">x</span></markup>")"#, 72., (0, 0)),
        Err(crate::IoError::Unsupported(_))
    ));
}
/// Pin independently observed placement and rotation rules without adding inactive source offsets twice.
#[test]
fn observed_stock_text_placement_keeps_inactive_offsets_and_legacy_rotation() {
    for (x, y) in [(3.25, -2.5), (20., 10.)] {
        let source = format!("(text \"Abg\") (font-size 20) (transformation (matrix 1.2 0.3 -0.2 0.9)) (offset-x {x}) (offset-y {y})");
        let parsed = decode(source.as_bytes(), 72., (13, 17)).unwrap();
        assert_source_origin(&parsed.layer, Affine { m: [1.2, -0.2, 0.3, 0.9, 13., 17.] });
        assert!(parsed.native_policy.iter().any(|note| note.contains("inactive metadata")));
    }
    for rotation in ["0", "1", "45", "90", "-30", "360"] {
        let source = format!("GDT10{{Abg}}{{1}}{{0}}{{{rotation}}}{{0}}{{ff2200}}{{0}}{{-*-sans-medium-r-normal-*-20-*-*-*-*-*-iso8859-1}}");
        let parsed = decode_gdyntext(source.as_bytes(), 72., (13, 17)).unwrap();
        assert_source_origin(&parsed.layer, Affine::translate(13., 17.));
        assert_eq!(parsed.layer.text, "Abg");
        if rotation != "0" {
            assert!(parsed.native_policy.iter().any(|note| note.contains("rotation control") && note.contains("inactive")));
        }
    }
    for source in [br#"(text "x") (offset-x NaN)"#.as_slice(), br#"(text "x") (offset-y inf)"#, br#"(text "x") (offset-x 1) (offset-x 2)"#] {
        assert!(decode(source, 72., (0, 0)).is_err());
    }
}
/// Reject inherited-style expansion that would exceed the bounded materialized native text allowance.
#[test]
fn inherited_markup_styles_cannot_amplify_native_run_allocations() {
    let markup = format!("<markup><span font_family=\"{}\">{}</span></markup>", "A".repeat(40_000), "<b>x</b>".repeat(1000));
    assert!(matches!(markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()), Err(crate::IoError::Unsupported(_))));
    let resources: String = (0..100).map(|index| format!(" (markupfont \"f{index}\" (font (family \"Requested\")))")).collect();
    let config = format!("(markup \"<markup>x</markup>\"{resources}) (language \"{}\")", "a".repeat(40_000));
    assert!(matches!(decode(config.as_bytes(), 72., (0, 0)), Err(crate::IoError::Unsupported(_))));
}

/// Check historical font strings and markup font descriptions preserve requested native family, style and supported feature settings.
#[test]
fn historical_font_descriptions_and_markup_font_requests_keep_native_styles() {
    let parsed =
        decode(br##"(text "Legacy face") (font "Requested Family Bold Italic 24 @wght=650 #ss01=1") (font-size 12) (font-size-unit points)"##, 144., (0, 0))
            .unwrap();
    let style = &parsed.layer.runs[0].style;
    assert_eq!(style.font_family, "Requested Family");
    assert_eq!(style.font_style, "Bold Italic");
    assert_eq!(style.weight, 700);
    assert!(style.italic);
    assert_eq!(style.size_pt, 12.);
    assert_eq!(style.variations, vec![FontVariation { axis: "wght".into(), value: 650. }]);
    assert_eq!(style.features, vec![FontFeature { tag: "ss01".into(), value: 1 }]);
    let parsed = decode(br#"(markup "<markup><span font_desc=\"Requested Family Regular Italic 20px\">x</span></markup>")"#, 144., (0, 0)).unwrap();
    let style = &parsed.layer.runs[0].style;
    assert_eq!(style.font_family, "Requested Family");
    assert_eq!(style.size_pt, 10.);
    assert!(style.italic);
    assert_eq!(style.weight, 400);
    let parsed = decode(
        br#"(markup "<markup><span font=\"r\">x</span></markup>" (markupfont "r" (font (fullname "Requested Bold") (weight 200) (slant 100))))"#,
        72.,
        (0, 0),
    )
    .unwrap();
    assert_eq!(parsed.layer.runs[0].style.weight, 700);
    assert!(parsed.layer.runs[0].style.italic);
}
