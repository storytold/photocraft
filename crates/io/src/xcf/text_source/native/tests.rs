use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::Layer;

// Original source configurations and saved pixels; no external fonts/assets.
fn source(raw: &str) -> TextSource<'_> {
    let mut scan = craft_xcf::records::text::gimp(raw.as_bytes(), 0, Default::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(scan.stopped.is_none());
    TextSource::Configuration(scan.records.remove(0))
}

fn saved(format: PixelFormat) -> Layer {
    let mut layer = Layer::raster("Original saved name", format);
    let LayerContent::Raster(surface) = &mut layer.content else { panic!("raster") };
    surface.write_pixel(5, 6, &vec![0.25; format.channels()]);
    layer
}

#[test]
fn fixed_text_keeps_authoritative_pixels_and_layer_name_at_every_depth() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut layer = saved(PixelFormat::new(ColorMode::Rgb, sample, true));
        let LayerContent::Raster(surface) = &layer.content else { panic!("raster") };
        let pixels = surface.pixel(5, 6);
        let generation = surface.generation();
        let source = source("(text \"אב 123, text\") (font-size 12) (font-size-unit points) (box-mode fixed) (box-width 40) (box-height 20)");
        let mapped = map_layer(
            &mut layer,
            &source,
            NativeOptions { resolution: (72., 72.), offset: (5, 6), colors: Some(&colors) },
            &mut NativeBudget::default(),
            &mut Budget::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert!(matches!(mapped, Mapping::Converted { .. }));
        assert_eq!(layer.name, "Original saved name");
        let LayerContent::Text(text) = &layer.content else { panic!("editable") };
        assert!(text.keep_layer_name && text.cache_uses_raster_blending);
        assert_eq!(text.cache.as_ref().unwrap().generation(), generation);
        assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), pixels);
        assert_eq!(text.paragraphs[0].style.direction, TextDirection::Auto);
        assert_eq!(text.transform.m, [1., 0., 0., 1., 5., 6.]);
    }
}

#[test]
fn explicit_and_automatic_paragraph_directions_survive_native_mapping() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for (fields, direction) in [
        ("", TextDirection::Auto),
        ("(base-direction ltr)", TextDirection::Ltr),
        ("(base-direction rtl)", TextDirection::Rtl),
        ("(base-direction rtl) (base-dir-auto yes)", TextDirection::Auto),
    ] {
        let raw = format!("(text \"אב 123, text\") (box-mode fixed) {fields}");
        let source = source(&raw);
        let mut layer = saved(PixelFormat::RGBA8);
        map_layer(
            &mut layer,
            &source,
            NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
            &mut NativeBudget::default(),
            &mut Budget::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let LayerContent::Text(text) = &layer.content else { panic!("editable") };
        assert_eq!(text.paragraphs[0].style.direction, direction);
    }
}

#[test]
fn source_units_affine_order_and_inactive_offsets_have_independent_native_meaning() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    let source = source(
        "(text \"text\") (font-size 12) (font-size-unit points) (box-mode fixed) (box-width 40) (box-height 20) (transformation (matrix 1 0.25 0.5 1)) (offset-x 111) (offset-y -222)",
    );
    let mut layer = saved(PixelFormat::RGBA8);
    let mapped = map_layer(
        &mut layer,
        &source,
        NativeOptions { resolution: (144., 72.), offset: (7, 9), colors: Some(&colors) },
        &mut NativeBudget::default(),
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let Mapping::Converted { notes } = mapped else { panic!("editable") };
    assert!(notes.iter().any(|note| note.contains("inactive")));
    let LayerContent::Text(text) = &layer.content else { panic!("editable") };
    assert_eq!(text.transform.m, [2., 1., 0.25, 1., 7., 9.]);
    assert_eq!(text.runs[0].style.size_pt, 6.);
    assert!(matches!(text.shape, TextShape::Box { width: 20., height: 20., .. }));
}

#[test]
fn native_literals_use_the_internal_profile_without_changing_cached_pixels() {
    let colors = NativeColors::new(&craft_cms::Builtin::LinearSrgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    let source = source("(text \"color\") (box-mode fixed) (color (color-rgba 0.5 0.5 0.5 0.4))");
    let mut layer = saved(PixelFormat::RGBA32F);
    let LayerContent::Raster(surface) = &layer.content else { panic!("raster") };
    let pixels = surface.pixel(5, 6);
    map_layer(
        &mut layer,
        &source,
        NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
        &mut NativeBudget::default(),
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let LayerContent::Text(text) = &layer.content else { panic!("editable") };
    assert!((0.15..0.3).contains(&text.color.c[0]));
    assert_eq!(text.color.alpha, 0.4);
    assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), pixels);
}

#[test]
fn unsupported_modified_and_budget_refused_text_leave_the_original_raster() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for source in [
        source("(text \"text\") (font-size-unit unknown)"),
        TextSource::ModifiedPixels { configuration: None },
        TextSource::Incomplete(craft_xcf::records::StopReason::WorkLimit),
    ] {
        let mut layer = saved(PixelFormat::RGBA8);
        let LayerContent::Raster(surface) = &layer.content else { panic!("raster") };
        let generation = surface.generation();
        let mut budget = NativeBudget::default();
        let storage = budget.storage_left;
        let mapped = map_layer(
            &mut layer,
            &source,
            NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
            &mut budget,
            &mut Budget::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert!(matches!(mapped, Mapping::Preserved { .. }));
        let LayerContent::Raster(surface) = &layer.content else { panic!("saved raster") };
        assert_eq!(surface.generation(), generation);
        assert_eq!(budget.storage_left, storage);
    }
    let mut layer = saved(PixelFormat::RGBA8);
    let source = source("(text \"bounded point origin\")");
    let mut budget = NativeBudget { layout_left: 0, ..Default::default() };
    let mapped = map_layer(
        &mut layer,
        &source,
        NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
        &mut budget,
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(matches!(mapped, Mapping::Preserved { .. }));
    assert!(matches!(layer.content, LayerContent::Raster(_)));
    assert!(matches!(
        map_layer(
            &mut layer,
            &source,
            NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
            &mut NativeBudget::default(),
            &mut Budget::default(),
            &Interrupt::cancel_only(&|| true)
        ),
        Err(crate::IoError::Cancelled)
    ));
}

#[test]
fn point_anchor_maps_the_native_logical_top_left_through_source_affine_once() {
    let mut text = TextLayer { transform: Affine { m: [1., 0.5, 0.25, 2., 7., 9.] }, ..Default::default() };
    anchor_bounds(&mut text, [-3., -7., 20., 12.]).unwrap();
    let source_origin = text.transform.apply(photocraft_geom::Point::new(-3., -7.));
    assert_eq!(source_origin, photocraft_geom::Point::new(7., 9.));
}

#[test]
fn portable_numeric_rgb_maps_opaque_colors_without_guessing_float_blob_order() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for (literal, expected_alpha) in [("(color-rgb 0.2 0.4 0.8)", 1.), ("(color-rgba 0.2 0.4 0.8 0.3)", 0.3)] {
        let raw = format!("(text \"Portable source\") (box-mode fixed) (box-width 40) (box-height 20) (color {literal})");
        let mut layer = saved(PixelFormat::RGBA8);
        let mapped = map_layer(
            &mut layer,
            &source(&raw),
            NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
            &mut NativeBudget::default(),
            &mut Budget::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert!(matches!(mapped, Mapping::Converted { .. }));
        let LayerContent::Text(text) = &layer.content else { panic!("editable portable text") };
        for (actual, expected) in text.color.c.iter().take(3).zip([0.2, 0.4, 0.8]) {
            assert!((actual - expected).abs() < 0.001);
        }
        assert_eq!(text.color.alpha, expected_alpha);
    }
    // Both producer byte orders remain ambiguous here: drawable byte order is
    // not evidence for these independent GEGL color payloads.
    for samples in [[0.2f32, 0.4, 0.8, 1.].map(f32::to_le_bytes), [0.2f32, 0.4, 0.8, 1.].map(f32::to_be_bytes)] {
        let bytes = samples.into_iter().flatten().map(|byte| format!("\\{byte:03o}")).collect::<String>();
        let raw = format!("(text \"Saved source\") (box-mode fixed) (color (color \"RGBA float\" 16 \"{bytes}\" 0))");
        let mut layer = saved(PixelFormat::RGBA8);
        let generation = layer.surface().unwrap().generation();
        let mapped = map_layer(
            &mut layer,
            &source(&raw),
            NativeOptions { resolution: (72., 72.), offset: (0, 0), colors: Some(&colors) },
            &mut NativeBudget::default(),
            &mut Budget::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert!(matches!(mapped, Mapping::Preserved { reason, .. } if reason.contains("byte-order evidence")));
        assert!(matches!(layer.content, LayerContent::Raster(_)));
        assert_eq!(layer.surface().unwrap().generation(), generation);
    }
}

#[test]
fn portable_base_color_keeps_unicode_markup_runs_editable() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    let raw = r##"(markup "<markup>hé<span foreground=\"#c04020\" size=\"18432\" weight=\"bold\">λlo</span>\n漢字</markup>")
        (font "GimpFont" (fullname "Sans") (family "Sans-serif") (style "Regular") (weight 80) (slant 0))
        (font-size 24) (box-mode fixed) (box-width 120) (box-height 60)
        (color (color-rgb 0.2 0.4 0.8))"##;
    let mut layer = saved(PixelFormat::RGBA8);
    let generation = layer.surface().unwrap().generation();
    let mapped = map_layer(
        &mut layer,
        &source(raw),
        NativeOptions { resolution: (72., 72.), offset: (-3, 7), colors: Some(&colors) },
        &mut NativeBudget::default(),
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(matches!(mapped, Mapping::Converted { .. }));
    let LayerContent::Text(text) = &layer.content else { panic!("editable portable markup") };
    assert_eq!(text.text, "héλlo\n漢字");
    assert!(text.runs.iter().any(|run| run.style.weight >= 700));
    assert!(text.runs.iter().any(|run| run.style.weight < 700));
    assert_eq!(text.runs.iter().map(|run| run.len).sum::<usize>(), text.text.len());
    assert_eq!(text.cache.as_ref().unwrap().generation(), generation);
}

#[test]
fn inherited_spacing_is_bounded_after_relative_markup_size_for_both_signs() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for spacing in [1_024_000, -1_024_000] {
        for (size, admitted) in [("50%", true), ("25%", false)] {
            let raw = format!(
                r#"(markup "<markup><span size='2048' letter_spacing='{spacing}'><span size='{size}'>A</span></span></markup>")
                (font-size 12) (font-size-unit points) (box-mode fixed) (box-width 40) (box-height 20)"#
            );
            let mut layer = saved(PixelFormat::RGBA8);
            let before = layer.clone();
            let mapped = map_layer(
                &mut layer,
                &source(&raw),
                NativeOptions { resolution: (72., 72.), offset: (5, 6), colors: Some(&colors) },
                &mut NativeBudget::default(),
                &mut Budget::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            if admitted {
                assert!(matches!(mapped, Mapping::Converted { .. }));
                let LayerContent::Text(text) = &layer.content else { panic!("admitted editable text") };
                assert_eq!(text.text, "A");
                assert_eq!(text.runs.len(), 1);
                assert_eq!(text.runs[0].style.tracking, if spacing > 0 { 1_000_000. } else { -1_000_000. });
                assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), before.surface().unwrap().pixel(5, 6));
            } else {
                assert!(matches!(mapped, Mapping::Preserved { reason, .. } if reason.contains("native tracking bounds")));
                assert_eq!(layer, before, "refused formatting keeps every saved pixel and source layer property");
            }
        }
    }
}

#[test]
fn source_vertical_columns_preserve_qualified_orientation_and_roman_rotation_with_auto_bidi() {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    for (direction, base) in [
        ("ttb-rtl", Direction::TtbRtl),
        ("ttb-rtl-upright", Direction::TtbRtlUpright),
        ("ttb-ltr", Direction::TtbLtr),
        ("ttb-ltr-upright", Direction::TtbLtrUpright),
    ] {
        for auto in ["no", "yes"] {
            let raw = format!("(text \"Vertical ABC אב\") (box-mode fixed) (box-width 40) (box-height 20) (base-direction {direction}) (base-dir-auto {auto})");
            let evidence = source(&raw);
            let TextSource::Configuration(record) = &evidence else { panic!("typed source text") };
            // Auto paragraph direction must not erase the independent writing mode.
            assert_eq!(record.base_direction, Some(base));
            assert_eq!(record.automatic_direction, Some(auto == "yes"));
            assert_eq!(record.direction, if auto == "yes" { Direction::Auto } else { base });
            let mut layer = saved(PixelFormat::RGBA8);
            let before = layer.clone();
            let mapped = map_layer(
                &mut layer,
                &evidence,
                NativeOptions { resolution: (72., 72.), offset: (5, 6), colors: Some(&colors) },
                &mut NativeBudget::default(),
                &mut Budget::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            if direction.starts_with("ttb-ltr") {
                assert!(matches!(mapped, Mapping::Preserved { reason, .. } if reason.contains("left-to-right vertical source columns")));
                assert_eq!(layer, before);
            } else {
                assert!(matches!(mapped, Mapping::Converted { .. }));
                let LayerContent::Text(text) = &layer.content else { panic!("qualified vertical native text") };
                assert_eq!(text.orientation, Orientation::Vertical);
                assert_eq!(text.paragraphs[0].style.direction, TextDirection::Auto);
                assert!(text.runs.iter().all(|run| run.style.rotate_roman == (direction == "ttb-rtl")));
                assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), before.surface().unwrap().pixel(5, 6));
            }
        }
    }
}

#[test]
fn owned_legacy_latin1_fields_keep_editable_text_font_request_and_saved_pixels() {
    let raw = br"GDT10{caf\351}{1}{0}{0}{0}{0xff00aa}{0}{-source-example-medium-r-normal--12-120-75-75-p-60-iso8859-1}";
    let mut scan = craft_xcf::records::text::gdyntext(raw, 0, Default::default(), &mut Budget::default(), &|| true).unwrap();
    assert!(scan.stopped.is_none());
    let record = scan.records.remove(0);
    assert_eq!(record.raw, raw);
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    let mut layer = saved(PixelFormat::RGBA8);
    let before = layer.surface().unwrap().pixel(5, 6);
    let mapped = map_layer(
        &mut layer,
        &TextSource::Configuration(record),
        NativeOptions { resolution: (72., 72.), offset: (5, 6), colors: Some(&colors) },
        &mut NativeBudget::default(),
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(matches!(mapped, Mapping::Converted { .. }));
    let LayerContent::Text(text) = &layer.content else { panic!("editable legacy text") };
    assert_eq!(text.text, "café");
    assert_eq!(text.font_family, "example");
    // The eight GDyntext fields carry no base direction. Preserve the shared
    // record's Auto policy instead of inventing an explicit left-to-right one.
    assert_eq!(text.paragraphs[0].style.direction, TextDirection::Auto);
    assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), before);
}

fn mapped_alpha_markup(markup: &str, sample: SampleType) -> TextLayer {
    let colors = NativeColors::new(&craft_cms::Builtin::Srgb.profile().to_bytes(), ColorMode::Rgb).unwrap();
    let raw = format!(r#"(markup "{markup}") (font-size 12) (box-mode fixed) (box-width 80) (box-height 20) (color (color-rgba 0 0 1 0.25))"#);
    let mut layer = saved(PixelFormat::new(ColorMode::Rgb, sample, true));
    let pixels = layer.surface().unwrap().pixel(5, 6);
    let generation = layer.surface().unwrap().generation();
    let mapped = map_layer(
        &mut layer,
        &source(&raw),
        NativeOptions { resolution: (72., 72.), offset: (5, 6), colors: Some(&colors) },
        &mut NativeBudget::default(),
        &mut Budget::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    assert!(matches!(mapped, Mapping::Converted { .. }));
    let LayerContent::Text(text) = layer.content else { panic!("editable markup") };
    assert_eq!(text.cache.as_ref().unwrap().pixel(5, 6), pixels);
    assert_eq!(text.cache.as_ref().unwrap().generation(), generation);
    assert!(text.cache_uses_raster_blending);
    text
}

fn run_color_at(text: &TextLayer, byte: usize) -> photocraft_color::Color {
    let mut end = 0;
    text.runs
        .iter()
        .find_map(|run| {
            end += run.len;
            (byte < end).then_some(run.style.color)
        })
        .unwrap()
}

fn assert_run_rgb_at(text: &TextLayer, byte: usize, expected: [f32; 3]) {
    let color = run_color_at(text, byte);
    let actual = &color.c[..3];
    // ICC transforms can leave sub-ULP residuals in otherwise zero components.
    for (actual_component, expected_component) in actual.iter().zip(expected) {
        assert!((actual_component - expected_component).abs() < 1e-6, "byte {byte}: {actual:?} vs {expected:?}");
    }
}

#[test]
fn rgb_markup_foregrounds_preserve_inherited_alpha_and_sibling_scopes_at_all_depths() {
    // Pango's foreground/alpha attributes are independent for RGB; RGBA supplies both.
    // Public reference: https://docs.gtk.org/Pango/pango_markup.html
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for foreground in ["red", "#f00", "#ff0000", "#fff000000", "#ffff00000000"] {
            let markup = format!("<markup><span alpha='50%'>A<span foreground='{foreground}'>B</span>C</span>D</markup>");
            let text = mapped_alpha_markup(&markup, sample);
            assert_eq!(text.text, "ABCD");
            for (index, alpha) in [(0, 0.5), (1, 0.5), (2, 0.5), (3, 0.25)] {
                assert_eq!(run_color_at(&text, index).alpha, alpha, "{foreground}, byte {index}");
            }
            assert_run_rgb_at(&text, 1, [1., 0., 0.]);
            assert_run_rgb_at(&text, 2, [0., 0., 1.]);
            // Native saves retain these editable run attributes as well as the saved pixels.
            let mut doc = photocraft_doc::Document::new("Markup alpha", photocraft_geom::Size::new(80, 20), ColorMode::Rgb, sample);
            doc.layers.push(Layer::new("Editable", LayerContent::Text(text.clone())));
            let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Text(saved) = &reopened.layers[0].content else { panic!("native text") };
            assert_eq!(saved.runs, text.runs);
        }
    }
}

#[test]
fn rgba_and_explicit_markup_alpha_override_while_nested_rgb_only_changes_color() {
    for (foreground, alpha) in [("#f008", 8. / 15.), ("#ff000080", 128. / 255.), ("#ffff000000008000", 32768. / 65535.)] {
        let markup = format!("<markup><span alpha='50%'><span foreground='{foreground}'>A<span foreground='blue'>B</span></span>C</span></markup>");
        let text = mapped_alpha_markup(&markup, SampleType::F32);
        assert_eq!(text.text, "ABC");
        assert!((run_color_at(&text, 0).alpha - alpha).abs() < 1e-6);
        assert!((run_color_at(&text, 1).alpha - alpha).abs() < 1e-6);
        assert_run_rgb_at(&text, 1, [0., 0., 1.]);
        assert_eq!(run_color_at(&text, 2).alpha, 0.5);
    }
    for attributes in ["alpha='75%' foreground='red'", "foreground='red' alpha='75%'"] {
        let markup = format!("<markup><span alpha='50%'><span {attributes}>A</span>B</span></markup>");
        let text = mapped_alpha_markup(&markup, SampleType::F32);
        assert_eq!(run_color_at(&text, 0).alpha, 0.75);
        assert_eq!(run_color_at(&text, 1).alpha, 0.5);
    }
}
