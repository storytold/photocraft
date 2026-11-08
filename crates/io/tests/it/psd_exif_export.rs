//! #12385: a layered PSD's camera EXIF (image resource 1058) reaches the flat file it is exported
//! to, without an orientation, pixel size or thumbnail that no longer matches the pixels.

use crate::common::*;
use photocraft_codecs::exif_orientation;
use photocraft_io::*;

/// A big-endian TIFF stream: IFD0 = `ifd0` entries plus an Exif IFD pointer, the Exif IFD =
/// `exif` entries, and IFD1 holding a JPEG thumbnail pointer. Entries are `(tag, type, count,
/// payload)`; payloads over four bytes go after the directories.
fn tiff(ifd0: &[(u16, u16, u32, Vec<u8>)], exif: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
    let dir_len = |n: usize| 2 + n * 12 + 4;
    let ifd0_off = 8usize;
    let exif_off = ifd0_off + dir_len(ifd0.len() + 1);
    let ifd1_off = exif_off + dir_len(exif.len());
    let ifd1: [(u16, u16, u32, Vec<u8>); 2] = [(513, 4, 1, vec![0; 4]), (514, 4, 1, 4u32.to_be_bytes().to_vec())];
    let mut data_off = ifd1_off + dir_len(ifd1.len());
    let mut out = b"MM\0*".to_vec();
    out.extend_from_slice(&(ifd0_off as u32).to_be_bytes());
    let mut extra = Vec::new();
    let mut write_dir = |out: &mut Vec<u8>, entries: &[(u16, u16, u32, Vec<u8>)], next: u32| {
        out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        for (tag, ty, count, payload) in entries {
            out.extend_from_slice(&tag.to_be_bytes());
            out.extend_from_slice(&ty.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
            if payload.len() <= 4 {
                let mut v = payload.clone();
                v.resize(4, 0);
                out.extend_from_slice(&v);
            } else {
                out.extend_from_slice(&(data_off as u32).to_be_bytes());
                extra.extend_from_slice(payload);
                data_off += payload.len();
            }
        }
        out.extend_from_slice(&next.to_be_bytes());
    };
    let mut ifd0: Vec<_> = ifd0.to_vec();
    ifd0.push((34665, 4, 1, (exif_off as u32).to_be_bytes().to_vec()));
    ifd0.sort_by_key(|e| e.0);
    write_dir(&mut out, &ifd0, ifd1_off as u32);
    write_dir(&mut out, exif, 0);
    write_dir(&mut out, &ifd1, 0);
    // The thumbnail pointer names the first bytes after the directories.
    let thumb_at = out.len() + extra.len();
    let tail = [0xFF, 0xD8, 0xFF, 0xD9];
    out.extend_from_slice(&extra);
    out.extend_from_slice(&tail);
    // Point IFD1's JPEGInterchangeFormat at the thumbnail.
    let ifd1_entry = ifd1_off + 2;
    out[ifd1_entry + 8..ifd1_entry + 12].copy_from_slice(&(thumb_at as u32).to_be_bytes());
    out
}

fn ascii(s: &str) -> (u32, Vec<u8>) {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    (v.len() as u32, v)
}

/// Camera EXIF as a raw developer leaves it, plus the stale fields a flattened, resized export
/// must not repeat: Orientation 6, the sensor's pixel size and a thumbnail.
fn camera_exif() -> Vec<u8> {
    let (mc, make) = ascii("Canon");
    let (oc, model) = ascii("Canon EOS R5");
    let (dc, date) = ascii("2026:11:01 10:20:30");
    let (lc, lens) = ascii("RF24-105mm F4 L IS USM");
    tiff(
        &[(271, 2, mc, make), (272, 2, oc, model), (274, 3, 1, 6u16.to_be_bytes().to_vec())],
        &[
            (34855, 3, 1, 400u16.to_be_bytes().to_vec()),
            (36867, 2, dc, date),
            (40962, 4, 1, 8192u32.to_be_bytes().to_vec()),
            (40963, 4, 1, 5464u32.to_be_bytes().to_vec()),
            (42036, 2, lc, lens),
        ],
    )
}

/// Two layers over a 9 × 6, 16-bit canvas, as a developed raw is saved, with the camera's EXIF.
fn layered_doc() -> photocraft_doc::Document {
    use photocraft_color::{ColorMode, SampleType};
    let mut d = photocraft_doc::Document::new("photo", photocraft_geom::Size::new(9, 6), ColorMode::Rgb, SampleType::U16);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, false));
    let mut top = raster("Top", fmt, d.bounds(), 7, true);
    top.opacity = 0.5;
    d.layers.push(top);
    d.metadata.exif = Some(std::sync::Arc::new(camera_exif()));
    d
}

/// [`dump`]'s stand-in tag for a next-IFD pointer.
const NEXT: u16 = 0xFFFF;

fn exported(ext: &str) -> (photocraft_doc::Document, Vec<String>) {
    let psd = export(&layered_doc(), "photo.psd", &ExportOptions::default()).unwrap().bytes;
    let back = import("photo.psd", &psd).unwrap().document;
    let opts = ExportOptions { xmp: XmpEmbed::All, ..ExportOptions::default() };
    let out = export(&back, &format!("photo.{ext}"), &opts).unwrap();
    (import(&format!("photo.{ext}"), &out.bytes).unwrap().document, out.warnings)
}

#[test]
fn a_layered_psds_camera_exif_reaches_jpeg_png_tiff_and_webp() {
    for ext in ["jpg", "png", "tif", "webp"] {
        let (doc, warnings) = exported(ext);
        assert!(warnings.iter().any(|w| w.contains("flattened")), "{ext}: the layers were flattened: {warnings:?}");
        let exif = doc.metadata.exif.as_deref().unwrap_or_else(|| panic!("{ext}: the camera EXIF"));
        let camera = photocraft_algo::exif::read(exif);
        assert_eq!(camera.make.as_deref(), Some("Canon"), "{ext}");
        assert_eq!(camera.model.as_deref(), Some("Canon EOS R5"), "{ext}");
        assert_eq!(camera.iso, Some(400.0), "{ext}");
        assert_eq!(camera.lens.as_deref(), Some("RF24-105mm F4 L IS USM"), "{ext}");
        assert_eq!(camera.date_taken.as_deref(), Some("2026:11:01 10:20:30"), "{ext}");
    }
}

#[test]
fn the_exported_exif_names_the_files_own_size_orientation_and_no_thumbnail() {
    for ext in ["jpg", "png", "tif", "webp"] {
        let (doc, _) = exported(ext);
        let exif = doc.metadata.exif.as_deref().unwrap();
        let tags = dump(exif);
        assert_eq!(exif_orientation(exif), 1, "{ext}: the pixels are upright");
        assert!(tags.contains(&(40962, 9)) && tags.contains(&(40963, 6)), "{ext}: PixelX/YDimension 9 × 6, not the sensor's: {tags:?}");
        assert!(!tags.iter().any(|t| t.0 == NEXT), "{ext}: no thumbnail directory: {tags:?}");
    }
}

/// Straight from the document, without a PSD save in between: the flat export itself writes the
/// file's size and drops the thumbnail.
#[test]
fn a_flat_export_names_the_documents_size_without_a_psd_save() {
    for ext in ["jpg", "png", "tif", "webp"] {
        let out = export(&layered_doc(), &format!("photo.{ext}"), &ExportOptions::default()).unwrap();
        let doc = import(&format!("photo.{ext}"), &out.bytes).unwrap().document;
        let exif = doc.metadata.exif.as_deref().unwrap_or_else(|| panic!("{ext}: the camera EXIF"));
        let tags = dump(exif);
        assert!(tags.contains(&(40962, 9)) && tags.contains(&(40963, 6)), "{ext}: {tags:?}");
        assert!(!tags.iter().any(|t| t.0 == NEXT), "{ext}: no thumbnail directory: {tags:?}");
        assert_eq!(photocraft_algo::exif::read(exif).make.as_deref(), Some("Canon"), "{ext}");
    }
}

/// The PSD save itself writes the document's size, so a cropped photo's PSD doesn't keep the
/// sensor's either.
#[test]
fn the_psd_save_writes_the_documents_size() {
    let psd = export(&layered_doc(), "photo.psd", &ExportOptions::default()).unwrap().bytes;
    let back = import("photo.psd", &psd).unwrap().document;
    let exif = back.metadata.exif.as_deref().unwrap();
    let tags = dump(exif);
    assert!(tags.contains(&(40962, 9)) && tags.contains(&(40963, 6)), "{tags:?}");
    assert_eq!(photocraft_algo::exif::read(exif).make.as_deref(), Some("Canon"));
}

/// The tags of IFD0 and the Exif IFD as `(tag, value or offset)`, with `(NEXT, offset)` for a
/// next-IFD pointer that names one (the thumbnail's IFD1).
fn dump(e: &[u8]) -> Vec<(u16, u32)> {
    let b = e.strip_prefix(b"Exif\0\0").unwrap_or(e);
    let le = &b[..2] == b"II";
    let u16_at = |i: usize| if le { u16::from_le_bytes([b[i], b[i + 1]]) } else { u16::from_be_bytes([b[i], b[i + 1]]) };
    let u32_at =
        |i: usize| if le { u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) } else { u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) };
    let mut out = vec![];
    let mut dirs = vec![u32_at(4) as usize];
    let mut k = 0;
    while let Some(d) = dirs.pop() {
        k += 1;
        if d == 0 || d + 2 > b.len() || k > 6 {
            continue;
        }
        let n = u16_at(d) as usize;
        for i in 0..n {
            let e = d + 2 + 12 * i;
            let tag = u16_at(e);
            let ty = u16_at(e + 2);
            let v = if ty == 3 { u16_at(e + 8) as u32 } else { u32_at(e + 8) };
            out.push((tag, v));
            if tag == 34665 {
                dirs.push(v as usize);
            }
        }
        let next = u32_at(d + 2 + 12 * n);
        if next != 0 {
            out.push((NEXT, next));
            dirs.push(next as usize);
        }
    }
    out
}
