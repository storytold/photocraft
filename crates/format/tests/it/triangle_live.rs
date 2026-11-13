//! A live triangle (#13350) is saved in `triangle_live`, beside `live`, so builds from before the
//! kind still open the file and show the saved path.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Fill, Layer, LayerContent, LiveShape, Path, ShapeLayer, Subpath};
use photocraft_format::*;
use serde_json::{Value, json};

fn triangle_json() -> Value {
    json!({"kind": "triangle", "rect": [400.0, 300.0, 200.0, 100.0], "corner_radius": 20.0})
}

fn path() -> Path {
    Path::new(vec![Subpath::polygon(&[(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)])])
}

fn document(live: LiveShape) -> Document {
    let mut doc = Document::new("Triangle", photocraft_doc::Size::new(800, 600), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(
        "Triangle 1",
        LayerContent::Shape(ShapeLayer { path: path(), fill: Some(Fill::Solid(photocraft_doc::Color::BLACK)), live: Some(live), ..Default::default() }),
    ));
    doc
}

fn live(v: Value) -> LiveShape {
    serde_json::from_value(v).unwrap()
}

fn content(doc: &Document) -> Value {
    let bytes = save_to_bytes(doc, &SaveOptions::default()).unwrap();
    let json = serde_json::to_value(read_manifest(&bytes).unwrap()).unwrap();
    json["document"]["layers"][0]["content"].clone()
}

#[test]
fn a_live_triangle_round_trips_through_pcraft() {
    let doc = document(live(triangle_json()));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn a_live_triangle_is_written_beside_live_so_older_builds_open_the_file() {
    let saved = content(&document(live(triangle_json())));
    assert!(saved["live"].is_null());
    assert_eq!(saved["triangle_live"], triangle_json());
    assert_eq!(saved["path"], serde_json::to_value(path()).unwrap());

    // An old reader that knows only the earlier kinds.
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct LegacyShape {
        live: Option<Value>,
        path: Path,
    }
    let legacy: LegacyShape = serde_json::from_value(saved).unwrap();
    assert_eq!(legacy, LegacyShape { live: None, path: path() });
}

#[test]
fn a_three_sided_polygon_from_an_older_file_stays_a_polygon() {
    let doc = document(LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 3, star_ratio: 1.0, corner_radius: 0.0 });
    let saved = content(&doc);
    assert_eq!(saved["live"]["kind"], "polygon");
    assert!(saved.get("triangle_live").is_none());
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn the_triangle_kind_uses_these_json_names() {
    assert_eq!(serde_json::to_value(live(triangle_json())).unwrap(), triangle_json());
    let bare = live(json!({"kind": "triangle", "rect": [0.0, 0.0, 10.0, 10.0]}));
    assert_eq!(serde_json::to_value(bare).unwrap()["corner_radius"], json!(0.0));
}
