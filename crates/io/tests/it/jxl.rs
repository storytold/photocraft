//! JPEG XL opens with the `jxl` feature; without it the file is still recognised and the error says
//! what this build lacks (never a panic or "unrecognized format").

#[cfg(feature = "jxl")]
use crate::common::raster;
#[cfg(feature = "jxl")]
use photocraft_color::{ColorMode, SampleType};
#[cfg(feature = "jxl")]
use photocraft_io::{ExportOptions, export};
use photocraft_io::import;

#[cfg(not(feature = "jxl"))]
#[test]
fn jxl_without_the_feature_says_it_is_not_in_this_build() {
    let codestream = [0xff, 0x0a, 0, 0];
    for name in ["photo.jxl", "PHOTO.JXL", "noext"] {
        let e = import(name, &codestream).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("isn't included in this build"), "{name}: {e}");
    }
}

#[cfg(feature = "jxl")]
#[test]
fn a_jxl_export_opens_again_with_the_same_size_and_depth() {
    let mut d = photocraft_doc::Document::new("s", photocraft_geom::Size::new(9, 6), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, true));
    let exported = export(&d, "jxl", &ExportOptions::default()).expect("export");
    let back = import("photo.jxl", &exported.bytes).expect("a JPEG XL file opens").document;
    assert_eq!(back.size, d.size);
    assert_eq!((back.mode, back.depth), (d.mode, d.depth));
}

#[cfg(feature = "jxl")]
#[test]
fn a_cut_off_jxl_file_is_an_error_not_a_panic() {
    let mut d = photocraft_doc::Document::new("s", photocraft_geom::Size::new(9, 6), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    d.layers.push(raster("Background", fmt, d.bounds(), 3, false));
    let bytes = export(&d, "jxl", &ExportOptions::default()).expect("export").bytes;
    for end in 0..bytes.len() {
        let _ = import("photo.jxl", &bytes[..end]);
    }
}
