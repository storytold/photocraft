//! DTF regression: a spot is a fifth sample, never a second transparency sample.
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export, import};
use photocraft_raster::Surface;

fn document(depth: SampleType, transparent: bool) -> Document {
    let mut doc = Document::new("DTF", Size::new(3, 1), ColorMode::Rgb, depth);
    let mut art = Surface::new(doc.pixel_format());
    art.write_region(Rect::new(0, 0, 3, 1), &[1., 0., 0., 1., 0., 1., 0., if transparent { 0.5 } else { 1. }, 0., 0., 1., if transparent { 0. } else { 1. }]);
    doc.layers.push(Layer::new("Artwork", LayerContent::Raster(art)));
    let mut plate = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    plate.write_region(doc.bounds(), &[0., 0.5, 1.]);
    let mut channel = AlphaChannel::new("w1", plate);
    channel.spot = Some((Color::rgb(1., 1., 1.), 1.));
    doc.channels.push(channel);
    doc
}

fn raster(doc: &Document) -> Vec<u8> {
    let LayerContent::Raster(surface) = &doc.layers[0].content else { panic!("raster layer expected") };
    surface.to_interleaved(doc.bounds())
}

fn assert_spot(a: &Document, b: &Document) {
    assert_eq!(b.channels.len(), 1);
    assert_eq!(b.channels[0].name, "w1");
    assert_eq!(b.channels[0].spot, Some((Color::rgb(1., 1., 1.), 1.)));
    assert_eq!(a.channels[0].surface.to_interleaved(a.bounds()), b.channels[0].surface.to_interleaved(b.bounds()));
}

#[test]
fn rgb_transparency_and_white_spot_roundtrip_all_depths() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for transparent in [false, true] {
            let doc = document(depth, transparent);
            for layered in [false, true] {
                let opts = ExportOptions { tiff_layers: layered, ..Default::default() };
                let out = export(&doc, "dtf.tif", &opts).unwrap();
                assert!(out.warnings.is_empty(), "{:?}", out.warnings);
                let img = photocraft_codecs::decode(&out.bytes).unwrap_or_else(|e| panic!("{depth:?} transparent={transparent} layered={layered}: {e}"));
                assert_eq!(img.meta.tiff_extra_samples.len(), 1);
                let plate = &img.meta.tiff_extra_samples[0];
                match depth {
                    SampleType::U8 => {
                        assert_eq!(plate.first(), Some(&255));
                        assert_eq!(plate.last(), Some(&0));
                    }
                    SampleType::U16 => {
                        assert_eq!(&plate[..2], &u16::MAX.to_ne_bytes());
                        assert_eq!(&plate[4..], &0u16.to_ne_bytes());
                    }
                    SampleType::F32 => {
                        assert_eq!(&plate[..4], &1f32.to_ne_bytes());
                        assert_eq!(&plate[8..], &0f32.to_ne_bytes());
                    }
                }
                assert_eq!(img.layout().has_alpha(), transparent);
                let back = import("dtf.tif", &out.bytes).unwrap();
                assert!(back.warnings.is_empty(), "{:?}", back.warnings);
                assert_spot(&doc, &back.document);
                // Compare RGB and straight transparency against the PSD path, which shares the compositor.
                let psd = export(&doc, "dtf.psd", &Default::default()).unwrap();
                let psd_back = import("dtf.psd", &psd.bytes).unwrap().document;
                assert_spot(&doc, &psd_back);
                if layered {
                    assert_eq!(raster(&back.document), raster(&psd_back));
                } else {
                    let pixels = |d: &Document| {
                        let LayerContent::Raster(s) = &d.layers[0].content else { panic!("raster") };
                        s.read_region(d.bounds())
                    };
                    let (tiff_px, psd_px) = (pixels(&back.document), pixels(&psd_back));
                    for (a, b) in tiff_px.as_chunks::<4>().0.iter().zip(psd_px.as_chunks::<4>().0) {
                        assert!((a[3] - b[3]).abs() < 1e-6);
                        for i in 0..3 {
                            assert!((a[i] * a[3] - b[i] * b[3]).abs() < 1.0 / 255.0);
                        }
                    }
                }
            }
        }
    }
}

/// Supplied large oracles remain outside source control. Set PHOTOCRAFT_SPOT_PSD/TIFF
/// to run this test; absence is an explicit ignored test, not a passing oracle check.
#[test]
#[ignore = "requires supplied Photoshop PSD via PHOTOCRAFT_SPOT_PSD"]
fn supplied_photoshop_psd() {
    check_reference("PHOTOCRAFT_SPOT_PSD");
}

#[test]
#[ignore = "requires supplied Photoshop TIFF via PHOTOCRAFT_SPOT_TIFF"]
fn supplied_photoshop_tiff() {
    check_reference("PHOTOCRAFT_SPOT_TIFF");
}

fn check_reference(var: &str) {
    let path = std::env::var(var).expect("set the reference path");
    let bytes = std::fs::read(&path).unwrap();
    let input = import(&path, &bytes).unwrap();
    let doc = input.document;
    assert_eq!(doc.channels.len(), 1);
    assert_eq!(doc.channels[0].name, "w1");
    assert_eq!(doc.channels[0].spot, Some((Color::rgb(1., 1., 1.), 1.)));
    for layered in [false, true] {
        let out = export(&doc, "reference.tif", &ExportOptions { tiff_layers: layered, ..Default::default() }).unwrap();
        let back = import("reference.tif", &out.bytes).unwrap();
        assert_spot(&doc, &back.document);
        if let Ok(dir) = std::env::var("PHOTOCRAFT_SPOT_OUTPUT") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(format!("{dir}/{var}-{layered}.tif"), out.bytes).unwrap();
        }
    }
}

#[test]
fn damaged_layer_data_falls_back_to_composite_without_losing_spot() {
    let doc = document(SampleType::U8, true);
    let mut out = export(&doc, "dtf.tif", &ExportOptions { tiff_layers: true, ..Default::default() }).unwrap();
    let signature = photocraft_psd::tiff::SIGNATURE;
    let at = out.bytes.windows(signature.len()).position(|s| s == signature).unwrap();
    out.bytes[at] = b'X';
    let back = import("dtf.tif", &out.bytes).unwrap();
    assert!(back.warnings.iter().any(|w| w.contains("opened flattened")));
    assert_spot(&doc, &back.document);
}
