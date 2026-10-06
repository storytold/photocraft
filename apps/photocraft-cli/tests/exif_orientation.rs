//! The reported headless workflow: oriented JPEG -> PNG / edited JPEG / native save.

use std::path::Path;
use std::process::Command;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, decode, encode};

fn run(input: &Path, output: &Path, command: Option<&str>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_photocraft-cli"));
    cmd.arg("run").arg(input);
    if let Some(command) = command {
        cmd.args(["--cmd", command]);
    }
    let result = cmd.arg("--out").arg(output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

fn exif(orientation: u8) -> Vec<u8> {
    vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0, 0, 0, 0, 0]
}

#[test]
fn open_and_export_consume_orientation_once_even_after_edit_or_native_save() {
    let dir = std::env::temp_dir().join(format!("photocraft-exif-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.jpg");
    // Six uniform 16x16 patches keep JPEG assertions away from lossy boundaries.
    let pixels: Vec<u8> = (0..32).flat_map(|y| (0..48).map(move |x| 20 + (y / 16 * 3 + x / 16) as u8 * 40)).collect();
    let mut img = Image::from_u8(48, 32, ChannelLayout::Gray, pixels).unwrap();
    img.meta.exif = Some(exif(6));
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 100, ..Default::default() }).unwrap();
    assert!(bytes.windows(26).any(|b| b == exif(6)));
    std::fs::write(&input, bytes).unwrap();

    let plain = dir.join("plain.png");
    run(&input, &plain, None);
    let plain_bytes = std::fs::read(&plain).unwrap();
    // PNG IHDR gives physical stored dimensions, without our orientation-aware decoder.
    assert_eq!(u32::from_be_bytes(plain_bytes[16..20].try_into().unwrap()), 32);
    assert_eq!(u32::from_be_bytes(plain_bytes[20..24].try_into().unwrap()), 48);
    assert!(plain_bytes.windows(26).any(|b| b == exif(1)));
    let upright = decode(&plain_bytes).unwrap();
    for (x, y, value) in [(8, 8, 140), (24, 8, 20), (8, 40, 220), (24, 40, 100)] {
        assert!((upright.get(x, y, 0) * 255.0 - value as f32).abs() <= 2.0);
    }

    let rotated = dir.join("rotated.jpg");
    run(&input, &rotated, Some("image.imageRotation.90cw"));
    let rotated_bytes = std::fs::read(&rotated).unwrap();
    assert!(rotated_bytes.windows(26).any(|b| b == exif(1)));
    let rotated_img = decode(&rotated_bytes).unwrap();
    // Import already made it upright; an explicit extra 90cw must now turn it landscape.
    assert_eq!(rotated_img.dimensions(), (48, 32));
    for (x, y, value) in [(8, 8, 220), (40, 8, 140), (8, 24, 100), (40, 24, 20)] {
        assert!((rotated_img.get(x, y, 0) * 255.0 - value as f32).abs() <= 3.0);
    }

    let native = dir.join("upright.pcraft");
    let reopened = dir.join("reopened.png");
    run(&input, &native, None);
    run(&native, &reopened, None);
    let reopened_bytes = std::fs::read(&reopened).unwrap();
    assert!(reopened_bytes.windows(26).any(|b| b == exif(1)));
    let back = decode(&reopened_bytes).unwrap();
    assert_eq!(back.dimensions(), upright.dimensions());
    assert_eq!(back.data(), upright.data());
    std::fs::remove_dir_all(dir).unwrap();
}
