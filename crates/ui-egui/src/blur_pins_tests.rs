//! Blur Gallery glue: the bounds a dialog opens with, and the engine's pin payload per kind.

use craft::imaging::ui::{BlurKind, BlurPin, BlurPins, IrisShape, TiltBand};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use serde_json::json;

use crate::PhotocraftApp;
use crate::blur_pins::{COMMAND, IRIS, SPIN, initial, kind_of, pins_of, write_pins};

fn app_with(selection: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 100, "y": 50, "width": 200, "height": 100})).unwrap();
    }
    app
}

#[test]
fn it_opens_with_one_selected_pin_in_the_middle_of_the_bounds() {
    for (selection, want) in [(false, [0.0, 0.0, 400.0, 300.0]), (true, [100.0, 50.0, 300.0, 150.0])] {
        let mut app = app_with(selection);
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(app.framework.dialogs.get::<HostedParamDialog>().is_some(), "the dialog is hosted");
        let (kind, bounds, start) = initial(&mut app, COMMAND).unwrap();
        assert_eq!((kind, bounds), (BlurKind::Field, want));
        let pins = BlurPins::new(kind, bounds, start);
        assert_eq!((pins.pins.len(), pins.selected, pins.bounds), (1, Some(0), want));
    }
    assert!(initial(&mut app_with(false), "filter.gaussianBlur").is_none());
}

/// The pins of each kind, written as the engine's command keys and read back.
#[test]
fn each_kind_writes_the_engines_pin_payload() {
    let bounds = [0.0, 0.0, 400.0, 300.0];
    let mut iris = BlurPin::new(0.5, 0.5, 20.0);
    iris.iris = Some(IrisShape::new(0.2, 0.1, 30.0, 40.0, 0.5));
    // Spin has neither roundness nor feather, so its pin carries the Spin defaults for both.
    let spin_shape = IrisShape::spin();
    let mut spin = BlurPin::new(0.5, 0.5, 20.0);
    spin.iris = Some(IrisShape::new(0.2, 0.1, 30.0, spin_shape.roundness, spin_shape.feather));
    let mut tilt = BlurPin::new(0.4, 0.6, 20.0);
    tilt.tilt = Some(TiltBand::new(20.0, 0.2, 0.3));
    let table = [
        (COMMAND, BlurPin::new(0.5, 0.5, 20.0), vec!["blur", "x", "y"]),
        (IRIS, iris, vec!["angle", "blur", "feather", "radiusX", "radiusY", "roundness", "x", "y"]),
        (SPIN, spin, vec!["angle", "blurAngle", "radiusX", "radiusY", "x", "y"]),
    ];
    for (command, start, keys) in table {
        let kind = kind_of(command).unwrap();
        let pins = BlurPins::new(kind, bounds, start);
        let mut fields = DialogFields::new();
        write_pins(&pins, |k, v| fields.set(k, v));
        let Some(FieldValue::Json(list)) = fields.get("pins") else { panic!("{command}: pins are Json") };
        let mut got: Vec<&str> = list[0].as_object().unwrap().keys().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(got, keys, "{command}");
        for top in ["centerX", "radiusX"] {
            assert!(!fields.contains(top), "{command}: no top-level {top}");
        }
        assert_eq!(pins_of(&fields, kind, bounds), pins, "{command}: round trip");
    }
    let pins = BlurPins::new(BlurKind::TiltShift, bounds, tilt);
    let mut fields = DialogFields::new();
    write_pins(&pins, |k, v| fields.set(k, v));
    for key in ["centerX", "centerY", "angle", "focus", "transition", "blur"] {
        assert!(fields.number(key).is_some(), "Tilt-Shift {key}");
    }
    assert!(!fields.contains("pins"));
    assert_eq!(pins_of(&fields, BlurKind::TiltShift, bounds), pins);
    // The untagged read of an array of objects is a list of Json: the same pins.
    let mut fields = DialogFields::new();
    fields.set("pins", FieldValue::List(vec![FieldValue::Json(json!({"x": 0.1, "y": 0.2, "blur": 7.0}))]));
    assert_eq!(pins_of(&fields, BlurKind::Field, bounds).pins, vec![BlurPin::new(0.1, 0.2, 7.0)]);
}
