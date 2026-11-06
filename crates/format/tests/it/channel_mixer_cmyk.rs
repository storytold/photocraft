//! A CMYK Channel Mixer's ink rows (`Adjustment::ChannelMixer::cmyk`, #8047) survive a `.pcraft`
//! round trip; manifests written before the field existed load as a mixer without them
//! (`cmyk: None`), as `#[serde(default)]` promises.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_format::*;

/// The first layer's adjustment, if the first layer is an adjustment layer.
fn adjustment_of(d: &Document) -> Option<&Adjustment> {
    match &d.layers.first()?.content {
        LayerContent::Adjustment(a) => Some(a),
        _ => None,
    }
}

/// Saves and reloads a CMYK mixer, then drops the `cmyk` key from its manifest entry (what
/// builds before #8047 wrote) and checks the older entry still loads, with `cmyk: None` and the
/// rest of the mixer kept.
#[test]
fn cmyk_mixer_rows_round_trip_and_older_manifests_load_without_them() {
    let mut doc = Document::new("mixer", Size::new(8, 8), ColorMode::Cmyk, SampleType::U8);
    let matrix = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    let rows = [[0.8, 0.3, -0.1, 0.0, 0.0], [0.1, 0.9, 0.0, 0.0, 0.05], [0.0, 0.2, 0.7, 0.0, 0.0], [0.0; 5]];
    let mixer = Adjustment::ChannelMixer { matrix, monochrome: false, cmyk: Some(rows) };
    doc.layers.push(Layer::new("Channel Mixer 1", LayerContent::Adjustment(mixer.clone())));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(adjustment_of(&load_from_bytes(&bytes).unwrap()), Some(&mixer));

    let m = read_manifest(&bytes).unwrap();
    let mut v = serde_json::to_value(&m.document).unwrap();
    let fields = &mut v["layers"][0]["content"]["adjustment"]["ChannelMixer"];
    assert!(fields.as_object_mut().unwrap().remove("cmyk").is_some(), "{fields}");
    let old: manifest::DocM = serde_json::from_value(v).unwrap();
    let manifest::ContentM::Adjustment { adjustment } = &old.layers[0].content else { panic!("not an adjustment layer") };
    assert_eq!(*adjustment, Adjustment::ChannelMixer { matrix, monochrome: false, cmyk: None });

    // An RGB mixer writes no `cmyk` key, so builds before #8047 read it exactly as before.
    let rgb = Adjustment::ChannelMixer { matrix: [[0.5, 0.5, 0.0, 0.0], matrix[1], matrix[2]], monochrome: false, cmyk: None };
    let json = serde_json::to_value(&rgb).unwrap();
    assert!(json["ChannelMixer"].get("cmyk").is_none(), "{json}");
}
