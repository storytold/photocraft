//! #11799: the Perspective Crop refuses a document that holds a type layer or a smart object, and
//! a Bitmap mode document, with the reference application's alerts, before anything is rasterized
//! or warped. Nothing about the document changes on a refusal: no history step, no warped pixels,
//! and the other layers keep their kind.
//!
//! Measured (issue #11799): the alert fires for the *document*, not the active layer, so a type
//! layer or a smart object counts wherever it sits - visible or hidden, active or not, also inside
//! a group. A Levels adjustment layer, a Color Fill layer, a rectangle shape layer, a group, a
//! mask, a style, a locked and a hidden layer do not trigger it: those documents still crop.
//! Keeping shape and fill layers live instead of rasterizing them is #11800, so this file pins
//! only that they still crop.

use photocraft_doc::{Color, ColorMode, Document, Fill, Group, Layer, LayerContent, SampleType, ShapeLayer, Size, SmartObject, SmartSource, TextLayer};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::Session;

/// What the reference shows (an alert with one OK button) for a type layer or a smart object.
const UNSUPPORTED: &str = "Could not use the perspective crop tool because the image contains unsupported layers types.";
/// What it shows for a Bitmap mode document.
const NOT_BITMAP: &str = "Could not use the perspective crop tool because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

const CMD: &str = "image.perspectiveCrop";
/// A skewed quad on the 160 x 120 fixture, and the upright output it is mapped onto.
const QUAD: [[f64; 2]; 4] = [[30.0, 12.0], [140.0, 25.0], [128.0, 110.0], [14.0, 95.0]];

fn crop_params() -> Value {
    json!({"corners": QUAD, "width": 100, "height": 80})
}

fn rgb_format() -> photocraft_color::PixelFormat {
    Document::with_background("f", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE).pixel_format()
}

/// 160 x 120 RGB/8 with a white Background and `extra` on top, in a fresh session: nothing has
/// been edited yet, so any history step the crop records shows up as `past_len() > 0`.
fn session(extra: Vec<Layer>) -> Session {
    let mut doc = Document::with_background("refuse", Size::new(160, 120), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    for l in extra {
        doc.insert_above(None, l);
    }
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

/// A type layer reading "Hello", visible or hidden.
fn type_layer(visible: bool) -> Layer {
    let mut l = Layer::new("Hello", LayerContent::Text(TextLayer { text: "Hello".into(), ..Default::default() }));
    l.visible = visible;
    l
}

/// A smart object with a red cache and a linked source, visible or hidden.
fn smart_layer(visible: bool) -> Layer {
    let mut cache = Surface::new(rgb_format());
    cache.fill_rect(Rect::from_xywh(20, 20, 60, 40), &[1.0, 0.0, 0.0, 1.0]);
    let mut l = Layer::new("Photo", LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: "photo.psd".into() }, Affine::IDENTITY, Some(cache))));
    l.visible = visible;
    l
}

/// A red rectangle shape layer: what the crop rasterizes today alongside a type layer, so a
/// refusal that comes before the rasterize leaves it a shape.
fn shape_layer() -> Layer {
    let path = photocraft_vector::shapes::rect(20.0, 20.0, 40.0, 30.0);
    Layer::new("Rectangle 1", LayerContent::Shape(ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(0.8, 0.2, 0.1))), ..Default::default() }))
}

/// A group holding `children`, visible or hidden.
fn group_of(children: Vec<Layer>, visible: bool) -> Layer {
    let mut l = Layer::new("Group 1", LayerContent::Group(Group { children, expanded: true, artboard: None, frame_stroke: None }));
    l.visible = visible;
    l
}

/// Every layer as a refusal must leave it: name, kind, visibility and, for a type layer, its text.
type LayerRecord = (String, &'static str, bool, String);

/// What a refusal must leave alone: the layers, the canvas size, the document revision, the steps.
type Snapshot = (Vec<LayerRecord>, Size, u64, usize);

/// Every layer in `doc`, bottom to top. A refusal leaves this list exactly as it was, so it is both
/// the layer-kind and the text check.
fn fingerprint(doc: &Document) -> Vec<LayerRecord> {
    doc.walk()
        .iter()
        .map(|(_, _, l)| {
            let text = match &l.content {
                LayerContent::Text(t) => t.text.clone(),
                _ => String::new(),
            };
            (l.name.clone(), l.content.kind_name(), l.visible, text)
        })
        .collect()
}

/// What a refusal must leave alone.
fn snapshot(s: &Session) -> Snapshot {
    let d = s.active().unwrap();
    (fingerprint(&d.doc), d.doc.size, d.revision, d.history.past_len())
}

/// The document holds a type layer or a smart object, wherever it sits; every other layer keeps
/// its kind, because the refusal comes before `rasterize_all`.
fn assert_refused(s: &mut Session, want: &str, case: &str) {
    let before = snapshot(s);
    let err = s.execute(CMD, crop_params()).unwrap_err().to_string();
    assert_eq!(err, want, "{case}");
    let d = s.active().unwrap();
    assert_eq!(fingerprint(&d.doc), before.0, "{case}: layers unchanged");
    assert_eq!(d.doc.size, before.1, "{case}: canvas unchanged");
    assert_eq!(d.revision, before.2, "{case}: document unchanged");
    assert_eq!(d.history.past_len(), 0, "{case}: no history step");
}

/// A type layer anywhere in the document - visible, hidden, or inside a group - is the reference's
/// unsupported-layers alert, not a rasterization (#11799).
#[test]
fn a_type_layer_anywhere_refuses_the_perspective_crop() {
    for (case, extra) in [
        ("visible", vec![type_layer(true), shape_layer()]),
        ("hidden", vec![type_layer(false), shape_layer()]),
        ("in a group", vec![group_of(vec![type_layer(true), shape_layer()], true), shape_layer()]),
        ("in a hidden group", vec![group_of(vec![type_layer(true)], false), shape_layer()]),
    ] {
        let mut s = session(extra);
        assert_refused(&mut s, UNSUPPORTED, &format!("{case} type layer"));
        let d = s.active().unwrap();
        let text: Vec<&str> = d
            .doc
            .walk()
            .iter()
            .filter_map(|(_, _, l)| match &l.content {
                LayerContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, ["Hello"], "{case}: the type layer is still a type layer with its text");
        assert!(d.doc.walk().iter().any(|(_, _, l)| l.content.kind_name() == "Shape"), "{case}: nothing was rasterized");
    }
}

/// The same for a smart object: visible, hidden, or inside a group.
#[test]
fn a_smart_object_anywhere_refuses_the_perspective_crop() {
    for (case, extra) in [
        ("visible", vec![smart_layer(true), shape_layer()]),
        ("hidden", vec![smart_layer(false), shape_layer()]),
        ("in a group", vec![group_of(vec![smart_layer(false)], true), shape_layer()]),
    ] {
        let mut s = session(extra);
        assert_refused(&mut s, UNSUPPORTED, &format!("{case} smart object"));
    }
}

/// Both at once is still one refusal with the same message, and still changes nothing.
#[test]
fn a_type_layer_and_a_smart_object_refuse_once() {
    let mut s = session(vec![type_layer(false), smart_layer(true), shape_layer()]);
    assert_refused(&mut s, UNSUPPORTED, "type layer and smart object");
}

/// A Bitmap mode document is the reference's Bitmap alert (#11799).
#[test]
fn a_bitmap_document_refuses_the_perspective_crop() {
    let doc = Document::with_background("bitmap", Size::new(160, 120), ColorMode::Bitmap, SampleType::U8, Color::WHITE);
    let mut s = Session::new();
    s.add_document(doc, None);
    assert_refused(&mut s, NOT_BITMAP, "Bitmap document");
}

/// The measured rows that keep working: a document of only pixel, adjustment, Color Fill, shape
/// and group layers still crops, in one undo step (#11799). Whether the shape and the fill stay
/// live is #11800, so this pins the crop, not their kind.
#[test]
fn pixel_adjustment_fill_shape_and_group_layers_still_crop() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500})).unwrap();
    s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#3366cc"})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [200, 300, 150, 100], "fill": "#cc3333"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "G"})).unwrap();
    let before = s.active().unwrap().history.past_len();
    let r = s.execute(CMD, json!({"corners": [[100.0, 50.0], [700.0, 60.0], [690.0, 450.0], [110.0, 440.0]], "width": 600, "height": 400})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(600), Some(400)));
    assert_eq!(s.active().unwrap().doc.size, Size::new(600, 400), "the crop went through");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one undo step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.size, Size::new(800, 500), "one undo restores it");
}

/// Only Bitmap refuses on the mode: RGB, Grayscale and CMYK documents are unaffected (#11799).
#[test]
fn rgb_grayscale_and_cmyk_documents_are_not_refused() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        let doc = Document::with_background("m", Size::new(160, 120), mode, SampleType::U8, Color::WHITE);
        let mut s = Session::new();
        s.add_document(doc, None);
        s.execute(CMD, crop_params()).unwrap();
        assert_eq!(s.active().unwrap().doc.size, Size::new(100, 80), "{mode:?} cropped");
    }
}
