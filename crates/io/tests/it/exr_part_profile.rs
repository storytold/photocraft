//! Selecting an EXR part must interpret its linear samples like ordinary File > Open.

use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_io::{ExportOptions, ImportOptions, ImportResult, export, exr_parts, import, import_file};
use photocraft_raster::Interrupt;

fn exr(layout: ChannelLayout, depth: CodecSample) -> Vec<u8> {
    // Exactly representable in half precision, including negative and HDR luminance.
    let samples: Vec<f32> = [-0.25, 0.25, 0.5, 2.0]
        .into_iter()
        .zip([1.0, 0.5, 0.25, 0.0])
        .flat_map(|(v, a)| match layout {
            ChannelLayout::Gray => vec![v],
            ChannelLayout::GrayA => vec![v, a],
            ChannelLayout::Rgb => vec![v, 0.125, 0.75],
            ChannelLayout::Rgba => vec![v, 0.125, 0.75, a],
            _ => panic!("not an EXR layout"),
        })
        .collect();
    let image = Image::from_f32(4, 1, layout, &samples).unwrap().convert(layout, depth);
    codecs::encode(&image, Format::OpenExr, &Default::default()).unwrap()
}

fn selected_part(bytes: &[u8]) -> ImportResult {
    let entries = exr_parts::entries(bytes).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, exr_parts::EntryKind::Part);
    let selection = exr_parts::Selection { parts: vec![entries[0].name.clone()], groups: vec![], view: "both".into(), precomp: false };
    exr_parts::import("selected", bytes, &selection).unwrap()
}

fn samples(doc: &Document) -> Vec<f32> {
    assert_eq!(doc.layers.len(), 1);
    doc.layers[0].surface().unwrap().read_region(doc.bounds())
}

/// File > Open of the file from disk (the band decoder).
fn opened(bytes: &[u8]) -> ImportResult {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("photocraft-exr-part-profile-{}-{n}.exr", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let result = import_file("ordinary.exr", &path, &ImportOptions::default(), &Interrupt::NONE);
    let _ = std::fs::remove_file(&path);
    result.unwrap()
}

/// A gray part opens as RGB 32-bit linear sRGB like File > Open (#14069).
#[test]
fn selected_gray_exr_part_keeps_linear_profile_and_float_samples() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA] {
            let bytes = exr(layout, depth);
            let selected = selected_part(&bytes);
            let ordinary = opened(&bytes);
            let doc = &selected.document;
            assert_eq!((doc.mode, doc.depth), (ColorMode::Rgb, SampleType::F32));
            assert_eq!(doc.size, ordinary.document.size);
            assert_eq!(selected.warnings, ordinary.warnings);
            assert_eq!(selected.warnings.len(), usize::from(depth == CodecSample::F16));
            let expected = codecs::decode(&bytes).unwrap().convert(ChannelLayout::Rgba, CodecSample::F32).to_f32_samples().unwrap();
            assert_eq!(samples(doc), expected, "{layout:?} {depth:?}: gray in R, G and B; alpha unchanged");
            assert_eq!(samples(doc), samples(&ordinary.document));
            assert_eq!(doc.icc_profile, Some(Builtin::LinearSrgb.profile().to_bytes()), "{layout:?} {depth:?}: linear light");
            assert_eq!(doc.icc_profile, ordinary.document.icc_profile);

            // The tagged samples convert to display sRGB, rather than being interpreted as sRGB.
            let profile = Profile::parse(doc.icc_profile.as_deref().unwrap()).unwrap();
            assert_eq!(profile.color_space, ColorSpace::Rgb);
            let transform = Transform::new(&profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
            let mut display = samples(doc);
            transform.apply(&mut display, 4).unwrap();
            for i in [1, 2] {
                let expected_gray = photocraft_color::convert::linear_to_srgb(expected[i * 4]);
                for c in 0..3 {
                    assert!((display[i * 4 + c] - expected_gray).abs() < 1e-4, "{layout:?} {depth:?}: display gray");
                }
                assert_eq!(display[i * 4 + 3], expected[i * 4 + 3], "alpha is not gamma encoded");
            }
        }
    }
}

#[test]
fn selected_gray_exr_part_exports_like_ordinary_import() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA] {
            let bytes = exr(layout, depth);
            let selected = selected_part(&bytes).document;
            let ordinary = opened(&bytes).document;
            for ext in ["exr", "tga"] {
                let output = export(&selected, ext, &ExportOptions::default()).unwrap();
                let control = export(&ordinary, ext, &ExportOptions::default()).unwrap();
                let output = codecs::decode(&output.bytes).unwrap();
                let control = codecs::decode(&control.bytes).unwrap();
                assert_eq!(output.layout(), control.layout());
                assert_eq!(output.to_normalized(), control.to_normalized(), "{layout:?} {depth:?} {ext}: same colours as File > Open");
                let output = output.convert(ChannelLayout::Rgba, CodecSample::F32).to_f32_samples().unwrap();
                let original = samples(&selected);
                if ext == "exr" {
                    assert_eq!(output, original, "linear EXR samples, including negative/HDR and alpha, survive re-export");
                } else {
                    for i in [1, 2] {
                        let expected = photocraft_color::convert::linear_to_srgb(original[i * 4]);
                        assert!((output[i * 4] - expected).abs() <= 1.0 / 255.0, "TGA uses display-encoded values");
                        assert!((output[i * 4 + 3] - original[i * 4 + 3]).abs() <= 1.0 / 255.0, "TGA preserves alpha");
                    }
                }
            }
        }
    }
}

#[test]
fn selected_rgb_exr_part_keeps_linear_srgb() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Rgb, ChannelLayout::Rgba] {
            let bytes = exr(layout, depth);
            let doc = selected_part(&bytes).document;
            let ordinary = import("ordinary.exr", &bytes).unwrap().document;
            assert_eq!((doc.mode, doc.depth), (ColorMode::Rgb, SampleType::F32));
            assert_eq!(doc.icc_profile, Some(Builtin::LinearSrgb.profile().to_bytes()));
            assert_eq!(doc.icc_profile, ordinary.icc_profile);
            let expected = codecs::decode(&bytes).unwrap().convert(ChannelLayout::Rgba, CodecSample::F32).to_f32_samples().unwrap();
            assert_eq!(samples(&doc), expected, "{layout:?} {depth:?}: RGB and alpha stay unchanged");
            assert_eq!(samples(&doc), samples(&ordinary));
        }
    }
}
