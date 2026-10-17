//! Shape paint JSON preserves model colours and all gradient/pattern placement fields.
use photocraft_color::{Color, ColorMode};
use photocraft_doc::{Fill, GradientStyle, LayerContent, LayerId, ShapeLayer, ShapeStroke};
use photocraft_engine::{
    Session,
    vector_cmds::{shape_fill, shape_paint_params},
};
use serde_json::json;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":48,"depth":depth,"background":"transparent"})).unwrap();
    s
}

fn active_shape(s: &Session) -> &ShapeLayer {
    let st = s.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape layer") };
    shape
}

#[test]
fn exact_paints_roundtrip_through_create_edit_and_undo() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let pattern = s.patterns.items[0].clone();
        let mut fills = vec![
            Fill::Solid(Color::rgba(0.1234567, 0.3737, 0.919191, 0.4737)),
            Fill::Solid(Color { mode: ColorMode::Grayscale, c: [0.314159, 0.0, 0.0, 0.0], alpha: 0.3737 }),
            Fill::Solid(Color { mode: ColorMode::Cmyk, c: [0.17, 0.28, 0.39, 0.41], alpha: 0.73 }),
            Fill::Solid(Color { mode: ColorMode::Lab, c: [57.3, -14.8, 23.9, 0.0], alpha: 0.63 }),
            Fill::Pattern { name: "Stored pattern alias".into(), id: pattern.id, scale: 0.3737, angle: 27.3, link: false, phase: (-7.3, 4.1) },
        ];
        for style in [GradientStyle::Linear, GradientStyle::Radial, GradientStyle::Angle, GradientStyle::Reflected, GradientStyle::Diamond] {
            fills.push(Fill::Gradient {
                stops: vec![(0.1737, Color::rgba(0.12345, 0.34567, 0.56789, 0.61)), (0.9137, Color::rgba(0.9191, 0.8181, 0.7171, 0.82))],
                angle: 37.3,
                scale: 0.3737,
                style,
                reverse: true,
                opacity_stops: vec![(0.07, 0.23), (0.91, 0.79)],
                midpoints: vec![0.27],
                offset: (-0.13, 0.21),
                dither: true,
                align: false,
            });
        }
        for fill in fills {
            let stroke = ShapeStroke { paint: fill.clone(), opacity: 0.3737, width: 2.3, dashes: vec![2.1, 1.3], ..Default::default() };
            let before_parse = s.active().unwrap().doc.clone();
            let paint = shape_paint_params(Some(&fill), Some(&stroke));
            assert_eq!(shape_fill(&s, &paint["fill"]).unwrap(), Some(fill.clone()));
            assert_eq!(s.active().unwrap().doc, before_parse, "view-only paint validation edited the document");
            let mut create = paint.clone();
            create["rect"] = json!([8, 8, 40, 24]);
            s.execute("shape.create", create).unwrap();
            assert_eq!(active_shape(&s).fill, Some(fill.clone()));
            assert_eq!(active_shape(&s).stroke, Some(stroke.clone()));
            s.execute("shape.edit", json!({"fill":"#0000ff","stroke":null})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            s.execute("shape.edit", paint).unwrap();
            assert_eq!(active_shape(&s).fill, Some(fill));
            assert_eq!(active_shape(&s).stroke, Some(stroke));
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            let after = s.active().unwrap().doc.clone();
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, after);
        }
    }
}

#[test]
fn solid_alpha_and_library_patterns_render_and_embed_in_the_same_history_step() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let library = s.patterns.items.clone();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let id = LayerId(s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":[0.17,0.31,0.43,0.3737]})).unwrap()["layer"].as_u64().unwrap());
        let alpha: [f32; 4] = serde_json::from_value(s.execute("document.pixel", json!({"x":24,"y":20})).unwrap()).unwrap();
        assert!((alpha[3] - 0.3737).abs() < 0.005, "depth={depth}: {alpha:?}");
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).is_some());
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        s.execute("shape.edit", json!({"fill":{"pattern":library[0].name},"stroke":{"width":4,"fill":{"pattern":library[1].id}}})).unwrap();
        assert_eq!(s.active().unwrap().doc.patterns.len(), 2);
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        let pixel: [f32; 4] = serde_json::from_value(s.execute("document.pixel", json!({"x":24,"y":20})).unwrap()).unwrap();
        assert!(pixel[3] > 0.99 && pixel[0] > 0.1, "library pattern did not render: {pixel:?}");
        let stroke_pixel: [f32; 4] = serde_json::from_value(s.execute("document.pixel", json!({"x":7,"y":20})).unwrap()).unwrap();
        assert!(stroke_pixel[3] > 0.99, "library pattern stroke did not render: {stroke_pixel:?}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.active().unwrap().doc.patterns.is_empty());
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.patterns.len(), 2);
        // A freshly created patterned shape carries its own tile too.
        let mut fresh = session(depth);
        fresh.execute("shape.create", json!({"rect":[8,8,40,24],"fill":{"pattern":library[0].id}})).unwrap();
        assert_eq!(fresh.active().unwrap().doc.patterns.len(), 1);
        let pixel: [f32; 4] = serde_json::from_value(fresh.execute("document.pixel", json!({"x":24,"y":20})).unwrap()).unwrap();
        assert!(pixel[3] > 0.99);
        fresh.execute("edit.undo", json!({})).unwrap();
        assert!(fresh.active().unwrap().doc.patterns.is_empty());
    }
}

#[test]
fn imported_document_pattern_resolves_without_a_global_library_entry() {
    let mut s = session(16);
    let imported = s.patterns.items[0].clone();
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(imported.clone());
    s.patterns.items.clear();
    let fill = Fill::Pattern { name: "Imported name preserved".into(), id: imported.id, scale: 0.3737, angle: -17.3, link: false, phase: (3.1, -2.7) };
    let paint = shape_paint_params(Some(&fill), None);
    let before = s.active().unwrap().doc.clone();
    assert_eq!(shape_fill(&s, &paint["fill"]).unwrap(), Some(fill.clone()));
    assert_eq!(s.active().unwrap().doc, before);
    let mut create = paint;
    create["rect"] = json!([8, 8, 40, 24]);
    s.execute("shape.create", create).unwrap();
    assert_eq!(active_shape(&s).fill, Some(fill));
    assert_eq!(s.active().unwrap().doc.patterns.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn legacy_defaults_and_malformed_paint_inputs_are_explicit_and_atomic() {
    let mut s = session(8);
    assert_eq!(
        shape_fill(&s, &json!({"gradient":{}})).unwrap(),
        Some(Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], 90.0, 1.0, GradientStyle::Linear, false))
    );
    assert_eq!(shape_fill(&s, &json!("#ff000080")).unwrap(), Some(Fill::Solid(Color::rgba(1.0, 0.0, 0.0, 128.0 / 255.0))));
    s.execute("shape.create", json!({"rect":[8,8,40,24]})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for fill in [
        json!([0, "bad", 1, 0.5]),
        json!([0, 0, 0, 2]),
        json!([0, 0, 0, 1, 1]),
        json!({"gradient":null}),
        json!({"gradient":{"stops":[[0,"#000000"],["bad","#ffffff"]]}}),
        json!({"gradient":{"stops":[[2,"#000000"]]}}),
        json!({"gradient":{"opacityStops":[[0,2]]}}),
        json!({"gradient":{"midpoints":[-0.1]}}),
        json!({"gradient":{"offset":[0]}}),
        json!({"gradient":{"scale":0}}),
        json!({"gradient":{"scale":1e100}}),
        json!({"gradient":{"align":"false"}}),
        json!({"gradient":{"dither":null}}),
        json!({"gradient":{"style":"unknown"}}),
        json!({"pattern":1}),
        json!({"pattern":"missing pattern"}),
        json!({"pattern":"Checkerboard","link":0}),
        json!({"pattern":"Checkerboard","phase":[0,1e100]}),
        json!({"pattern":"Checkerboard","id":1}),
        json!({"color":{"mode":"Rgb","c":[0,0,0,0],"alpha":-1}}),
        json!({"gradient":{"stops":vec![json!([0,"#000000"]);4097]}}),
    ] {
        assert!(shape_fill(&s, &fill).is_err(), "{fill}");
        assert!(s.execute("shape.edit", json!({"fill":fill.clone()})).is_err(), "{fill}");
        assert!(s.execute("shape.create", json!({"rect":[0,0,5,5],"stroke":{"fill":fill}})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
