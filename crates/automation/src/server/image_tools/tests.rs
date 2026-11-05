//! Unit tests of the task tools' parameter planning (no engine involved).

use super::plan::{self, Step};
use super::*;

fn ok<T>(r: Result<T, String>) -> T {
    r.unwrap_or_else(|e| panic!("unexpected error: {e}"))
}

fn err<T: std::fmt::Debug>(r: Result<T, String>) -> String {
    r.expect_err("expected an error")
}

fn size(s: &Step) -> (u64, u64) {
    (s.params["width"].as_u64().unwrap(), s.params["height"].as_u64().unwrap())
}

#[test]
fn resize_one_side_keeps_the_aspect_ratio() {
    let s = ok(plan::resize(&ResizeParams { width: Some(1000), ..Default::default() }, (4000, 3000)));
    assert_eq!(s.id, "image.imageSize");
    assert_eq!(size(&s), (1000, 750));
    let s = ok(plan::resize(&ResizeParams { height: Some(300), ..Default::default() }, (4000, 3000)));
    assert_eq!(size(&s), (400, 300));
}

#[test]
fn resize_exact_percent_and_resample() {
    let s = ok(plan::resize(&ResizeParams { width: Some(10), height: Some(20), resample: Some(Resample::Lanczos), ..Default::default() }, (5, 5)));
    assert_eq!(size(&s), (10, 20));
    assert_eq!(s.params["resample"], "lanczos");
    let s = ok(plan::resize(&ResizeParams { percent: Some(50.0), ..Default::default() }, (801, 600)));
    assert_eq!(size(&s), (401, 300));
}

#[test]
fn resize_fit_never_enlarges_unless_asked() {
    let fit = ResizeParams { width: Some(1080), height: Some(1080), fit: true, ..Default::default() };
    assert_eq!(size(&ok(plan::resize(&fit, (4000, 2000)))), (1080, 540));
    assert_eq!(size(&ok(plan::resize(&fit, (500, 250)))), (500, 250));
    let enlarge = ResizeParams { enlarge: true, ..fit };
    assert_eq!(size(&ok(plan::resize(&enlarge, (500, 250)))), (1080, 540));
}

#[test]
fn resize_rejects_bad_combinations_and_huge_targets() {
    assert!(err(plan::resize(&ResizeParams::default(), (10, 10))).contains("needs"));
    assert!(err(plan::resize(&ResizeParams { percent: Some(50.0), width: Some(3), ..Default::default() }, (10, 10))).contains("not both"));
    assert!(err(plan::resize(&ResizeParams { width: Some(3), fit: true, ..Default::default() }, (10, 10))).contains("both"));
    assert!(err(plan::resize(&ResizeParams { width: Some(3), enlarge: true, ..Default::default() }, (10, 10))).contains("fit"));
    assert!(err(plan::resize(&ResizeParams { percent: Some(f64::NAN), ..Default::default() }, (10, 10))).contains("finite"));
    assert!(err(plan::resize(&ResizeParams { percent: Some(0.0), ..Default::default() }, (10, 10))).contains("between"));
    assert!(err(plan::resize(&ResizeParams { width: Some(100_000), height: Some(100_000), ..Default::default() }, (10, 10))).contains("megapixel"));
    // Rounding to zero pixels is refused, not sent to the engine.
    assert!(err(plan::resize(&ResizeParams { width: Some(1), ..Default::default() }, (10_000, 1))).contains("less than 1"));
}

#[test]
fn crop_validates_the_rectangle() {
    let s = ok(plan::crop(&CropParams { x: 10, y: 20, width: 30, height: 40, index: None }, (100, 100)));
    assert_eq!(s.id, "image.crop");
    assert_eq!(s.params, json!({"x": 10, "y": 20, "width": 30, "height": 40}));
    assert!(err(plan::crop(&CropParams { x: 0, y: 0, width: 0, height: 5, index: None }, (100, 100))).contains("at least 1"));
    assert!(err(plan::crop(&CropParams { x: 100, y: 0, width: 5, height: 5, index: None }, (100, 100))).contains("outside"));
    assert!(err(plan::crop(&CropParams { x: -10, y: -10, width: 10, height: 10, index: None }, (100, 100))).contains("outside"));
}

#[test]
fn rotate_picks_lossless_turns_and_arbitrary_angles() {
    let r = |angle: f64| plan::rotate(&RotateParams { angle, ..Default::default() });
    assert_eq!(ok(r(90.0)).id, "image.imageRotation.90cw");
    assert_eq!(ok(r(-270.0)).id, "image.imageRotation.90cw");
    assert_eq!(ok(r(270.0)).id, "image.imageRotation.90ccw");
    assert_eq!(ok(r(-90.0)).id, "image.imageRotation.90ccw");
    assert_eq!(ok(r(180.0)).id, "image.imageRotation.180");
    assert_eq!(ok(r(-180.0)).id, "image.imageRotation.180");
    let s = ok(r(-15.0));
    assert_eq!(s.id, "image.rotation.arbitrary");
    assert_eq!(s.params["direction"], "ccw");
    assert_eq!(s.params["angle"], 15.0);
    assert_eq!(ok(r(375.0)).params["direction"], "cw");
    assert!(err(r(0.0)).contains("does not rotate"));
    assert!(err(r(720.0)).contains("does not rotate"));
    assert!(err(r(f64::INFINITY)).contains("finite"));
    let lossless_with_interp = plan::rotate(&RotateParams { angle: 90.0, interpolation: Some(Interpolation::Nearest), index: None });
    assert!(err(lossless_with_interp).contains("lossless"));
}

#[test]
fn flips_and_adjustments_map_to_engine_commands() {
    assert_eq!(plan::flip(&FlipParams { direction: FlipDirection::Horizontal, index: None }).id, "image.imageRotation.flipCanvasHorizontal");
    assert_eq!(plan::flip(&FlipParams { direction: FlipDirection::Vertical, index: None }).id, "image.imageRotation.flipCanvasVertical");
    let a = |amount: f64| AmountParams { amount, index: None };
    assert_eq!(ok(plan::brightness(&a(40.0))).params, json!({"brightness": 40.0, "contrast": 0}));
    assert_eq!(ok(plan::contrast(&a(-20.0))).params, json!({"brightness": 0, "contrast": -20.0}));
    assert_eq!(ok(plan::saturation(&a(-100.0))).params, json!({"saturation": -100.0}));
    assert!(err(plan::brightness(&a(151.0))).contains("between -150 and 150"));
    assert!(err(plan::contrast(&a(-51.0))).contains("between -50 and 100"));
    assert!(err(plan::saturation(&a(f64::NAN))).contains("finite"));
}

#[test]
fn blur_and_sharpen_kinds() {
    let s = ok(plan::blur(&BlurParams::default()));
    assert_eq!((s.id, s.params), ("filter.blur.gaussianBlur", json!({"radius": 2.0})));
    let s = ok(plan::blur(&BlurParams { kind: BlurKind::Motion, angle: Some(45.0), distance: Some(20.0), ..Default::default() }));
    assert_eq!((s.id, s.params), ("filter.blur.motionBlur", json!({"angle": 45.0, "distance": 20.0})));
    assert!(err(plan::blur(&BlurParams { kind: BlurKind::Motion, radius: Some(3.0), ..Default::default() })).contains("does not apply"));
    assert!(err(plan::blur(&BlurParams { radius: Some(5000.0), ..Default::default() })).contains("between"));
    assert_eq!(ok(plan::blur(&BlurParams { kind: BlurKind::Surface, ..Default::default() })).id, "filter.blur.surfaceBlur");
    assert_eq!(ok(plan::blur(&BlurParams { kind: BlurKind::Box, radius: Some(4.0), ..Default::default() })).params, json!({"radius": 4.0}));

    let s = ok(plan::sharpen(&SharpenParams::default()));
    assert_eq!((s.id, s.params), ("filter.sharpen.unsharpMask", json!({"amount": 50.0, "radius": 1.0, "threshold": 0.0})));
    assert_eq!(ok(plan::sharpen(&SharpenParams { kind: SharpenKind::Smart, ..Default::default() })).params["reduceNoise"], 10.0);
    assert!(err(plan::sharpen(&SharpenParams { kind: SharpenKind::Basic, amount: Some(3.0), ..Default::default() })).contains("does not apply"));
    assert!(err(plan::sharpen(&SharpenParams { kind: SharpenKind::Smart, radius: Some(65.0), ..Default::default() })).contains("between"));
}

#[test]
fn text_validation() {
    let t = TextParams {
        text: "Hello".into(),
        x: 10.0,
        y: 40.0,
        size: Some(24.0),
        color: Some("#ff0000".into()),
        align: Some(TextAlign::Center),
        ..Default::default()
    };
    let s = ok(plan::text(&t));
    assert_eq!(s.id, "type.create");
    assert_eq!(s.params["align"], "center");
    assert_eq!(s.params["color"], "#ff0000");
    assert!(err(plan::text(&TextParams { text: "  ".into(), ..Default::default() })).contains("empty"));
    assert!(err(plan::text(&TextParams { text: "a".into(), color: Some("red".into()), ..Default::default() })).contains("#rrggbb"));
    assert!(err(plan::text(&TextParams { text: "a".repeat(10_001), ..Default::default() })).contains("longer"));
    assert!(plan::is_hex_color("#A0b1C2"));
    assert!(!plan::is_hex_color("#abc"));
    assert!(!plan::is_hex_color("#gggggg"));
}

#[test]
fn batch_naming_and_filters() {
    assert_eq!(ok(plan::batch_output_name("photo.JPG", "", None)), "photo.jpg");
    assert_eq!(ok(plan::batch_output_name("photo.jpg", "_small", Some("webp"))), "photo_small.webp");
    assert_eq!(ok(plan::batch_output_name("a.b.png", "", Some(".PNG"))), "a.b.png");
    assert!(err(plan::batch_output_name("noext", "", None)).contains("no extension"));
    assert!(err(plan::batch_output_name("a.png", "/../x", None)).contains("separators"));
    assert!(plan::batch_wants("a.png", None));
    assert!(plan::batch_wants("b.PSD", None));
    assert!(!plan::batch_wants("notes.txt", None));
    assert!(!plan::batch_wants(".hidden.png", None));
    assert!(plan::batch_wants("c.jpeg", Some(&["jpeg".to_string()])));
    assert!(!plan::batch_wants("c.png", Some(&[".jpeg".to_string()])));
    assert_eq!(plan::join("out/", "a.png"), "out/a.png");
    assert_eq!(plan::join(".", "a.png"), "a.png");
    assert_eq!(plan::join("", "a.png"), "a.png");
}

#[test]
fn batch_operations_deserialize_strictly() {
    let ops: Vec<BatchOp> = serde_json::from_value(json!([
        {"tool": "resize_image", "width": 100},
        {"tool": "apply_blur", "kind": "box", "radius": 3},
        {"tool": "remove_background"}
    ]))
    .unwrap();
    assert_eq!(ops.iter().map(BatchOp::name).collect::<Vec<_>>(), ["resize_image", "apply_blur", "remove_background"]);
    match &ops[2] {
        BatchOp::RemoveBackground(p) => assert!(p.refine && p.apply_mask && !p.trim),
        other => panic!("unexpected {other:?}"),
    }
    assert!(serde_json::from_value::<BatchOp>(json!({"tool": "resize_image", "widht": 100})).is_err(), "unknown fields are rejected");
    assert!(serde_json::from_value::<BatchOp>(json!({"tool": "run_shell", "cmd": "rm"})).is_err(), "unknown tools are rejected");
}

#[test]
fn replies_are_measured_with_their_duplicated_text() {
    // Just over half the response budget: fits once, but not as structuredContent + text.
    let half = crate::budgets::MAX_RESPONSE_BYTES / 2 + 1024;
    let r = structured_ok("batch_process", json!({"files": "x".repeat(half)}));
    assert_eq!(r.is_error, Some(true), "an oversized reply becomes an error");
    let sc = r.structured_content.unwrap();
    assert!(sc["error"].as_str().unwrap().contains("may have completed"), "{sc}");

    let small = structured_ok("resize_image", json!({"result": 1}));
    assert_eq!(small.is_error, Some(false));
    assert_eq!(small.structured_content.unwrap()["ok"], true);

    // A huge error text is cut, never dropped.
    let r = structured_err("open_image", &"e".repeat(crate::budgets::MAX_RESPONSE_BYTES));
    let msg = r.structured_content.unwrap()["error"].as_str().unwrap().to_owned();
    assert!(msg.ends_with("(error text truncated)") && msg.chars().count() < MAX_ERROR_CHARS + 64);
}

fn png_header(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

#[test]
fn header_sizes_are_read_without_decoding() {
    assert_eq!(plan::header_size(&png_header(640, 480)), Some((640, 480)));
    // PSD: signature, version, 6 reserved, channels, height, width.
    let mut psd = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
    psd.extend_from_slice(&300u32.to_be_bytes());
    psd.extend_from_slice(&500u32.to_be_bytes());
    assert_eq!(plan::header_size(&psd), Some((500, 300)));
    // JPEG: SOI, an APP0 segment to skip, then SOF0 with height 1080 and width 1920.
    let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0xAA, 0xBB];
    jpg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x04, 0x38, 0x07, 0x80, 0x03]);
    assert_eq!(plan::header_size(&jpg), Some((1920, 1080)));
    // A DHT (0xC4) is not a frame header.
    let dht_only = vec![0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x02, 0xFF, 0xD9];
    assert_eq!(plan::header_size(&dht_only), None);
    // Truncated or unknown headers say nothing (and never panic).
    for junk in [&b""[..], b"\x89PNG", b"8BPS\0\x01", &[0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF], b"hello world"] {
        assert_eq!(plan::header_size(junk), None, "{junk:?}");
    }
}

#[test]
fn declared_sizes_over_the_budget_are_refused() {
    assert!(plan::check_declared_size("a.png", Some((16_000, 16_000))).is_ok(), "256 million pixels pass");
    let e = plan::check_declared_size("big.png", Some((20_000, 20_000))).unwrap_err();
    assert!(e.contains("20000 x 20000") && e.contains("not decoded"), "{e}");
    assert!(plan::check_declared_size("x", Some((u32::MAX, u32::MAX))).is_err(), "no overflow");
    assert!(plan::check_declared_size("unknown.tif", None).is_ok(), "unknown sizes are left to the decoder");
}
