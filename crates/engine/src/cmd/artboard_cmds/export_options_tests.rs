//! Artboards to Files' Export Options (#13636): each per-file-type option reaches the written
//! file, no option leaves the output as it was, and bad values are refused before anything is
//! written.

use super::*;
use photocraft_geom::Rect;

/// 64 x 48, transparent, with two boards: "Clear" (transparent, a red square inside, half the
/// board left empty) and "White" (white background, empty).
fn session() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "background": "transparent"})).unwrap();
    s.edit("Red", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster("Red", fmt);
        l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 24), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap();
    let clear = s.execute("layer.new.artboardFromLayers", json!({"name": "Clear", "background": "transparent", "width": 32, "height": 24})).unwrap();
    let white = s.execute("layer.new.artboard", json!({"rect": [40, 0, 16, 16], "name": "White", "background": "white"})).unwrap();
    (s, clear["layer"].as_u64().unwrap(), white["layer"].as_u64().unwrap())
}

/// Exports `board` with `extra` params as `format` and returns the file's bytes.
fn export(s: &mut Session, dir: &str, board: u64, format: &str, extra: Value) -> Vec<u8> {
    let mut p = json!({"dir": dir, "prefix": "", "format": format, "artboards": [board]});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    let r = s.execute("file.export.artboardsToFiles", p).unwrap();
    std::fs::read(r["files"][0].as_str().unwrap()).unwrap()
}

/// A fresh, empty folder for one test's files.
fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("photocraft-13636-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// PNG IHDR: (colour type, interlace method).
fn png_header(b: &[u8]) -> (u8, u8) {
    assert_eq!(&b[12..16], b"IHDR");
    (b[25], b[28])
}

fn has(b: &[u8], needle: &[u8]) -> bool {
    b.windows(needle.len()).any(|w| w == needle)
}

/// A little- or big-endian TIFF's first-IFD value of `tag` (SHORT or LONG), if present.
fn tiff_tag(b: &[u8], tag: u16) -> Option<u32> {
    let le = &b[0..2] == b"II";
    let u16_at = |o: usize| if le { u16::from_le_bytes([b[o], b[o + 1]]) } else { u16::from_be_bytes([b[o], b[o + 1]]) };
    let u32_at =
        |o: usize| if le { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) } else { u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) };
    let ifd = u32_at(4) as usize;
    (0..usize::from(u16_at(ifd)))
        .map(|i| ifd + 2 + i * 12)
        .find(|&e| u16_at(e) == tag)
        .map(|e| if u16_at(e + 2) == 3 { u32::from(u16_at(e + 8)) } else { u32_at(e + 8) })
}

#[test]
fn without_options_the_files_are_as_before() {
    let (mut s, clear, white) = session();
    let dir = &tmp("plain");
    let png = export(&mut s, dir, clear, "png", json!({}));
    assert_eq!(png_header(&png), (6, 0), "RGBA, not interlaced");
    assert!(has(&png, b"iCCP"), "the profile is embedded");
    let bmp = export(&mut s, dir, white, "bmp", json!({}));
    assert_eq!(u16::from_le_bytes([bmp[28], bmp[29]]), 24, "an opaque board is a 24-bit BMP");
    let tif = export(&mut s, dir, white, "tiff", json!({}));
    assert_eq!(tiff_tag(&tif, 259), Some(1), "uncompressed");
    let psd = export(&mut s, dir, clear, "psd", json!({}));
    let doc = photocraft_io::import("x.psd", &psd).unwrap().document;
    assert_eq!(doc.artboards().len(), 1, "the board is kept");
    assert!(doc.icc_profile.is_none(), "a document without a profile of its own writes a PSD without one, as before");
    // Explicit nulls are the same as no option.
    let nulls = json!({"iccProfile": null, "bmpBits": null, "pngInterlaced": null, "transparency": null, "preserveArtboard": null, "tiffCompression": null});
    assert_eq!(export(&mut s, dir, clear, "png", nulls.clone()), png);
    assert_eq!(export(&mut s, dir, white, "bmp", nulls), bmp);
}

#[test]
fn bmp_depth_reaches_the_file() {
    let (mut s, clear, white) = session();
    let dir = &tmp("bmp");
    let bits = |b: &[u8]| u16::from_le_bytes([b[28], b[29]]);
    // 32 bits on an opaque board adds an opaque alpha; 24 on a transparent one drops it.
    let b32 = export(&mut s, dir, white, "bmp", json!({"bmpBits": 32}));
    assert_eq!(bits(&b32), 32);
    let b24 = export(&mut s, dir, clear, "bmp", json!({"bmpBits": 24}));
    assert_eq!(bits(&b24), 24);
    let img = photocraft_io::import("x.bmp", &b24).unwrap().document;
    let px = photocraft_compose::flatten(&img).px.clone();
    // The empty right half of the board is white, the square red.
    assert_eq!(px[(10 * 32 + 28) as usize], [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(px[(10 * 32 + 4) as usize], [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn tiff_compression_reaches_the_file() {
    let (mut s, _, white) = session();
    let dir = &tmp("tiff");
    for (name, tag) in [("none", 1), ("lzw", 5), ("zip", 8)] {
        let tif = export(&mut s, dir, white, "tiff", json!({"tiffCompression": name}));
        assert_eq!(tiff_tag(&tif, 259), Some(tag), "{name}");
    }
}

#[test]
fn png_interlace_and_transparency_reach_the_file() {
    let (mut s, clear, _) = session();
    let dir = &tmp("png");
    let png = export(&mut s, dir, clear, "png", json!({"pngInterlaced": true}));
    assert_eq!(png_header(&png), (6, 1), "Adam7");
    let png = export(&mut s, dir, clear, "png", json!({"transparency": true, "pngInterlaced": false}));
    assert_eq!(png_header(&png), (6, 0));
    let png = export(&mut s, dir, clear, "png", json!({"transparency": false}));
    assert_eq!(png_header(&png).0, 2, "Transparency off writes RGB without alpha");
    let img = photocraft_io::import("x.png", &png).unwrap().document;
    let px = photocraft_compose::flatten(&img).px.clone();
    assert_eq!(px[(10 * 32 + 28) as usize], [1.0, 1.0, 1.0, 1.0], "the empty part is white");
    assert_eq!(px[(10 * 32 + 4) as usize], [1.0, 0.0, 0.0, 1.0]);
    // Transparency is a PNG option: a PSD keeps its transparent board.
    let psd = export(&mut s, dir, clear, "psd", json!({"transparency": false}));
    let doc = photocraft_io::import("x.psd", &psd).unwrap().document;
    assert_eq!(doc.layers.len(), 1, "no Background added");
}

#[test]
fn include_icc_profile_reaches_the_file() {
    let (mut s, clear, white) = session();
    let dir = &tmp("icc");
    for on in [true, false] {
        let p = json!({"iccProfile": on});
        assert_eq!(has(&export(&mut s, dir, clear, "png", p.clone()), b"iCCP"), on, "png {on}");
        assert_eq!(has(&export(&mut s, dir, white, "jpg", p.clone()), b"ICC_PROFILE\0"), on, "jpg {on}");
        assert_eq!(tiff_tag(&export(&mut s, dir, white, "tiff", p.clone()), 34675).is_some(), on, "tiff {on}");
        let psd = export(&mut s, dir, clear, "psd", p);
        assert_eq!(photocraft_io::import("x.psd", &psd).unwrap().document.icc_profile.is_some(), on, "psd {on}");
    }
    // The source document is not given one.
    assert!(s.active().unwrap().doc.icc_profile.is_none());
}

#[test]
fn psd_options_reach_the_file() {
    let (mut s, clear, _) = session();
    let dir = &tmp("psd");
    let merged = |b: &[u8]| craft_psd::PsdFile::from_bytes(b).unwrap().has_real_merged_data();
    for on in [true, false] {
        assert_eq!(merged(&export(&mut s, dir, clear, "psd", json!({"maximizeCompatibility": on}))), Some(on), "{on}");
    }
    let psd = export(&mut s, dir, clear, "psd", json!({"preserveArtboard": false}));
    let doc = photocraft_io::import("x.psd", &psd).unwrap().document;
    assert!(doc.artboards().is_empty(), "written as a plain group");
    let group = doc.layers.first().unwrap();
    assert_eq!(group.name, "Clear");
    assert_eq!(group.children().unwrap().first().unwrap().name, "Red");
    let psd = export(&mut s, dir, clear, "psd", json!({"preserveArtboard": true}));
    assert_eq!(photocraft_io::import("x.psd", &psd).unwrap().document.artboards().len(), 1);
    // The source document keeps its board.
    assert_eq!(s.active().unwrap().doc.artboards().len(), 2);
}

#[test]
fn jpeg_quality_reaches_the_file() {
    let (mut s, clear, _) = session();
    let dir = &tmp("jpeg");
    s.execute("filter.noise.addNoise", json!({"amount": 60})).ok();
    let low = export(&mut s, dir, clear, "jpg", json!({"quality": 0}));
    let frac = export(&mut s, dir, clear, "jpg", json!({"quality": 7.6}));
    let high = export(&mut s, dir, clear, "jpg", json!({"quality": 12}));
    assert!(low.len() < frac.len() && frac.len() < high.len(), "{} {} {}", low.len(), frac.len(), high.len());
}

#[test]
fn bad_option_values_are_errors_and_write_nothing() {
    let (mut s, clear, _) = session();
    let dir = &tmp("bad");
    for bad in [
        json!({"tiffCompression": "jpeg"}),
        json!({"tiffCompression": "rle"}),
        json!({"tiffCompression": 5}),
        json!({"bmpBits": 16}),
        json!({"bmpBits": 17}),
        json!({"bmpBits": "24"}),
        json!({"bmpBits": -1}),
        json!({"quality": "high"}),
        json!({"quality": [8]}),
        json!({"iccProfile": "yes"}),
        json!({"pngInterlaced": 1}),
        json!({"transparency": {}}),
        json!({"preserveArtboard": "false"}),
        json!({"maximizeCompatibility": 0}),
        json!({"tgaBits": 16}),
    ] {
        let mut p = json!({"dir": dir, "format": "png", "artboards": [clear]});
        if let (Some(p), Some(bad)) = (p.as_object_mut(), bad.as_object()) {
            p.extend(bad.clone());
        }
        let r = s.execute("file.export.artboardsToFiles", p);
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0, "{bad}: nothing written");
    }
}
