use super::*;
use photocraft_cms::Builtin;

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#808080"/></svg>"##;

#[test]
fn svg_builtin_srgb_keeps_direct_render_at_each_depth() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="4" height="8" fill="#336699"/><rect x="4" width="4" height="4" fill="#e65c1a" fill-opacity="0.5"/></svg>"##;
    for depth in DEPTHS {
        let s = session(depth);
        let mut doc = (*s.active().unwrap().doc).clone();
        for explicit_profile in [false, true] {
            doc.icc_profile = explicit_profile.then(|| Builtin::Srgb.profile().to_bytes());
            let expected = svg_source_image(svg, doc.pixel_format(), 1.0).unwrap();
            let actual = svg_source_image_in(svg, &doc, 1.0).unwrap();
            assert_eq!(actual.bounds, expected.bounds);
            assert_eq!(actual.surface, expected.surface, "sRGB at {depth}-bit, explicit={explicit_profile}");
        }
    }
}

#[test]
fn svg_smart_rerender_checks_embedded_raster_dimensions() {
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    header.extend_from_slice(&32768u32.to_be_bytes());
    header.extend_from_slice(&32768u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let data = header.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg =
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><image width=\"1\" height=\"1\" href=\"data:image/png,{data}\"/></svg>");
    assert!(svg_source_image(svg.as_bytes(), PixelFormat::RGBA32F, 1.0).unwrap_err().to_string().contains("decoded pixel budget"));
    let s = session(8);
    let smart = SmartObject::new(SmartSource::Embedded { file_name: "Clipboard.svg".into(), bytes: Arc::new(svg.into_bytes()) }, Affine::IDENTITY, None);
    assert!(render(&s.active().unwrap().doc, &smart).unwrap_err().to_string().contains("decoded pixel budget"));
}

#[test]
fn svg_smart_contents_and_stack_reject_excessive_editable_caches() {
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">{}</svg>", "<rect width=\"256\" height=\"256\"/>".repeat(400));
    assert!(decode_source("Clipboard.svg", svg.as_bytes()).unwrap_err().to_string().contains("cache budget"));
    assert!(stack_image("Clipboard.svg", svg.as_bytes(), PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).is_err());
}

/// Warp setup needs only the source frame. An SVG whose editable layer caches exceed the budget
/// renders through the bounded whole-drawing path, so it must still get that frame, while Stack
/// Mode and Edit Contents keep refusing to build its layers.
#[test]
fn svg_warp_frame_uses_bounded_bounds_while_stack_and_contents_stay_strict() {
    // 256 × 256 × 400 shapes = 26,214,400 pixels of layer caches, over the 24 MP budget.
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">{}</svg>",
        "<rect width=\"256\" height=\"256\" fill=\"#336699\"/>".repeat(400)
    );
    let s = session(8);
    let doc = s.active().unwrap().doc.clone();
    let mut smart =
        SmartObject::new(SmartSource::Embedded { file_name: "Clipboard.svg".into(), bytes: Arc::new(svg.clone().into_bytes()) }, Affine::IDENTITY, None);
    let bounds = [0.0, 0.0, 256.0, 256.0];
    assert_eq!(source_box(&doc, &smart), Some(bounds));
    smart.warp = Some(photocraft_geom::warp::Warp::custom(photocraft_geom::warp::BezierMesh::identity(bounds, 1, 1), bounds));
    assert_eq!(source_box(&doc, &smart), Some(bounds), "a warped SVG keeps its frame");
    let warped = render(&doc, &smart).unwrap().unwrap();
    assert!((warped.pixel(10, 10)[2] - 0x99 as f32 / 255.0).abs() < 0.01, "the bounded warp render draws the SVG");
    // Strict paths are unchanged.
    assert!(decode_source("Clipboard.svg", svg.as_bytes()).unwrap_err().to_string().contains("cache budget"));
    smart.warp = None;
    smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
    assert_eq!(source_box(&doc, &smart), None, "Stack Mode needs the strict layered import");
    // The canvas limit is exact: 24,000,000 pixels gets a frame, one more row does not.
    let canvas = |h: u32| format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" height=\"{h}\"><rect width=\"8\" height=\"8\"/></svg>");
    let frame = |svg: String| {
        source_box(&doc, &SmartObject::new(SmartSource::Embedded { file_name: "Big.svg".into(), bytes: Arc::new(svg.into_bytes()) }, Affine::IDENTITY, None))
    };
    assert_eq!(frame(canvas(6000)), Some([0.0, 0.0, 4000.0, 6000.0]));
    assert_eq!(frame(canvas(6001)), None);
}

#[test]
fn svg_smart_render_cache_tracks_destination_profile_and_transform() {
    for depth in DEPTHS {
        let s = session(depth);
        let srgb = (*s.active().unwrap().doc).clone();
        let mut linear = srgb.clone();
        linear.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "clipboard.svg".into(), bytes: Arc::new(SVG.to_vec()) }, Affine::IDENTITY, None);
        let original = render(&srgb, &smart).unwrap().unwrap();
        let corrected = render(&linear, &smart).unwrap().unwrap();
        assert!((original.pixel(3, 3)[0] - 128.0 / 255.0).abs() < 0.01);
        assert!((corrected.pixel(3, 3)[0] - 0.216).abs() < 0.01, "linear profile at {depth}-bit");
        assert_eq!(render(&srgb, &smart).unwrap().unwrap(), original, "profile-specific placed cache");
        smart.transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 4.0, 2.0] };
        let scaled = render(&linear, &smart).unwrap().unwrap();
        assert!((scaled.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "scale must keep corrected colours");
        assert_eq!(scaled.format(), linear.pixel_format());
        let bounds = [0.0, 0.0, 8.0, 8.0];
        smart.warp = Some(photocraft_geom::warp::Warp::custom(photocraft_geom::warp::BezierMesh::identity(bounds, 1, 1), bounds));
        let warped = render(&linear, &smart).unwrap().unwrap();
        assert!((warped.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "warp must keep corrected colours");
        linear.layers = vec![Layer::new("Saved SVG", LayerContent::Smart(smart.clone()))];
        let saved = encode_source(&linear).unwrap();
        let reopened = decode_source("saved.pcraft", &saved).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("smart object lost on save") };
        assert_eq!(restored.source, smart.source);
        assert_eq!(restored.transform, smart.transform);
        let rerendered = render(&reopened, restored).unwrap().unwrap();
        assert!((rerendered.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "saved source keeps ICC and warp");
    }
}
