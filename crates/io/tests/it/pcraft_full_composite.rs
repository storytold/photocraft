//! With [`ExportOptions::pcraft_full_composite`] on, a `.pcraft` save writes the flattened
//! document at its canvas size as `composite/full.png`, next to the 256 px thumbnail and the
//! 1024 px preview, so other apps can read the whole image without decoding tiles. Off (the
//! default), the save writes only the two reduced previews.

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_format::zip::ZipReader;
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export};
use photocraft_raster::Surface;

/// A background, a soft Multiply layer at 60 % and a layer that leaves part of the canvas clear.
fn doc(w: u32, h: u32, depth: SampleType) -> Document {
    let mut d = Document::new("full", Size::new(w, h), ColorMode::Rgb, depth);
    let fmt = d.pixel_format();
    let layer = |name: &str, seed: usize, r: Rect| {
        let n = r.width() as usize * r.height() as usize * fmt.bytes_per_pixel();
        let px: Vec<u8> = (0..n).map(|i| ((i * 31 + seed * 17 + i / 7) % 251) as u8).collect();
        Layer::new(name, LayerContent::Raster(Surface::from_interleaved(fmt, r, &px)))
    };
    d.layers.push(layer("Back", 1, Rect::new(0, 0, w as i32, h as i32 / 2)));
    let mut soft = layer("Soft", 2, Rect::new(w as i32 / 4, h as i32 / 4, w as i32, h as i32));
    soft.opacity = 0.6;
    soft.blend = BlendMode::Multiply;
    d.layers.push(soft);
    d
}

fn on() -> ExportOptions {
    ExportOptions { pcraft_full_composite: true, ..Default::default() }
}

/// The bundle's member named by the manifest's `full_composite`, decoded: size and RGBA8 pixels.
fn full_composite(bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
    let png = photocraft_format::read_full_composite(bytes).expect("manifest").expect("the manifest names the full-size composite");
    assert_eq!(png, ZipReader::new(bytes).expect("zip").read_by_name("composite/full.png", usize::MAX).expect("entry"));
    let image = photocraft_codecs::decode(&png).expect("png");
    (image.dimensions(), image.to_rgba8())
}

fn flattened(doc: &Document) -> Vec<u8> {
    photocraft_compose::render(doc, doc.bounds()).to_rgba8().pixels
}

#[test]
/// Cached and uncached native previews retain native RGB samples with their exact profile,
/// independently of the full-preview option, and normalize correctly through the tested CMS policy.
fn reduced_native_previews_carry_their_rgb_profile_without_converting_pixels() {
    use photocraft_cms::{Builtin, Intent, Profile, Transform};
    use std::sync::Arc;

    for (builtin, normalized) in [(Builtin::Srgb, [64u8, 128, 191]), (Builtin::LinearSrgb, [137, 188, 225]), (Builtin::DisplayP3, [32, 129, 197])] {
        let mut d = Document::new("native", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
        let profile = builtin.profile().to_bytes();
        d.icc_profile = Some(profile.clone());
        let mut surface = Surface::new(d.pixel_format());
        surface.write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
        d.layers.push(Layer::new("pixel", LayerContent::Raster(surface)));
        for cached in [false, true] {
            let cache = cached.then(photocraft_io::PcraftExportCache::default);
            for full in [false, true, false] {
                let opts = ExportOptions { pcraft_full_composite: full, pcraft_cache: cache.clone(), ..Default::default() };
                let bytes = export(&d, "native.pcraft", &opts).unwrap().bytes;
                let zip = ZipReader::new(&bytes).unwrap();
                assert_eq!(zip.find("composite/full.png").is_some(), full);
                for name in ["thumb.png", "composite/preview.png"].into_iter().chain(full.then_some("composite/full.png")) {
                    let png = zip.read_by_name(name, usize::MAX).unwrap();
                    let image = photocraft_codecs::decode(&png).unwrap();
                    assert_eq!(image.icc.as_deref(), Some(profile.as_slice()), "{builtin:?}, {cached}, {full}, {name}");
                    assert_eq!(image.to_rgba8(), [64, 128, 191, 255]);
                    assert!(!png_has_srgb(&png));
                    let source = Profile::parse(image.icc.as_ref().unwrap()).unwrap();
                    let transform = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
                    let mut pixel = [64.0 / 255.0, 128.0 / 255.0, 191.0 / 255.0, 1.0];
                    transform.apply(&mut pixel, 4);
                    for (value, expected) in pixel[..3].iter().zip(normalized) {
                        assert!((value * 255.0 - f32::from(expected)).abs() <= 1.0, "{builtin:?}: {pixel:?}");
                    }
                }
            }
        }
        assert!(Arc::ptr_eq(d.icc_profile.as_ref().unwrap(), &profile));
    }
}

/// Inspect chunk boundaries in an encoder-produced PNG for a competing sRGB declaration.
fn png_has_srgb(png: &[u8]) -> bool {
    let mut chunks = &png[8..];
    while chunks.len() >= 12 {
        let len = u32::from_be_bytes(chunks[..4].try_into().unwrap()) as usize;
        if &chunks[4..8] == b"sRGB" {
            return true;
        }
        chunks = &chunks[len + 12..];
    }
    false
}

#[test]
/// Reduced RGBA previews use adapted RGB gray profiles or the compositor's sRGB fallback,
/// never a document's incompatible gray, CMYK or Lab profile.
fn reduced_native_profiles_follow_the_compositor_color_model() {
    use photocraft_cms::{Builtin, ColorSpace, Profile};
    use std::sync::Arc;

    let srgb = Builtin::Srgb.profile().to_bytes();
    let gray = Builtin::GrayGamma22.profile();
    let gray_rgb = gray.gray_as_rgb().unwrap().to_bytes();
    let default_gray = Builtin::SGray.profile().gray_as_rgb().unwrap().to_bytes();
    for (mode, own, expected) in [
        (ColorMode::Rgb, None, srgb.clone()),
        (ColorMode::Rgb, Some(Arc::new(vec![0; 128])), srgb.clone()),
        (ColorMode::Rgb, Some(gray.to_bytes()), srgb.clone()),
        (ColorMode::Grayscale, Some(gray.to_bytes()), gray_rgb.clone()),
        (ColorMode::Bitmap, Some(gray.to_bytes()), gray_rgb.clone()),
        (ColorMode::Duotone, Some(gray.to_bytes()), gray_rgb),
        (ColorMode::Grayscale, None, default_gray),
        (ColorMode::Cmyk, Some(Builtin::CoatedCmyk.profile().to_bytes()), srgb.clone()),
        (ColorMode::Lab, Some(Builtin::LabD50.profile().to_bytes()), srgb),
    ] {
        let mut d = Document::new("policy", Size::new(2, 1), mode, SampleType::U8);
        d.icc_profile = own;
        let bytes = export(&d, "policy.pcraft", &ExportOptions::default()).unwrap().bytes;
        let zip = ZipReader::new(&bytes).unwrap();
        for (name, side) in [("thumb.png", 256), ("composite/preview.png", 1024)] {
            let png = zip.read_by_name(name, usize::MAX).unwrap();
            let image = photocraft_codecs::decode(&png).unwrap();
            assert_eq!(image.icc.as_deref(), Some(expected.as_slice()), "{mode:?}, {name}");
            assert_eq!(Profile::parse(image.icc.as_ref().unwrap()).unwrap().color_space, ColorSpace::Rgb);
            assert_eq!(image.to_rgba8(), photocraft_compose::thumbnail(&d, side).pixels);
        }
    }
}

#[test]
/// Default saves tag both reduced previews and omit the full PNG at every supported sample depth.
fn a_default_save_writes_the_bundle_without_the_full_composite() {
    for depth in SampleType::ALL {
        let d = doc(1500, 700, depth);
        let bytes = export(&d, "a.pcraft", &ExportOptions::default()).expect("export").bytes;
        assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None, "{depth:?}");
        assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none(), "{depth:?}");
        // Native saves supply the profile of both reduced previews explicitly.
        let mut named = d.clone();
        named.name = "a.pcraft".into();
        let previews = photocraft_format::SaveOptions {
            thumbnail: Some(photocraft_compose::thumbnail(&named, 256)),
            composite: Some(photocraft_compose::thumbnail(&named, 1024)),
            preview_icc: Some(std::sync::Arc::new(photocraft_io::pcraft_full_composite_icc(&named))),
            ..Default::default()
        };
        assert!(bytes == photocraft_format::save_to_bytes(&named, &previews).expect("save"), "{depth:?}: the default save changed");
    }
}

#[test]
fn a_pcraft_save_writes_the_flattened_document_at_its_canvas_size() {
    for depth in SampleType::ALL {
        let d = doc(300, 200, depth);
        let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
        let (size, pixels) = full_composite(&bytes);
        assert_eq!(size, (300, 200), "{depth:?}");
        assert!(pixels == flattened(&d), "{depth:?}: the full-size composite differs from the flattened document");
        assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0), "{depth:?}: transparent pixels stay transparent");
        let mut saved = d.clone();
        saved.name = "a.pcraft".into();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).expect("load"), saved, "{depth:?}");
    }
}

#[test]
fn a_document_larger_than_the_preview_keeps_its_full_size() {
    let d = doc(1500, 700, SampleType::U8);
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    let (size, pixels) = full_composite(&bytes);
    assert_eq!(size, (1500, 700));
    assert!(pixels == flattened(&d), "the full-size composite differs from the flattened document");
    let preview = ZipReader::new(&bytes).expect("zip").read_by_name("composite/preview.png", usize::MAX).expect("preview");
    assert_eq!(photocraft_codecs::decode(&preview).expect("png").dimensions(), (1024, 478));
}

#[test]
fn the_full_composite_carries_the_profile_of_its_pixels() {
    use photocraft_cms::{Builtin, ColorSpace, Profile};
    let full_icc = |d: &Document| {
        let bytes = export(d, "a.pcraft", &on()).expect("export").bytes;
        let png = ZipReader::new(&bytes).expect("zip").read_by_name("composite/full.png", usize::MAX).expect("entry");
        photocraft_codecs::decode(&png).expect("png").icc.expect("an embedded profile")
    };
    let srgb = Builtin::Srgb.profile().to_bytes().to_vec();
    let p3 = Builtin::DisplayP3.profile().to_bytes().to_vec();
    let mut d = doc(40, 30, SampleType::U8);
    d.icc_profile = Some(std::sync::Arc::new(p3.clone()));
    assert_eq!(full_icc(&d), p3, "the composite is in the document's RGB space");
    d.icc_profile = None;
    assert_eq!(full_icc(&d), srgb);
    // A CMYK document composites to sRGB: its CMYK profile does not describe the PNG.
    let mut cmyk = Document::new("cmyk", Size::new(40, 30), ColorMode::Cmyk, SampleType::U8);
    cmyk.icc_profile = Some(std::sync::Arc::new(Builtin::CoatedCmyk.profile().to_bytes().to_vec()));
    assert_eq!(full_icc(&cmyk), srgb);
    // A gray document's composite repeats its gray samples in R, G and B: the PNG carries its
    // gray curve as an RGB profile.
    let gray = Builtin::SGray.profile().clone();
    let mut g = Document::new("gray", Size::new(40, 30), ColorMode::Grayscale, SampleType::U8);
    g.icc_profile = Some(std::sync::Arc::new(gray.to_bytes().to_vec()));
    let tagged = Profile::parse(&full_icc(&g)).expect("profile");
    assert_eq!(tagged.color_space, ColorSpace::Rgb);
    assert_eq!(tagged.to_bytes(), gray.gray_as_rgb().expect("gray as RGB").to_bytes());
}

#[test]
fn a_canvas_past_the_render_limit_is_saved_without_it() {
    // 40000 x 30000 px is over the compositor's 2^30-pixel limit; the layers stay small.
    let mut d = doc(64, 64, SampleType::U8);
    d.size = Size::new(40_000, 30_000);
    assert!(photocraft_io::pcraft_full_composite(&d).is_none());
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None);
    assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none());
}

#[test]
fn a_sparse_canvas_past_the_composite_memory_budget_is_saved_without_it() {
    // The layer data stays tiny; an 8193 x 8192 RGBA8 composite exceeds 256 MiB.
    let mut d = doc(64, 64, SampleType::U8);
    d.size = Size::new(8193, 8192);
    assert!(photocraft_io::pcraft_full_composite(&d).is_none());
    let bytes = export(&d, "a.pcraft", &on()).expect("export").bytes;
    assert_eq!(photocraft_format::read_full_composite(&bytes).expect("manifest"), None);
    assert!(ZipReader::new(&bytes).expect("zip").find("composite/full.png").is_none());
}
