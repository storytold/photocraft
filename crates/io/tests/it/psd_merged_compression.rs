//! The merged image is written as Photoshop writes it: RLE at 8 bits, raw at 16 and 32 (#9716).
//!
//! Photoshop never writes a 16- or 32-bit merged image in RLE: every 16-bit (74) and 32-bit (43)
//! file in the PSD corpus stores it raw. macOS ImageIO, which Finder, Quick Look and Preview open
//! PSDs with, refuses one that is, so a 16-bit PSD saved by PhotoCraft had no preview there and
//! opened nowhere else. ZIP asked for explicitly is kept: Photoshop reads it at any depth.

use crate::common;

use common::raster;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::{Rect, Size};
use photocraft_io::*;
use photocraft_psd::{Compression, PsdFile, Version, testgen};

#[test]
fn merged_image_is_rle_at_8_bits_and_raw_deeper() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for path in ["x.psd", "x.psb"] {
                let case = format!("{mode:?} {depth:?} {path}");
                let mut doc = Document::new("t", Size::new(9, 7), mode, depth);
                let fmt = doc.pixel_format();
                doc.layers.push(raster("pixels", fmt, Rect::new(0, 0, 9, 7), 5, false));
                let file = PsdFile::from_bytes(&export(&doc, path, &ExportOptions::default()).expect("export").bytes).expect("parse");
                let want = if depth == SampleType::U8 { Compression::Rle } else { Compression::Raw };
                assert_eq!(file.image_data.compression, want, "{case}");
                assert!(file.image_data.decode(&file.header).is_ok(), "{case}: the merged image decodes");
            }
        }
    }
}

#[test]
fn a_16_bit_multichannel_merged_image_is_raw() {
    for (depth, want) in [(8, Compression::Rle), (16, Compression::Raw)] {
        let f = testgen::merged_only(Version::Psd, photocraft_psd::ColorMode::Multichannel, depth, Compression::Rle, 9, 5);
        let (doc, _) = psd_to_document(&f);
        let out = PsdFile::from_bytes(&export(&doc, "x.psd", &ExportOptions::default()).expect("export").bytes).expect("parse");
        assert_eq!(out.header.color_mode, photocraft_psd::ColorMode::Multichannel, "{depth}-bit");
        assert_eq!(out.image_data.compression, want, "{depth}-bit");
    }
}

#[test]
fn zip_asked_for_a_16_bit_merged_image_is_kept() {
    let mut doc = Document::new("t", Size::new(9, 7), ColorMode::Rgb, SampleType::U16);
    let fmt = doc.pixel_format();
    doc.layers.push(raster("pixels", fmt, Rect::new(0, 0, 9, 7), 5, false));
    for compression in [Compression::Zip, Compression::ZipPrediction] {
        let opts = PsdExportOptions { merged_compression: compression, ..Default::default() };
        assert_eq!(document_to_psd_with(&doc, &opts).0.image_data.compression, compression);
    }
}
