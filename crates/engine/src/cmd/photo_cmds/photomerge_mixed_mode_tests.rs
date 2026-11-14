use super::photomerge_whole_pixel_tests::{scene, tile};
use super::*;

#[test]
fn mixed_eight_bit_sources_keep_rgb_and_gray_samples_in_either_order() {
    let pixels = scene();
    for reverse in [false, true] {
        let mut session = Session::new();
        let mut gray = tile(&pixels, 0, ColorMode::Grayscale, SampleType::U8);
        let mut rgb = tile(&pixels, 200, ColorMode::Rgb, SampleType::U8);
        gray.icc_profile = Some(craft_cms::builtin::linear_gray().to_bytes());
        let rgb_profile = craft_cms::Builtin::Srgb.profile().to_bytes();
        rgb.icc_profile = Some(rgb_profile.clone());
        let gray_sample = gray.layers[0].surface().unwrap().rgba(22, 100);
        let rgb_sample = rgb.layers[0].surface().unwrap().rgba(150, 170);
        assert_ne!(rgb_sample[0], rgb_sample[1], "test RGB probe must be coloured");
        for doc in if reverse { [rgb, gray] } else { [gray, rgb] } {
            session.add_document(doc, None);
        }
        session.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "auto", "blend": false})).unwrap();
        let doc = &session.active().unwrap().doc;
        assert_eq!(doc.pixel_format(), PixelFormat::RGBA8);
        assert_eq!(doc.icc_profile.as_ref(), Some(&rgb_profile));
        let sample = |name, x, y| doc.layers.iter().find(|layer| layer.name == name).unwrap().surface().unwrap().rgba(x, y);
        assert_eq!(sample("tile-0", 22, 100), gray_sample);
        assert_eq!(sample("tile-200", 350, 170), rgb_sample);
    }
}

#[test]
fn all_gray_and_higher_depth_merge_keeps_first_source_format() {
    let pixels = scene();
    for (second_mode, depth) in [(ColorMode::Grayscale, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Rgb, SampleType::F32)] {
        let mut session = Session::new();
        let first = tile(&pixels, 0, ColorMode::Grayscale, depth);
        let fmt = first.pixel_format();
        session.add_document(first, None);
        session.add_document(tile(&pixels, 200, second_mode, depth), None);
        let (_, output, _) = load_sources(&session, &json!({"useOpenDocuments": true}), "file.automate.photomerge", Some(1 << 28)).unwrap();
        assert_eq!(output, fmt);
    }
}

#[test]
fn promotion_checks_the_combined_surface_budget_before_allocating() {
    let pixels = scene();
    let mut ordered = Session::new();
    for (x, mode) in [
        (0, ColorMode::Grayscale),
        (0, ColorMode::Rgb),
        (100, ColorMode::Grayscale),
        (200, ColorMode::Grayscale),
        (300, ColorMode::Grayscale),
        (400, ColorMode::Grayscale),
    ] {
        ordered.add_document(tile(&pixels, x, mode, SampleType::U8), None);
    }
    // Loading fits within the budget, but converting the later Gray surfaces
    // requires holding both formats at once.
    let expected_gray_fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let params = json!({"useOpenDocuments": true});
    let cmd = "file.automate.photomerge";
    let gray = surface_bytes(300, 300, expected_gray_fmt.bytes_per_pixel());
    let rgb = surface_bytes(300, 300, PixelFormat::RGBA8.bytes_per_pixel());
    let error = load_sources(&ordered, &params, cmd, Some(5 * rgb + gray)).err().unwrap();
    assert!(error.to_string().contains("converting"), "{error}");
    let (sources, fmt, _) = load_sources(&ordered, &params, cmd, Some(7 * rgb)).unwrap();
    assert_eq!(fmt, PixelFormat::RGBA8);
    assert!(sources.iter().all(|source| source.surf.format() == fmt));
}
