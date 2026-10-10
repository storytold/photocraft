//! ICC profiles through import/export: byte-exact PSD round trips with real profiles, PNG/JPEG
//! embedding and extraction, colour-managed CMYK → RGB for formats without CMYK.

mod common;

use std::sync::Arc;

use common::*;
use photocraft_cms::{Builtin, ColorSpace, Profile};
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::*;

fn single(mode: ColorMode, depth: SampleType, alpha: bool) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("s", photocraft_geom::Size::new(9, 6), mode, depth);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, alpha));
    d
}

fn real_profiles() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Builtin::ALL.iter().map(|b| b.profile().to_bytes().to_vec()).collect();
    for p in ["/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc", "/System/Library/ColorSync/Profiles/AdobeRGB1998.icc"] {
        if let Ok(b) = std::fs::read(p) {
            v.push(b);
        }
    }
    v
}

#[test]
fn psd_icc_roundtrip_byte_exact() {
    for icc in real_profiles() {
        let p = Profile::parse(&icc).unwrap();
        let mode = match p.color_space {
            ColorSpace::Cmyk => ColorMode::Cmyk,
            ColorSpace::Gray => ColorMode::Grayscale,
            ColorSpace::Lab => ColorMode::Lab,
            _ => ColorMode::Rgb,
        };
        let mut d = gen_doc(mode, SampleType::U8, Features::PIXELS);
        d.icc_profile = Some(Arc::new(icc.clone()));
        let r = export(&d, "x.psd", &ExportOptions::default()).unwrap();
        let back = import("x.psd", &r.bytes).unwrap().document;
        assert_eq!(back.icc_profile.as_deref(), Some(&icc), "{}", p.description);
    }
}

#[test]
fn png_and_jpeg_embed_and_extract_icc() {
    for b in [Builtin::Srgb, Builtin::DisplayP3, Builtin::AdobeRgbCompat, Builtin::ProPhotoCompat] {
        let icc = b.profile().to_bytes().to_vec();
        for (name, alpha) in [("x.png", true), ("x.jpg", false)] {
            let mut d = single(ColorMode::Rgb, SampleType::U8, alpha);
            d.icc_profile = Some(Arc::new(icc.clone()));
            let r = export(&d, name, &ExportOptions::default()).unwrap();
            let back = import(name, &r.bytes).unwrap().document;
            assert_eq!(back.icc_profile.as_deref(), Some(&icc), "{b:?} {name}");
            assert_eq!(Profile::parse(back.icc_profile.as_ref().unwrap()).unwrap().description, b.description());
        }
    }
    // Gray PNG with a gray profile.
    let icc = Builtin::GrayGamma22.profile().to_bytes().to_vec();
    let mut d = single(ColorMode::Grayscale, SampleType::U16, false);
    d.icc_profile = Some(Arc::new(icc.clone()));
    let r = export(&d, "g.png", &ExportOptions::default()).unwrap();
    assert_eq!(import("g.png", &r.bytes).unwrap().document.icc_profile.as_deref(), Some(&icc));
}

#[test]
fn cmyk_to_png_is_colour_managed_and_tagged_srgb() {
    // Native single-layer CMYK document written to PNG: converted through the CMYK profile.
    let mut d = single(ColorMode::Cmyk, SampleType::U8, false);
    d.icc_profile = Some(Builtin::CoatedCmyk.profile().to_bytes());
    let r = export(&d, "c.png", &ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("sRGB")), "{:?}", r.warnings);
    let back = import("c.png", &r.bytes).unwrap().document;
    assert_eq!(back.mode, ColorMode::Rgb);
    let icc = back.icc_profile.expect("tagged");
    assert_eq!(Profile::parse(&icc).unwrap().description, Builtin::Srgb.profile().description);
    // Lab documents never write Lab numbers as RGB.
    let d = single(ColorMode::Lab, SampleType::U8, false);
    let r = export(&d, "l.png", &ExportOptions::default()).unwrap();
    let back = import("l.png", &r.bytes).unwrap().document;
    assert_eq!(Profile::parse(back.icc_profile.as_ref().unwrap()).unwrap().color_space, ColorSpace::Rgb);
}

#[test]
fn cmyk_fills_survive_psd_roundtrip_at_every_depth() {
    use photocraft_doc::{Color, Document, Fill, GradientStyle, Layer, LayerContent};
    let icc = photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
        tvi: [0.26, 0.26, 0.26, 0.3],
        grid_a2b: 5,
        grid_b2a: 9,
        ..Default::default()
    })
    .to_bytes();
    let a = Color { mode: ColorMode::Cmyk, c: [0.2, 0.4, 0.6, 0.3], alpha: 0.7 };
    let b = Color { mode: ColorMode::Cmyk, c: [0.6, 0.2, 0.3, 0.1], alpha: 0.4 };
    for depth in SampleType::ALL {
        for profile in [None, Some(icc.clone())] {
            for style in [
                None,
                Some(GradientStyle::Linear),
                Some(GradientStyle::Radial),
                Some(GradientStyle::Angle),
                Some(GradientStyle::Reflected),
                Some(GradientStyle::Diamond),
            ] {
                let mut fill = style.map_or(Fill::Solid(a), |s| Fill::gradient(vec![(1.0, b), (0.0, a)], 35.0, 0.8, s, true));
                if let Fill::Gradient { midpoints, opacity_stops, dither, offset, .. } = &mut fill {
                    *midpoints = vec![0.25];
                    *opacity_stops = vec![(0.0, 0.6), (1.0, 0.9)];
                    *offset = (0.1, -0.2);
                    *dither = true;
                }
                let mut doc = Document::new("CMYK fill", photocraft_geom::Size::new(9, 6), ColorMode::Cmyk, depth);
                doc.icc_profile = profile.clone();
                doc.layers.push(Layer::new("Fill", LayerContent::Fill(fill)));
                let expected = photocraft_compose::flatten(&doc).px;
                assert!(max_diff(&expected, &photocraft_compose::render_tiled(&doc, doc.bounds(), 3).px) <= 1e-6);
                let bytes = export(&doc, "x.psd", &ExportOptions::default()).unwrap().bytes;
                let back = import("x.psd", &bytes).unwrap().document;
                let actual = photocraft_compose::flatten(&back).px;
                let diff = max_diff(&expected, &actual);
                assert!(diff <= 1e-6, "{depth:?} {style:?}: round-trip error {diff}");
                assert_eq!(back.icc_profile, doc.icc_profile);
                if style.is_none() {
                    let native = photocraft_raster::Surface::with_default(doc.pixel_format(), &[a.c[0], a.c[1], a.c[2], a.c[3], a.alpha]);
                    let actual = back.layers[0].fill_cache.as_ref().unwrap().surface.read_region(doc.bounds());
                    for (actual, expected) in actual.iter().zip(native.read_region(doc.bounds())) {
                        let tolerance = if depth == SampleType::F32 { 1e-6 } else { 0.0 };
                        assert!((actual - expected).abs() <= tolerance, "{depth:?}: {actual} vs {expected}");
                    }
                }
            }
        }
    }
}

#[cfg(feature = "corpus")]
#[test]
fn uncached_cmyk_gradient_corpus_roundtrip() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd-tools/colormodes/4x4_16bit_cmyk.psd");
    let bytes = std::fs::read(path).expect("run cargo xtask corpus --all");
    let doc = import("x.psd", &bytes).unwrap().document;
    assert!(doc.layers.iter().any(|l| matches!(l.content, photocraft_doc::LayerContent::Fill(_)) && l.fill_cache.is_none()));
    let expected = photocraft_compose::flatten(&doc).px;
    let saved = export(&doc, "x.psd", &ExportOptions::default()).unwrap().bytes;
    let back = import("x.psd", &saved).unwrap().document;
    let diff = max_diff(&expected, &photocraft_compose::flatten(&back).px);
    eprintln!("uncached 16-bit CMYK gradient round-trip max error: {diff:.8}");
    assert!(diff <= 1e-6, "round-trip error {diff}");
    assert_eq!(back.icc_profile, doc.icc_profile);
}

#[test]
fn rgb_fill_colours_separate_through_the_document_cmyk_profile() {
    use photocraft_cms::{Intent, Transform, synth};
    use photocraft_doc::{Color, Document, Fill, Layer, LayerContent};
    let profile = synth::cmyk_profile(&synth::CmykParams { tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() });
    let profile = Profile::parse(&profile.to_bytes()).unwrap();
    let rgb = [0.2, 0.4, 0.6];
    let mut expected = [0.0; 4];
    Transform::new(Builtin::Srgb.profile(), &profile, Intent::RelativeColorimetric, true).unwrap().eval_fast(&rgb, &mut expected);
    for depth in SampleType::ALL {
        let mut doc = Document::new("RGB fill", photocraft_geom::Size::new(1, 1), ColorMode::Cmyk, depth);
        doc.icc_profile = Some(profile.to_bytes());
        doc.layers.push(Layer::new("Fill", LayerContent::Fill(Fill::Solid(Color::rgb(rgb[0], rgb[1], rgb[2])))));
        let bytes = export(&doc, "x.psd", &ExportOptions::default()).unwrap().bytes;
        let back = import("x.psd", &bytes).unwrap().document;
        let samples = back.layers[0].fill_cache.as_ref().unwrap().surface.read_region(back.bounds());
        let tolerance = match depth {
            SampleType::U8 => 1.0 / 255.0,
            SampleType::U16 => 1.0 / 65535.0,
            SampleType::F32 => 1e-6,
        };
        for (actual, expected) in samples.iter().zip(expected) {
            assert!((actual - expected).abs() <= tolerance, "{depth:?}: {actual} vs {expected}");
        }
    }
}
