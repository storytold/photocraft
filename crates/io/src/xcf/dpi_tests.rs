use super::tests::{TestImage, TestLayer, W};
use super::*;

/// Author source pixel-sized text and a physical-sized box, so both conversions must use document DPI.
fn text_at_resolution(properties: Vec<Vec<u8>>) -> ImportResult {
    let mut source = TestImage::rgb8(1, 1);
    source.image_props.extend(properties.into_iter().map(|payload| (19, payload)));
    let config = br#"(text "Ag") (font "Inter") (font-size 24) (font-size-unit pixels) (box-mode fixed) (box-unit inches) (box-width 0.5) (box-height 0.25)"#;
    let mut parasite = W { out: Vec::new(), wide: true };
    parasite.string("gimp-text-layer");
    parasite.u32(0);
    parasite.u32(config.len() as u32);
    parasite.out.extend_from_slice(config);
    let mut layer = TestLayer::rgba("DPI text", 1, 1, vec![255, 0, 0, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite.out));
    source.layers.push(layer);
    import("resolution.xcf", &source.write(), &Interrupt::NONE).unwrap()
}

fn pair(x: f32, y: f32) -> Vec<u8> {
    [x.to_be_bytes(), y.to_be_bytes()].concat()
}

fn assert_matching_resolution(properties: Vec<Vec<u8>>, dpi: f32) {
    let imported = text_at_resolution(properties);
    let document = imported.document;
    assert_eq!(document.resolution_dpi, dpi);
    let LayerContent::Text(text) = &document.layers[0].content else { panic!("editable source text") };
    assert!((text.size_pt - 24. * 72. / dpi).abs() < 1e-5);
    assert!((text.runs[0].style.size_pt * dpi / 72. - 24.).abs() < 1e-4, "source pixel-sized text must stay 24 native pixels");
    let photocraft_doc::text::TextShape::Box { width, height, .. } = text.shape else { panic!("fixed source box") };
    assert!((width - 0.5 * dpi).abs() < 1e-4 && (height - 0.25 * dpi).abs() < 1e-4, "physical box dimensions must use the final document DPI");
}

#[test]
fn malformed_resolution_pairs_keep_default_dpi_for_text_and_physical_boxes() {
    assert_matching_resolution(Vec::new(), 72.);
    for malformed in [pair(300., f32::NAN), pair(300., f32::INFINITY), pair(-1., 300.), pair(1.0e6, 300.), 300f32.to_be_bytes().to_vec()] {
        assert_matching_resolution(vec![malformed], 72.);
    }
}

#[test]
fn duplicate_resolution_properties_use_the_last_valid_pair_for_all_native_units() {
    for (properties, expected) in [
        (vec![pair(300., 300.), pair(144., 144.)], 144.),
        (vec![pair(300., 300.), pair(144., f32::NAN)], 300.),
        (vec![pair(300., f32::NAN), pair(144., 144.)], 144.),
        (vec![pair(72., 72.), pair(300., f32::NAN)], 72.),
        (vec![pair(300., 150.)], 300.),
    ] {
        assert_matching_resolution(properties, expected);
    }
}
