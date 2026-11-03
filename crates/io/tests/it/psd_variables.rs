//! Image › Variables and Data Sets survive a PSD save and reopen (#10128), in resources 7000/7001.

use photocraft_doc::variables::Value;
use photocraft_doc::{ColorMode, DataSet, DataValue, Document, Layer, PixelAlign, PixelMethod, SampleType, Size, VarKind, VariableDef, Variables};
use photocraft_io::variables_map::{DATA_SETS, VARIABLES};
use photocraft_io::{ExportOptions, export, import};

fn template() -> Document {
    let mut d = Document::new("TestTemplate.psd", Size::new(32, 24), ColorMode::Rgb, SampleType::U8);
    let venue = Layer::raster("Venue", d.pixel_format());
    let photo = Layer::raster("Photo", d.pixel_format());
    let (venue_id, photo_id) = (venue.id, photo.id);
    d.layers.push(venue);
    d.layers.push(photo);
    d.variables = Variables {
        defs: vec![
            VariableDef { name: "VenueName".into(), layer: venue_id, kind: VarKind::TextReplacement },
            VariableDef { name: "showPhoto".into(), layer: photo_id, kind: VarKind::Visibility },
            VariableDef {
                name: "photo".into(),
                layer: photo_id,
                kind: VarKind::PixelReplacement { method: PixelMethod::Conform, align: PixelAlign::TopCenter, clip: true },
            },
        ],
        data_sets: vec![DataSet {
            name: "Data Set 1".into(),
            values: vec![
                DataValue { variable: "VenueName".into(), value: Value::Text("The Hall".into()) },
                DataValue { variable: "showPhoto".into(), value: Value::Visibility(false) },
                DataValue { variable: "photo".into(), value: Value::Pixels("photos/a.png".into()) },
            ],
        }],
        active: None,
    };
    d
}

/// Variables with their layers named instead of by id, to compare across a reopen.
fn by_layer_name(d: &Document) -> Vec<(String, String, VarKind)> {
    d.variables.defs.iter().map(|v| (v.name.clone(), d.layer(v.layer).map(|l| l.name.clone()).unwrap_or_default(), v.kind.clone())).collect()
}

fn save_and_reopen(d: &Document) -> (Document, Vec<u8>) {
    let out = export(d, "psd", &ExportOptions::default()).unwrap();
    (import("TestTemplate.psd", &out.bytes).unwrap().document, out.bytes)
}

#[test]
fn variables_and_data_sets_survive_save_as_psd() {
    let d = template();
    let (back, _) = save_and_reopen(&d);
    assert_eq!(by_layer_name(&back), by_layer_name(&d));
    assert_eq!(back.variables.data_sets, d.variables.data_sets);

    // A second save of the reopened file keeps the resources byte for byte.
    let (again, bytes2) = save_and_reopen(&back);
    assert_eq!(by_layer_name(&again), by_layer_name(&d));
    let f1 = photocraft_psd::PsdFile::from_bytes(&export(&back, "psd", &ExportOptions::default()).unwrap().bytes).unwrap();
    let f2 = photocraft_psd::PsdFile::from_bytes(&bytes2).unwrap();
    for id in [VARIABLES, DATA_SETS] {
        let r = |f: &photocraft_psd::PsdFile| f.resources.iter().find(|r| r.id == id).map(|r| r.data.clone());
        assert!(r(&f1).is_some(), "resource {id} written");
        assert_eq!(r(&f1), r(&f2), "resource {id} kept verbatim");
    }
}

#[test]
fn editing_or_clearing_variables_rewrites_the_resources() {
    let (mut back, _) = save_and_reopen(&template());
    back.variables.defs.retain(|v| v.name != "photo");
    back.variables.data_sets[0].values.retain(|v| v.variable != "photo");
    let (edited, _) = save_and_reopen(&back);
    assert_eq!(by_layer_name(&edited), by_layer_name(&back));
    assert_eq!(edited.variables.data_sets, back.variables.data_sets);

    let mut cleared = edited;
    cleared.variables = Variables::default();
    let (none, bytes) = save_and_reopen(&cleared);
    assert!(none.variables.is_empty());
    let f = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
    assert!(f.resources.iter().all(|r| r.id != VARIABLES && r.id != DATA_SETS));
}
