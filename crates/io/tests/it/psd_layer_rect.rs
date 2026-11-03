//! A resaved PSD keeps each layer record's rect while the layer's pixels still fit inside it,
//! as Photoshop does, instead of cropping margins that are zero in every channel (#10809).

use photocraft_doc::{Document, LayerContent};
use photocraft_io::*;
use photocraft_psd::testgen;
use photocraft_psd::{ChannelData, Compression, LayerRecord, PsdFile, Rect, TaggedBlock, Version};

/// A 3×3 pixel layer at x 1..4: an opaque centre pixel, margins zero in every channel.
fn margins_record() -> LayerRecord {
    let plane = |id: i16| {
        let mut p = vec![0u8; 9];
        p[4] = if id == -1 { 255 } else { 200 };
        ChannelData::new(id, Some(Compression::Raw), p)
    };
    LayerRecord {
        name: b"Margins".to_vec(),
        rect: Rect::from_xywh(1, 0, 3, 3),
        channels: (-1..3).map(plane).collect(),
        blocks: vec![TaggedBlock::unicode_name("Margins")],
        ..Default::default()
    }
}

fn open() -> Document {
    let mut f = testgen::small(Version::Psd, Compression::Raw);
    f.layers_mut().push(margins_record());
    import("x.psd", &f.to_bytes().expect("write psd")).expect("import").document
}

fn saved_rect(d: &Document) -> Rect {
    let bytes = export(d, "x.psd", &ExportOptions::default()).expect("export").bytes;
    let f = PsdFile::from_bytes(&bytes).expect("reparse");
    f.layers().iter().find(|r| r.name() == "Margins").expect("record").rect
}

#[test]
fn an_unedited_layer_keeps_its_stored_rect() {
    let d = open();
    assert_eq!(saved_rect(&d), Rect::from_xywh(1, 0, 3, 3));
    // And again after a second round trip.
    let bytes = export(&d, "x.psd", &ExportOptions::default()).expect("export").bytes;
    let d2 = import("x.psd", &bytes).expect("reimport").document;
    assert_eq!(saved_rect(&d2), Rect::from_xywh(1, 0, 3, 3));
}

#[test]
fn pixels_outside_the_stored_rect_write_their_own_bounds() {
    let mut d = open();
    let l = d.layers.iter_mut().find(|l| l.name == "Margins").expect("layer");
    let LayerContent::Raster(s) = &mut l.content else { panic!("not a pixel layer") };
    s.fill_rect(photocraft_geom::Rect::new(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    // The content now spans x 0..3, y 0..2: past the stored rect's left edge.
    assert_eq!(saved_rect(&d), Rect::from_xywh(0, 0, 3, 2));
}
