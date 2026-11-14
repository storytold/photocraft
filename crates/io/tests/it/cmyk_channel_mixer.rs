//! A CMYK document's Channel Mixer (`mixr` with four ink records, #8047) through PSD: an imported
//! block nobody edited is saved back exactly as it was read, whatever its layout; an edited one
//! is written in Photoshop's own 44-byte layout. With the `corpus` feature, Photoshop's own CMYK
//! mixers (photoshop corpus cmyk8 and cmyk16 channel-mixer.psd) are read, rendered on their inks
//! against the files' merged images, and saved back byte for byte.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_io::*;
use photocraft_psd::PsdFile;

/// The identity RGB matrix a CMYK mixer carries unused.
const RGB_IDENTITY: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

/// Big-endian i16 words, as a `mixr` block stores them.
fn i16s(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// The `mixr` block data of the first layer record that has one.
fn mixr(file: &PsdFile) -> Option<Vec<u8>> {
    file.layers().iter().find_map(|l| l.block(b"mixr")).map(|b| b.data.clone())
}

/// The document's first adjustment.
fn first_adjustment(d: &Document) -> Option<Adjustment> {
    d.layers.iter().find_map(|l| match &l.content {
        LayerContent::Adjustment(a) => Some(a.clone()),
        _ => None,
    })
}

/// Exports `d` and parses the PSD it became.
fn exported(d: &Document) -> PsdFile {
    PsdFile::from_bytes(&export(d, "x.psd", &ExportOptions::default()).unwrap().bytes).unwrap()
}

/// A CMYK PSD with a Channel Mixer layer over a white background whose `mixr` block is `block`,
/// as another writer could have stored it.
fn psd_with_mixr(block: &[u8]) -> Vec<u8> {
    let mut d = Document::with_background("mixer", Size::new(4, 4), ColorMode::Cmyk, SampleType::U8, Color::WHITE);
    let adj = Adjustment::ChannelMixer { matrix: RGB_IDENTITY, monochrome: false, cmyk: Some(photocraft_doc::adjust::CMYK_MIXER_IDENTITY) };
    d.layers.push(Layer::new("Channel Mixer 1", LayerContent::Adjustment(adj)));
    let mut file = exported(&d);
    let b = file.layers_mut().iter_mut().flat_map(|l| l.blocks.iter_mut()).find(|b| b.key == *b"mixr").unwrap();
    b.data = block.to_vec();
    file.to_bytes().unwrap()
}

/// A monochrome block carrying its first record alone (padded), and a four-record block followed
/// by bytes our writer never emits: both import as CMYK mixers (the monochrome one with a
/// warning, as no Photoshop sample confirms how it renders), are saved back unchanged while
/// unedited, and are rewritten as Photoshop's 44 bytes once edited.
#[test]
fn unedited_cmyk_mixer_blocks_are_saved_back_as_read() {
    let mut mono = i16s(&[1, 1, 30, 30, 30, 100, 0]);
    mono.extend([0, 0]);
    let mut long = i16s(&[1, 0, 80, 30, -10, 0, 0, 10, 90, 0, 0, 5, 0, 20, 70, 0, 0, 0, 0, 0, 0, 0]);
    long.extend([0, 0, 0, 0]);
    for (block, monochrome) in [(mono, true), (long, false)] {
        let imp = import("x.psd", &psd_with_mixr(&block)).unwrap();
        let warned = imp.warnings.iter().any(|w| w.contains("monochrome CMYK Channel Mixer"));
        assert_eq!(warned, monochrome, "{:?}", imp.warnings);
        let adj = first_adjustment(&imp.document).unwrap();
        assert!(matches!(adj, Adjustment::ChannelMixer { cmyk: Some(_), monochrome: m, .. } if m == monochrome), "{adj:?}");
        assert_eq!(mixr(&exported(&imp.document)), Some(block.clone()), "monochrome {monochrome}: saved back as read");

        let mut doc = imp.document;
        for l in &mut doc.layers {
            if let LayerContent::Adjustment(Adjustment::ChannelMixer { cmyk: Some(rows), .. }) = &mut l.content {
                rows[1][4] = 0.1;
            }
        }
        let edited = mixr(&exported(&doc)).unwrap();
        assert_eq!(edited.len(), 44, "monochrome {monochrome}: edited");
        assert_ne!(edited, block);
    }
}

#[cfg(feature = "corpus")]
mod corpus {
    use super::*;
    use std::path::Path;

    /// Photoshop's CMYK Channel Mixer (photoshop corpus cmyk8 and cmyk16 channel-mixer.psd, made
    /// with cyan 80/30/-10, magenta 10/90/0 +5 and yellow 0/20/70 and no black record): read as
    /// four ink rows with an empty black one, composited on the inks within 2 levels of 255 of
    /// Photoshop's merged image (black ink cleared everywhere), and saved back byte for byte.
    #[test]
    fn photoshop_cmyk_channel_mixers_read_render_and_save_back() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/adjustments");
        let rows = [[0.8, 0.3, -0.1, 0.0, 0.0], [0.1, 0.9, 0.0, 0.0, 0.05], [0.0, 0.2, 0.7, 0.0, 0.0], [0.0; 5]];
        for dir in ["cmyk8", "cmyk16"] {
            let path = root.join(dir).join("channel-mixer.psd");
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run `cargo xtask corpus --all`)", path.display()));
            let file = PsdFile::from_bytes(&bytes).unwrap();
            assert_eq!(mixr(&file).map(|d| d.len()), Some(44), "{dir}: Photoshop's block");
            let imp = import("channel-mixer.psd", &bytes).unwrap();
            let doc = &imp.document;
            assert_eq!(first_adjustment(doc), Some(Adjustment::ChannelMixer { matrix: RGB_IDENTITY, monochrome: false, cmyk: Some(rows) }), "{dir}");

            // Photoshop's merged image: planar big-endian samples, each stored as 1 - ink.
            let h = &file.header;
            let (w, ht) = (h.width as usize, h.height as usize);
            let wide = h.depth == 16;
            let merged = file.decode_merged().unwrap();
            let plane = w * ht * if wide { 2 } else { 1 };
            let ink = |c: usize, i: usize| -> f32 {
                let stored = if wide {
                    let at = c * plane + i * 2;
                    f32::from(u16::from_be_bytes([merged[at], merged[at + 1]])) / 65535.0
                } else {
                    f32::from(merged[c * plane + i]) / 255.0
                };
                1.0 - stored
            };
            let flat = photocraft_compose::flatten_to_surface(doc, doc.pixel_format(), None);
            let mut worst = 0.0f32;
            for y in 0..ht {
                for x in 0..w {
                    for (c, ours) in flat.pixel(x as i32, y as i32).into_iter().take(4).enumerate() {
                        worst = worst.max((ours - ink(c, y * w + x)).abs());
                    }
                }
            }
            assert!(worst <= 2.0 / 255.0, "{dir}: inks differ from Photoshop's by up to {:.2} of 255", worst * 255.0);

            assert_eq!(mixr(&exported(doc)), mixr(&file), "{dir}: saved back as read");
        }
    }
}
