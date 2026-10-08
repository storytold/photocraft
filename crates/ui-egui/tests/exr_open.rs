#![cfg(not(target_arch = "wasm32"))]
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_ui_egui::PhotocraftApp;

#[test]
fn synchronous_native_exr_open_uses_tile_admission_before_huge_pixel_decode() {
    let image = Image::from_f32(1, 1, ChannelLayout::Rgb, &[0.5, 2.0, -0.25]).unwrap();
    let bytes = photocraft_codecs::encode(&image, Format::OpenExr, &EncodeOptions::default()).unwrap();
    let mut header_end = 8;
    loop {
        if bytes[header_end] == 0 {
            header_end += 1;
            break;
        }
        for _ in 0..2 {
            while bytes[header_end] != 0 {
                header_end += 1;
            }
            header_end += 1;
        }
        let length = i32::from_le_bytes(bytes[header_end..header_end + 4].try_into().unwrap()) as usize;
        header_end += 4 + length;
    }
    let mut huge = bytes[..header_end].to_vec();
    let needle = b"dataWindow\0box2i\0\x10\0\0\0";
    let at = huge.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
    huge[at + 8..at + 12].copy_from_slice(&65535_i32.to_le_bytes());
    huge[at + 12..at + 16].copy_from_slice(&32767_i32.to_le_bytes());
    let offset = (header_end + 2048 * 8) as u64;
    for _ in 0..2048 {
        huge.extend_from_slice(&offset.to_le_bytes());
    }
    huge.extend_from_slice(&[0; 8]);
    let path = std::env::temp_dir().join(format!("photocraft-exr-ui-{}.exr", std::process::id()));
    std::fs::write(&path, huge).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    photocraft_raster::spill::configure(None, 0).unwrap();
    photocraft_raster::memory::configure(photocraft_raster::memory::Policy { percent: None, absolute_bytes: 1 << 30 });
    let error = app.open_path(path.to_str().unwrap()).unwrap_err();
    assert!(error.contains("decoded image tiles"), "{error}");
    assert!(app.session.documents().is_empty());
    std::fs::remove_file(path).unwrap();
}
