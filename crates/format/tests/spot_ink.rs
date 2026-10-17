//! Spot channels are stored as Photoshop shows them, black = solid ink (#6123). Files written
//! before that stored 1 = solid ink; their spot channels turn the right way round once on load,
//! and nothing else changes.

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Rect, Size};
use photocraft_format::*;

fn rebuild(bytes: &[u8], f: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
    let r = zip::ZipReader::new(bytes).unwrap();
    let mut z = zip::ZipWriter::new();
    for e in &r.entries {
        if let Some(d) = f(&e.name, r.read(e, usize::MAX).unwrap()) {
            z.add(&e.name, &d).unwrap();
        }
    }
    z.finish().unwrap()
}

/// A 4×1 document with a spot channel and an alpha channel holding `spot` and `alpha`. `old`
/// builds the spot channel as older builds did: no ink (0) wherever it has no tiles.
fn doc(spot: [f32; 4], alpha: [f32; 4], depth: SampleType, old: bool) -> Document {
    let mut d = Document::new("spot", Size::new(4, 1), ColorMode::Rgb, depth);
    let fmt = PixelFormat::new(ColorMode::Grayscale, depth, false);
    let mut s = if old { photocraft_raster::Surface::new(fmt) } else { AlphaChannel::paper(fmt) };
    s.write_region(Rect::new(0, 0, 4, 1), &spot);
    let mut a = photocraft_raster::Surface::new(fmt);
    a.write_region(Rect::new(0, 0, 4, 1), &alpha);
    d.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 0.3, 1.0), 0.5)), ..AlphaChannel::new("PMS 300", s) });
    d.channels.push(AlphaChannel::new("Alpha 1", a));
    d
}

fn values(c: &AlphaChannel) -> Vec<f32> {
    c.surface.read_region(Rect::new(0, 0, 4, 1)).chunks_exact(c.surface.channels()).map(|v| v[0]).collect()
}

#[test]
fn spot_channels_save_and_load_unchanged() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = doc([0.0, 0.25, 1.0, 1.0], [1.0, 0.0, 1.0, 0.0], depth, false);
        let bytes = save_to_bytes(&d, &SaveOptions::default()).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(
            &zip::ZipReader::new(&bytes[..])
                .unwrap()
                .entries
                .iter()
                .find(|e| e.name == "manifest.json")
                .map(|e| zip::ZipReader::new(&bytes[..]).unwrap().read(e, usize::MAX).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["document"]["channels"][0]["black_ink"], true, "{depth:?}");
        assert_eq!(load_from_bytes(&bytes).unwrap(), d, "{depth:?}");
    }
}

#[test]
fn an_older_files_spot_channels_are_inverted_once_on_load() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        // Written the old way: ink in the first two pixels stored 1 and 0.75.
        let old = doc([1.0, 0.75, 0.0, 0.0], [1.0, 0.0, 1.0, 0.0], depth, true);
        let bytes = save_to_bytes(&old, &SaveOptions::default()).unwrap();
        let before_flag = rebuild(&bytes, |name, data| {
            if name != "manifest.json" {
                return Some(data);
            }
            let mut m: serde_json::Value = serde_json::from_slice(&data).unwrap();
            for c in m["document"]["channels"].as_array_mut().unwrap() {
                assert!(c.as_object_mut().unwrap().remove("black_ink").is_some());
            }
            Some(serde_json::to_vec(&m).unwrap())
        });
        let loaded = load_from_bytes(&before_flag).unwrap();
        let tol = if depth == SampleType::U8 { 0.5 / 255.0 } else { 1e-4 };
        let spot = values(&loaded.channels[0]);
        for (got, want) in spot.iter().zip([0.0, 0.25, 1.0, 1.0]) {
            assert!((got - want).abs() <= tol, "{depth:?}: spot {spot:?}");
        }
        // Space beyond the old content is paper, not ink.
        assert_eq!(loaded.channels[0].surface.sample_channel(40, 0, 0), 1.0, "{depth:?}");
        assert_eq!(values(&loaded.channels[1]), [1.0, 0.0, 1.0, 0.0], "{depth:?}: the alpha channel is untouched");
        // Saved again, it is a new file and loads the same.
        let again = save_to_bytes(&loaded, &SaveOptions::default()).unwrap();
        assert_eq!(load_from_bytes(&again).unwrap(), loaded, "{depth:?}");
    }
}
