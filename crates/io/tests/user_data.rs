//! A tool's records in a document (`document.userData` in the XMP packet, persistent layer ids)
//! survive the files that keep the layers (PSD, PSB, layered TIFF, `.pcraft`), and flat exports
//! leave the user data out while keeping the rest of the packet (#4733 review).

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Size};
use photocraft_io::{ExportOptions, export, import, user_data};
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

const COPYRIGHT: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">Studio copyright</rdf:li></rdf:Alt></dc:rights></rdf:Description>\
</rdf:RDF></x:xmpmeta>";

fn records() -> Map<String, Value> {
    // Over 64 KB: alone it outgrows a JPEG's XMP segment.
    json!({"pipeline": {"loaded": [{"layer": 41, "version": 12}]}, "notes": "x".repeat(70_000)}).as_object().cloned().unwrap()
}

/// Two layers with persistent ids 41 and 7, and the records in the packet.
fn doc() -> Document {
    let mut d = Document::new("records", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    for (name, id, x) in [("bg", 41, 0), ("fg", 7, 4)] {
        let mut s = Surface::new(d.pixel_format());
        s.fill_rect(photocraft_geom::Rect::new(x, 0, x + 4, 6), &[0.5, 0.4, 0.3, 1.0]);
        let mut l = Layer::new(name, LayerContent::Raster(s));
        l.psd_id = Some(id);
        d.layers.push(l);
    }
    d.metadata.xmp = Some(user_data::write(Some(COPYRIGHT), &records()).unwrap());
    d
}

#[test]
fn flat_exports_leave_the_user_data_out_and_keep_the_rest_of_the_packet() {
    for ext in ["jpg", "png", "webp", "tif"] {
        let r = export(&doc(), &format!("out.{ext}"), &ExportOptions::default()).expect(ext);
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(!text.contains("UserData") && !text.contains("xxxxxxxxxx"), "{ext}: the user data leaked into a flat export");
        // The copyright stays: a JPEG used to lose the whole packet to the oversized property.
        let back = import(&format!("out.{ext}"), &r.bytes).expect(ext).document;
        let xmp = back.metadata.xmp.unwrap_or_default();
        assert!(xmp.contains("Studio copyright"), "{ext}: the rest of the packet must stay: {xmp}");
        assert!(user_data::read(Some(&xmp)).unwrap().is_empty(), "{ext}");
    }
}

#[test]
fn layered_files_keep_the_user_data_and_the_layer_ids() {
    let layered_tiff = ExportOptions { tiff_layers: true, ..ExportOptions::default() };
    let psb = ExportOptions { force_psb: true, ..ExportOptions::default() };
    for (name, opts) in [("out.psd", ExportOptions::default()), ("out.psb", psb), ("out.tif", layered_tiff), ("out.pcraft", ExportOptions::default())] {
        let r = export(&doc(), name, &opts).expect(name);
        let back = import(name, &r.bytes).expect(name).document;
        assert_eq!(user_data::read(back.metadata.xmp.as_deref()).unwrap(), records(), "{name}: user data");
        assert!(back.metadata.xmp.as_deref().unwrap_or_default().contains("Studio copyright"), "{name}");
        let ids: Vec<(String, Option<u32>)> = back.layers.iter().map(|l| (l.name.clone(), l.psd_id)).collect();
        assert_eq!(ids, [("bg".to_string(), Some(41)), ("fg".to_string(), Some(7))], "{name}: layer ids");
    }
}
