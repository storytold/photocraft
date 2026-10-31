use photocraft_doc::{Color, Effect, LayerContent, LayerId};
use photocraft_engine::Session;
use photocraft_psd::grd::{GrdColor, GrdGradient, GrdStop, write};
use serde_json::json;

/// Selecting an imported gradient styles editable text instead of only changing the tool.
#[test]
fn imported_gradient_selection_applies_to_text() {
    let preset = GrdGradient {
        name: "Imported Text Gradient".into(),
        stops: vec![
            GrdStop { location: 0.0, midpoint: 0.5, color: GrdColor::Rgb([1.0, 0.0, 0.0]) },
            GrdStop { location: 1.0, midpoint: 0.5, color: GrdColor::Rgb([0.0, 0.0, 1.0]) },
        ],
        ..Default::default()
    };
    let data = photocraft_engine::paint::tile::b64_encode(&write(&[preset]));
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "depth": depth})).unwrap();
        let text = s.execute("type.create", json!({"text": "Gradient", "size": 24, "x": 10, "y": 50, "color": "#000000"})).unwrap();
        let id = LayerId(text["layer"].as_u64().unwrap());
        s.execute("gradient.presets.importGrd", json!({"data": data, "group": "Imported"})).unwrap();
        let doc = &s.active().unwrap().doc;
        let before = photocraft_compose::render(doc, doc.bounds());
        s.execute("gradient.presets.select", json!({"preset": "Imported Text Gradient"})).unwrap();
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        assert!(matches!(&layer.content, LayerContent::Text(t) if t.text == "Gradient"), "text stays editable");
        let overlay = layer.effects.items.iter().find_map(|fx| match fx {
            Effect::GradientOverlay { gradient, .. } => Some(gradient),
            _ => None,
        });
        let overlay = overlay.expect("the selected text receives a gradient overlay");
        assert_eq!(overlay.stops, vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))]);
        let doc = &s.active().unwrap().doc;
        let after = photocraft_compose::render(doc, doc.bounds());
        assert_ne!(after.px, before.px, "the gradient changes the rendered text");
        assert!(after.px.iter().any(|p| p[3] > 0.5 && p[0] + p[2] - 2.0 * p[1] > 0.1), "the text has colored pixels");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).unwrap().effects.items.is_empty(), "one undo removes the overlay");
        let doc = &s.active().unwrap().doc;
        assert_eq!(photocraft_compose::render(doc, doc.bounds()).px, before.px, "undo restores the rendered text");
    }
}

fn text_session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = s.execute("type.create", json!({"text": "Gradient", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    (s, LayerId(id))
}

/// Changing the preset preserves overlay geometry, blending and unrelated layer effects.
#[test]
fn gradient_selection_preserves_existing_text_style() {
    let (mut s, id) = text_session();
    s.execute("layer.layerStyle.dropShadow", json!({"distance": 7})).unwrap();
    s.execute(
        "layer.layerStyle.gradientOverlay",
        json!({"angle": 37, "scale": 80, "style": "radial", "reverse": true, "opacity": 45, "blend": "multiply", "method": "perceptual"}),
    )
    .unwrap();
    let before = s.active().unwrap().doc.layer(id).unwrap().effects.items.clone();
    s.execute("gradient.presets.select", json!({"stops": [[0, "#ff0000"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 25]]})).unwrap();
    let after = &s.active().unwrap().doc.layer(id).unwrap().effects.items;
    assert_eq!(after.len(), before.len(), "replace the overlay instead of adding another");
    assert_eq!(after[0], before[0], "the drop shadow stays unchanged");
    let mut expected = before[1].clone();
    if let Effect::GradientOverlay { gradient, interpolation, .. } = &mut expected {
        gradient.stops = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
        gradient.opacity_stops = vec![(0.0, 1.0), (1.0, 0.25)];
        *interpolation = None;
    }
    assert_eq!(after[1], expected);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().effects.items, before);
}

/// Locked text rejects styling, while tool-only selection remains available.
#[test]
fn gradient_selection_respects_text_locks_and_tool_only_selection() {
    for locks in [json!({"all": true}), json!({"pixels": true})] {
        let (mut s, id) = text_session();
        s.execute("layer.lockLayers", locks).unwrap();
        let before = s.presets.gradient.clone();
        let err = s.execute("gradient.presets.select", json!({"preset": "Red 01"})).unwrap_err().to_string();
        assert!(err.contains("locked"), "{err}");
        assert_eq!(s.presets.gradient, before, "a refused edit leaves the current preset alone");
        s.execute("gradient.presets.select", json!({"preset": "Red 01", "applyToLayer": false})).unwrap();
        assert_eq!(s.presets.gradient.name, "Red 01");
        assert!(s.active().unwrap().doc.layer(id).unwrap().effects.items.is_empty());
    }
}
