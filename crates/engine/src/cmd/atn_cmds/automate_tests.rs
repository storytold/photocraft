//! #10723: File › Automate › Fit Image steps, the script event `docs/atn.md` measures ("Recorded
//! with source files"): `Msge`, `Hght` and `Wdth` in pixels, `limit` (Don't Enlarge).

use super::steps_automate::{FIT_IMAGE, FIT_IMAGE_EVENT};
use super::*;

const SOURCE: &str = "_atnFitImageDescriptor";

/// The measured descriptor, built here independently of the converter.
fn recorded(message: &str, width: f64, height: f64, dont_enlarge: bool) -> Descriptor {
    Descriptor::new("null")
        .with("Msge", D::Text(UnicodeString::new_nul(message)))
        .with("Hght", D::UnitFloat { unit: *b"#Pxl", value: height })
        .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: width })
        .with("limit", D::Boolean(dont_enlarge))
}

/// A Fit Image step as the reference application records it.
fn fit_item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::name(FIT_IMAGE_EVENT), dictionary: b"Fit Image".to_vec(), descriptor: Some(d), ..Default::default() }
}

/// Load `items` as action "A" through `actions.load`.
fn load(items: Vec<AtnItem>) -> (Session, Value) {
    let file = AtnFile {
        name: UnicodeString::new_nul("Automate"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

/// Action "A" written by `actions.save` and parsed back.
fn saved(s: &mut Session) -> Vec<AtnItem> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items
}

/// The recorded step loads as `file.automate.fitImage` with its three settings and saves back
/// byte for byte: the message, the descriptor class and the key order are the recording's.
#[test]
fn a_recorded_fit_image_step_loads_and_saves_back_unchanged() {
    let mut cjsp = recorded("Recorded message", 1200.0, 900.0, true);
    cjsp.class_id = Id::new("cjsp");
    let mut reordered = recorded("", 640.5, 480.0, false);
    reordered.items.reverse();
    for (d, width, height, dont_enlarge) in [
        (recorded("Recorded message", 1200.0, 900.0, true), 1200.0, 900.0, true),
        (cjsp, 1200.0, 900.0, true),
        (reordered, 640.5, 480.0, false),
        (recorded("", 300.0, 200.0, false), 300.0, 200.0, false),
    ] {
        let (mut s, r) = load(vec![fit_item(d.clone())]);
        assert_eq!(r["unsupported"], json!([]), "{r}");
        let (id, p) = &s.actions.list[0].steps[0];
        assert_eq!(id, FIT_IMAGE);
        assert_eq!((p["width"].as_f64(), p["height"].as_f64(), &p["dontEnlarge"]), (Some(width), Some(height), &json!(dont_enlarge)), "{p}");
        let back = saved(&mut s);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].event, AtnEvent::name(FIT_IMAGE_EVENT));
        assert_eq!(back[0].dictionary, b"Fit Image");
        assert_eq!(back[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(d.to_bytes()));
    }
}

/// The empty-message form in the measured key order is what a step recorded in PhotoCraft saves
/// as, so loading it keeps no descriptor of its own.
#[test]
fn the_canonical_form_carries_only_the_settings() {
    let (s, _) = load(vec![fit_item(recorded("", 300.0, 200.0, false))]);
    assert_eq!(s.actions.list[0].steps[0].1, json!({"width": 300, "height": 200, "dontEnlarge": false}));
    let item = export_item(FIT_IMAGE, &json!({"width": 300, "height": 200, "dontEnlarge": false}), "S").unwrap();
    assert_eq!(item.descriptor.map(|d| d.to_bytes()), Some(recorded("", 300.0, 200.0, false).to_bytes()));
}

/// A loaded step fits the document into the recorded box, and Don't Enlarge keeps a smaller
/// document as it is.
#[test]
fn a_loaded_fit_image_step_plays() {
    for (box_w, box_h, dont_enlarge, expected) in [
        (300.0, 300.0, false, (300, 200)),
        (300.0, 300.0, true, (300, 200)),
        (1200.0, 1200.0, false, (1200, 800)),
        (1200.0, 1200.0, true, (600, 400)),
        (900.0, 100.0, false, (150, 100)),
    ] {
        let (mut s, _) = load(vec![fit_item(recorded("Recorded message", box_w, box_h, dont_enlarge))]);
        s.execute("file.new", json!({"width": 600, "height": 400})).unwrap();
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        let size = s.active().unwrap().doc.size;
        assert_eq!((size.width, size.height), expected, "box {box_w}×{box_h}, don't enlarge {dont_enlarge}");
    }
}

/// Any other form keeps the reference step as it was: a missing, repeated or extra key, a side in
/// another unit or type, a side of zero (the native command reads it as "keep that side"), a
/// non-finite side, a message that isn't text, a limit that isn't a boolean.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let with = |key: &str, v: D| {
        let mut d = recorded("m", 300.0, 200.0, false);
        if let Some(slot) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
            slot.1 = v;
        }
        d
    };
    let mut missing = recorded("m", 300.0, 200.0, false);
    missing.items.retain(|(k, _)| !k.is("limit"));
    let mut repeated = recorded("m", 300.0, 200.0, false);
    repeated.items[3] = (Id::new("Hght"), D::UnitFloat { unit: *b"#Pxl", value: 200.0 });
    let forms = [
        missing,
        repeated,
        recorded("m", 300.0, 200.0, false).with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 72.0 }),
        with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 50.0 }),
        with("Hght", D::UnitFloat { unit: *b"#Rlt", value: 200.0 }),
        with("Wdth", D::Double(300.0)),
        with("Hght", D::Integer(200)),
        with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 0.0 }),
        with("Hght", D::UnitFloat { unit: *b"#Pxl", value: -5.0 }),
        with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: f64::INFINITY }),
        with("Msge", D::Integer(1)),
        with("limit", D::Integer(1)),
        Descriptor::new("null"),
    ];
    for d in forms {
        let (mut s, r) = load(vec![fit_item(d.clone())]);
        assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{d:?}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        let back = saved(&mut s);
        assert_eq!((&back[0].event, back[0].descriptor.as_ref().map(Descriptor::to_bytes)), (&AtnEvent::name(FIT_IMAGE_EVENT), Some(d.to_bytes())));
    }
}

/// A native step saves only when the event holds all of it: both sides set, Don't Enlarge as a
/// boolean, and the default resampling, since the event has no key for another method.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    let saves = |p: Value| export_item(FIT_IMAGE, &p, "S").and_then(|item| item.descriptor);
    assert_eq!(saves(json!({"width": 800, "height": 600})), Some(recorded("", 800.0, 600.0, false)));
    assert_eq!(saves(json!({"width": 800, "height": 600, "dontEnlarge": true, "resample": "bicubic"})), Some(recorded("", 800.0, 600.0, true)));
    assert_eq!(saves(json!({"width": 800, "height": 600, "dontEnlarge": null, "resample": null})), Some(recorded("", 800.0, 600.0, false)));
    for p in [
        json!({"width": 800, "height": 600, "resample": "nearest"}),
        json!({"width": 800, "height": 600, "resample": "bicubicAutomatic"}),
        json!({"width": 800}),
        json!({"width": 800, "height": 0}),
        json!({"width": null, "height": 600}),
        json!({"width": "800", "height": 600}),
        json!({"width": 800, "height": 600, "dontEnlarge": "true"}),
        json!({"width": 800, "height": 600, "scaleStyles": true}),
    ] {
        assert_eq!(saves(p.clone()), None, "{p}");
    }
}

/// A loaded step whose settings were changed saves the new settings with the recorded message,
/// not the stale recorded descriptor; an unchanged one saves the recording.
#[test]
fn changed_settings_save_with_the_recorded_message() {
    let mut original = recorded("Recorded message", 1200.0, 900.0, true);
    original.class_id = Id::new("cjsp");
    let (s, _) = load(vec![fit_item(original.clone())]);
    let mut p = s.actions.list[0].steps[0].1.clone();
    assert!(p.get(SOURCE).is_some(), "{p}");
    assert_eq!(export_item(FIT_IMAGE, &p, "S").and_then(|item| item.descriptor), Some(original));
    p["width"] = json!(500);
    p["dontEnlarge"] = json!(false);
    assert_eq!(export_item(FIT_IMAGE, &p, "S").and_then(|item| item.descriptor), Some(recorded("Recorded message", 500.0, 900.0, false)));
}

/// A damaged recorded descriptor doesn't stop a step with valid settings from saving: it saves
/// as a step recorded in PhotoCraft would (CodeRabbit review on steps_automate.rs:87-95).
#[test]
fn a_damaged_recorded_descriptor_saves_the_settings() {
    let other_event = photocraft_paint::tile::b64_encode(&desc("null").with("Rds ", px(2.0)).to_bytes());
    for damaged in [json!("not base64!"), json!(photocraft_paint::tile::b64_encode(b"not a descriptor")), json!(other_event), json!(7), json!(null)] {
        let p = json!({"width": 800, "height": 600, "dontEnlarge": true, SOURCE: damaged});
        assert_eq!(export_item(FIT_IMAGE, &p, "S").and_then(|item| item.descriptor), Some(recorded("", 800.0, 600.0, true)), "{p}");
    }
}
