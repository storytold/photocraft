use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};
use craft_cms::{Builtin, Intent, Profile, Transform};
use photocraft_color::ColorMode;
use photocraft_doc::{Fill, LayerContent};

fn parasite(name: &str, data: &[u8]) -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    w.string(name);
    w.u32(0);
    w.u32(data.len() as u32);
    w.out.extend_from_slice(data);
    w.out
}

fn source(mode: ColorMode, code: u32, assigned: Option<&[u8]>) -> Vec<u8> {
    let mut image = TestImage::rgb8(64, 64);
    image.version = 26;
    image.base_type = u32::from(mode == ColorMode::Grayscale);
    image.precision = code;
    if let Some(profile) = assigned {
        image.image_props.push((21, parasite("icc-profile", profile)));
    }
    // A closed rectangle with coincident handles; its interior has exactly the solid fill.
    let mut path = W { out: Vec::new(), wide: false };
    for value in [1, 0, 1] {
        path.u32(value);
    }
    path.string("Outline");
    for value in [72, 0, 0, 0, 1, 1, 1, 2, 12] {
        path.u32(value);
    }
    for (x, y) in [(4f32, 4f32), (32., 4.), (32., 32.), (4., 32.)] {
        for kind in [1, 0, 1] {
            path.u32(kind);
            path.u32(x.to_bits());
            path.u32(y.to_bits());
        }
    }
    image.image_props.push((25, path.out));
    let values: Vec<f32> = if mode == ColorMode::Grayscale { vec![1., 1.] } else { vec![1., 0., 1., 1.] };
    let pixels = match code {
        150 => values.iter().map(|v| (*v * 255.) as u8).collect(),
        250 => values.iter().flat_map(|v| ((*v * 65535.) as u16).to_be_bytes()).collect(),
        600 | 650 => values.iter().flat_map(|v| v.to_be_bytes()).collect(),
        _ => panic!("fixture precision"),
    };
    let mut text = TestLayer::rgba("Editable text", 1, 1, pixels);
    text.kind = if mode == ColorMode::Grayscale { 3 } else { 1 };
    text.x = 40;
    text.y = 40;
    text.props.push((26, 0u32.to_be_bytes().to_vec()));
    text.props.push((21, parasite("gimp-text-layer", br#"(text "M") (font "Sans") (font-size 20) (color (color-rgba 0.25 0.5 0.75 0.5))"#)));
    let mut shape = TestLayer::rgba("Editable vector", 1, 1, text.data.clone());
    shape.kind = text.kind;
    let mut style = W { out: Vec::new(), wide: false };
    for value in [0, 72, 1, 1] {
        style.u32(value);
    }
    for paint in [[64, 128, 192, 128], [255, 0, 0, 64]] {
        style.u32(0);
        style.u32(1);
        style.string("R'G'B'A u8");
        style.u32(4);
        style.out.extend_from_slice(&paint);
        style.u32(0);
        style.string("");
    }
    for value in [2f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
        style.u32(value);
    }
    shape.props.push((47, style.out));
    image.layers.extend([text, shape]);
    image.write()
}

fn expected(profile: &[u8], mode: ColorMode, rgba: [f32; 4]) -> Color {
    let destination = Profile::parse(profile).unwrap();
    let transform = Transform::new(Builtin::Srgb.profile(), &destination, Intent::RelativeColorimetric, false).unwrap();
    let mut c = [0.; 4];
    let channels = mode.color_channels();
    transform.convert_f32(&rgba[..3], 3, &mut c[..channels], channels, false).unwrap();
    Color { mode, c, alpha: rgba[3] }
}

fn assert_literal(actual: Color, expected: Color) {
    assert_eq!(actual.mode, expected.mode);
    assert_eq!(actual.alpha, expected.alpha);
    // The cached transform can reuse the built-in profile representation; the independent
    // transform reparses serialized ICC coefficients. Allow only their fixed-point rounding
    // difference, about 1.3 U16 sample steps for these literals, and keep mode/alpha exact.
    for (actual, expected) in actual.c.into_iter().zip(expected.c) {
        assert!((actual - expected).abs() <= 2e-5, "literal channel {actual} versus {expected}");
    }
}

#[test]
fn editable_text_and_vectors_use_internal_profile_without_inventing_an_assignment() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let assignment = if mode == ColorMode::Rgb { Builtin::DisplayP3 } else { Builtin::GrayGamma22 }.profile().to_bytes();
        for (code, assigned) in [(150, None), (250, None), (650, None), (600, None), (650, Some(assignment.as_slice()))] {
            let profile =
                crate::xcf::color::select_profile(mode, crate::xcf::samples::Precision::from_header(26, code).unwrap(), assigned.map(<[u8]>::to_vec)).unwrap();
            let imported = crate::import("editable-profile.xcf", &source(mode, code, assigned)).unwrap();
            let doc = &imported.document;
            assert_eq!(doc.icc_profile, profile.tag_document.then(|| profile.bytes.clone()), "{mode:?}/{code}");
            let text = doc
                .layers
                .iter()
                .find_map(|layer| match &layer.content {
                    LayerContent::Text(text) => Some(text),
                    _ => None,
                })
                .expect("editable text, not saved raster");
            let shape = doc
                .layers
                .iter()
                .find_map(|layer| match &layer.content {
                    LayerContent::Shape(shape) => Some(shape),
                    _ => None,
                })
                .expect("editable vector, not saved raster");
            assert!(text.cache.is_none() && shape.cache.is_none());
            assert_literal(text.runs[0].style.color, expected(&profile.bytes, mode, [0.25, 0.5, 0.75, 0.5]));
            let fill = expected(&profile.bytes, mode, [64. / 255., 128. / 255., 192. / 255., 128. / 255.]);
            let Some(Fill::Solid(actual_fill)) = &shape.fill else { panic!("solid vector fill") };
            assert_literal(*actual_fill, fill);
            let Fill::Solid(actual_stroke) = &shape.stroke.as_ref().unwrap().paint else { panic!("solid vector stroke") };
            assert_literal(*actual_stroke, expected(&profile.bytes, mode, [1., 0., 0., 64. / 255.]));
            let clip = photocraft_geom::Rect::new(16, 16, 17, 17);
            let pixels = photocraft_vector::render_shape(shape, doc.pixel_format(), clip).pixel(16, 16);
            assert_eq!(pixels.len(), mode.color_channels() + 1);
            assert!(pixels[..mode.color_channels()].iter().all(|sample| *sample > 0.05), "visible converted vector fill: {pixels:?}");
            assert!((pixels[mode.color_channels()] - fill.alpha).abs() < 0.005, "fill alpha survives conversion: {pixels:?}");
            let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_eq!(reopened.icc_profile, doc.icc_profile);
            assert_eq!(reopened.layers, doc.layers);
            let reopened_shape = reopened
                .layers
                .iter()
                .find_map(|layer| match &layer.content {
                    LayerContent::Shape(shape) => Some(shape),
                    _ => None,
                })
                .unwrap();
            assert_eq!(photocraft_vector::render_shape(reopened_shape, reopened.pixel_format(), clip).pixel(16, 16), pixels);
        }
    }
}

#[test]
fn literal_conversion_rejects_cms_stride_mismatch_before_changing_the_color() {
    let original = decode(br#"(text "M") (color (color-rgba 0.25 0.5 0.75 0.5))"#, 72., (0, 0)).unwrap().layer;
    let mut document =
        photocraft_doc::Document::new("Bad internal profile", photocraft_geom::Size::new(1, 1), ColorMode::Grayscale, photocraft_color::SampleType::F32);
    document.layers.push(photocraft_doc::Layer::new("Text", LayerContent::Text(original.clone())));
    let error = convert_colors(&mut document, &Builtin::Srgb.profile().to_bytes()).unwrap_err();
    assert!(error.to_string().contains("native literal color conversion"), "{error}");
    assert_eq!(document.layers[0].content, LayerContent::Text(original));
    assert!(document.icc_profile.is_none());
}
