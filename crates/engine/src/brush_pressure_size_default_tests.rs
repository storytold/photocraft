//! #11809: the Brush's Pressure for Size starts off, as Pressure for Opacity does, so a pen stroke
//! with the default brush keeps one size whatever the pressure.

use serde_json::json;

use crate::Session;

#[test]
fn both_pressure_toggles_start_off() {
    let mut s = Session::new();
    let b = s.execute("brush.get", json!({})).unwrap();
    assert_eq!(b["pressureSize"], json!(false), "{b}");
    assert_eq!(b["pressureOpacity"], json!(false), "{b}");
}

#[test]
fn a_default_brush_dab_keeps_its_size_at_any_pressure() {
    let bounds = |pressure: f64| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 80, "background": "transparent"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[40, 40, pressure]]})).unwrap();
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
    };
    let (light, firm) = (bounds(0.1), bounds(1.0));
    assert!(!firm.is_empty());
    assert_eq!(light, firm, "a light press paints the same size as a firm one");
}
