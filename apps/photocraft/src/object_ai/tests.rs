use super::*;
use craft_ai_core::CapabilityId;
use craft_ai_core::job::Output;
use craft_ai_core::segment::Found;

fn base() -> Request {
    Request::new(CapabilityId::parse("image.segment").unwrap())
}

fn gray_png(w: u32, h: u32, depth: png::BitDepth, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    enc.write_header().unwrap().write_image_data(data).unwrap();
    out
}

fn mask_output(png: Vec<u8>) -> Output {
    Output { payload: Payload::inline("image/png", png).unwrap(), label: None }
}

fn seg(area: Option<[f32; 4]>, clicks: Vec<(f32, f32, bool)>) -> SegmentRequest {
    SegmentRequest { rgb: vec![0; 2 * 2 * 3], width: 2, height: 2, area, clicks }
}

#[test]
fn the_picture_round_trips_through_png() {
    let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| i as u8).collect();
    let png = encode_rgb_png(&rgb, 4, 3).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (4, 3, png::ColorType::Rgb, png::BitDepth::Eight));
    assert_eq!(&buf[..info.buffer_size()], &rgb[..]);
}

#[test]
fn a_picture_of_the_wrong_size_is_refused() {
    assert!(encode_rgb_png(&[0; 5], 2, 2).is_err());
    assert!(encode_rgb_png(&[], 0, 0).is_err());
    assert!(encode_rgb_png(&[0; 12], u32::MAX, u32::MAX).is_err());
}

#[test]
fn the_request_carries_the_picture_box_and_clicks() {
    let r = build_request(base(), vec![1, 2, 3], &seg(Some([0.1, 0.2, 0.6, 0.9]), vec![(0.5, 0.5, true), (0.1, 0.1, false)])).unwrap();
    assert_eq!(r.inputs.len(), 1);
    assert_eq!(r.inputs[0].role, InputRole::Source);
    assert_eq!(segment::area(&r).unwrap(), Some(Area { x0: 0.1, y0: 0.2, x1: 0.6, y1: 0.9 }));
    assert_eq!(segment::clicks(&r).unwrap(), vec![Click { x: 0.5, y: 0.5, positive: true }, Click { x: 0.1, y: 0.1, positive: false }]);
    let r = build_request(base(), vec![1], &seg(None, Vec::new())).unwrap();
    assert_eq!(segment::area(&r).unwrap(), None);
    assert!(segment::clicks(&r).unwrap().is_empty());
}

#[test]
fn a_gray_mask_decodes_at_its_size() {
    let png = gray_png(2, 2, png::BitDepth::Eight, png::ColorType::Grayscale, &[0, 255, 255, 0]);
    assert_eq!(decode_mask(&png, 2, 2).unwrap(), vec![0, 255, 255, 0]);
}

#[test]
fn other_mask_formats_are_converted() {
    let sixteen = gray_png(2, 1, png::BitDepth::Sixteen, png::ColorType::Grayscale, &[0xFF, 0xFF, 0x12, 0x34]);
    assert_eq!(decode_mask(&sixteen, 2, 1).unwrap(), vec![0xFF, 0x12]);
    let rgb = gray_png(2, 1, png::BitDepth::Eight, png::ColorType::Rgb, &[255, 255, 255, 0, 0, 0]);
    assert_eq!(decode_mask(&rgb, 2, 1).unwrap(), vec![255, 0]);
    let alpha = gray_png(1, 1, png::BitDepth::Eight, png::ColorType::GrayscaleAlpha, &[200, 10]);
    assert_eq!(decode_mask(&alpha, 1, 1).unwrap(), vec![200]);
}

#[test]
fn a_mask_of_another_size_or_garbage_is_refused() {
    let png = gray_png(2, 2, png::BitDepth::Eight, png::ColorType::Grayscale, &[0, 255, 255, 0]);
    assert!(decode_mask(&png, 3, 2).is_err());
    assert!(decode_mask(b"not a png", 2, 2).is_err());
    assert!(decode_mask(&png[..png.len() / 2], 2, 2).is_err());
    assert!(decode_mask(&[], 0, 0).is_err());
}

#[test]
fn the_answer_follows_the_found_output() {
    let png = gray_png(2, 2, png::BitDepth::Eight, png::ColorType::Grayscale, &[0, 255, 255, 0]);
    // No Found output: matched when any pixel is selected.
    let a = answer_from_outputs(&[mask_output(png.clone())], 2, 2).unwrap();
    assert!(a.matched);
    assert_eq!(a.mask, vec![0, 255, 255, 0]);
    let empty = gray_png(2, 2, png::BitDepth::Eight, png::ColorType::Grayscale, &[0; 4]);
    assert!(!answer_from_outputs(&[mask_output(empty)], 2, 2).unwrap().matched);
    // The tool's own word wins.
    let no = segment::found_output(&Found { matched: false, score: None, name: None }).unwrap();
    assert!(!answer_from_outputs(&[mask_output(png.clone()), no], 2, 2).unwrap().matched);
    let yes = segment::found_output(&Found { matched: true, score: Some(0.9), name: None }).unwrap();
    assert!(answer_from_outputs(&[mask_output(png), yes], 2, 2).unwrap().matched);
}

#[test]
fn no_mask_is_an_error() {
    let no = segment::found_output(&Found { matched: true, score: None, name: None }).unwrap();
    assert!(answer_from_outputs(&[no], 2, 2).is_err());
    assert!(answer_from_outputs(&[], 2, 2).is_err());
}

#[test]
fn only_unavailable_or_off_computer_models_are_reported() {
    let why = status_reason(RoleStatus::Unavailable { reason: "Set up Select subject in the launcher.".into() });
    assert_eq!(why.as_deref(), Some("Set up Select subject in the launcher."));
    assert!(REMOTE_REFUSAL.contains("only uses a model on this computer"));
}

/// Runs the real tool on `CRAFT_TEST_PICTURE` (a PNG path), when the launcher has Select
/// subject set up on this computer: `cargo test -p photocraft -- --ignored object_ai`.
#[test]
#[ignore = "needs the model set up on this computer and CRAFT_TEST_PICTURE"]
fn live_model_returns_a_mask_of_the_picture_size() {
    let Ok(path) = std::env::var("CRAFT_TEST_PICTURE") else { return };
    let file = std::fs::File::open(&path).unwrap();
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let channels = info.color_type.samples();
    let rgb: Vec<u8> = buf[..info.buffer_size()].chunks_exact(channels).flat_map(|p| if channels >= 3 { [p[0], p[1], p[2]] } else { [p[0]; 3] }).collect();
    let model = SharedObjectModel::standard().unwrap();
    assert_eq!(model.unavailable(), None, "set up Select subject in the ArtCraft Launcher first");
    let req = SegmentRequest { rgb, width: info.width, height: info.height, area: Some([0.1, 0.1, 0.9, 0.9]), clicks: Vec::new() };
    let answer = model.segment(&req, &|| false).unwrap();
    assert_eq!(answer.mask.len(), (info.width * info.height) as usize);
}
