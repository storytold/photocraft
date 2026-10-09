//! Camera raw import: synthetic DNG / CR2 develop into a 16-bit ProPhoto
//! document; unsupported raw variants fall back to the embedded preview.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::{IoError, import};
use photocraft_raw::testgen::{Cr2Spec, DngSpec, TiffBuilder, Val, mosaic, scene};

#[test]
fn dng_opens_as_16_bit_prophoto() {
    let (w, h) = (40, 24);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.6, 1.0, 0.8]);
    let r = import("shot.dng", &spec.build()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (40, 24));
    assert_eq!(d.mode, ColorMode::Rgb);
    assert_eq!(d.depth, SampleType::U16);
    assert_eq!(d.layers.len(), 1);
    let icc = d.icc_profile.as_ref().expect("profile");
    assert_eq!(icc.as_slice(), &photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes()[..]);
    assert!(r.warnings.iter().any(|w| w.contains("DNG") && w.contains("ProPhoto")), "{:?}", r.warnings);
}

#[test]
fn cr2_opens() {
    let (w, h) = (48, 16);
    let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 256, 12000);
    let spec =
        Cr2Spec { width: w, height: h, data, precision: 14, components: 2, slices: vec![24, 24], borders: None, wb_rggb: None, orientation: 6, model_id: None };
    let r = import("IMG_0001.CR2", &spec.build()).unwrap();
    // Orientation 6 rotates the 48×16 sensor image to 16×48.
    assert_eq!((r.document.size.width, r.document.size.height), (16, 48));
    assert_eq!(r.document.depth, SampleType::U16);
}

/// A NEF-like file with Nikon's (undocumented) compression and a full-size
/// baseline JPEG preview in IFD0.
fn nef_with_preview() -> Vec<u8> {
    let img = Image::from_u8(32, 20, ChannelLayout::Rgb, vec![180; 32 * 20 * 3]).unwrap();
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let mut t = TiffBuilder::default();
    let strip = t.blob(vec![0; 64]);
    let preview = t.blob(jpeg.clone());
    let raw = t.ifd(vec![
        (256, Val::Long(vec![8])),
        (257, Val::Long(vec![8])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![34713])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (279, Val::Long(vec![64])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let ifd0 = t.ifd(vec![
        (271, Val::Ascii("NIKON CORPORATION".into())),
        (330, Val::Ifds(vec![raw])),
        (513, Val::Blobs(vec![preview])),
        (514, Val::Long(vec![jpeg.len() as u32])),
    ]);
    t.chain = vec![ifd0];
    t.build()
}

#[test]
fn unsupported_raw_falls_back_to_the_embedded_preview() {
    let r = import("DSC_0001.NEF", &nef_with_preview()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (32, 20));
    assert!(r.warnings.first().is_some_and(|w| w.contains("Nikon compressed NEF") && w.contains("embedded")), "{:?}", r.warnings);
}

#[test]
fn unsupported_raw_without_preview_is_a_clear_error() {
    let mut cr3 = vec![0, 0, 0, 24];
    cr3.extend_from_slice(b"ftypcrx ");
    cr3.extend_from_slice(&[0; 12]);
    match import("IMG_0001.CR3", &cr3) {
        Err(e @ IoError::Raw(_)) => assert!(e.to_string().contains("CR3"), "{e}"),
        Err(e) => panic!("expected a raw error, got {e}"),
        Ok(_) => panic!("CR3 must not decode yet"),
    }
}

#[test]
fn truncated_raws_do_not_panic() {
    let b = nef_with_preview();
    for n in (0..b.len()).step_by(7) {
        let _ = import("x.nef", &b[..n]);
    }
}

/// Camera-space patches, each with a tonal ramp.
fn patches(w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut seed = 99u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let colours: Vec<[f32; 3]> = (0..96).map(|_| [0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd()]).collect();
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let ramp = 0.15 + 0.85 * ((x * 12) % w) as f32 / w as f32;
            colours[(y * 8 / h) * 12 + x * 12 / w].map(|v| v * ramp)
        })
        .collect()
}

/// A "camera JPEG" of `cam`: more saturated than sRGB primaries, with a brightening curve.
fn camera_rendering(cam: &[[f32; 3]], w: usize, h: usize) -> Vec<u8> {
    let k = [[1.3f32, -0.2, -0.1], [-0.15, 1.25, -0.1], [0.0, -0.25, 1.25]];
    let px: Vec<u8> = cam
        .iter()
        .flat_map(|c| {
            (0..3).map(move |i| {
                let v = (k[i][0] * c[0] + k[i][1] * c[1] + k[i][2] * c[2]).clamp(0.0, 1.0);
                let v = 1.6 * v / (1.0 + 0.6 * v);
                let e = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                (e * 255.0).round() as u8
            })
        })
        .collect();
    jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, px).unwrap())
}

fn jpeg(img: Image) -> Vec<u8> {
    photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap()
}

/// An uncompressed NEF-like raw of `cam` with an optional JPEG preview in IFD0.
fn nef_uncompressed(cam: &[[f32; 3]], w: usize, h: usize, preview: Option<Vec<u8>>) -> Vec<u8> {
    let data = mosaic(cam, w, [0, 1, 1, 2], 0, 4095);
    let mut t = TiffBuilder::default();
    let strip = t.blob(data.iter().flat_map(|v| v.to_le_bytes()).collect());
    let raw = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![w as u32])),
        (257, Val::Long(vec![h as u32])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![h as u32])),
        (279, Val::Long(vec![(data.len() * 2) as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let mut ifd0 = vec![(271, Val::Ascii("NIKON CORPORATION".into())), (330, Val::Ifds(vec![raw]))];
    if let Some(j) = preview {
        let len = j.len() as u32;
        let b = t.blob(j);
        ifd0.extend([(513, Val::Blobs(vec![b])), (514, Val::Long(vec![len]))]);
    }
    let ifd0 = t.ifd(ifd0);
    t.chain = vec![ifd0];
    t.build()
}

fn fitted(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("fitted to the camera's embedded JPEG"))
}

fn fallback(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("sRGB primaries"))
}

#[test]
fn uncalibrated_raw_takes_its_colour_from_the_camera_jpeg() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let r = import("DSC_0002.NEF", &nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)))).unwrap();
    assert!(fitted(&r.warnings) && !fallback(&r.warnings), "{:?}", r.warnings);
    assert_eq!((r.document.size.width, r.document.size.height), (480, 320));
    assert_eq!(r.document.depth, SampleType::U16);
}

#[test]
fn unusable_previews_keep_the_neutral_fallback() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let mut seed = 5u32;
    let noise: Vec<u8> = (0..w * h * 3)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        })
        .collect();
    let mut garbage = jpeg(Image::from_u8(64, 48, ChannelLayout::Rgb, vec![90; 64 * 48 * 3]).unwrap());
    let sos = garbage.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    garbage.truncate(sos + 8);
    garbage.extend(std::iter::repeat_n(0xAB, 200));
    let previews = [
        None,
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, noise).unwrap())),
        Some(jpeg(Image::from_u8(240, 240, ChannelLayout::Rgb, vec![128; 240 * 240 * 3]).unwrap())),
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Gray, vec![128; w * h]).unwrap())),
        Some(garbage),
    ];
    for (i, p) in previews.into_iter().enumerate() {
        let r = import("DSC_0003.NEF", &nef_uncompressed(&cam, w, h, p)).unwrap();
        assert!(fallback(&r.warnings) && !fitted(&r.warnings), "preview {i}: {:?}", r.warnings);
    }
}

#[test]
fn truncated_raws_with_a_fittable_preview_do_not_panic() {
    let (w, h) = (96, 64);
    let cam = patches(w, h);
    let b = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    for n in (0..b.len()).step_by(211) {
        let _ = import("x.nef", &b[..n]);
    }
}
