//! #11775, #16513: Record Measurements uses source-specific label prefixes ("Measurement N",
//! "Ruler N", "Count N") with independent per-source sequences.

use serde_json::{Value, json};

use crate::Session;

fn five_counts() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    for i in 0..5 {
        s.execute("count.add", json!({"x": 10 + 10 * i, "y": 10})).unwrap();
    }
    s
}

fn record(s: &mut Session, source: &str) -> Value {
    let r = s.execute("image.analysis.recordMeasurements", json!({"source": source})).unwrap();
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{source}: {r}");
    rows[0]["values"].clone()
}

#[test]
fn a_count_row_is_labelled_count_n() {
    for source in ["count", "auto"] {
        let mut s = five_counts();
        let row = record(&mut s, source);
        assert_eq!(row["label"], json!("Count 1"), "{source}: {row}");
        assert_eq!(row["count"], json!(5), "{source}: {row}");
        assert_eq!(row["source"], json!("Count Tool"), "{source}: {row}");
    }
}

#[test]
fn each_source_uses_its_own_label_prefix_and_independent_sequence() {
    let mut s = five_counts();
    // A failed ruler measurement must not consume "Ruler 1".
    assert!(s.execute("image.analysis.recordMeasurements", json!({"source": "ruler"})).is_err());

    s.execute("image.analysis.rulerTool", json!({"start": [32, 64], "end": [62, 104]})).unwrap();
    let ruler1 = record(&mut s, "ruler");
    assert_eq!(ruler1["source"], json!("Ruler Tool"), "{ruler1}");
    assert_eq!(ruler1["label"], json!("Ruler 1"), "{ruler1}");

    let ruler2 = record(&mut s, "ruler");
    assert_eq!(ruler2["label"], json!("Ruler 2"), "{ruler2}");

    let count1 = record(&mut s, "count");
    assert_eq!(count1["source"], json!("Count Tool"), "{count1}");
    assert_eq!(count1["label"], json!("Count 1"), "{count1}");

    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let sel1 = record(&mut s, "selection");
    assert_eq!(sel1["source"], json!("Selection"), "{sel1}");
    assert_eq!(sel1["label"], json!("Measurement 1"), "{sel1}");

    // `auto` picks the selection over ruler/counts and advances the selection sequence only.
    let auto = record(&mut s, "auto");
    assert_eq!(auto["source"], json!("Selection"), "{auto}");
    assert_eq!(auto["label"], json!("Measurement 2"), "{auto}");

    let count2 = record(&mut s, "count");
    assert_eq!(count2["label"], json!("Count 2"), "{count2}");

    let ruler3 = record(&mut s, "ruler");
    assert_eq!(ruler3["label"], json!("Ruler 3"), "{ruler3}");
}
