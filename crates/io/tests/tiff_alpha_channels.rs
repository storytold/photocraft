//! #5301: a TIFF's alpha channel. Photoshop writes a document's alpha channel as an
//! "unspecified" extra sample (ExtraSamples 0); it opens as an alpha channel over an opaque
//! image, not as transparency, and a document's alpha channel is written back that way.

mod common;
use common::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::{Rect, Size};
use photocraft_io::*;

/// An opaque one-layer document of `mode` with the alpha channels `names`.
fn doc_with_channels(mode: ColorMode, names: &[&str]) -> Document {
    let (w, h) = (40, 30);
    let mut d = Document::new("t", Size::new(w, h), mode, SampleType::U8);
    let canvas = Rect::new(0, 0, w as i32, h as i32);
    d.layers.push(raster("Background", PixelFormat::new(mode, SampleType::U8, true), canvas, 3, false));
    for (i, name) in names.iter().enumerate() {
        d.channels.push(AlphaChannel::new(*name, mask_surface(SampleType::U8, canvas, 0.0, 10 + i as u32)));
    }
    d
}

fn channel_values(d: &Document, i: usize) -> Vec<f32> {
    d.channels[i].surface.read_region(d.bounds())
}

fn tiff(doc: &Document, layers: bool) -> ExportResult {
    export(doc, "x.tif", &ExportOptions { tiff_layers: layers, ..Default::default() }).expect("export")
}

#[test]
fn a_flat_tiff_keeps_one_alpha_channel_with_its_name() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        let d = doc_with_channels(mode, &["Letters"]);
        let r = tiff(&d, false);
        assert!(r.warnings.iter().all(|w| !w.contains("alpha channel")), "{mode:?}: {:?}", r.warnings);
        let img = photocraft_codecs::decode(&r.bytes).unwrap();
        assert!(img.meta.alpha_is_channel, "{mode:?}: written as an unspecified extra sample");
        let back = import("x.tif", &r.bytes).unwrap().document;
        assert_eq!(back.channels.len(), 1, "{mode:?}");
        assert_eq!(back.channels[0].name, "Letters");
        assert_eq!(channel_values(&back, 0), channel_values(&d, 0), "{mode:?}: the channel's values");
        assert_eq!(back.layers.len(), 1);
        assert_eq!(back.layers[0].name, "Background", "{mode:?}: opaque, not transparency");
    }
}

#[test]
fn a_flat_tiff_names_the_alpha_channels_it_leaves_out() {
    let d = doc_with_channels(ColorMode::Rgb, &["Kept", "Second", "Third"]);
    let r = tiff(&d, false);
    assert!(r.warnings.iter().any(|w| w.contains("one alpha channel") && w.contains("\"Second\", \"Third\"")), "{:?}", r.warnings);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Kept"]);
}

#[test]
fn transparency_stays_the_extra_sample_and_the_channels_are_named_as_left_out() {
    let mut d = doc_with_channels(ColorMode::Rgb, &["Letters"]);
    d.layers[0] = raster("Layer 0", PixelFormat::new(ColorMode::Rgb, SampleType::U8, true), Rect::new(5, 5, 20, 20), 3, true);
    let r = tiff(&d, false);
    assert!(r.warnings.iter().any(|w| w.contains("holds the transparency") && w.contains("\"Letters\"")), "{:?}", r.warnings);
    let img = photocraft_codecs::decode(&r.bytes).unwrap();
    assert!(img.layout().has_alpha() && !img.meta.alpha_is_channel);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert!(back.channels.is_empty());
}

#[test]
fn a_layered_tiff_keeps_one_alpha_channel_with_its_name() {
    let d = doc_with_channels(ColorMode::Rgb, &["Letters"]);
    let r = tiff(&d, true);
    assert!(r.warnings.iter().all(|w| !w.contains("alpha channel")), "{:?}", r.warnings);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Letters"]);
    assert_eq!(channel_values(&back, 0), channel_values(&d, 0));
    // Another reader sees an opaque image with an alpha channel, as Photoshop wrote it.
    assert!(photocraft_codecs::decode(&r.bytes).unwrap().meta.alpha_is_channel);
}
