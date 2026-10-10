//! Flat formats through photocraft-codecs, and export warnings.

use crate::common;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::*;

fn single(mode: ColorMode, depth: SampleType, alpha: bool) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("s", photocraft_geom::Size::new(9, 6), mode, depth);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, alpha));
    d
}

fn pixels_eq(a: &photocraft_doc::Document, b: &photocraft_doc::Document, tol: f32) {
    let sa = a.layers[0].surface().unwrap();
    let sb = b.layers[0].surface().unwrap();
    let r = a.bounds();
    let (va, vb) = (sa.read_region(r), sb.read_region(r));
    assert_eq!(va.len(), vb.len());
    let m = va.iter().zip(&vb).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(m <= tol, "max diff {m}");
}

macro_rules! codec_rt {
    ($name:ident, $ext:expr, $mode:expr, $depth:expr, $alpha:expr, $tol:expr) => {
        #[test]
        fn $name() {
            let d = single($mode, $depth, $alpha);
            let r = export(&d, $ext, &ExportOptions::default()).expect("export");
            let back = import(concat!("x.", $ext), &r.bytes).expect("import").document;
            assert_eq!(back.size, d.size);
            assert_eq!(back.mode, d.mode, "mode");
            assert_eq!(back.depth, d.depth, "depth");
            pixels_eq(&d, &back, $tol);
        }
    };
}

codec_rt!(png_rgba8, "png", ColorMode::Rgb, SampleType::U8, true, 0.0);
codec_rt!(png_rgb8_opaque, "png", ColorMode::Rgb, SampleType::U8, false, 0.0);
codec_rt!(png_rgba16, "png", ColorMode::Rgb, SampleType::U16, true, 0.0);
codec_rt!(png_gray8, "png", ColorMode::Grayscale, SampleType::U8, false, 0.0);
codec_rt!(png_graya16, "png", ColorMode::Grayscale, SampleType::U16, true, 0.0);
codec_rt!(tga_rgb8_opaque, "tga", ColorMode::Rgb, SampleType::U8, false, 0.0);
codec_rt!(tga_gray8, "tga", ColorMode::Grayscale, SampleType::U8, false, 0.0);
codec_rt!(tiff_rgba8, "tiff", ColorMode::Rgb, SampleType::U8, true, 0.0);
codec_rt!(tiff_rgb16, "tif", ColorMode::Rgb, SampleType::U16, false, 0.0);
codec_rt!(tiff_cmyk8, "tiff", ColorMode::Cmyk, SampleType::U8, false, 0.0);
codec_rt!(tiff_cmyka16, "tiff", ColorMode::Cmyk, SampleType::U16, true, 0.0);
codec_rt!(tiff_gray8, "tiff", ColorMode::Grayscale, SampleType::U8, false, 0.0);

/// EXR stores linear light: a linear document round-trips exactly; an sRGB (untagged) one is
/// linearised on export and comes back tagged linear sRGB with the same colours.
#[test]
fn exr_rgba32() {
    let mut d = single(ColorMode::Rgb, SampleType::F32, true);
    d.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    let r = export(&d, "exr", &ExportOptions::default()).expect("export");
    let back = import("x.exr", &r.bytes).expect("import").document;
    assert_eq!((back.size, back.mode, back.depth), (d.size, d.mode, d.depth));
    assert_eq!(back.icc_profile.as_deref(), d.icc_profile.as_deref(), "tagged linear sRGB");
    pixels_eq(&d, &back, 0.0);
    // Untagged (sRGB) → linearised.
    let srgb = single(ColorMode::Rgb, SampleType::F32, true);
    let r = export(&srgb, "exr", &ExportOptions::default()).expect("export");
    let back = import("x.exr", &r.bytes).expect("import").document;
    let (a, b) = (srgb.layers[0].surface().unwrap().pixel(2, 3), back.layers[0].surface().unwrap().pixel(2, 3));
    for c in 0..3 {
        assert!((photocraft_color::convert::srgb_to_linear(a[c]) - b[c]).abs() < 1e-4, "{a:?} -> {b:?}");
    }
    assert!((a[3] - b[3]).abs() < 1e-6, "alpha kept");
}

/// An opaque document filled with `#808080`.
fn mid_gray(mode: ColorMode, depth: SampleType) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("g", photocraft_geom::Size::new(4, 4), mode, depth);
    let fmt = d.pixel_format();
    let mut s = photocraft_raster::Surface::new(fmt);
    let v = 128.0 / 255.0;
    s.fill_rect(d.bounds(), &photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
    d.layers.push(photocraft_doc::Layer::new("Background", photocraft_doc::LayerContent::Raster(s)));
    d
}

/// OpenEXR and Radiance HDR store linear sRGB in every mode: an sGray `#808080` is written as
/// its linear value, as an RGB `#808080` is, rather than as the display-encoded 0.5 (#2386).
#[test]
fn gray_exr_and_hdr_store_linear_light() {
    let linear = photocraft_color::convert::srgb_to_linear(128.0 / 255.0);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (ext, tol) in [("exr", 2e-5), ("hdr", 2e-3)] {
            for mode in [ColorMode::Grayscale, ColorMode::Rgb] {
                let r = export(&mid_gray(mode, depth), ext, &ExportOptions::default()).expect("export");
                let img = photocraft_codecs::decode(&r.bytes).expect("decode");
                let samples = img.to_normalized();
                let worst = samples.iter().map(|v| (v - linear).abs()).fold(0.0f32, f32::max);
                assert!(worst <= tol, "{mode:?} {depth:?} .{ext}: stored samples differ from {linear} by {worst}");
            }
        }
    }
}

/// A grayscale OpenEXR holds linear luminance, so it opens as grayscale tagged with a linear gray
/// profile, in which its samples are the colours saved.
#[test]
fn gray_exr_round_trips() {
    use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let r = export(&mid_gray(ColorMode::Grayscale, depth), "exr", &ExportOptions::default()).expect("export");
        let back = import("x.exr", &r.bytes).expect("import").document;
        assert_eq!(back.mode, ColorMode::Grayscale, "{depth:?}");
        let profile = Profile::parse(back.icc_profile.as_deref().expect("tagged")).expect("profile");
        assert_eq!(profile.color_space, ColorSpace::Gray, "{depth:?}");
        let t = Transform::new(&profile, Builtin::SGray.profile(), Intent::RelativeColorimetric, false).expect("transform");
        let mut v = [back.layers[0].surface().expect("raster").pixel(1, 1)[0]];
        t.apply(&mut v, 1);
        assert!((v[0] - 128.0 / 255.0).abs() < 1e-4, "{depth:?}: reopened as {} in sGray", v[0]);
    }
}

fn smooth(mode: ColorMode) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("s", photocraft_geom::Size::new(32, 16), mode, SampleType::U8);
    let fmt = d.pixel_format();
    let mut s = photocraft_raster::Surface::new(fmt);
    let n = fmt.channels();
    let mut v = Vec::new();
    for y in 0..16 {
        for x in 0..32 {
            for c in 0..n - 1 {
                v.push(((x * 4 + y * 2 + c * 20) as f32 / 255.0).min(1.0));
            }
            v.push(1.0);
        }
    }
    s.write_region(d.bounds(), &v);
    d.layers.push(photocraft_doc::Layer::new("Background", photocraft_doc::LayerContent::Raster(s)));
    d
}

fn jpeg_rt(mode: ColorMode, ext: &str) {
    let d = smooth(mode);
    let r = export(&d, ext, &ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.to_lowercase().contains("lossy")), "{:?}", r.warnings);
    let back = import("x.jpg", &r.bytes).unwrap().document;
    assert_eq!(back.mode, mode);
    let (a, b) = (d.layers[0].surface().unwrap().read_region(d.bounds()), back.layers[0].surface().unwrap().read_region(d.bounds()));
    let mean = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32;
    assert!(mean < 0.02, "mean error {mean}");
}

#[test]
fn jpeg_rgb8() {
    jpeg_rt(ColorMode::Rgb, "jpg");
}

#[test]
fn jpeg_gray8() {
    jpeg_rt(ColorMode::Grayscale, "jpeg");
}

#[test]
fn metadata_roundtrip_png() {
    let mut d = single(ColorMode::Rgb, SampleType::U8, false);
    d.resolution_dpi = 300.0;
    d.icc_profile = Some(std::sync::Arc::new(sample_icc()));
    d.metadata.xmp = Some("<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".into());
    // DPI and ICC always travel with the file; XMP unless Export As asks for none (#647).
    let none = ExportOptions { xmp: photocraft_io::XmpEmbed::None, ..ExportOptions::default() };
    let r = export(&d, "png", &none).unwrap();
    let back = import("x.png", &r.bytes).unwrap().document;
    assert!((back.resolution_dpi - 300.0).abs() < 1.0);
    assert_eq!(back.icc_profile, d.icc_profile);
    assert_eq!(back.metadata.xmp, None);
    let r = export(&d, "png", &ExportOptions::default()).unwrap();
    let back = import("x.png", &r.bytes).unwrap().document;
    assert_eq!(back.metadata.xmp, d.metadata.xmp);
}

fn sample_icc() -> Vec<u8> {
    // Arbitrary bytes are fine for PNG iCCP (the codec does not validate).
    let mut v = vec![0u8; 128];
    v[36..40].copy_from_slice(b"acsp");
    v
}

#[test]
fn layered_to_png_warns_flatten() {
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    let r = export(&d, "out.png", &ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("flattened")), "{:?}", r.warnings);
    let back = import("out.png", &r.bytes).unwrap().document;
    // Composite equals our flatten.
    let flat = photocraft_compose::flatten(&d).px;
    let s = back.layers[0].surface().unwrap();
    let px: Vec<[f32; 4]> = s.read_region(back.bounds()).as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2], p[3]]).collect();
    assert!(max_diff(&px, &flat) <= 1.0 / 255.0 + 1e-4);
}

#[test]
fn sixteen_bit_to_jpeg_warns_depth() {
    let d = single(ColorMode::Rgb, SampleType::U16, false);
    let r = export(&d, "a.jpg", &ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.to_lowercase().contains("bit")), "{:?}", r.warnings);
}

#[test]
fn alpha_to_jpeg_warns() {
    let d = single(ColorMode::Rgb, SampleType::U8, true);
    let r = export(&d, "a.jpg", &ExportOptions::default()).unwrap();
    assert!(r.warnings.len() >= 2, "{:?}", r.warnings);
}

#[test]
fn cmyk_layered_to_png_warns_conversion() {
    let d = gen_doc(ColorMode::Cmyk, SampleType::U8, Features::PIXELS);
    let r = export(&d, "a.png", &ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("RGB")), "{:?}", r.warnings);
}

#[test]
fn effects_are_written_to_psd_and_read_back() {
    let mut d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    d.layers[1].effects.items.push(photocraft_doc::Effect::ColorOverlay {
        common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
        color: photocraft_color::Color::BLACK,
    });
    d.layers[1].effects.items.push(photocraft_doc::Effect::default_drop_shadow());
    let r = export(&d, "a.psd", &ExportOptions::default()).unwrap();
    assert!(!r.warnings.iter().any(|w| w.contains("effects")), "{:?}", r.warnings);
    let back = import("a.psd", &r.bytes).unwrap().document;
    assert_eq!(back.layers[1].effects.items.len(), 2);
    assert!(back.layers[1].effects.psd_raw.is_some());
}

#[test]
fn unknown_extension_errors() {
    let d = single(ColorMode::Rgb, SampleType::U8, false);
    assert!(matches!(export(&d, "a.xyz", &ExportOptions::default()), Err(IoError::UnknownFormat(_))));
}

#[test]
fn garbage_import_errors() {
    assert!(import("x", b"definitely not an image").is_err());
    assert!(matches!(import("x.psd", b"8BPS\0\x01"), Err(IoError::Psd(_))));
}

#[test]
fn extension_forms() {
    let d = single(ColorMode::Rgb, SampleType::U8, false);
    for e in ["png", ".png", "a/b/c.PNG", "C:\\x\\y.png"] {
        let r = export(&d, e, &ExportOptions::default()).unwrap();
        assert!(r.bytes.starts_with(b"\x89PNG"), "{e}");
    }
}

/// Asserts how a flat file opened: the locked Background when opaque, a normal "Layer 0" (as in
/// Photoshop) when it has transparency.
fn assert_opens_as(what: &str, d: &photocraft_doc::Document, transparent: bool) {
    let [l] = &d.layers[..] else { panic!("{what}: {} layers", d.layers.len()) };
    if transparent {
        assert_eq!(l.name, "Layer 0", "{what}");
        assert!(!l.locks.transparency && !l.locks.position, "{what}: unlocked");
    } else {
        assert_eq!(l.name, "Background", "{what}");
        assert!(l.locks.transparency && l.locks.position, "{what}: locked");
    }
}

#[test]
fn background_lock_follows_alpha() {
    let r = export(&single(ColorMode::Rgb, SampleType::U8, false), "png", &ExportOptions::default()).unwrap();
    assert_opens_as("opaque PNG", &import("a.png", &r.bytes).unwrap().document, false);
    let r = export(&single(ColorMode::Rgb, SampleType::U8, true), "png", &ExportOptions::default()).unwrap();
    assert_opens_as("RGBA PNG", &import("a.png", &r.bytes).unwrap().document, true);
}

#[test]
fn transparent_flat_images_open_as_layer_0() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType as CS};
    let (w, h) = (6u32, 4u32);
    let n = (w * h) as usize;
    let enc = |layout: ChannelLayout, sample: CS, format: Format| {
        let ch = layout.channels();
        let img = match sample {
            CS::U8 => Image::from_u8(w, h, layout, (0..n * ch).map(|i| if (i + 1) % ch == 0 && i < ch * 5 { 0 } else { 200 }).collect()),
            _ => Image::from_u16(w, h, layout, &(0..n * ch).map(|i| if (i + 1) % ch == 0 && i < ch * 5 { 0 } else { 50_000 }).collect::<Vec<_>>()),
        }
        .unwrap();
        photocraft_codecs::encode(&img, format, &EncodeOptions::default()).unwrap()
    };
    let cases: Vec<(&str, Vec<u8>, bool)> = vec![
        ("a.png", enc(ChannelLayout::Rgba, CS::U8, Format::Png), true),
        ("a16.png", enc(ChannelLayout::Rgba, CS::U16, Format::Png), true),
        ("ga.png", enc(ChannelLayout::GrayA, CS::U8, Format::Png), true),
        ("a.tif", enc(ChannelLayout::Rgba, CS::U8, Format::Tiff), true),
        ("ga.tif", enc(ChannelLayout::GrayA, CS::U16, Format::Tiff), true),
        ("pal.png", photocraft_codecs::encode_png_indexed(w, h, &[0, 1].repeat(n / 2), &[[255, 0, 0], [0, 0, 255]], Some(0)).unwrap(), true),
        ("pal_opaque.png", photocraft_codecs::encode_png_indexed(w, h, &[0, 1].repeat(n / 2), &[[255, 0, 0], [0, 0, 255]], None).unwrap(), false),
        ("rgb.png", enc(ChannelLayout::Rgb, CS::U8, Format::Png), false),
        ("g.png", enc(ChannelLayout::Gray, CS::U16, Format::Png), false),
        ("rgb.tif", enc(ChannelLayout::Rgb, CS::U8, Format::Tiff), false),
        ("rgb.jpg", enc(ChannelLayout::Rgb, CS::U8, Format::Jpeg), false),
    ];
    for (name, bytes, transparent) in cases {
        let d = import(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e}")).document;
        assert_opens_as(name, &d, transparent);
        // Photoshop numbers the next new layer after "Layer 0" as "Layer 1".
        assert_eq!(d.next_layer_name("Layer"), "Layer 1", "{name}");
    }
}

#[test]
fn layer_0_saves_back_to_png_and_reopens_as_layer_0() {
    let r = export(&single(ColorMode::Rgb, SampleType::U8, true), "png", &ExportOptions::default()).unwrap();
    let d = import("a.png", &r.bytes).unwrap().document;
    assert_opens_as("first open", &d, true);
    let again = export(&d, "b.png", &ExportOptions::default()).unwrap();
    assert!(again.warnings.iter().all(|w| !w.contains("flattened")), "a lone Layer 0 is written natively: {:?}", again.warnings);
    let back = import("b.png", &again.bytes).unwrap().document;
    assert_opens_as("reopened", &back, true);
    pixels_eq(&d, &back, 0.0);
}

#[test]
fn psd_to_png_to_psd_chain() {
    let d = gen_doc(ColorMode::Rgb, SampleType::U16, Features::ALL);
    let psd = export(&d, "a.psd", &ExportOptions::default()).unwrap();
    let d2 = import("a.psd", &psd.bytes).unwrap().document;
    let png = export(&d2, "a.png", &ExportOptions::default()).unwrap();
    let d3 = import("a.png", &png.bytes).unwrap().document;
    assert_eq!(d3.depth, SampleType::U16);
    let psd2 = export(&d3, "b.psd", &ExportOptions::default()).unwrap();
    assert!(photocraft_psd::PsdFile::from_bytes(&psd2.bytes).is_ok());
}

/// A one-layer document of 16-pixel-wide columns, each one straight colour (colour channels, then alpha).
fn columns(mode: ColorMode, depth: SampleType, cols: &[&[f32]]) -> photocraft_doc::Document {
    let mut d = photocraft_doc::Document::new("c", photocraft_geom::Size::new(16 * cols.len() as u32, 16), mode, depth);
    let mut s = photocraft_raster::Surface::new(d.pixel_format());
    for (i, px) in cols.iter().enumerate() {
        let x = 16 * i as i32;
        s.fill_rect(photocraft_geom::Rect::new(x, 0, x + 16, 16), px);
    }
    d.layers.push(photocraft_doc::Layer::new("Layer", photocraft_doc::LayerContent::Raster(s)));
    d
}

/// The pixel at the centre of each column of `doc`.
fn column_pixels(doc: &photocraft_doc::Document, n: usize) -> Vec<Vec<f32>> {
    let s = doc.layers[0].surface().unwrap();
    (0..n).map(|i| s.pixel(16 * i as i32 + 8, 8)).collect()
}

/// Asserts the leading (colour) channels of `got` are within `tol` of `want`.
fn assert_colors(got: &[Vec<f32>], want: &[Vec<f32>], tol: f32, what: &str) {
    for (g, w) in got.iter().zip(want) {
        assert!(w.iter().zip(g).all(|(a, b)| (a - b).abs() <= tol), "{what}: got {got:?}, want {want:?}");
    }
}

/// Formats without alpha get the document composited over white, as flattening does, instead of
/// its alpha dropped (which shows the colours stored under transparent pixels).
#[test]
fn transparency_is_composited_over_white_for_formats_without_alpha() {
    let rgb: [&[f32]; 3] = [&[0.0, 0.0, 0.0, 0.0], &[0.0, 0.0, 1.0, 0.5], &[0.0, 0.63, 0.0, 1.0]];
    let over_white = vec![vec![1.0, 1.0, 1.0], vec![0.5, 0.5, 1.0], vec![0.0, 0.63, 0.0]];
    let gray: [&[f32]; 2] = [&[0.0, 0.0], &[0.0, 0.5]];
    // CMYK white is no ink.
    let cmyk: [&[f32]; 2] = [&[1.0, 1.0, 1.0, 1.0, 0.0], &[0.0, 0.0, 0.0, 1.0, 0.5]];
    let cases = [
        (ColorMode::Rgb, SampleType::U8, &rgb[..], over_white.clone()),
        (ColorMode::Rgb, SampleType::U16, &rgb[..], over_white.clone()),
        (ColorMode::Rgb, SampleType::F32, &rgb[..], over_white),
        (ColorMode::Grayscale, SampleType::U8, &gray[..], vec![vec![1.0], vec![0.5]]),
        (ColorMode::Cmyk, SampleType::U8, &cmyk[..], vec![vec![0.0; 4], vec![0.0, 0.0, 0.0, 0.5]]),
    ];
    for (mode, depth, cols, want) in cases {
        let what = format!("{mode:?} {depth:?}");
        let d = columns(mode, depth, cols);
        let r = export(&d, "a.jpg", &ExportOptions::default()).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("composited over white")), "{what}: {:?}", r.warnings);
        assert!(!r.warnings.iter().any(|w| w.contains("alpha will be discarded")), "{what}: {:?}", r.warnings);
        let back = import("a.jpg", &r.bytes).unwrap().document;
        assert_eq!(back.mode, mode, "{what}");
        assert_colors(&column_pixels(&back, cols.len()), &want, 0.03, &what);
        // PNG keeps the transparency itself.
        let r = export(&d, "a.png", &ExportOptions::default()).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("composited")), "{what}: {:?}", r.warnings);
        let back = import("a.png", &r.bytes).unwrap().document;
        let alpha = column_pixels(&back, cols.len())[1].last().copied().unwrap();
        assert!((alpha - 0.5).abs() <= 1.0 / 255.0, "{what}: alpha {alpha}");
    }
    // Opaque documents are written as before.
    let r = export(&columns(ColorMode::Rgb, SampleType::U8, &[&[0.2, 0.4, 0.6, 1.0]]), "a.jpg", &ExportOptions::default()).unwrap();
    assert!(!r.warnings.iter().any(|w| w.contains("composited")), "{:?}", r.warnings);
}

/// Formats that can't embed a profile get sRGB values (what an untagged file means), converted
/// through the colour engine, instead of values that only mean something under the dropped profile.
#[test]
fn non_srgb_rgb_is_converted_to_srgb_for_formats_without_a_profile() {
    use photocraft_cms::{Builtin, Intent, Transform};
    let cols: [&[f32]; 3] = [&[0.0, 0.05, 0.0, 1.0], &[0.2, 0.5, 0.8, 1.0], &[1.0, 0.25, 0.0, 1.0]];
    for (profile, depth) in [(Builtin::LinearSrgb, SampleType::F32), (Builtin::LinearSrgb, SampleType::U16), (Builtin::DisplayP3, SampleType::U8)] {
        let mut d = columns(ColorMode::Rgb, depth, &cols);
        d.icc_profile = Some(profile.profile().to_bytes());
        let stored = column_pixels(&d, cols.len());
        let t = Transform::new(profile.profile(), Builtin::Srgb.profile(), Intent::Perceptual, true).unwrap();
        let want: Vec<Vec<f32>> = stored
            .iter()
            .map(|p| {
                let mut v = p.clone();
                t.apply(&mut v, 4);
                // The formats hold 8-bit sRGB: out-of-gamut colours clip.
                v.iter().map(|c| c.clamp(0.0, 1.0)).collect()
            })
            .collect();
        for ext in ["bmp", "gif", "qoi", "tga"] {
            let what = format!("{profile:?} {depth:?} {ext}");
            let r = export(&d, ext, &ExportOptions::default()).unwrap();
            assert!(r.warnings.iter().any(|w| w.contains("converted to sRGB")), "{what}: {:?}", r.warnings);
            assert!(!r.warnings.iter().any(|w| w.contains("ICC profile")), "{what}: {:?}", r.warnings);
            let back = import(&format!("a.{ext}"), &r.bytes).unwrap().document;
            assert_eq!(back.icc_profile, None, "{what}");
            assert_colors(&column_pixels(&back, cols.len()), &want, 1.5 / 255.0, &what);
        }
        // Formats that embed the profile keep it and the values.
        let r = export(&d, "a.png", &ExportOptions::default()).unwrap();
        let back = import("a.png", &r.bytes).unwrap().document;
        assert_eq!(back.icc_profile, d.icc_profile, "{profile:?} {depth:?}");
        assert_colors(&column_pixels(&back, cols.len()), &stored, 1e-4, &format!("{profile:?} {depth:?} png"));
    }
    // sRGB (tagged or not) is written unchanged.
    for icc in [None, Some(Builtin::Srgb.profile().to_bytes())] {
        let mut d = columns(ColorMode::Rgb, SampleType::U8, &cols);
        d.icc_profile = icc;
        let r = export(&d, "a.bmp", &ExportOptions::default()).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("converted to sRGB")), "{:?}", r.warnings);
        let back = import("a.bmp", &r.bytes).unwrap().document;
        assert_colors(&column_pixels(&back, cols.len()), &column_pixels(&d, cols.len()), 0.0, "sRGB bmp");
    }
}

/// #518: a JPEG cut off inside its image data opens with a warning, never silently.
#[test]
fn truncated_jpeg_imports_with_a_warning() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let full = export(&smooth(mode), "jpg", &ExportOptions::default()).unwrap().bytes;
        assert_eq!(import("x.jpg", &full).unwrap().warnings, Vec::<String>::new(), "{mode:?}");
        // Cut inside the scan data (most of this small file is headers).
        let r = import("x.jpg", &full[..full.len() - 4]).unwrap();
        assert_eq!(r.document.size, photocraft_geom::Size::new(32, 16));
        assert!(r.warnings.first().is_some_and(|w| w.starts_with("JPEG data ends early")), "{mode:?}: {:?}", r.warnings);
    }
}

/// A GIF of `frames` identical 1x1 black frames.
fn gif_frames(frames: usize) -> Vec<u8> {
    let mut b = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xFF\xFF\xFF".to_vec();
    for _ in 0..frames {
        b.extend_from_slice(b"\x2C\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0");
    }
    b.push(0x3B);
    b
}

/// #523: an animation opens its first frame, unchanged, with a warning that the rest was left out.
#[test]
fn animation_imports_the_first_frame_with_a_warning() {
    let one = import("a.gif", &gif_frames(1)).unwrap();
    assert_eq!(one.warnings, Vec::<String>::new());
    let three = import("a.gif", &gif_frames(3)).unwrap();
    assert_eq!(three.warnings, ["only the first of 3 frames was imported"]);
    pixels_eq(&one.document, &three.document, 0.0);
}

/// Alpha channel values for the #2124 tests: white (selected) on the left, mid grey, black.
fn alpha_value(x: u32) -> f32 {
    match x {
        0..=3 => 1.0,
        4 => g(128),
        _ => 0.0,
    }
}

/// `d` with one alpha channel holding [`alpha_value`] (Photoshop's "Alpha 1").
fn with_alpha_channel(mut d: photocraft_doc::Document, name: &str) -> photocraft_doc::Document {
    let fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, d.depth, false);
    let mut s = photocraft_raster::Surface::new(fmt);
    let r = d.bounds();
    let vals: Vec<f32> = (r.y0..r.y1).flat_map(|_| (r.x0..r.x1).map(|x| alpha_value(x as u32))).collect();
    s.write_region(r, &vals);
    d.channels.push(photocraft_doc::AlphaChannel::new(name, s));
    d
}

/// Header and decoded pixels of a TGA file.
fn tga(bytes: &[u8]) -> (u8, u8, photocraft_codecs::Image) {
    (bytes[16], bytes[17] & 0x0f, photocraft_codecs::decode(bytes).expect("decode tga"))
}

/// #2124: Photoshop writes the document's alpha channel as a 32-bit Targa's alpha (8 alpha bits
/// in the descriptor), and the file opens back with that alpha. 16-bit documents are written as
/// 8-bit Targa without panicking.
#[test]
fn tga_writes_the_alpha_channel_as_32_bit_alpha() {
    for depth in [SampleType::U8, SampleType::U16] {
        let d = with_alpha_channel(single(ColorMode::Rgb, depth, false), "Alpha 1");
        let r = export(&d, "a.tga", &ExportOptions::default()).unwrap();
        let (bpp, alpha_bits, img) = tga(&r.bytes);
        assert_eq!((bpp, alpha_bits), (32, 8), "{depth:?}");
        assert_eq!(img.layout(), photocraft_codecs::ChannelLayout::Rgba, "{depth:?}");
        let (w, _) = img.dimensions();
        for (i, px) in img.data().as_chunks::<4>().0.iter().enumerate() {
            let x = i as u32 % w;
            assert_eq!(px[3], (alpha_value(x) * 255.0).round() as u8, "{depth:?} pixel {i}");
        }
        // The colours are the document's (opaque Background, untouched by the alpha).
        let want = single(ColorMode::Rgb, SampleType::U8, false);
        let back = import("a.tga", &r.bytes).unwrap().document;
        let (sa, sb) = (want.layers[0].surface().unwrap(), back.layers[0].surface().unwrap());
        let (va, vb) = (sa.read_region(want.bounds()), sb.read_region(back.bounds()));
        for (i, (a, b)) in va.as_chunks::<4>().0.iter().zip(vb.as_chunks::<4>().0.iter()).enumerate() {
            assert!(a[..3].iter().zip(&b[..3]).all(|(p, q)| (p - q).abs() <= 1.0 / 255.0), "{depth:?} colour {i}: {a:?} {b:?}");
            assert_eq!(b[3], 1.0, "{depth:?} the Background stays opaque");
        }
        // Round trip, as in Photoshop: the alpha opens back as the "Alpha 1" channel (#2225).
        let [ch] = &back.channels[..] else { panic!("{depth:?}: one alpha channel, got {}", back.channels.len()) };
        assert_eq!(ch.name, "Alpha 1");
        for (i, a) in ch.surface.read_region(back.bounds()).iter().enumerate() {
            assert!((a - alpha_value(i as u32 % 9)).abs() <= 0.5 / 255.0, "{depth:?} alpha {i}: {a}");
        }
    }
}

/// A 64×64 uncompressed Targa (bottom-left origin), byte for byte like the files the Photoshop
/// measurements in #2225 opened: red everywhere, alpha (when `bpp` is 32) 255 on the left half and
/// 0 on the right, with `alpha_bits` in the descriptor.
fn tga_file(bpp: u8, alpha_bits: u8) -> Vec<u8> {
    let mut b = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 64, 0, bpp, alpha_bits];
    for _y in 0..64 {
        for x in 0..64 {
            b.extend_from_slice(&[0, 0, 255]);
            if bpp == 32 {
                b.push(if x < 32 { 255 } else { 0 });
            }
        }
    }
    b
}

/// #2225, measured on Photoshop 27.11: a 32-bit Targa opens as an opaque, locked Background plus
/// an "Alpha 1" channel holding the fourth byte (also when the descriptor declares 0 alpha bits),
/// and the stored colours stay visible where the alpha is 0. A 24-bit Targa has no channel.
#[test]
fn tga_32_bit_opens_its_alpha_as_a_channel_like_photoshop() {
    for alpha_bits in [8, 0] {
        let d = import("a.tga", &tga_file(32, alpha_bits)).unwrap().document;
        let [bg] = &d.layers[..] else { panic!("one layer") };
        assert_eq!(bg.name, "Background", "alpha bits {alpha_bits}");
        assert!(bg.locks.transparency && bg.locks.position);
        let px = bg.surface().unwrap().read_region(d.bounds());
        for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(*p, [1.0, 0.0, 0.0, 1.0], "alpha bits {alpha_bits}: pixel {i} keeps its red, opaque");
        }
        let [ch] = &d.channels[..] else { panic!("alpha bits {alpha_bits}: one channel, got {}", d.channels.len()) };
        assert_eq!((ch.name.as_str(), ch.spot), ("Alpha 1", None));
        let a = ch.surface.read_region(d.bounds());
        assert_eq!((a[64 * 32 + 10], a[64 * 32 + 54]), (1.0, 0.0), "alpha bits {alpha_bits}");
    }
    let d = import("a.tga", &tga_file(24, 0)).unwrap().document;
    assert!(d.channels.is_empty());
    assert_eq!(d.layers[0].name, "Background");
}

/// Layer transparency is still written as the Targa's alpha when there is no alpha channel (a
/// choice left to the maintainers in #2225); the file opens back as Photoshop opens it, with the
/// alpha as "Alpha 1" and the colours of transparent pixels kept.
#[test]
fn tga_transparency_reopens_as_alpha_1() {
    let d = single(ColorMode::Rgb, SampleType::U8, true);
    let r = export(&d, "a.tga", &ExportOptions::default()).unwrap();
    let back = import("a.tga", &r.bytes).unwrap().document;
    let src = d.layers[0].surface().unwrap().read_region(d.bounds());
    let got = back.layers[0].surface().unwrap().read_region(back.bounds());
    let alpha = back.channels.first().expect("Alpha 1").surface.read_region(back.bounds());
    for (i, ((s, g), a)) in src.as_chunks::<4>().0.iter().zip(got.as_chunks::<4>().0.iter()).zip(&alpha).enumerate() {
        assert_eq!(&g[..3], &s[..3], "colour {i}");
        assert_eq!((g[3], *a), (1.0, s[3]), "alpha {i}");
    }
}

/// #2124: as in Photoshop, the alpha channel (not layer transparency) is the alpha; transparent
/// pixels are composited over white. Spot channels are not alpha channels.
#[test]
fn tga_alpha_channel_wins_over_transparency_and_skips_spot_channels() {
    let mut d = with_alpha_channel(single(ColorMode::Rgb, SampleType::U8, true), "Alpha 2");
    let spot = photocraft_raster::Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    let spot =
        photocraft_doc::AlphaChannel { spot: Some((photocraft_color::Color::rgb(1.0, 0.0, 0.0), 1.0)), ..photocraft_doc::AlphaChannel::new("Spot", spot) };
    d.channels.insert(0, spot);
    let r = export(&d, "a.tga", &ExportOptions::default()).unwrap();
    let (bpp, alpha_bits, img) = tga(&r.bytes);
    assert_eq!((bpp, alpha_bits), (32, 8));
    let src = d.layers[0].surface().unwrap().read_region(d.bounds());
    for (i, (px, s)) in img.data().as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0.iter()).enumerate() {
        assert_eq!(px[3], (alpha_value(i as u32 % 9) * 255.0).round() as u8, "alpha {i}");
        let over_white = s[0] * s[3] + 1.0 - s[3];
        assert!((f32::from(px[0]) / 255.0 - over_white).abs() <= 1.0 / 255.0, "red {i}: {} vs {over_white}", px[0]);
    }
}

/// #2124: Photoshop writes no channel as the alpha when there are several, and grayscale Targas
/// have no alpha at all.
#[test]
fn tga_ignores_several_alpha_channels_and_grayscale() {
    let d = with_alpha_channel(with_alpha_channel(single(ColorMode::Rgb, SampleType::U8, false), "Alpha 1"), "Alpha 2");
    let r = export(&d, "a.tga", &ExportOptions::default()).unwrap();
    assert_eq!(tga(&r.bytes).0, 24);
    assert!(r.warnings.iter().any(|w| w.contains("one alpha channel")), "{:?}", r.warnings);
    let d = with_alpha_channel(single(ColorMode::Grayscale, SampleType::U8, false), "Alpha 1");
    let r = export(&d, "a.tga", &ExportOptions::default()).unwrap();
    assert_eq!(tga(&r.bytes).0, 8);
}
