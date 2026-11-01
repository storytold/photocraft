//! Editing a Photoshop Vibrance layer keeps the file's Vibrance method (#8838).
//!
//! Photoshop (27.11 in the issue) marks its newer Vibrance method with `useLegacy` false in the
//! `vibA` descriptor, and a block without the key opens with the legacy method. The model carries
//! only the two sliders, so the block regenerated on an edit dropped the key: the edited layer
//! reopened in Photoshop with the legacy method's look. PhotoCraft previews the newer method with
//! the legacy one, so opening such a layer says so.

use crate::common;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Size;
use photocraft_io::*;
use photocraft_psd::PsdFile;
use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value, VersionedDescriptor};

/// A `vibA` block as Photoshop writes it: a one-NUL name, the items, padding to 4 bytes.
fn photoshop_vib(items: &[(&str, Value)]) -> Vec<u8> {
    let mut d = Descriptor::new("null");
    d.name = UnicodeString::new_nul("");
    for (k, v) in items {
        d = d.with(k, v.clone());
    }
    let mut b = VersionedDescriptor::new(d).to_bytes();
    b.resize(b.len().next_multiple_of(4), 0);
    b
}

/// The saved `vibA` descriptor, checking the block is padded to 4 as Photoshop pads it.
fn saved_vib(bytes: &[u8]) -> Descriptor {
    assert_eq!(strict_block_errors(bytes), Vec::<String>::new());
    let f = PsdFile::from_bytes(bytes).unwrap();
    let data = f.layers().iter().find_map(|l| l.block(b"vibA")).map(|b| b.data.clone()).expect("vibA block");
    assert_eq!(data.len() % 4, 0, "vibA is padded to 4 inside its length");
    VersionedDescriptor::parse_prefix(&data).unwrap().0.descriptor
}

/// A Background and a Vibrance layer holding `block` as a PSD would (kept verbatim on save).
fn doc_with(depth: SampleType, block: Vec<u8>) -> Document {
    let mut doc = Document::new("v", Size::new(6, 4), ColorMode::Rgb, depth);
    let fmt = doc.pixel_format();
    doc.layers.push(raster("Background", fmt, doc.bounds(), 3, false));
    doc.layers.push(Layer::new("Vibrance 1", LayerContent::Adjustment(Adjustment::Unsupported { psd_key: "vibA".into(), raw: block.into() })));
    doc
}

/// Saves `doc` in `path`'s format and opens the result, warnings included.
fn reopen(doc: &Document, path: &str) -> ImportResult {
    let saved = export(doc, path, &ExportOptions::default()).unwrap().bytes;
    import(path, &saved).unwrap()
}

/// The open warnings that name the Vibrance layer.
fn vibrance_notes(opened: &ImportResult) -> Vec<&String> {
    opened.warnings.iter().filter(|w| w.contains("\"Vibrance 1\"")).collect()
}

/// Sets the layer's sliders, as an edit in the Properties panel does.
fn set_vibrance(doc: &mut Document, vibrance: f32, saturation: f32) {
    doc.layers[1].content = LayerContent::Adjustment(Adjustment::Vibrance { vibrance, saturation });
}

/// A layer in Photoshop's newer method opens as an editable Vibrance with a note about its
/// preview, and every save after an edit, through PSD or `.pcraft`, keeps the method and the
/// keys the model doesn't carry (here `tint`).
#[test]
fn editing_a_modern_vibrance_keeps_its_method() {
    let tint = Value::Integer(12);
    // Laid out as Photoshop 27.11 writes the newer method (#8838's fixture), plus a `tint` key.
    let modern = photoshop_vib(&[("useLegacy", Value::Boolean(false)), ("vibrance", Value::Integer(28)), ("Strt", Value::Integer(-7)), ("tint", tint.clone())]);
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let opened = reopen(&doc_with(depth, modern.clone()), "x.psd");
        let notes = vibrance_notes(&opened);
        assert_eq!(notes.len(), 1, "{depth:?}: {:?}", opened.warnings);
        assert!(notes[0].contains("newer method") && notes[0].contains("legacy method"), "{depth:?}: {}", notes[0]);
        let mut opened = opened.document;
        assert_eq!(opened.layers[1].content, LayerContent::Adjustment(Adjustment::Vibrance { vibrance: 28.0, saturation: -7.0 }), "{depth:?}");

        // The issue's edit: Vibrance 28 to 29, Saturation unchanged.
        set_vibrance(&mut opened, 29.0, -7.0);
        let edited = export(&opened, "x.psd", &ExportOptions::default()).unwrap().bytes;
        let d = saved_vib(&edited);
        assert_eq!(d.get("useLegacy"), Some(&Value::Boolean(false)), "{depth:?}: the method survives the edit");
        assert_eq!(d.get("tint"), Some(&tint), "{depth:?}: so do keys the model doesn't carry");
        assert_eq!((d.get("vibrance"), d.get("Strt")), (Some(&Value::Integer(29)), Some(&Value::Integer(-7))), "{depth:?}");
        let reopened = import("x.psd", &edited).unwrap();
        assert_eq!(vibrance_notes(&reopened).len(), 1, "{depth:?}: still the newer method");
        assert_eq!(reopened.document.layers[1].content, opened.layers[1].content, "{depth:?}");
        // Saving the reopened file again changes nothing.
        let again = export(&reopened.document, "x.psd", &ExportOptions::default()).unwrap().bytes;
        assert_eq!(saved_vib(&again), d, "{depth:?}");

        // Through the native format: the file's block travels with the layer, and so does the note.
        let native = reopen(&opened, "x.pcraft");
        assert_eq!(vibrance_notes(&native).len(), 1, "{depth:?}: .pcraft open: {:?}", native.warnings);
        let mut native = native.document;
        set_vibrance(&mut native, 40.0, 0.0);
        let d = saved_vib(&export(&native, "x.psd", &ExportOptions::default()).unwrap().bytes);
        assert_eq!(d.get("useLegacy"), Some(&Value::Boolean(false)), "{depth:?}: via .pcraft");
        assert_eq!(d.get("tint"), Some(&tint), "{depth:?}: tint via .pcraft");
        assert_eq!((d.get("vibrance"), d.get("Strt")), (Some(&Value::Integer(40)), None), "{depth:?}: 0 is omitted, as Photoshop omits it");
    }
}

/// Layers in the legacy method, Photoshop's or PhotoCraft's own, stay legacy through an edit and
/// open without a note: PhotoCraft renders them as Photoshop does.
#[test]
fn editing_a_legacy_vibrance_stays_legacy() {
    // The shape of the Photoshop-authored oracles (adjustments/rgb8/vibrance.psd): no method key.
    let legacy = photoshop_vib(&[("vibrance", Value::Integer(60)), ("Strt", Value::Integer(-15))]);
    let opened = reopen(&doc_with(SampleType::U8, legacy), "x.psd");
    assert_eq!(vibrance_notes(&opened), Vec::<&String>::new());
    let mut opened = opened.document;
    set_vibrance(&mut opened, -20.0, 35.0);
    let d = saved_vib(&export(&opened, "x.psd", &ExportOptions::default()).unwrap().bytes);
    assert_eq!(d.get("useLegacy"), None);
    assert_eq!((d.get("vibrance"), d.get("Strt")), (Some(&Value::Integer(-20)), Some(&Value::Integer(35))));
    assert_eq!(vibrance_notes(&reopen(&opened, "x.pcraft")), Vec::<&String>::new());
    // A Vibrance made in PhotoCraft has no file block: it is written fresh, with the legacy method
    // PhotoCraft renders.
    let mut fresh = Document::new("v", Size::new(6, 4), ColorMode::Rgb, SampleType::U8);
    fresh.layers.push(Layer::new("Vibrance 1", LayerContent::Adjustment(Adjustment::Vibrance { vibrance: 10.0, saturation: 5.0 })));
    let saved = export(&fresh, "x.psd", &ExportOptions::default()).unwrap().bytes;
    let d = saved_vib(&saved);
    assert_eq!(d.get("useLegacy"), None);
    assert_eq!((d.get("vibrance"), d.get("Strt")), (Some(&Value::Integer(10)), Some(&Value::Integer(5))));
    assert_eq!(vibrance_notes(&import("x.psd", &saved).unwrap()), Vec::<&String>::new());
}

/// A slider stored as something other than a finite number is not read as 0, which an edit would
/// then write into the file's own descriptor: the block stays unsupported, is saved back byte for
/// byte, and opening says so. A NaN slider used to reach the model, where it made Save as
/// `.pcraft` fail (a manifest cannot hold NaN).
#[test]
fn malformed_vibrance_sliders_stay_unsupported() {
    let legacy_off = ("useLegacy", Value::Boolean(false));
    for bad in [Value::Double(f64::NAN), Value::Double(f64::INFINITY), Value::Text(UnicodeString::new_nul("28")), Value::Boolean(true)] {
        for key in ["vibrance", "Strt"] {
            let block = photoshop_vib(&[(key, bad.clone()), legacy_off.clone()]);
            let opened = reopen(&doc_with(SampleType::U16, block.clone()), "x.psd");
            let what = format!("{key} = {bad:?}");
            let LayerContent::Adjustment(Adjustment::Unsupported { psd_key, raw }) = &opened.document.layers[1].content else {
                panic!("{what}: {:?}", opened.document.layers[1].content)
            };
            assert_eq!((psd_key.as_str(), &raw[..]), ("vibA", &block[..]), "{what}");
            let notes = vibrance_notes(&opened);
            assert_eq!(notes.len(), 1, "{what}: {:?}", opened.warnings);
            assert!(notes[0].contains("Vibrance settings could not be read"), "{what}: {}", notes[0]);
            // Saved back as it was, directly and through the native format.
            let saved = export(&opened.document, "x.psd", &ExportOptions::default()).unwrap().bytes;
            let f = PsdFile::from_bytes(&saved).unwrap();
            assert_eq!(f.layers().iter().find_map(|l| l.block(b"vibA")).map(|b| b.data.clone()), Some(block.clone()), "{what}");
            let native = reopen(&opened.document, "x.pcraft");
            assert_eq!(native.document.layers[1].content, opened.document.layers[1].content, "{what}: .pcraft");
        }
    }
}
