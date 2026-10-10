//! A file whose horizontal and vertical resolutions differ opens at the horizontal one, and the
//! import says the vertical one is not kept (#1018). Equal axes, the common case, open silently.

use crate::common;

use common::*;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, Metadata, SampleType as CSample};
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::*;
use photocraft_psd::resources::ids;
use photocraft_psd::testgen;
use photocraft_psd::{Compression, ImageResource, ResolutionInfo, Version};

fn resolution_notes(r: &ImportResult) -> Vec<&String> {
    r.warnings.iter().filter(|w| w.contains("vertical resolution")).collect()
}

fn png(dpi: (f32, f32)) -> Vec<u8> {
    let meta = Metadata { dpi: Some(dpi), ..Default::default() };
    let img = Image::from_raw(2, 2, ChannelLayout::Rgb, CSample::U8, vec![10u8; 12]).unwrap().with_meta(meta);
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

#[test]
fn png_with_unequal_axes_reports_the_dropped_vertical_resolution() {
    let r = import("x.png", &png((300.0, 150.0))).unwrap();
    assert!((r.document.resolution_dpi - 300.0).abs() < 0.01, "{}", r.document.resolution_dpi);
    let notes = resolution_notes(&r);
    assert_eq!(notes.len(), 1, "{:?}", r.warnings);
    assert!(notes[0].contains("150.01") && notes[0].contains("300.00"), "{notes:?}");
}

#[test]
fn png_with_equal_axes_opens_silently() {
    let r = import("x.png", &png((300.0, 300.0))).unwrap();
    assert!((r.document.resolution_dpi - 300.0).abs() < 0.01, "{}", r.document.resolution_dpi);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

fn psd(ri: ResolutionInfo) -> Vec<u8> {
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    f.resources.retain(|r| r.id != ids::RESOLUTION_INFO);
    f.resources.push(ImageResource::new(ids::RESOLUTION_INFO, ri.to_bytes()));
    f.to_bytes().unwrap()
}

#[test]
fn psd_with_unequal_axes_reports_the_dropped_vertical_resolution() {
    let ri = ResolutionInfo { v_res_fixed: ResolutionInfo::from_dpi(150.0).v_res_fixed, ..ResolutionInfo::from_dpi(300.0) };
    let r = import("x.psd", &psd(ri)).unwrap();
    assert!((r.document.resolution_dpi - 300.0).abs() < 1e-3, "{}", r.document.resolution_dpi);
    let notes = resolution_notes(&r);
    assert_eq!(notes.len(), 1, "{:?}", r.warnings);
    assert!(notes[0].contains("150.00") && notes[0].contains("300.00"), "{notes:?}");
}

#[test]
fn psd_with_equal_axes_opens_silently() {
    let r = import("x.psd", &psd(ResolutionInfo::from_dpi(300.0))).unwrap();
    assert!((r.document.resolution_dpi - 300.0).abs() < 1e-3, "{}", r.document.resolution_dpi);
    assert!(resolution_notes(&r).is_empty(), "{:?}", r.warnings);
    // Each axis is read in its own unit: 100 pixels per centimetre on both is 254 ppi on both.
    let mut cm = ResolutionInfo::from_dpi(100.0);
    cm.h_res_unit = 2;
    cm.v_res_unit = 2;
    let r = import("x.psd", &psd(cm)).unwrap();
    assert!((r.document.resolution_dpi - 254.0).abs() < 1e-3, "{}", r.document.resolution_dpi);
    assert!(resolution_notes(&r).is_empty(), "{:?}", r.warnings);
}

/// A layered TIFF opens through the PSD importer with the TIFF's own resolution tags.
fn layered_tiff(dpi: (f32, f32)) -> Vec<u8> {
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    let out = export(&d, "x.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).unwrap();
    let mut img = photocraft_codecs::decode(&out.bytes).unwrap();
    assert!(img.meta.photoshop_layers.is_some(), "layer data");
    img.meta.dpi = Some(dpi);
    photocraft_codecs::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap()
}

#[test]
fn layered_tiff_with_unequal_axes_reports_the_dropped_vertical_resolution() {
    let r = import("x.tif", &layered_tiff((300.0, 150.0))).unwrap();
    assert!(r.document.layers.len() > 1, "opened with its layers");
    assert!((r.document.resolution_dpi - 300.0).abs() < 1e-3, "{}", r.document.resolution_dpi);
    let notes = resolution_notes(&r);
    assert_eq!(notes.len(), 1, "{:?}", r.warnings);
    assert!(notes[0].contains("150.00") && notes[0].contains("300.00"), "{notes:?}");
    // Equal axes: no note.
    let r = import("x.tif", &layered_tiff((300.0, 300.0))).unwrap();
    assert!(resolution_notes(&r).is_empty(), "{:?}", r.warnings);
}
