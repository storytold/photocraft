//! Public CPU literal regression for HDR pass-through groups with no-op clipped layers.
use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_compose::render;
use photocraft_doc::{Document, Layer};
use photocraft_geom::Size;

/// Place literal HDR pixels in a group with an empty or zero-opacity clip to isolate reconstruction.
fn fixture(rgb: [f32; 3], opacity: f32, isolated: bool, visible: bool, empty: bool) -> Document {
    let mut doc = Document::new("HDR clip no-op", Size::new(2, 2), ColorMode::Rgb, SampleType::F32);
    let mut child = Layer::raster("normal HDR", PixelFormat::RGBA32F);
    child.surface_mut().unwrap().fill_rect(doc.bounds(), &[rgb[0], rgb[1], rgb[2], 1.0]);
    let mut group = Layer::group("group", vec![child]);
    group.opacity = opacity;
    if isolated {
        group.blend = BlendMode::Normal;
    }
    let mut clip = Layer::raster("no-op clip", PixelFormat::RGBA32F);
    clip.clipped = true;
    clip.visible = visible;
    if !empty {
        clip.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 1.0, 0.0, 1.0]);
        clip.opacity = 0.0;
    }
    doc.layers = vec![group, clip];
    doc
}

/// Compare every public-renderer pixel with an independent literal, retaining all failed cases.
fn assert_literals(cases: Vec<(Document, [f32; 4])>) {
    let mut failures = Vec::new();
    for (doc, expected) in cases {
        let output = render(&doc, doc.bounds());
        assert_eq!(output.px.len(), 4, "the public renderer returns the 2x2 canvas");
        eprintln!(
            "opacity {} isolated {} visible {}: {:?}, literal {expected:?}",
            doc.layers[0].opacity,
            doc.layers[0].blend == BlendMode::Normal,
            doc.layers[1].visible,
            output.px
        );
        for (i, p) in output.px.iter().enumerate() {
            if !p.iter().zip(expected).all(|(got, want)| (got - want).abs() < 1e-6) {
                failures.push(format!("opacity {} pixel {i}: {p:?} vs literal {expected:?}", doc.layers[0].opacity));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Empty visible clips must preserve signed RGB and let group opacity affect only alpha.
#[test]
fn an_empty_clip_does_not_clip_pass_through_hdr() {
    // Independent no-op oracle: Normal over transparency keeps straight RGB;
    // group opacity changes alpha, and an all-transparent clip contributes zero.
    assert_literals([1.0, 0.5].into_iter().map(|opacity| (fixture([2.0, -0.25, 0.5], opacity, false, true, true), [2.0, -0.25, 0.5, opacity])).collect());
}

/// A populated clip at zero opacity is equally inert, despite entering the clipping path.
#[test]
fn a_zero_opacity_clip_does_not_clip_pass_through_hdr() {
    assert_literals([1.0, 0.5].into_iter().map(|opacity| (fixture([2.0, -0.25, 0.5], opacity, false, true, false), [2.0, -0.25, 0.5, opacity])).collect());
}

/// Visibility, isolation and in-range controls distinguish reconstruction from ordinary grouping.
#[test]
fn hidden_clips_isolated_groups_and_sdr_are_controls() {
    let mut cases = Vec::new();
    for opacity in [1.0, 0.5] {
        cases.push((fixture([2.0, -0.25, 0.5], opacity, false, false, true), [2.0, -0.25, 0.5, opacity]));
        cases.push((fixture([2.0, -0.25, 0.5], opacity, true, true, true), [2.0, -0.25, 0.5, opacity]));
        cases.push((fixture([0.25, 0.5, 0.75], opacity, false, true, true), [0.25, 0.5, 0.75, opacity]));
    }
    assert_literals(cases);
}

/// Author pixels in the actual requested storage depth, including zero-alpha RGB payloads.
fn depth_fixture(depth: SampleType, opacity: f32, alpha: f32, empty_clip: bool) -> Document {
    let mut doc = Document::new("depth and alpha control", Size::new(2, 2), ColorMode::Rgb, depth);
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut child = Layer::raster("child", format);
    child.surface_mut().unwrap().fill_rect(doc.bounds(), &[1.0, 0.0, 1.0, alpha]);
    assert!(!child.surface().unwrap().content_bounds().is_empty(), "authored pixels, including RGB at zero alpha");
    let mut group = Layer::group("group", vec![child]);
    group.opacity = opacity;
    let mut clip = Layer::raster("clip", format);
    clip.clipped = true;
    if !empty_clip {
        clip.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 1.0, 0.0, 1.0]);
        clip.opacity = 0.0;
    }
    doc.layers = vec![group, clip];
    doc
}

/// U8 and U16 no-op clips retain their existing in-range pixels at both group opacities.
#[test]
fn actual_integer_depths_keep_the_bounded_clipping_path() {
    let mut cases = Vec::new();
    for depth in [SampleType::U8, SampleType::U16] {
        for opacity in [1.0, 0.5] {
            for empty_clip in [true, false] {
                cases.push((depth_fixture(depth, opacity, 1.0, empty_clip), [1.0, 0.0, 1.0, opacity]));
            }
        }
    }
    assert_literals(cases);
}

/// Unpremultiplication at zero alpha must produce transparent black at every document depth.
#[test]
fn fully_transparent_clipping_output_is_transparent_black() {
    let mut cases = Vec::new();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for opacity in [1.0, 0.5] {
            cases.push((depth_fixture(depth, opacity, 0.0, true), [0.0; 4]));
        }
    }
    assert_literals(cases);
}

/// An active half-opacity Normal clip mixes literal HDR colors without bounding RGB.
#[test]
fn an_active_normal_clip_preserves_its_literal_hdr_result() {
    let mut cases = Vec::new();
    for opacity in [1.0, 0.5] {
        let mut doc = fixture([2.0, -0.25, 0.5], opacity, false, true, false);
        let bounds = doc.bounds();
        let clip = &mut doc.layers[1];
        clip.opacity = 0.5;
        clip.surface_mut().unwrap().fill_rect(bounds, &[0.25, 0.0, 0.75, 1.0]);
        // Normal at half coverage: (base + clip) / 2. Group opacity changes alpha.
        cases.push((doc, [1.125, -0.125, 0.625, opacity]));
    }
    assert_literals(cases);
}

/// Screen leaves the grey backdrop intact, but an opaque white clip adds one during reconstruction.
/// The unclamped result is above white, making this sensitive to removal of the integer clamp.
fn integer_clamp_fixture(depth: SampleType) -> Document {
    let mut doc = Document::new("integer reconstruction clamp", Size::new(2, 2), ColorMode::Rgb, depth);
    let format = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut base = Layer::raster("grey base", format);
    base.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.5, 0.5, 0.5, 1.0]);
    let mut child = Layer::raster("black Screen", format);
    child.blend = BlendMode::Screen;
    child.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 0.0, 0.0, 1.0]);
    let group = Layer::group("pass-through", vec![child]);
    let mut clip = Layer::raster("white clip", format);
    clip.clipped = true;
    clip.surface_mut().unwrap().fill_rect(doc.bounds(), &[1.0; 4]);
    doc.layers = vec![base, group, clip];
    doc
}

/// Exercise the integer clamp with an above-white reconstructed value rather than a no-op clip.
#[test]
fn integer_reconstruction_still_clamps_above_white() {
    assert_literals([SampleType::U8, SampleType::U16].into_iter().map(|depth| (integer_clamp_fixture(depth), [1.0; 4])).collect());
}
