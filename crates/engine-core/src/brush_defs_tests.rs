//! #14275: the sampled-tip limits in core: 5000 px for Define Brush Preset's tips, 2500 px for an
//! image file imported as a brush.

use serde_json::json;

use super::sampled_brush;

#[test]
fn a_painted_extent_over_5000_px_is_rejected() {
    let mut v = vec![0.0; 5001];
    v[0] = 1.0;
    v[4999] = 1.0;
    assert_eq!(sampled_brush("test", &v, 5001, 1).unwrap().1, 5000);
    v[5000] = 1.0;
    assert!(sampled_brush("test", &v, 5001, 1).is_err());
}

fn black_png(width: u32) -> String {
    let img = photocraft_codecs::Image::from_u8(width, 4, photocraft_codecs::ChannelLayout::Gray, vec![0; width as usize * 4]).unwrap();
    photocraft_paint::tile::b64_encode(&photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap())
}

#[test]
fn importing_an_image_as_a_brush_keeps_the_2500_px_limit() {
    let mut s = crate::session();
    let r = s.execute("brush.presets.importImage", json!({"data": black_png(2500), "fileName": "Bar.png"})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(2500), Some(4)));
    let presets = s.tools.presets.len();
    let err = s.execute("brush.presets.importImage", json!({"data": black_png(2501), "fileName": "Bar.png"})).unwrap_err();
    assert!(err.to_string().contains("max 2500 px"), "{err}");
    assert_eq!(s.tools.presets.len(), presets);
}
