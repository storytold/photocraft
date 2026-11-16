//! Recorded identifiers must match Photoshop, even when a local round trip would accept a typo.

use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::json;

/// A small marked document makes canvas translation and image resampling distinguishable.
fn sized_document(width: u32, height: u32, dpi: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":width,"height":height,"background":"white"})).unwrap();
    s.execute("image.imageSize", json!({"resolution":dpi,"resample":"none"})).unwrap();
    s.edit("Mark", |doc, _| {
        doc.layers[0].surface_mut().unwrap().write_region(photocraft_geom::Rect::from_xywh(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Replay saved bytes, so the geometry witness covers import, export and execution together.
fn replay_saved(s: &mut Session, action: &Action) {
    let (bytes, _) = atn_cmds::export("Identifiers", &[action], &[], false).unwrap();
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps, action.steps);
    s.actions.list = imported.actions;
    let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
    assert_eq!(result["ran"], 1);
    assert!(result.get("failed").is_none(), "{result}");
}

/// Percent canvas dimensions use each playback document and leave its pixels unscaled.
#[test]
fn recorded_canvas_percent_resolves_at_playback_and_keeps_an_omitted_side() {
    for both_sides in [false, true] {
        let mut d = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 200.0 })
            .with("Hrzn", enm("HrzL", "Left"))
            .with("Vrtc", enm("VrtL", "Top "));
        if both_sides {
            d = d.with("Hght", D::UnitFloat { unit: *b"#Prc", value: 200.0 });
        }
        let action = recorded(AtnEvent::Code(*b"CnvS"), d);
        assert_eq!(action.steps[0].0, "image.canvasSize", "recorded canvas percentages must map");
        assert_eq!(action.steps[0].1["units"], "percent");
        assert!(matches!(saved(&action).descriptor.unwrap().get("Wdth"), Some(D::UnitFloat {unit,value}) if unit==b"#Prc" && *value==200.0));
        for (width, height) in [(40, 30), (10, 15)] {
            let mut s = sized_document(width, height, 72);
            replay_saved(&mut s, &action);
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(width * 2, if both_sides { height * 2 } else { height }));
            assert_eq!(s.execute("document.pixel", json!({"x":0,"y":0})).unwrap(), json!([1.0, 0.0, 0.0, 1.0]));
            assert_eq!(s.execute("document.pixel", json!({"x":1,"y":0})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]), "canvas expansion must not stretch the mark");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(width, height));
        }
    }
}

/// A missing resampler means print-size editing, which the native point-size resize cannot express.
#[test]
fn review_point_resize_without_interpolation_stays_kept() {
    for key in ["Wdth", "Hght"] {
        for interpolation in [None, Some(D::Boolean(false)), Some(enm("wrong", "Nrst"))] {
            let mut d = Descriptor::new("null").with(key, D::UnitFloat { unit: *b"#Rlt", value: 72.0 });
            if let Some(value) = interpolation {
                d = d.with("Intr", value);
            }
            let action = recorded(AtnEvent::Code(*b"ImgS"), d.clone());
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "point sizes without a valid Intr must stay unsupported");
            assert_eq!(saved(&action).descriptor, Some(d));
        }
    }
}

/// Null optional values have the same export semantics as omitted values.
#[test]
fn review_point_resize_exports_with_null_optional_values() {
    for extras in [json!({"resolution":null}), json!({"percent":null}), json!({"resolution":null,"percent":null})] {
        let mut params = json!({"width":72,"units":"pt","resample":"nearest","scaleStyles":false});
        params.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());
        let action = Action::new("Recorded", vec![("image.imageSize".into(), params)]);
        let (bytes, skipped) = atn_cmds::export("Points", &[&action], &[], false).expect("null values must not prevent point-size export");
        assert!(skipped.is_empty());
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps[0].1, json!({"width":72,"units":"pt","resample":"nearest","scaleStyles":false}));
    }
}

/// The native default resampler must be explicit in a physical-size recording.
#[test]
fn review_point_resize_exports_the_default_resampler() {
    for extras in [json!({}), json!({"resample":null})] {
        let mut params = json!({"width":72,"units":"pt","scaleStyles":false});
        params.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());
        let action = Action::new("Recorded", vec![("image.imageSize".into(), params.clone())]);
        let (bytes, skipped) = atn_cmds::export("Points", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty(), "a point-size export must include its native default resampler");
        assert_eq!(imported.actions[0].steps[0].1["resample"], "bicubic");
        let mut expected = sized_document(40, 20, 144);
        expected.execute("image.imageSize", params).unwrap();
        let mut actual = sized_document(40, 20, 144);
        actual.actions.list = imported.actions;
        let result = actual.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        let (a, b) = (&actual.active().unwrap().doc, &expected.active().unwrap().doc);
        assert_eq!(a.size, b.size);
        assert_eq!(a.layers[0].surface().unwrap().read_region(a.bounds()), b.layers[0].surface().unwrap().read_region(b.bounds()));
    }
}

/// Invalid metadata must keep the whole Copy/Cut event, before a layer or source pixels can change.
#[test]
fn review_copy_cut_opacity_is_validated_before_any_step_runs() {
    for copy in [true, false] {
        for opacity in [D::Double(-1.0), D::Double(101.0), D::Double(f64::NAN), D::Boolean(true), D::UnitFloat { unit: *b"#Pxl", value: 50.0 }] {
            let d = Descriptor::new("null")
                .with("Nw  ", D::Descriptor(Descriptor::new("Lyr ").with("Opct", opacity)))
                .with("Usng", enm("UsrM", "Slct"))
                .with("Cpy ", D::Boolean(copy));
            let action = recorded(AtnEvent::Code(*b"Mk  "), d);
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "invalid opacity must keep Copy/Cut before mutation");
            let mut s = sized_document(8, 6, 72);
            s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":2})).unwrap();
            let before = s.active().unwrap().doc.clone();
            s.actions.list = vec![action];
            let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
            assert!(result.get("failed").is_some());
            assert_eq!(s.active().unwrap().doc, before);
        }
        for opacity in [0.0, 50.0, 100.0] {
            let d = Descriptor::new("null")
                .with("Nw  ", D::Descriptor(Descriptor::new("Lyr ").with("Opct", D::Double(opacity))))
                .with("Usng", enm("UsrM", "Slct"))
                .with("Cpy ", D::Boolean(copy));
            let action = recorded(AtnEvent::Code(*b"Mk  "), d);
            assert!(action.steps.iter().all(|(id, _)| id != atn_cmds::PHOTOSHOP_STEP));
            assert_eq!(action.steps.iter().find(|(id, _)| id == "layer.setProps").unwrap().1["opacity"], opacity / 100.0);
        }
    }
}

/// Every named layer anchor must affect actual playback geometry, not just the saved parameters.
#[test]
fn review_layer_transform_uses_the_recorded_anchor() {
    for depth in [8, 16, 32] {
        for (code, fx, fy) in
            [("Qcs0", 0, 0), ("Qcs4", 1, 0), ("Qcs1", 2, 0), ("Qcs7", 0, 1), ("Qcsa", 1, 1), ("Qcs5", 2, 1), ("Qcs3", 0, 2), ("Qcs6", 1, 2), ("Qcs2", 2, 2)]
        {
            let d = Descriptor::new("null")
                .with("FTcs", enm("QCSt", code))
                .with(
                    "Ofst",
                    D::Descriptor(
                        Descriptor::new("Ofst")
                            .with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: 0.0 })
                            .with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: 0.0 }),
                    ),
                )
                .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
                .with("Hght", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
                .with("Intr", enm("Intp", "Nrst"));
            let action = recorded(AtnEvent::Code(*b"Trnf"), d);
            let mut s = Session::new();
            s.execute("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            s.edit("Mark", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(20, 20, 40, 40), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
            replay_saved(&mut s, &action);
            let state = s.active().unwrap();
            let bounds = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
            assert_eq!(bounds, photocraft_geom::Rect::new(20 + 5 * fx, 20 + 5 * fy, 30 + 5 * fx, 30 + 5 * fy), "{code}, depth {depth}");
            // Measured in the reference application: Qcs6 keeps the bottom centre fixed, Qcs1 the
            // top-right corner.
            match code {
                "Qcs6" => assert_eq!(bounds, photocraft_geom::Rect::new(25, 30, 35, 40), "depth {depth}"),
                "Qcs1" => assert_eq!(bounds, photocraft_geom::Rect::new(30, 20, 40, 30), "depth {depth}"),
                _ => {}
            }
            s.execute("edit.undo", json!({})).unwrap();
            if code == "Qcs0" {
                s.edit("Mark rotation", |doc, active| {
                    doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(22, 30, &[0.0, 1.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
                s.execute("edit.transform", json!({"rotate":90,"reference":"topLeft","interpolation":"nearest"})).unwrap();
                let state = s.active().unwrap();
                assert_eq!(
                    state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds(),
                    photocraft_geom::Rect::new(0, 20, 20, 40)
                );
                assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap().rgba(9, 22), [0.0, 1.0, 0.0, 1.0]);
                s.execute("edit.undo", json!({})).unwrap();
            }
            let before = s.active().unwrap().doc.clone();
            assert!(s.execute("edit.transform", json!({"scaleX":50,"reference":"unknown"})).is_err());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

/// Physical dimensions retain points and resolve current dpi before rounding or style scaling.
#[test]
fn recorded_image_points_use_current_resolution_and_keep_style_scaling() {
    use photocraft_color::{BlendMode, Color};
    use photocraft_doc::{Effect, Effects, FxCommon, FxPaint, StrokeFx, StrokePosition};
    for dpi in [72, 144] {
        for scale_styles in [None, Some(false), Some(true)] {
            let mut d = Descriptor::new("null")
                .with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 72.0 })
                .with("CnsP", D::Boolean(true))
                .with("Intr", enm("Intp", "Nrst"));
            if let Some(value) = scale_styles {
                d = d.with("scaleStyles", D::Boolean(value));
            }
            let action = recorded(AtnEvent::Code(*b"ImgS"), d);
            assert_eq!(action.steps[0].0, "image.imageSize", "recorded point dimensions must map");
            assert_eq!(action.steps[0].1["units"], "pt");
            assert_eq!(action.steps[0].1["width"], 72);
            assert!(matches!(saved(&action).descriptor.unwrap().get("Wdth"), Some(D::UnitFloat {unit,value}) if unit==b"#Rlt" && *value==72.0));
            let mut s = sized_document(40, 20, dpi);
            s.edit("Style", |doc, _| {
                doc.layers[0].effects = Effects {
                    enabled: true,
                    items: vec![Effect::Stroke(StrokeFx {
                        common: FxCommon::new(BlendMode::Normal, 1.0),
                        size: 20.0,
                        position: StrokePosition::Outside,
                        paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
                        overprint: false,
                    })],
                    ..Default::default()
                };
                Ok(())
            })
            .unwrap();
            replay_saved(&mut s, &action);
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.size, photocraft_geom::Size::new(dpi, dpi / 2));
            let Effect::Stroke(stroke) = &doc.layers[0].effects.items[0] else { panic!("stroke lost") };
            assert_eq!(stroke.size, if scale_styles.unwrap_or(false) { 20.0 * dpi as f32 / 40.0 } else { 20.0 });
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(40, 20));
        }
    }
    let d = Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 0.36 }).with("Intr", enm("Intp", "Nrst"));
    let action = recorded(AtnEvent::Code(*b"ImgS"), d);
    let mut s = sized_document(40, 20, 300);
    replay_saved(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(2, 1), "round pixels after point conversion");
}

/// The layered false/false recording writes a class and explicit layer-retaining choices.
#[test]
fn recorded_mode_uses_a_class_and_retains_layers_with_false_flags() {
    let d = Descriptor::new("null").with("T   ", D::Class(class("Grys"))).with("Fltt", D::Boolean(false)).with("Rstr", D::Boolean(false));
    let action = recorded(AtnEvent::Code(*b"CnvM"), d);
    assert_eq!(action.steps, vec![("image.mode.grayscale".into(), json!({}))]);
    let written = saved(&action).descriptor.unwrap();
    assert!(matches!(written.get("T   "),Some(D::Class(c)) if c.class_id.is("Grys")), "mode target must save as a class");
    assert_eq!(written.get("Fltt"), Some(&D::Boolean(false)));
    assert_eq!(written.get("Rstr"), Some(&D::Boolean(false)));
    let mut s = sized_document(4, 2, 72);
    s.execute("layer.new.layer", json!({"name":"Second"})).unwrap();
    replay_saved(&mut s, &action);
    assert_eq!(s.active().unwrap().doc.mode, photocraft_doc::ColorMode::Grayscale);
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
}

/// Unsupported dimension combinations and destructive mode choices must not become different edits.
#[test]
fn unrepresented_units_and_mode_choices_stay_verbatim() {
    let point = || Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Rlt", value: 72.0 });
    let mode = || Descriptor::new("null").with("T   ", D::Class(class("Grys")));
    let canvas = || Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 200.0 });
    for (event, d) in [
        (*b"CnvM", mode().with("Fltt", D::Boolean(true))),
        (*b"CnvM", mode().with("Rstr", D::Boolean(true))),
        (*b"CnvM", mode().with("Rstr", D::Integer(0))),
        (*b"CnvS", canvas().with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 40.0 })),
        (*b"CnvS", canvas().with("Rltv", D::Boolean(true))),
        (*b"ImgS", point().with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 40.0 })),
        (*b"ImgS", point().with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 144.0 })),
        (*b"ImgS", point().with("CnsP", D::Boolean(false))),
    ] {
        let action = recorded(AtnEvent::Code(event), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Explicit false layer flags mean the same thing as omitted flags for a bit-depth conversion.
#[test]
fn review_10702_depth_conversion_accepts_false_layer_flags() {
    for (depth, command) in [(8, "image.mode.bits8"), (16, "image.mode.bits16"), (32, "image.mode.bits32")] {
        for flags in [vec![], vec!["Fltt"], vec!["Rstr"], vec!["Fltt", "Rstr"]] {
            let mut d = Descriptor::new("null").with("Dpth", D::Integer(depth));
            for key in flags {
                d = d.with(key, D::Boolean(false));
            }
            let action = recorded(AtnEvent::Code(*b"CnvM"), d);
            assert_eq!(action.steps, vec![(command.into(), json!({}))]);
            let mut s = sized_document(2, 1, 72);
            s.execute("layer.new.layer", json!({"name":"Retained"})).unwrap();
            replay_saved(&mut s, &action);
            assert_eq!(s.active().unwrap().doc.layers.len(), 2);
            assert_eq!(s.active().unwrap().doc.depth.bits(), depth as u32);
        }
    }
    for key in ["Fltt", "Rstr"] {
        for value in [D::Boolean(true), D::Integer(0), D::Text(UnicodeString::new_nul("false"))] {
            let d = Descriptor::new("null").with("Dpth", D::Integer(16)).with(key, value);
            let action = recorded(AtnEvent::Code(*b"CnvM"), d.clone());
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
            assert_eq!(saved(&action).descriptor, Some(d));
        }
    }
}

/// A recorded name is resolved at playback, including names that collide with native sentinels.
#[test]
fn review_10702_transparency_names_resolve_at_playback() {
    use photocraft_doc::LayerId;
    use photocraft_geom::Rect;

    for name in ["Subject", "merged", ""] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with(
            "T   ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) },
            ]),
        );
        let action = recorded(AtnEvent::Code(*b"setd"), d.clone());
        assert_eq!(action.steps[0].0, "select.loadSelection");
        let mut s = sized_document(2, 1, 72);
        for alpha in [[1.0, 0.0], [0.0, 1.0]] {
            let id = LayerId(s.execute("layer.new.layer", json!({"name":"Placeholder"})).unwrap()["layer"].as_u64().unwrap());
            s.edit("Fixture", |doc, _| {
                let layer = doc.layer_mut(id).unwrap();
                layer.name = name.into();
                layer.surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 2, 1), &[1.0, 0.0, 0.0, alpha[0], 1.0, 0.0, 0.0, alpha[1]]);
                Ok(())
            })
            .unwrap();
        }
        s.execute("layer.new.layer", json!({"name":"Active"})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#ffffff"})).unwrap();
        s.actions.list = vec![action.clone()];
        let played = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        let doc = &s.active().unwrap().doc;
        let selected = doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]);
        assert_eq!(selected, vec![0.0, 1.0], "{name:?}: use the topmost named layer, not the active layer or merged image");
        assert!(s.undo());
        assert!(s.active().unwrap().doc.selection.is_none());
        assert_eq!(saved(&action).descriptor, Some(d));
        replay_saved(&mut s, &action);
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![0.0, 1.0]);
        for layer in [json!({"name":"Missing"}), json!({"name":1}), json!({"name":name,"extra":true})] {
            assert!(s.execute("select.loadSelection", json!({"channel":"transparency","layer":layer})).is_err());
            assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![0.0, 1.0]);
        }
        for sentinel in ["merged", ""] {
            let native = Action::new("Recorded", vec![("select.loadSelection".into(), json!({"channel":"transparency","layer":sentinel}))]);
            let reloaded = recorded(AtnEvent::Code(*b"setd"), saved(&native).descriptor.unwrap());
            let (id, params) = &reloaded.steps[0];
            s.execute(id, params.clone()).unwrap();
            let doc = &s.active().unwrap().doc;
            assert_eq!(doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]), vec![1.0, 1.0]);
        }
    }
}

/// A group's transparency loads the whole canvas, as ⌘-clicking its thumbnail always did (the
/// group's combined transparency is not composited yet), rather than failing.
#[test]
fn group_transparency_sources_load_the_canvas() {
    use photocraft_doc::LayerId;
    use photocraft_geom::Rect;

    for name in ["Group", "merged", ""] {
        let mut s = sized_document(2, 1, 72);
        let child = LayerId(s.execute("layer.new.layer", json!({"name":"Child"})).unwrap()["layer"].as_u64().unwrap());
        s.edit("Child pixels", |doc, _| {
            doc.layer_mut(child).unwrap().surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 2, 1), &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0]);
            Ok(())
        })
        .unwrap();
        let group = LayerId(s.execute("layer.groupLayers", json!({"name":"Group"})).unwrap()["layer"].as_u64().unwrap());
        s.edit("Group name", |doc, _| {
            doc.layer_mut(group).unwrap().name = name.into();
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({"name":"Active"})).unwrap();
        s.execute("edit.fill", json!({"color":"#ffffff"})).unwrap();
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":"Child"})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![1.0, 0.0]);
        let descriptor = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with(
            "T   ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Trsp") },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) },
            ]),
        );
        let action = recorded(AtnEvent::Code(*b"setd"), descriptor.clone());
        let (id, params) = &action.steps[0];
        let result = s.execute(id, params.clone());
        let doc = &s.active().unwrap().doc;
        let alpha = doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]);
        assert!(result.is_ok(), "group {name:?} returned {result:?}");
        assert_eq!(alpha, vec![1.0, 1.0], "group {name:?}");
        // ⌘-click on the group's thumbnail (by id) loads the same.
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":group.0})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.selection.as_ref().map(|mask| mask.read_region(doc.bounds())).unwrap_or(vec![1.0; 2]), vec![1.0, 1.0]);
        assert_eq!(saved(&action).descriptor, Some(descriptor));

        // The group's explicit mask remains a valid source.
        s.execute("select.loadSelection", json!({"channel":"transparency","layer":"Child"})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({"layer":group.0})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("select.loadSelection", json!({"channel":"mask","layer":params["layer"]})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().read_region(Rect::from_xywh(0, 0, 2, 1)), vec![1.0, 0.0]);
    }
}

/// New unit validation must fail before editing, while no-resample still ignores size fields.
#[test]
fn native_units_validate_before_edit_and_no_resample_remains_resolution_only() {
    let mut s = sized_document(40, 20, 300);
    for (id, params, refuse_export) in [
        ("image.canvasSize", json!({"width":200,"units":"unknown"}), true),
        ("image.canvasSize", json!({"width":"200","units":"percent"}), true),
        ("image.canvasSize", json!({"width":200,"units":"percent","relative":true}), true),
        ("image.imageSize", json!({"width":72,"units":1}), true),
        ("image.imageSize", json!({"width":"72","units":"pt"}), true),
        // A finite recorded point value is representable; overflow depends on playback dpi.
        ("image.imageSize", json!({"width":1e308,"units":"pt"}), false),
    ] {
        let before = s.execute("document.inspect", json!({})).unwrap();
        assert!(s.execute(id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(s.execute("document.inspect", json!({})).unwrap(), before);
        let action = Action::new("Invalid", vec![(id.into(), params)]);
        assert_eq!(atn_cmds::export("Identifiers", &[&action], &[], false).is_err(), refuse_export);
    }
    let params = json!({"width":1e308,"units":"pt","resolution":144,"resample":"none"});
    s.execute("image.imageSize", params.clone()).unwrap();
    assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(40, 20));
    assert_eq!(s.active().unwrap().doc.resolution_dpi, 144.0);
    let action = Action::new("Resolution", vec![("image.imageSize".into(), params)]);
    let d = saved(&action).descriptor.unwrap();
    assert!(d.get("Wdth").is_none() && d.get("Hght").is_none());
}

fn class(id: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(id) }
}

fn enm(ty: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(ty), value: Id::new(value) }
}

fn blue() -> Descriptor {
    Descriptor::new("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(255.0))
}

/// Load a reference-shaped descriptor through the public ATN parser.
fn recorded(event: AtnEvent, d: Descriptor) -> Action {
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("Identifiers"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Recorded"),
            items: vec![AtnItem { event, descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
        encoding: None,
    })
    .unwrap();
    atn_cmds::import(&bytes).unwrap().actions.remove(0)
}

/// Parse the actual saved bytes, so matching import/export mistakes cannot cancel out.
fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Identifiers", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    file.actions[0].items[0].clone()
}

const LAYERS_TO_FILES_EVENT: &str = "6f1c2cf5-4a97-4e32-8f59-f5d7a087adef";

/// Settings-shaped script descriptor; the message belongs to the recording's locale.
fn recorded_layer_export(quality: u8) -> Descriptor {
    Descriptor::new("cjsp")
        .with("Msge", D::Text(UnicodeString::new("Recorded script settings")))
        .with("destination", D::Text(UnicodeString::new("output")))
        .with("fileNamePrefix", D::Text(UnicodeString::new("Proof")))
        .with("visibleOnly", D::Boolean(true))
        .with("FlTy", D::Double(1.0))
        .with("icc", D::Boolean(true))
        .with("jpegQuality", D::Text(UnicodeString::new(&quality.to_string())))
        .with("psdMaxComp", D::Boolean(true))
        .with("tiffCompression", D::Text(UnicodeString::new("TIFFEncoding.NONE")))
        .with("tiffJpegQuality", D::Text(UnicodeString::new("8")))
        .with("pdfEncoding", D::Text(UnicodeString::new("PDFEncoding.JPEG")))
        .with("pdfJpegQuality", D::Text(UnicodeString::new("8")))
        .with("targaDepth", D::Text(UnicodeString::new("TargaBitsPerPixels.TWENTYFOUR")))
        .with("bmpDepth", D::Text(UnicodeString::new("BMPDepthType.TWENTYFOUR")))
        .with("png24Transparency", D::Boolean(true))
        .with("png24Interlaced", D::Boolean(false))
        .with("png24Trim", D::Boolean(true))
        .with("png8Transparency", D::Boolean(true))
        .with("png8Interlaced", D::Boolean(false))
        .with("png8Trim", D::Boolean(true))
}

#[test]
fn recorded_layers_to_files_maps_jpeg_settings_and_preserves_descriptor_bytes() {
    for quality in 0..=12 {
        let d = recorded_layer_export(quality);
        let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d.clone());
        let (id, p) = &action.steps[0];
        assert_eq!(id, "file.export.layersToFiles");
        for (key, value) in [
            ("dir", json!("output")),
            ("prefix", json!("Proof")),
            ("format", json!("jpg")),
            ("quality", json!(quality)),
            ("visibleOnly", json!(true)),
            ("groups", json!("recursive")),
            ("trim", json!(false)),
            ("jpegFormat", json!("baseline")),
        ] {
            assert_eq!(p[key], value, "{key}");
        }
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

#[test]
fn recorded_layers_to_files_edits_keep_locale_message_and_inactive_settings() {
    let d = recorded_layer_export(8);
    let mut action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d.clone());
    let p = &mut action.steps[0].1;
    p["dir"] = json!("another-output");
    p["prefix"] = json!("Edited");
    p["quality"] = json!(4);
    p["visibleOnly"] = json!(false);
    let back = saved(&action).descriptor.unwrap();
    for key in ["Msge", "psdMaxComp", "tiffCompression", "png24Trim"] {
        assert_eq!(back.get(key), d.get(key), "{key} must remain attached to its recording");
    }
    let replay = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), back);
    for key in ["dir", "prefix", "quality", "visibleOnly"] {
        assert_eq!(replay.steps[0].1[key], action.steps[0].1[key]);
    }
}

#[test]
fn recorded_layers_to_files_keeps_unknown_or_unrepresented_settings() {
    for (key, value) in [
        ("FlTy", D::Double(2.0)),
        ("FlTy", D::Integer(1)),
        ("icc", D::Boolean(false)),
        ("jpegQuality", D::Text(UnicodeString::new("13"))),
        ("jpegQuality", D::Text(UnicodeString::new("1.5"))),
        ("jpegQuality", D::Integer(8)),
        ("visibleOnly", D::Integer(1)),
        ("destination", D::Boolean(true)),
        ("futureSetting", D::Boolean(true)),
        ("png8Trim", D::Integer(1)),
        ("Msge", D::Boolean(true)),
    ] {
        let mut d = recorded_layer_export(8);
        if let Some((_, v)) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
            *v = value;
        } else {
            d = d.with(key, value);
        }
        let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{key}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    let mut repeated = recorded_layer_export(8);
    repeated.items.push((Id::new("jpegQuality"), D::Text(UnicodeString::new("4"))));
    let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), repeated.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), repeated.to_bytes());
    for key in ["Msge", "destination", "fileNamePrefix", "visibleOnly", "FlTy", "icc", "jpegQuality"] {
        let mut d = recorded_layer_export(8);
        d.items.retain(|(k, _)| !k.is(key));
        let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "missing {key}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    let mut d = recorded_layer_export(8);
    for (key, value) in &mut d.items {
        if key.is("destination") {
            *value = D::Text(UnicodeString(vec![0xd800]));
        }
    }
    let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "invalid UTF-16 must not become a different path");
    assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
}

#[test]
fn recorded_layers_to_files_refuses_native_options_the_script_cannot_represent() {
    let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), recorded_layer_export(8));
    for (key, value) in [
        ("groups", json!("merged")),
        ("trim", json!(true)),
        ("format", json!("png")),
        ("jpegFormat", json!("progressive")),
        ("quality", json!(1.5)),
        ("futureSetting", json!(true)),
        ("_atnExportLayersDescriptor", json!("broken")),
    ] {
        let mut edited = action.clone();
        edited.steps[0].1[key] = value;
        assert!(atn_cmds::export("Proof", &[&edited], &[], false).is_err(), "{key}");
    }
}

/// Import, save, load and playback must actually write JPEGs with stable leaf numbering.
#[test]
fn recorded_layers_to_files_replays_saved_settings_into_a_temporary_directory() {
    let output = tempfile::tempdir().unwrap();
    let mut d = recorded_layer_export(8);
    for (key, value) in &mut d.items {
        if key.is("destination") {
            *value = D::Text(UnicodeString::new(output.path().to_str().unwrap()));
        }
    }
    let action = recorded(AtnEvent::Name(LAYERS_TO_FILES_EVENT.into()), d);
    let mut s = sized_document(4, 4, 72);
    s.edit("Nested", |doc, _| {
        doc.layers[0].surface_mut().unwrap().write_region(photocraft_geom::Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0].repeat(16));
        let mut hidden = doc.layers[0].clone();
        hidden.id = photocraft_doc::LayerId::fresh();
        hidden.name = "Hidden".into();
        hidden.visible = false;
        let mut child = doc.layers[0].clone();
        child.id = photocraft_doc::LayerId::fresh();
        child.name = "Child".into();
        doc.layers.push(hidden);
        doc.layers.push(photocraft_doc::Layer::group("Group", vec![child]));
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    replay_saved(&mut s, &action);
    let mut files: Vec<_> = std::fs::read_dir(output.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, ["Proof_0000s_0000_Child.jpg", "Proof_0001_Background.jpg"]);
    for file in files {
        let image = photocraft_codecs::decode(&std::fs::read(output.path().join(file)).unwrap()).unwrap();
        assert_eq!(image.dimensions(), (4, 4));
        let rgba = image.to_rgba8();
        assert!(rgba[0] > 150 && rgba[1] < 120 && rgba[2] < 120, "the isolated red mark must be exported");
    }
    assert_eq!(s.active().unwrap().doc, before);
}

/// Check the enum type and value in saved bytes independently of the command parameters.
fn enum_value<'a>(d: &'a Descriptor, key: &str, ty: &str) -> &'a Id {
    let Some(D::Enumerated { type_id, value }) = d.get(key) else { panic!("missing {key} enumeration") };
    assert_eq!(*type_id, Id::new(ty));
    value
}

/// Make's opacity belongs to the layer around the adjustment descriptor.
fn adjustment_make(t: Descriptor, opacity: Option<D>) -> Descriptor {
    let mut using = Descriptor::new("AdjL").with("Type", D::Descriptor(t));
    if let Some(opacity) = opacity {
        using = using.with("Opct", opacity);
    }
    Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Class(class("AdjL"))])).with("Usng", D::Descriptor(using))
}

/// Ported from #8659: the saved event changes, while legacy imports and pixel results agree.
#[test]
fn recorded_vibrance_uses_the_string_event_and_keeps_legacy_playback() {
    let d = Descriptor::new("Vbrn").with("vibrance", D::Integer(47)).with("Strt", D::Integer(-19));
    for depth in [8, 16, 32] {
        for event in [AtnEvent::name("vibrance"), AtnEvent::Code(*b"Vbrn")] {
            let action = recorded(event, d.clone());
            assert_eq!(action.steps, vec![("image.adjustments.vibrance".into(), json!({"vibrance":47,"saturation":-19}))]);
            let written = saved(&action);
            assert_eq!(written.event, AtnEvent::name("vibrance"), "Vibrance must save by its string event");
            // Saved with the `vibrance` class the reference application records (#8534); the
            // top-level class does not change what it plays.
            let mut want = d.clone();
            want.class_id = Descriptor::new("vibrance").class_id;
            assert_eq!(written.descriptor, Some(want));
            let create = || {
                let mut s = Session::new();
                s.execute("file.new", json!({"width":4,"height":3,"depth":depth})).unwrap();
                s.execute("edit.fill", json!({"contents":"color","color":"#6594c2"})).unwrap();
                s
            };
            let mut actual = create();
            replay_saved(&mut actual, &action);
            let mut expected = create();
            expected.execute("image.adjustments.vibrance", action.steps[0].1.clone()).unwrap();
            for channel in 0..4 {
                assert_eq!(
                    actual.active().unwrap().doc.layers[0].surface().unwrap().sample_channel(1, 1, channel),
                    expected.active().unwrap().doc.layers[0].surface().unwrap().sample_channel(1, 1, channel)
                );
            }
        }
    }
}

/// The reported Gradient Map Make and the metadata-only default both round-trip atomically.
#[test]
fn recorded_adjustment_opacity_keeps_gradient_settings_and_defaults() {
    let full =
        saved(&Action::new("Recorded", vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#0000ff"],[1,"#ff0000"]]}))])).descriptor.unwrap();
    let default = Descriptor::new("GdMp").with("presetKind", enm("presetKindType", "presetKindDefault"));
    for t in [full, default] {
        let action = recorded(AtnEvent::Code(*b"Mk  "), adjustment_make(t.clone(), Some(D::UnitFloat { unit: *b"#Prc", value: 70.0 })));
        assert_eq!(action.steps.len(), 1);
        assert_eq!(action.steps[0].0, "layer.newAdjustmentLayer.gradientMap");
        assert_eq!(action.steps[0].1["opacity"], json!(179.0 / 255.0), "Make must retain surrounding opacity");
        if t.get("Grad").is_some() {
            assert_eq!(action.steps[0].1["stops"], json!([[0, "#0000ff"], [1, "#ff0000"]]));
        }
        let written = saved(&action).descriptor.unwrap();
        let Some(D::Descriptor(using)) = written.get("Usng") else { panic!("missing adjustment metadata") };
        // The recorded 70 % saves back as 70 %, not as the 8-bit level 179/255 (70.196 %).
        assert!(matches!(using.get("Opct"), Some(D::UnitFloat { unit, value }) if unit == b"#Prc" && *value == 70.0), "{:?}", using.get("Opct"));
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":3,"depth":depth})).unwrap();
            let base = s.active().unwrap().active_layer.unwrap();
            replay_saved(&mut s, &action);
            let state = s.active().unwrap();
            assert_eq!(state.doc.layers.len(), 2);
            assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().opacity, 179.0_f32 / 255.0);
            assert_eq!(state.doc.layer(base).unwrap().opacity, 1.0);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 1, "one undo removes the complete Make");
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 2);
        }
    }
}

/// Invalid supplied opacity cannot become a silent default or create a layer.
#[test]
fn adjustment_opacity_validation_and_endpoints_are_atomic() {
    let t = Descriptor::new("GdMp").with("presetKind", enm("presetKindType", "presetKindDefault"));
    for bad in [
        D::UnitFloat { unit: *b"#Pxl", value: 70.0 },
        D::UnitFloat { unit: *b"#Prc", value: -1.0 },
        D::UnitFloat { unit: *b"#Prc", value: 101.0 },
        D::Text(UnicodeString::new_nul("70")),
        D::Boolean(false),
        D::Double(f64::NAN),
    ] {
        let action = recorded(AtnEvent::Code(*b"Mk  "), adjustment_make(t.clone(), Some(bad)));
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        saved(&action);
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":3})).unwrap();
    for bad in [json!(null), json!("70"), json!(-0.1), json!(1.1)] {
        let before = s.execute("document.inspect", json!({})).unwrap();
        assert!(s.execute("layer.newAdjustmentLayer.gradientMap", json!({"opacity":bad})).is_err());
        assert_eq!(s.execute("document.inspect", json!({})).unwrap(), before);
        assert!(
            atn_cmds::export("Identifiers", &[&Action::new("Bad", vec![("layer.newAdjustmentLayer.gradientMap".into(), json!({"opacity":bad}))])], &[], false)
                .is_err()
        );
    }
    for opacity in [None, Some(0.0), Some(1.0)] {
        let p = opacity.map_or_else(|| json!({}), |opacity| json!({"opacity":opacity}));
        s.execute("layer.newAdjustmentLayer.gradientMap", p.clone()).unwrap();
        let state = s.active().unwrap();
        assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().opacity, opacity.unwrap_or(1.0));
        let action = Action::new("Recorded", vec![("layer.newAdjustmentLayer.gradientMap".into(), p)]);
        let reloaded = recorded(AtnEvent::Code(*b"Mk  "), saved(&action).descriptor.unwrap());
        assert_eq!(reloaded.steps.len(), 1);
        assert_eq!(reloaded.steps[0].0, action.steps[0].0);
        match opacity {
            None => assert_eq!(reloaded.steps[0].1, json!({})),
            Some(opacity) => assert_eq!(reloaded.steps[0].1["opacity"].as_f64(), Some(f64::from(opacity))),
        }
        s.execute("edit.undo", json!({})).unwrap();
    }
}

/// Canonical event names and old code imports share the existing gradient parameters.
#[test]
fn gradient_map_writes_the_named_event_and_keeps_legacy_code_imports() {
    let stop = |location, color| {
        D::Descriptor(Descriptor::new("Clrt").with("Lctn", D::Integer(location)).with("Mdpn", D::Integer(50)).with("Clr ", D::Descriptor(color)))
    };
    let red = Descriptor::new("RGBC").with("Rd  ", D::Double(255.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0));
    let d = Descriptor::new("GdMp")
        .with("Grad", D::Descriptor(Descriptor::new("Grdn").with("Clrs", D::List(vec![stop(0, blue()), stop(4096, red)]))))
        .with("Rvrs", D::Boolean(true))
        .with("Dthr", D::Boolean(false));
    for event in [AtnEvent::name("gradientMapEvent"), AtnEvent::Code(*b"GdMp")] {
        let action = recorded(event, d.clone());
        assert_eq!(action.steps, vec![("image.adjustments.gradientMap".into(), json!({"stops":[[0,"#0000ff"],[1,"#ff0000"]],"reverse":true,"dither":false}))]);
        let written = saved(&action);
        assert_eq!(written.event, AtnEvent::name("gradientMapEvent"));
        assert!(written.descriptor.unwrap().class_id.is("GdMp"));
    }
    assert_eq!(recorded(AtnEvent::name("GdMp"), d).steps[0].0, atn_cmds::PHOTOSHOP_STEP);
}

/// The measured outside identifier changes while inside, centre and unknown values stay safe.
#[test]
fn stroke_outside_writes_otsd_without_changing_inside_or_center() {
    for (input, location, canonical) in [("Otsd", "outside", "Otsd"), ("OtsF", "outside", "Otsd"), ("Insd", "inside", "Insd"), ("Cntr", "center", "Cntr")] {
        let d = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 2.0 })
            .with("Lctn", enm("StrL", input))
            .with("Clr ", D::Descriptor(blue()));
        let action = recorded(AtnEvent::Code(*b"Strk"), d);
        assert_eq!(action.steps[0].0, "edit.stroke");
        assert_eq!(action.steps[0].1["location"], location);
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Lctn", "StrL"), Id::new(canonical));
    }
    let unknown = Descriptor::new("null").with("Lctn", enm("StrL", "futureLocation"));
    let action = recorded(AtnEvent::Code(*b"Strk"), unknown.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(unknown));
}

/// A group blend uses Photoshop's actual identifier and reaches the native group command.
#[test]
fn pass_through_uses_the_string_identifier_and_replays_on_a_group() {
    for input in ["passThrough", "PsTh"] {
        let target = D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Group") }]);
        let d = Descriptor::new("null").with("null", target).with("T   ", D::Descriptor(Descriptor::new("Lyr ").with("Md  ", enm("BlnM", input))));
        let action = recorded(AtnEvent::Code(*b"setd"), d);
        assert_eq!(action.steps, vec![("layer.setProps".into(), json!({"layer":"Group","blend":"Pass Through"}))]);
        let written = saved(&action).descriptor.unwrap();
        let Some(D::Descriptor(props)) = written.get("T   ") else { panic!("layer properties lost") };
        assert_eq!(*enum_value(props, "Md  ", "BlnM"), Id::new("passThrough"));
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("layer.new.group", json!({"name":"Group"})).unwrap();
        s.execute("layer.setProps", json!({"blend":"Normal"})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().blend, photocraft_color::BlendMode::PassThrough);
    }
}

fn relative_transform(horizontal: D, vertical: D) -> Descriptor {
    Descriptor::new("null")
        .with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]))
        .with("FTcs", enm("QCSt", "Qcsa"))
        .with("Ofst", D::Descriptor(Descriptor::new("Ofst").with("Hrzn", horizontal).with("Vrtc", vertical)))
        .with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
        .with("Hght", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
        .with("Intr", enm("Intp", "Nrst"))
}

#[test]
fn recorded_transform_zero_points_import_and_export_as_zero_pixels() {
    for (unit_x, dx, unit_y, dy) in [(*b"#Rlt", 0.0, *b"#Rlt", -0.0), (*b"#Rlt", 0.0, *b"#Pxl", 2.0), (*b"#Pxl", 3.5, *b"#Rlt", 0.0)] {
        for selection in [false, true] {
            let mut d = relative_transform(D::UnitFloat { unit: unit_x, value: dx }, D::UnitFloat { unit: unit_y, value: dy });
            if selection {
                d.items.iter_mut().find(|(key, _)| key.is("null")).unwrap().1 =
                    D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]);
            }
            let action = recorded(AtnEvent::name("transform"), d);
            let id = if selection { "select.transformSelection" } else { "edit.transform" };
            assert_eq!(action.steps[0].0, id);
            let params = &action.steps[0].1;
            assert_eq!(params["dx"].as_f64(), Some(dx));
            assert_eq!(params["dy"].as_f64(), Some(dy));
            assert_eq!(params["scaleX"], 50);
            assert_eq!(params["scaleY"], 50);
            assert_eq!(params["interpolation"], "nearest");
            let written = saved(&action).descriptor.unwrap();
            let Some(D::Descriptor(offset)) = written.get("Ofst") else { panic!("offset lost") };
            for (key, expected) in [("Hrzn", dx), ("Vrtc", dy)] {
                assert!(matches!(offset.get(key), Some(D::UnitFloat { unit, value }) if unit == b"#Pxl" && *value == expected));
            }
            assert_eq!(recorded(AtnEvent::Code(*b"Trnf"), written).steps, action.steps);
        }
    }
}

#[test]
fn recorded_transform_zero_points_replay_independently_of_depth_and_resolution() {
    let point =
        recorded(AtnEvent::Code(*b"Trnf"), relative_transform(D::UnitFloat { unit: *b"#Rlt", value: 0.0 }, D::UnitFloat { unit: *b"#Rlt", value: 0.0 }));
    let pixel =
        recorded(AtnEvent::Code(*b"Trnf"), relative_transform(D::UnitFloat { unit: *b"#Pxl", value: 0.0 }, D::UnitFloat { unit: *b"#Pxl", value: 0.0 }));
    let setup = |depth, dpi| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
        s.execute("image.imageSize", json!({"resolution":dpi,"resample":"none"})).unwrap();
        s.execute("select.rect", json!({"x":1,"y":2,"width":4,"height":4})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    };
    let pixels = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
    };
    for depth in [8, 16, 32] {
        for dpi in [72, 300] {
            let mut actual = setup(depth, dpi);
            let mut expected = setup(depth, dpi);
            let before = pixels(&actual);
            replay_saved(&mut actual, &point);
            replay_saved(&mut expected, &pixel);
            assert_eq!(pixels(&actual), pixels(&expected), "{depth} bits, {dpi} dpi");
            assert_ne!(pixels(&actual), before, "the transform must actually scale the layer");
            let st = actual.active().unwrap();
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds().size(), photocraft_geom::Size::new(2, 2));
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(pixels(&actual), before);
        }
    }
}

#[test]
fn recorded_transform_nonzero_points_keep_their_units_and_precision() {
    for (ux, dx, uy, dy) in [
        (*b"#Rlt", 0.36, *b"#Rlt", -1.08),
        (*b"#Rlt", -2.842_170_943_040_401e-14, *b"#Rlt", -2.842_170_943_040_401e-14),
        (*b"#Rlt", 1.0, *b"#Rlt", 0.0),
        (*b"#Rlt", 1.0, *b"#Pxl", 0.0),
        (*b"#Pxl", 0.0, *b"#Rlt", -1.0),
    ] {
        for event in [AtnEvent::Code(*b"Trnf"), AtnEvent::name("transform")] {
            let action = recorded(event, relative_transform(D::UnitFloat { unit: ux, value: dx }, D::UnitFloat { unit: uy, value: dy }));
            assert_eq!(action.steps[0].0, "edit.transform");
            assert_eq!(action.steps[0].1["offsetUnits"], "pt");
            assert_eq!(action.steps[0].1["dx"].as_f64(), Some(dx));
            assert_eq!(action.steps[0].1["dy"].as_f64(), Some(dy));
            let back = saved(&action).descriptor.unwrap();
            let Some(D::Descriptor(offset)) = back.get("Ofst") else { panic!("offset") };
            for (key, value) in [("Hrzn", dx), ("Vrtc", dy)] {
                assert_eq!(offset.get(key), Some(&D::UnitFloat { unit: *b"#Rlt", value }));
            }
            assert_eq!(recorded(AtnEvent::Code(*b"Trnf"), back).steps, action.steps);
        }
    }
}

#[test]
fn recorded_transform_nonzero_points_resolve_against_the_playback_dpi() {
    let setup = |depth, dpi| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":32,"height":32,"depth":depth,"resolution":dpi,"background":"transparent"})).unwrap();
        s.execute("select.rect", json!({"x":8,"y":10,"width":8,"height":8})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    };
    let action =
        recorded(AtnEvent::Code(*b"Trnf"), relative_transform(D::UnitFloat { unit: *b"#Rlt", value: 1.0 }, D::UnitFloat { unit: *b"#Rlt", value: -1.0 }));
    for depth in [8, 16, 32] {
        for dpi in [72, 144, 300] {
            let mut actual = setup(depth, dpi);
            let mut expected = setup(depth, dpi);
            let before = actual.active().unwrap().doc.clone();
            replay_saved(&mut actual, &action);
            expected
                .execute("edit.transform", json!({"scaleX":50,"scaleY":50,"dx":f64::from(dpi)/72.0,"dy":-f64::from(dpi)/72.0,"interpolation":"nearest"}))
                .unwrap();
            let pixels = |s: &Session| s.active().unwrap().doc.layers[0].surface().unwrap().read_region(s.active().unwrap().doc.bounds());
            assert_eq!(pixels(&actual), pixels(&expected), "{depth} bits, {dpi} dpi");
            assert_ne!(actual.active().unwrap().doc, before);
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(actual.active().unwrap().doc, before);
        }
    }
}

#[test]
fn recorded_transform_nonzero_points_reject_invalid_native_units_and_values() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"resolution":300,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({"dx":1,"offsetUnits":"in"}),
        json!({"dx":1,"offsetUnits":true}),
        json!({"dx":"wrong","offsetUnits":"pt"}),
        json!({"matrix":[1,0,0,1,2,3],"offsetUnits":"pt"}),
    ] {
        assert!(s.execute("edit.transform", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc, before);
        let action = Action::new("Recorded", vec![("edit.transform".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&action], &[], false).is_err(), "{p}");
    }
    assert!(s.execute("edit.transform", json!({"dx":1e308,"offsetUnits":"pt"})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("edit.transform", json!({"dx":1,"offsetUnits":"pt"})).unwrap();
}

#[test]
fn recorded_transform_points_keep_unrepresentable_offsets() {
    for value in [
        D::UnitFloat { unit: *b"#Rlt", value: f64::NAN },
        D::UnitFloat { unit: *b"#Pxl", value: f64::INFINITY },
        D::UnitFloat { unit: *b"#Prc", value: 0.0 },
        D::Double(0.0),
    ] {
        for x_axis in [false, true] {
            let zero = D::UnitFloat { unit: *b"#Rlt", value: 0.0 };
            let d = if x_axis { relative_transform(value.clone(), zero) } else { relative_transform(zero, value.clone()) };
            let action = recorded(AtnEvent::Code(*b"Trnf"), d.clone());
            assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{value:?}");
            assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
        }
    }
    let mut selection = relative_transform(D::UnitFloat { unit: *b"#Rlt", value: 1.0 }, D::UnitFloat { unit: *b"#Rlt", value: 0.0 });
    selection.items.iter_mut().find(|(key, _)| key.is("null")).unwrap().1 =
        D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]);
    let mixed = relative_transform(D::UnitFloat { unit: *b"#Rlt", value: 1.0 }, D::UnitFloat { unit: *b"#Pxl", value: 2.0 });
    for d in [selection, mixed] {
        let action = recorded(AtnEvent::Code(*b"Trnf"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for offset in [
        D::Integer(0),
        D::Descriptor(
            Descriptor::new("Ofst")
                .with("Hrzn", D::UnitFloat { unit: *b"#Rlt", value: 1.0 })
                .with("Vrtc", D::UnitFloat { unit: *b"#Rlt", value: 0.0 })
                .with("unknown", D::Boolean(true)),
        ),
    ] {
        let mut d = relative_transform(D::UnitFloat { unit: *b"#Pxl", value: 0.0 }, D::UnitFloat { unit: *b"#Pxl", value: 0.0 });
        d.items.iter_mut().find(|(key, _)| key.is("Ofst")).unwrap().1 = offset;
        let action = recorded(AtnEvent::name("transform"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

fn slice_selection(x: f64, y: f64) -> Descriptor {
    Descriptor::new("slct")
        .with("null", D::Reference(vec![ReferenceItem::Class(class("slice"))]))
        .with(
            "At  ",
            D::Descriptor(
                Descriptor::new("Pnt ").with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: x }).with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: y }),
            ),
        )
        .with("addToSelection", D::Boolean(false))
}

#[test]
fn recorded_slice_selection_imports_and_exports_pixel_points() {
    for (x, y) in [(101.0, 115.0), (10.5, 20.25)] {
        for event in [AtnEvent::Code(*b"slct"), AtnEvent::name("select")] {
            let d = slice_selection(x, y);
            let action = recorded(event, d.clone());
            assert_eq!(action.steps.len(), 1);
            assert_eq!(action.steps[0].0, "slice.select");
            assert_eq!(action.steps[0].1["point"][0].as_f64(), Some(x));
            assert_eq!(action.steps[0].1["point"][1].as_f64(), Some(y));
            let written = saved(&action).descriptor.unwrap();
            for key in ["null", "At  ", "addToSelection"] {
                assert_eq!(written.get(key), d.get(key));
            }
            let mut without_default_flag = d.clone();
            without_default_flag.items.retain(|(key, _)| !key.is("addToSelection"));
            assert_eq!(recorded(AtnEvent::name("select"), without_default_flag).steps, action.steps);
        }
    }
}

#[test]
fn recorded_slice_selection_replays_without_editing_or_promoting_auto_slices() {
    let action = recorded(AtnEvent::name("select"), slice_selection(101.0, 115.0));
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":320,"height":240,"depth":depth})).unwrap();
        let id = s.execute("slice.new", json!({"rect":[0,0,2,1]})).unwrap()["slice"].clone();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let before = st.doc.clone();
        let history = st.history.entries();
        replay_saved(&mut s, &action);
        let selected = s.execute("slice.list", json!({})).unwrap()["selected"].clone();
        assert_eq!(selected["origin"], "auto");
        assert!(selected["id"].is_null());
        let st = s.active().unwrap();
        assert_eq!(st.doc, before, "selection must not promote the auto slice");
        assert_eq!(st.history.entries(), history);
        assert!(!st.is_dirty());
        let stored = recorded(AtnEvent::name("select"), slice_selection(0.5, 0.5));
        replay_saved(&mut s, &stored);
        assert_eq!(s.execute("slice.list", json!({})).unwrap()["selected"]["id"], id);
        s.execute("file.new", json!({"width":320,"height":240})).unwrap();
        assert!(s.execute("slice.list", json!({})).unwrap()["selected"].is_null());
        s.execute("document.activate", json!({"offset":-1})).unwrap();
        assert_eq!(s.execute("slice.list", json!({})).unwrap()["selected"]["id"], id, "selection belongs to its document");
    }
}

#[test]
fn recorded_slice_selection_keeps_unknown_options_units_and_reference_forms() {
    let base = slice_selection(10.0, 20.0);
    let mut cases = vec![base.clone().with("unknown", D::Boolean(true))];
    for value in [D::Boolean(true), D::Integer(0)] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(k, _)| k.is("addToSelection")).unwrap().1 = value;
        cases.push(d);
    }
    for key in ["null", "At  ", "addToSelection"] {
        let mut d = base.clone();
        d.items.push((Id::new(key), d.get(key).unwrap().clone()));
        cases.push(d);
    }
    for reference in [
        vec![ReferenceItem::Class(class("Path"))],
        vec![ReferenceItem::Class(class("slice")), ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") }],
    ] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(k, _)| k.is("null")).unwrap().1 = D::Reference(reference);
        cases.push(d);
    }
    let mut wrong_point = base.clone();
    let D::Descriptor(point) = &mut wrong_point.items.iter_mut().find(|(k, _)| k.is("At  ")).unwrap().1 else { panic!("point") };
    point.class_id = Id::new("Rctn");
    cases.push(wrong_point);
    for value in [D::UnitFloat { unit: *b"#Rlt", value: 10.0 }, D::UnitFloat { unit: *b"#Pxl", value: f64::NAN }, D::Double(10.0)] {
        let mut d = base.clone();
        let D::Descriptor(point) = &mut d.items.iter_mut().find(|(k, _)| k.is("At  ")).unwrap().1 else { panic!("point") };
        point.items.iter_mut().find(|(k, _)| k.is("Hrzn")).unwrap().1 = value;
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("select"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for p in [json!({"clear":true}), json!({"point":[1.0,2.0],"add":true}), json!({"point":[1.0,2.0],"unknown":true})] {
        let native = Action::new("Recorded", vec![("slice.select".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{p}");
    }
}

#[test]
fn recorded_slice_selection_validates_native_inputs_and_locking_before_changing_state() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":80})).unwrap();
    s.execute("slice.new", json!({"rect":[10,10,30,20]})).unwrap();
    s.execute("slice.select", json!({"point":[20.0,20.0]})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let selected = s.execute("slice.list", json!({})).unwrap()["selected"].clone();
    s.execute("slice.select", json!({"point":[20.0,20.0],"coalesce":"selection"})).unwrap();
    for p in [
        json!({}),
        json!({"point":[]}),
        json!({"point":[1]}),
        json!({"point":[1,2,3]}),
        json!({"point":[null,2]}),
        json!({"point":[f64::MAX,2]}),
        json!({"point":[-1,2]}),
        json!({"point":[1,2],"clear":true}),
        json!({"clear":false}),
    ] {
        assert!(s.execute("slice.select", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.execute("slice.list", json!({})).unwrap()["selected"], selected);
    }
    s.execute("view.lockSlices", json!({"on":true})).unwrap();
    assert!(s.execute("slice.select", json!({"point":[90.0,70.0]})).is_err());
    assert_eq!(s.execute("slice.list", json!({})).unwrap()["selected"], selected);
    s.execute("slice.select", json!({"clear":true})).unwrap();
    assert!(s.execute("slice.list", json!({})).unwrap()["selected"].is_null());
    assert!(Session::new().execute("slice.select", json!({"point":[1,2]})).is_err());
}

#[test]
fn recorded_slice_selection_keeps_stored_identity_and_invalidates_changed_auto_regions() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":80})).unwrap();
    let id = s.execute("slice.new", json!({"rect":[10,10,30,20]})).unwrap()["slice"].clone();
    s.execute("slice.select", json!({"point":[20.0,20.0]})).unwrap();
    s.execute("slice.set", json!({"slice":id,"rect":[50,40,30,20]})).unwrap();
    assert_eq!(s.execute("slice.list", json!({})).unwrap()["selected"]["id"], id);
    s.execute("slice.select", json!({"point":[90.0,70.0]})).unwrap();
    let auto = s.execute("slice.list", json!({})).unwrap()["selected"].clone();
    s.execute("slice.set", json!({"slice":id,"rect":[0,0,50,80]})).unwrap();
    let selected = s.execute("slice.list", json!({})).unwrap()["selected"].clone();
    assert!(selected.is_null(), "a changed auto region must not retarget its old display number: {auto} -> {selected}");
}

#[test]
fn recorded_slice_selection_preserves_floating_pixels() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":80,"background":"transparent"})).unwrap();
    s.execute("slice.new", json!({"rect":[10,10,30,20]})).unwrap();
    s.execute("select.rect", json!({"x":10,"y":10,"width":10,"height":10})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
    s.execute("select.float", json!({"dx":5,"dy":0})).unwrap();
    assert!(s.active().unwrap().floating.is_some());
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    for p in [json!({"point":[90.0,70.0]}), json!({"clear":true})] {
        s.execute("slice.select", p).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc, before, "slice view state must not commit the floating pixels");
        assert_eq!(st.history.entries(), history);
        assert!(st.floating.is_some());
        assert_eq!(st.floating.as_ref().unwrap().revision, st.revision);
    }
    assert!(s.execute("slice.select", json!({"point":[-1,2]})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert!(s.active().unwrap().floating.is_some());
}

fn slice_division(down: Option<i32>, across: Option<i32>) -> Descriptor {
    let mut d = Descriptor::new("divide")
        .with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("slice"), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]));
    for (key, count) in [("slicesDown", down), ("slicesAcross", across)] {
        if let Some(count) = count {
            d = d.with(key, D::Integer(count));
        }
    }
    d
}

#[test]
fn recorded_slice_division_imports_and_exports_counts_on_the_current_target() {
    for (down, across, params) in [
        (Some(5), None, json!({"horizontal":5})),
        (None, Some(3), json!({"vertical":3})),
        (Some(5), Some(3), json!({"horizontal":5,"vertical":3})),
        (Some(1), Some(2), json!({"horizontal":1,"vertical":2})),
        (Some(1000), Some(10), json!({"horizontal":1000,"vertical":10})),
    ] {
        let d = slice_division(down, across);
        let action = recorded(AtnEvent::name("divide"), d.clone());
        assert_eq!(action.steps, vec![("slice.divide".into(), params)]);
        let written = saved(&action);
        assert_eq!(written.event, AtnEvent::name("divide"));
        for key in ["null", "slicesDown", "slicesAcross"] {
            assert_eq!(written.descriptor.as_ref().unwrap().get(key), d.get(key));
        }
    }
}

#[test]
fn recorded_slice_division_replays_on_stored_and_auto_slices_at_each_depth() {
    let action = recorded(AtnEvent::name("divide"), slice_division(Some(5), Some(3)));
    for depth in [8, 16, 32] {
        for stored in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":103,"height":83,"depth":depth})).unwrap();
            let rect = if stored {
                s.execute("slice.new", json!({"rect":[11,13,61,47],"name":"Options","url":"https://example.test/"})).unwrap();
                photocraft_geom::Rect::from_xywh(11, 13, 61, 47)
            } else {
                s.active().unwrap().doc.bounds()
            };
            let selection = recorded(AtnEvent::name("select"), slice_selection(f64::from(rect.x0 + 1), f64::from(rect.y0 + 1)));
            replay_saved(&mut s, &selection);
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            replay_saved(&mut s, &action);
            let st = s.active().unwrap();
            let parts = &st.doc.slices.list;
            assert_eq!(parts.len(), 15);
            for (index, slice) in parts.iter().enumerate() {
                let (row, column) = (index as i32 / 3, index as i32 % 3);
                let x = |i| rect.x0 + rect.width() as i32 * i / 3;
                let y = |i| rect.y0 + rect.height() as i32 * i / 5;
                assert_eq!(slice.rect, photocraft_geom::Rect::new(x(column), y(row), x(column + 1), y(row + 1)));
                assert_eq!(slice.origin, photocraft_doc::slices::SliceOrigin::User);
            }
            if stored {
                assert_eq!(parts[0].name, "Options");
                assert_eq!(parts[0].url, "https://example.test/");
            }
            let mut only_slices_changed = (*before).clone();
            only_slices_changed.slices = st.doc.slices.clone();
            assert_eq!(*st.doc, only_slices_changed);
            assert_eq!(st.history.past_len(), past + 1, "auto promotion and division form one undo step");
            let divided = st.doc.clone();
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc, divided);
        }
    }
}

#[test]
fn recorded_slice_division_keeps_unsupported_counts_options_and_references() {
    let base = slice_division(Some(5), None);
    let mut cases = vec![base.clone().with("pixelsDown", D::Integer(10)), base.clone().with("unknown", D::Boolean(false))];
    for (down, across) in [(None, None), (Some(1), None), (Some(0), None), (Some(-2), None), (Some(1001), None), (Some(101), Some(100))] {
        cases.push(slice_division(down, across));
    }
    for key in ["null", "slicesDown"] {
        cases.push(base.clone().with(key, base.get(key).unwrap().clone()));
    }
    for value in [D::Double(2.0), D::Boolean(true), D::UnitFloat { unit: *b"#Pxl", value: 10.0 }] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(key, _)| key.is("slicesDown")).unwrap().1 = value;
        cases.push(d);
    }
    for reference in [
        vec![ReferenceItem::Class(class("slice"))],
        vec![ReferenceItem::Enumerated { class: class("slice"), type_id: Id::new("wrong"), value: Id::new("Trgt") }],
        vec![ReferenceItem::Enumerated { class: class("slice"), type_id: Id::new("Ordn"), value: Id::new("Frst") }],
        vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }],
        vec![
            ReferenceItem::Enumerated { class: class("slice"), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
            ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") },
        ],
    ] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(key, _)| key.is("null")).unwrap().1 = D::Reference(reference);
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("divide"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for params in [
        json!({}),
        json!({"horizontal":1}),
        json!({"horizontal":0}),
        json!({"horizontal":1001}),
        json!({"horizontal":2.5}),
        json!({"horizontal":101,"vertical":100}),
        json!({"horizontal":2,"slice":1}),
        json!({"horizontal":2,"number":1}),
        json!({"horizontal":2,"unknown":true}),
    ] {
        let native = Action::new("Recorded", vec![("slice.divide".into(), params.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{params}");
    }
}

#[test]
fn recorded_slice_division_requires_a_valid_unlocked_current_target() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":80})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("slice.divide", json!({"horizontal":2})).is_err(), "no implicit first-slice fallback");
    s.execute("slice.select", json!({"point":[90,70]})).unwrap();
    for params in [json!({"horizontal":1000}), json!({"horizontal":2,"slice":null}), json!({"horizontal":2,"number":0})] {
        assert!(s.execute("slice.divide", params).is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
    s.execute("view.lockSlices", json!({"on":true})).unwrap();
    assert!(s.execute("slice.divide", json!({"horizontal":2})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("view.lockSlices", json!({"on":false})).unwrap();
    s.execute("slice.new", json!({"rect":[0,0,10,10]})).unwrap();
    s.execute("slice.select", json!({"point":[90,70]})).unwrap();
    s.execute("slice.new", json!({"rect":[0,0,50,80]})).unwrap();
    // Re-select an auto region, then change its geometry without giving it a replacement target.
    s.execute("slice.select", json!({"point":[90,70]})).unwrap();
    s.execute("slice.set", json!({"slice":2,"rect":[0,0,60,80]})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    assert!(s.execute("slice.divide", json!({"horizontal":2})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), past);
    s.execute("view.clearSlices", json!({})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
    s.execute("layer.newLayerBasedSlice", json!({})).unwrap();
    s.execute("slice.select", json!({"point":[10,10]})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("slice.divide", json!({"horizontal":2})).is_err(), "layer-based targets need an explicit promotion");
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn recorded_slice_division_continues_the_parent_only_after_success() {
    for selected in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":100,"height":80})).unwrap();
        if selected {
            s.execute("slice.select", json!({"point":[50,40]})).unwrap();
        }
        let before = s.active().unwrap().doc.clone();
        let mut action = recorded(AtnEvent::name("divide"), slice_division(Some(5), None));
        action.steps.push(("layer.renameLayer".into(), json!({"name":"AfterDivide"})));
        let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
        s.actions.list = atn_cmds::import(&bytes).unwrap().actions;
        let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        if selected {
            assert_eq!(result["ran"], 2, "{result}");
            assert!(result.get("failed").is_none(), "{result}");
            assert_eq!(s.active().unwrap().doc.slices.list.len(), 5);
            assert_eq!(s.active().unwrap().doc.layers[0].name, "AfterDivide");
        } else {
            assert_eq!(result["ran"], 0, "{result}");
            assert!(result.get("failed").is_some(), "{result}");
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}

#[test]
fn recorded_slice_division_resolves_a_selection_from_the_same_saved_action() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":100,"height":80})).unwrap();
    s.execute("slice.new", json!({"rect":[0,0,10,10]})).unwrap();
    let mut action = recorded(AtnEvent::name("select"), slice_selection(50.0, 50.0));
    action.steps.extend(recorded(AtnEvent::name("divide"), slice_division(Some(5), None)).steps);
    let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    s.actions.list = imported.actions;
    let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
    assert_eq!(result["ran"], 2, "{result}");
    assert!(result.get("failed").is_none(), "{result}");
    let parts = &s.active().unwrap().doc.slices.list;
    assert_eq!(parts.len(), 6, "the originally selected user slice remains intact");
    assert_eq!(parts[0].rect, photocraft_geom::Rect::from_xywh(0, 0, 10, 10));
    assert!(parts.iter().skip(1).all(|slice| slice.rect.y0 >= 10));
}

fn web_path(path: &str) -> D {
    let units: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
    let mut bytes = b"txtu".to_vec();
    bytes.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    D::Path(bytes)
}

fn web_destination(path: &str, nested: bool, folder: bool) -> Descriptor {
    let mut settings = Descriptor::new("SaveForWeb").with("Op  ", enm("SWOp", "OpSa")).with("Fmt ", enm("IRFm", "PN24")).with("DIDr", D::Boolean(folder));
    if nested {
        settings = settings.with("In  ", web_path(path));
    }
    let d = Descriptor::new("null").with("Usng", D::Descriptor(settings));
    if nested { d } else { d.with("In  ", web_path(path)) }
}

#[test]
fn recorded_web_destination_imports_outer_and_nested_paths_and_saves_nested() {
    for nested in [false, true] {
        for folder in [false, true] {
            for path in ["/out/图片.png", r"C:\exports\image.png"] {
                for event in [AtnEvent::Code(*b"Expr"), AtnEvent::name("export")] {
                    let action = recorded(event, web_destination(path, nested, folder));
                    let key = if folder { "folder" } else { "path" };
                    assert_eq!(action.steps, vec![("file.export.saveForWebLegacy".into(), json!({key:path,"format":"png24"}))]);
                    let written = saved(&action).descriptor.unwrap();
                    assert!(written.get("In  ").is_none(), "canonical export keeps the destination with its SaveForWeb settings");
                    let Some(D::Descriptor(settings)) = written.get("Usng") else { panic!("SaveForWeb settings") };
                    assert_eq!(settings.get("In  "), Some(&web_path(path)));
                    assert_eq!(settings.get("DIDr"), folder.then_some(&D::Boolean(true)));
                    assert_eq!(settings.get("Mtt "), Some(&D::Boolean(false)), "the native default has no matte");
                    let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
                    let back = atn_cmds::import(&bytes).unwrap();
                    assert!(back.unsupported.is_empty());
                    assert_eq!(back.actions[0].steps[0].1[key], path);
                }
            }
        }
    }
}

#[test]
fn recorded_web_destination_keeps_ambiguous_malformed_and_unrepresented_forms() {
    let base = web_destination("/out/image.png", false, false);
    let nested = web_destination("/out/image.png", true, false);
    let mut cases = vec![nested.clone().with("In  ", web_path("/other/image.png")), nested.clone().with("In  ", web_path("/out/image.png"))];
    for key in ["Usng", "In  "] {
        cases.push(base.clone().with(key, base.get(key).unwrap().clone()));
    }
    for (key, value) in [
        ("In  ", D::Text(UnicodeString::new_nul("/out/image.png"))),
        ("In  ", web_path("")),
        ("In  ", web_path("/out/bad\0name.png")),
        ("DIDr", D::Integer(1)),
        ("Op  ", enm("SWOp", "OpOp")),
        ("Op  ", enm("wrong", "OpSa")),
        ("unknown", D::Boolean(false)),
        ("SWsl", enm("STsl", "SLAl")),
        ("SWmd", enm("STmd", "Other")),
        ("ovNC", D::List(vec![])),
        ("Trns", D::Integer(1)),
        ("Qlty", D::Double(f64::NAN)),
        ("Qlty", D::Double(20.5)),
        ("Qlty", D::Integer(101)),
        ("Mtt ", D::Boolean(true)),
    ] {
        let mut d = if key == "In  " { nested.clone() } else { base.clone() };
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.retain(|(k, _)| !k.is(key));
        settings.items.push((Id::new(key), value));
        cases.push(d);
    }
    let mut duplicate = nested.clone();
    let Some(D::Descriptor(settings)) = duplicate.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.push((Id::new("In  "), web_path("/other/image.png")));
    cases.push(duplicate);
    let mut wrong_class = base.clone();
    let Some(D::Descriptor(settings)) = wrong_class.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.class_id = Id::new("Other");
    cases.push(wrong_class);
    for corruption in 0..4 {
        let D::Path(mut bytes) = web_path("/out/image.png") else { panic!("path") };
        match corruption {
            0 => bytes[12..14].copy_from_slice(&0xd800_u16.to_le_bytes()),
            1 => bytes[4..8].copy_from_slice(&0_u32.to_le_bytes()),
            2 => {
                bytes.pop();
            }
            _ => bytes.extend_from_slice(&[0, 0]),
        }
        let mut d = nested.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.iter_mut().find(|(key, _)| key.is("In  ")).unwrap().1 = D::Path(bytes);
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("export"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

#[test]
fn recorded_web_destination_refuses_native_settings_the_descriptor_cannot_represent() {
    for extra in [
        json!({"slices":"selected"}),
        json!({"numbers":[1]}),
        json!({"html":true}),
        json!({"metadata":"other"}),
        json!({"convertToSrgb":"false"}),
        json!({"width":10}),
        json!({"quality":101}),
        json!({"quality":2.5}),
        json!({"transparency":"yes"}),
        json!({"unknown":true}),
        json!({"matte":"#ffffff80"}),
        json!({"matte":"teal"}),
    ] {
        let mut params = json!({"path":"/out/image.png","format":"png24"});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let native = Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), params.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{params}");
    }
    for params in [json!({"folder":""}), json!({"path":"bad\0name.png"}), json!({"path":"/out/image.png","format":"webp"})] {
        let native = Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), params.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{params}");
    }
}

#[test]
fn recorded_web_destination_plays_into_only_the_recorded_file_or_folder() {
    let dir = tempfile::tempdir().unwrap();
    for depth in [8, 16, 32] {
        for folder in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":6,"depth":depth,"name":"First.psd","background":"transparent"})).unwrap();
            let path = if folder { dir.path().to_path_buf() } else { dir.path().join(format!("fixed-{depth}.png")) };
            let action = recorded(AtnEvent::name("export"), web_destination(&path.to_string_lossy(), true, folder));
            // The saved form makes native defaults explicit; require stability after that normalization.
            let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
            let action = atn_cmds::import(&bytes).unwrap().actions.remove(0);
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            replay_saved(&mut s, &action);
            let out = if folder { dir.path().join("First.png") } else { path.clone() };
            let decoded = photocraft_io::import("out.png", &std::fs::read(&out).unwrap()).unwrap().document;
            assert_eq!(decoded.size, photocraft_geom::Size::new(8, 6));
            assert_eq!(photocraft_compose::flatten(&decoded).px.first().unwrap()[3], 0.0, "no default matte may fill the transparent canvas");
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), past);
            assert!(s.prefs().dialogs.get("file.export.saveForWebLegacy").is_some_and(|p| p.get("path").is_none() && p.get("folder").is_none()));
            if folder {
                s.execute("file.new", json!({"width":4,"height":3,"name":"Second.psd"})).unwrap();
                replay_saved(&mut s, &action);
                let second = photocraft_io::import("out.png", &std::fs::read(dir.path().join("Second.png")).unwrap()).unwrap().document;
                assert_eq!(second.size, photocraft_geom::Size::new(4, 3));
            }
        }
    }
}

#[test]
fn recorded_web_destination_with_unmapped_slice_settings_stops_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unmapped.png");
    let mut d = web_destination(&path.to_string_lossy(), true, false);
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.push((Id::new("SWsl"), enm("STsl", "SLAl")));
    let mut action = recorded(AtnEvent::name("export"), d);
    action.steps.push(("layer.renameLayer".into(), json!({"name":"AfterExport"})));
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":6})).unwrap();
    let before = s.active().unwrap().doc.clone();
    s.actions.list = vec![action];
    let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
    assert_eq!(result["ran"], 0, "{result}");
    assert!(result.get("failed").is_some(), "{result}");
    assert!(!path.exists());
    assert_eq!(s.active().unwrap().doc, before);
}

fn web_color_metadata(mut d: Descriptor, profile: &str, metadata: &str, embed: bool) -> Descriptor {
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.extend([(Id::new("SWch"), enm("STch", profile)), (Id::new("SWmd"), enm("STmd", metadata)), (Id::new("EICC"), D::Boolean(embed))]);
    d
}

#[test]
fn recorded_web_color_metadata_imports_and_exports_all_existing_choices() {
    for (code, metadata) in [("MDNn", "none"), ("MDCp", "copyright"), ("MDCC", "copyrightAndContact"), ("MDAx", "allExceptCameraInfo"), ("MDAl", "all")] {
        for (profile, convert) in [("CHsR", true), ("CHDc", false)] {
            for embed in [false, true] {
                for event in [AtnEvent::Code(*b"Expr"), AtnEvent::name("export")] {
                    let d = web_color_metadata(web_destination("/out/image.png", true, false), profile, code, embed);
                    let action = recorded(event, d);
                    assert_eq!(action.steps[0].0, "file.export.saveForWebLegacy");
                    let p = &action.steps[0].1;
                    assert_eq!(p["metadata"], metadata);
                    assert_eq!(p["convertToSrgb"], convert);
                    assert_eq!(p["embedIcc"], embed, "embedding is independent of conversion");
                    let saved = saved(&action).descriptor.unwrap();
                    let Some(D::Descriptor(settings)) = saved.get("Usng") else { panic!("SaveForWeb") };
                    assert_eq!(enum_value(settings, "SWmd", "STmd"), &Id::Code(code.as_bytes().try_into().unwrap()));
                    assert_eq!(enum_value(settings, "SWch", "STch"), &Id::Code(profile.as_bytes().try_into().unwrap()));
                    assert_eq!(settings.get("EICC"), Some(&D::Boolean(embed)));
                    let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
                    let back = atn_cmds::import(&bytes).unwrap();
                    assert_eq!(back.actions[0].steps[0].1["metadata"], metadata);
                    assert_eq!(back.actions[0].steps[0].1["convertToSrgb"], convert);
                }
            }
        }
    }
    let native =
        Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), json!({"path":"/out/image.jpg","metadata":"contact","convertToSrgb":false}))]);
    let written = saved(&native).descriptor.unwrap();
    let Some(D::Descriptor(settings)) = written.get("Usng") else { panic!("SaveForWeb") };
    assert!(enum_value(settings, "SWmd", "STmd").is("MDCC"), "native contact is an alias of copyrightAndContact");
}

#[test]
fn recorded_web_color_metadata_preserves_unknown_enums_and_extra_output_options() {
    let base = web_color_metadata(web_destination("/out/image.png", true, false), "CHsR", "MDCC", false);
    let mut cases = Vec::new();
    for (key, value) in [
        ("SWch", enm("STch", "Other")),
        ("SWch", enm("wrong", "CHsR")),
        ("SWch", D::Boolean(true)),
        ("SWmd", enm("STmd", "Other")),
        ("SWmd", enm("wrong", "MDCC")),
        ("SWmd", D::Integer(1)),
        ("SWsl", enm("STsl", "SLAl")),
        ("ovNC", D::List(vec![])),
    ] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.retain(|(k, _)| !k.is(key));
        settings.items.push((Id::new(key), value));
        cases.push(d);
    }
    for key in ["SWch", "SWmd"] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.push((Id::new(key), settings.get(key).unwrap().clone()));
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("export"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for extra in [json!({"metadata":"other"}), json!({"metadata":1}), json!({"convertToSrgb":"yes"}), json!({"metadata":"all","slices":"all"})] {
        let mut p = json!({"path":"/out/image.png"});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let native = Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{p}");
    }
}

#[test]
fn recorded_web_color_metadata_filters_the_written_jpeg_xmp() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":6})).unwrap();
    s.execute("file.fileInfo", json!({"title":"MetadataTitle","author":"ExampleCreator","copyright":"MetadataNotice"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    for (code, notice, creator, title) in
        [("MDNn", false, false, false), ("MDCp", true, false, false), ("MDCC", true, true, false), ("MDAx", true, true, true), ("MDAl", true, true, true)]
    {
        let path = dir.path().join(format!("{code}.jpg"));
        let mut d = web_color_metadata(web_destination(&path.to_string_lossy(), true, false), "CHsR", code, false);
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.iter_mut().find(|(key, _)| key.is("Fmt ")).unwrap().1 = enm("IRFm", "JPEG");
        let action = recorded(AtnEvent::name("export"), d);
        let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
        let action = atn_cmds::import(&bytes).unwrap().actions.remove(0);
        replay_saved(&mut s, &action);
        let image = photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap();
        let xmp = image.meta.xmp.unwrap_or_default();
        assert_eq!(xmp.contains("MetadataNotice"), notice, "{code}");
        assert_eq!(xmp.contains("ExampleCreator"), creator, "{code}");
        assert_eq!(xmp.contains("MetadataTitle"), title, "{code}");
        assert_eq!(s.active().unwrap().doc, before, "metadata filtering affects the export, not the source");
    }
}

#[test]
fn recorded_web_color_metadata_converts_pixels_independently_of_profile_embedding() {
    let dir = tempfile::tempdir().unwrap();
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6,"depth":depth})).unwrap();
        let source_profile = craft_cms::Builtin::DisplayP3.profile().to_bytes();
        s.edit("Tagged color", |doc, _| {
            doc.icc_profile = Some(source_profile.clone());
            let pixel = photocraft_raster::from_rgba(&doc.pixel_format(), [0.25, 0.5, 0.75, 1.0]);
            let bounds = doc.bounds();
            doc.layers[0].surface_mut().unwrap().fill_rect(bounds, &pixel);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let mut by_conversion = Vec::new();
        for (profile, convert) in [("CHDc", false), ("CHsR", true)] {
            let mut by_embedding = Vec::new();
            for embed in [false, true] {
                let path = dir.path().join(format!("{depth}-{profile}-{embed}.png"));
                let action =
                    recorded(AtnEvent::name("export"), web_color_metadata(web_destination(&path.to_string_lossy(), true, false), profile, "MDNn", embed));
                let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
                let action = atn_cmds::import(&bytes).unwrap().actions.remove(0);
                replay_saved(&mut s, &action);
                let out = photocraft_io::import("out.png", &std::fs::read(path).unwrap()).unwrap().document;
                let expected = if convert { craft_cms::Builtin::Srgb.profile().to_bytes() } else { source_profile.clone() };
                assert_eq!(out.icc_profile, embed.then_some(expected), "{depth} {profile} {embed}");
                by_embedding.push(photocraft_compose::flatten(&out).px[0]);
                assert_eq!(s.active().unwrap().doc, before, "color conversion must not edit the source");
            }
            assert_eq!(by_embedding[0], by_embedding[1], "embedding changes only the profile");
            by_conversion.push(by_embedding[0]);
        }
        assert!((0..3).any(|i| (by_conversion[0][i] - by_conversion[1][i]).abs() > 0.01), "conversion must change tagged wide-gamut pixels at {depth} bits");
    }
}

fn web_jpeg_recording(path: &str, pass: i32) -> Descriptor {
    let mut d = web_destination(path, true, false);
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.iter_mut().find(|(key, _)| key.is("Fmt ")).unwrap().1 = enm("IRFm", "JPEG");
    settings.items.extend([(Id::new("Pass"), D::Integer(pass)), (Id::new("Optm"), D::Boolean(true)), (Id::new("Qlty"), D::Integer(80))]);
    d
}

#[test]
fn recorded_web_jpeg_pass_maps_baseline_and_progressive_and_writes_pass() {
    for (pass, progressive) in [(1, false), (3, true)] {
        for optimized in [false, true] {
            for legacy in [false, true] {
                let mut d = web_jpeg_recording("/out/image.jpg", pass);
                let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
                settings.items.iter_mut().find(|(key, _)| key.is("Optm")).unwrap().1 = D::Boolean(optimized);
                if legacy {
                    settings.items.retain(|(key, _)| !key.is("Pass"));
                    settings.items.push((Id::new("Prgr"), D::Boolean(progressive)));
                }
                for event in [AtnEvent::Code(*b"Expr"), AtnEvent::name("export")] {
                    let action = recorded(event, d.clone());
                    assert_eq!(action.steps[0].0, "file.export.saveForWebLegacy");
                    assert_eq!(action.steps[0].1["format"], "jpeg");
                    assert_eq!(action.steps[0].1["progressive"], progressive);
                    assert_eq!(action.steps[0].1["optimized"], optimized);
                    let saved = saved(&action).descriptor.unwrap();
                    let Some(D::Descriptor(settings)) = saved.get("Usng") else { panic!("SaveForWeb") };
                    assert_eq!(settings.get("Pass"), Some(&D::Integer(pass)));
                    assert!(settings.get("Prgr").is_none(), "JPEG exports its recorded integer scan choice");
                    let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
                    let back = atn_cmds::import(&bytes).unwrap();
                    assert!(back.unsupported.is_empty());
                    assert_eq!(back.actions[0].steps[0].1["progressive"], progressive);
                }
            }
        }
    }
}

#[test]
fn recorded_web_jpeg_pass_preserves_other_scans_conflicts_and_unrepresented_settings() {
    let base = web_jpeg_recording("/out/image.jpg", 3);
    let mut cases = Vec::new();
    for value in [D::Integer(-1), D::Integer(0), D::Integer(2), D::Integer(4), D::Double(3.0), D::Boolean(true)] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.iter_mut().find(|(key, _)| key.is("Pass")).unwrap().1 = value;
        cases.push(d);
    }
    for (key, value) in [("Prgr", D::Boolean(false)), ("Prgr", D::Boolean(true)), ("Pass", D::Integer(3)), ("blur", D::Double(1.0)), ("QChS", D::Integer(1))] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.push((Id::new(key), value));
        cases.push(d);
    }
    for format in [None, Some(enm("IRFm", "PN24")), Some(enm("IRFm", "GIFf"))] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.retain(|(key, _)| !key.is("Fmt "));
        if let Some(format) = format {
            settings.items.push((Id::new("Fmt "), format));
        }
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("export"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

/// Read JPEG frame markers before the first scan, independently of the native progressive flag.
fn web_jpeg_frame_marker(bytes: &[u8]) -> Option<u8> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut rest = bytes.get(2..)?;
    loop {
        if rest.first() != Some(&0xff) {
            return None;
        }
        let marker = *rest.get(1)?;
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            return Some(marker);
        }
        if matches!(marker, 0xda | 0xd9) {
            return None;
        }
        let len = usize::from(u16::from_be_bytes(rest.get(2..4)?.try_into().ok()?));
        if len < 2 {
            return None;
        }
        rest = rest.get(2 + len..)?;
    }
}

fn web_neutral_jpeg_recording() -> Descriptor {
    let mut d = web_jpeg_recording("proof.jpg", 1);
    let Some(D::Descriptor(u)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else {
        panic!("SaveForWeb");
    };
    u.items.extend([
        (Id::new("blur"), D::Double(0.0)),
        (Id::new("QChS"), D::Integer(0)),
        (Id::new("QCUI"), D::Integer(0)),
        (Id::new("QChT"), D::Boolean(false)),
        (Id::new("QChV"), D::Boolean(false)),
    ]);
    d
}

#[test]
fn recorded_web_neutral_jpeg_options_map_and_keep_the_recorded_descriptor() {
    let d = web_neutral_jpeg_recording();
    let action = recorded(AtnEvent::name("export"), d.clone());
    assert_eq!(action.steps[0].0, "file.export.saveForWebLegacy");
    assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    let mut edited = action.clone();
    edited.steps[0].1["quality"] = json!(37);
    let back = saved(&edited).descriptor.unwrap();
    let Some(D::Descriptor(u)) = back.get("Usng") else {
        panic!("SaveForWeb");
    };
    for (key, value) in [("blur", D::Double(0.0)), ("QChS", D::Integer(0)), ("QCUI", D::Integer(0)), ("QChT", D::Boolean(false)), ("QChV", D::Boolean(false))] {
        assert_eq!(u.get(key), Some(&value));
    }
    assert_eq!(u.get("Qlty"), Some(&D::Integer(37)));
}

#[test]
fn recorded_web_neutral_jpeg_options_export_explicit_resets_for_native_steps() {
    let action = Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), json!({"path":"proof.jpg","format":"jpeg"}))]);
    let back = saved(&action).descriptor.unwrap();
    let Some(D::Descriptor(u)) = back.get("Usng") else {
        panic!("SaveForWeb");
    };
    assert_eq!(u.get("blur"), Some(&D::Double(0.0)));
    assert_eq!(u.get("QChS"), Some(&D::Integer(0)));
    assert_eq!(u.get("QCUI"), Some(&D::Integer(0)));
    assert_eq!(u.get("QChT"), Some(&D::Boolean(false)));
    assert_eq!(u.get("QChV"), Some(&D::Boolean(false)));
}

#[test]
fn recorded_web_neutral_jpeg_options_keep_active_malformed_and_other_format_forms() {
    for (key, value) in [
        ("blur", D::Double(0.5)),
        ("blur", D::Integer(0)),
        ("QChS", D::Integer(1)),
        ("QChS", D::Double(0.0)),
        ("QCUI", D::Integer(1)),
        ("QChT", D::Boolean(true)),
        ("QChV", D::Boolean(true)),
        ("Fmt ", enm("IRFm", "PN24")),
    ] {
        let mut d = web_neutral_jpeg_recording();
        let Some(D::Descriptor(u)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else {
            panic!("SaveForWeb");
        };
        *u.items.iter_mut().find(|(k, _)| k.is(key)).unwrap() = (Id::new(key), value);
        let action = recorded(AtnEvent::name("export"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{key}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    let action = recorded(AtnEvent::name("export"), web_neutral_jpeg_recording());
    let mut edited = action.clone();
    edited.steps[0].1["_atnWebDescriptor"] = json!("broken");
    assert!(atn_cmds::export("Proof", &[&edited], &[], false).is_err());
}

#[test]
fn recorded_web_jpeg_pass_writes_the_expected_frame_at_every_document_depth() {
    let dir = tempfile::tempdir().unwrap();
    for depth in [8, 16, 32] {
        for (pass, marker) in [(1, 0xc0), (3, 0xc2)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":6,"depth":depth})).unwrap();
            s.execute("edit.fill", json!({"contents":"color","color":"#3366cc"})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            let path = dir.path().join(format!("{depth}-{pass}.jpg"));
            let action = recorded(AtnEvent::name("export"), web_jpeg_recording(&path.to_string_lossy(), pass));
            let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
            let action = atn_cmds::import(&bytes).unwrap().actions.remove(0);
            replay_saved(&mut s, &action);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(web_jpeg_frame_marker(&bytes), Some(marker), "Pass {pass} at {depth} bits");
            assert_eq!(photocraft_codecs::decode(&bytes).unwrap().dimensions(), (8, 6));
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), past);
        }
    }
}

fn web_all_slice_images(mut d: Descriptor, images_folder: bool) -> Descriptor {
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.extend([
        (Id::new("SWsl"), enm("STsl", "SLAl")),
        (Id::new("SImg"), D::Boolean(true)),
        (Id::new("SHTM"), D::Boolean(false)),
        (Id::new("ovSF"), D::Boolean(images_folder)),
        (Id::new("ovSN"), D::Text(UnicodeString::new_nul("images"))),
    ]);
    d
}

#[test]
fn recorded_web_slice_output_maps_all_images_to_a_directory() {
    for images in [false, true] {
        for nested in [false, true] {
            let d = web_all_slice_images(web_destination("/out/slices", nested, true), images);
            for event in [AtnEvent::Code(*b"Expr"), AtnEvent::name("export")] {
                let action = recorded(event, d.clone());
                assert_eq!(action.steps[0].0, "file.export.saveForWebLegacy");
                let p = &action.steps[0].1;
                assert_eq!(p["dir"], "/out/slices");
                assert_eq!(p["slices"], "all");
                assert_eq!(p["html"], false);
                assert_eq!(p["imagesFolder"], if images { "images" } else { "" });
                assert!(p.get("folder").is_none() && p.get("path").is_none(), "slice output must not become a whole-image save");
                let written = saved(&action).descriptor.unwrap();
                let Some(D::Descriptor(settings)) = written.get("Usng") else { panic!("SaveForWeb") };
                assert_eq!(settings.get("DIDr"), Some(&D::Boolean(true)));
                assert!(enum_value(settings, "SWsl", "STsl").is("SLAl"));
                for key in ["SImg", "SHTM", "ovSF", "ovSN"] {
                    let Some(D::Descriptor(source)) = d.get("Usng") else { panic!("source") };
                    assert_eq!(settings.get(key), source.get(key), "{key}");
                }
            }
        }
    }
    let mut d = web_all_slice_images(web_destination("/out/slices", true, true), true);
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.iter_mut().find(|(key, _)| key.is("ovSN")).unwrap().1 = D::Text(UnicodeString::new_nul("assets/切片"));
    let action = recorded(AtnEvent::name("export"), d);
    assert_eq!(action.steps[0].1["imagesFolder"], "assets/切片");
    let written = saved(&action).descriptor.unwrap();
    let Some(D::Descriptor(settings)) = written.get("Usng") else { panic!("SaveForWeb") };
    assert_eq!(settings.get("ovSN"), Some(&D::Text(UnicodeString::new_nul("assets/切片"))));
}

#[test]
fn recorded_web_slice_output_allows_an_absent_inactive_folder_name() {
    let mut d = web_all_slice_images(web_destination("/out/slices", true, true), false);
    let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
    settings.items.retain(|(key, _)| !key.is("ovSN"));
    let action = recorded(AtnEvent::name("export"), d);
    assert_eq!(action.steps[0].0, "file.export.saveForWebLegacy");
    assert_eq!(action.steps[0].1["imagesFolder"], "");
    assert_eq!(action.steps[0].1["dir"], "/out/slices");
    assert_eq!(action.steps[0].1["slices"], "all");
}

#[test]
fn recorded_web_slice_output_writes_user_and_auto_pieces_at_each_depth() {
    let root = tempfile::tempdir().unwrap();
    for depth in [8, 16, 32] {
        for images_folder in ["", "images", "assets/切片"] {
            let images = !images_folder.is_empty();
            let out = root.path().join(format!(
                "{depth}-{}",
                if images_folder == "" {
                    "root"
                } else if images_folder == "images" {
                    "default"
                } else {
                    "custom"
                }
            ));
            let mut s = Session::new();
            s.execute("file.new", json!({"width":32,"height":24,"depth":depth,"name":"Export.psd"})).unwrap();
            s.execute("slice.new", json!({"rect":[0,0,16,12],"name":"left"})).unwrap();
            s.execute("slice.new", json!({"rect":[16,0,16,12],"name":"right"})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            let mut d = web_all_slice_images(web_destination(&out.to_string_lossy(), true, true), images);
            let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
            settings.items.iter_mut().find(|(key, _)| key.is("ovSN")).unwrap().1 =
                D::Text(UnicodeString::new_nul(if images { images_folder } else { "images" }));
            let action = recorded(AtnEvent::name("export"), d);
            let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
            let action = atn_cmds::import(&bytes).unwrap().actions.remove(0);
            replay_saved(&mut s, &action);
            let files = if images { out.join(images_folder) } else { out.clone() };
            let mut names: Vec<_> = std::fs::read_dir(&files).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            assert_eq!(names, ["Export_03.png", "left.png", "right.png"], "All includes auto slices");
            for (name, size) in [("left.png", (16, 12)), ("right.png", (16, 12)), ("Export_03.png", (32, 12))] {
                assert_eq!(photocraft_codecs::decode(&std::fs::read(files.join(name)).unwrap()).unwrap().dimensions(), size);
            }
            assert!(!out.join("Export.png").exists(), "do not replace pieces with the whole image");
            assert!(!out.join("Export.html").exists(), "images only");
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().history.past_len(), past);
            assert!(s.prefs().dialogs.get("file.export.saveForWebLegacy").is_some_and(|p| p.get("dir").is_none()));
            s.execute("file.new", json!({"width":8,"height":6,"name":"Second.psd"})).unwrap();
            replay_saved(&mut s, &action);
            assert_eq!(photocraft_codecs::decode(&std::fs::read(files.join("Second.png")).unwrap()).unwrap().dimensions(), (8, 6));
        }
    }
}

#[test]
fn recorded_web_slice_output_keeps_incomplete_or_unrepresented_output_choices() {
    let base = web_all_slice_images(web_destination("/out/slices", true, true), true);
    let mut cases = Vec::new();
    for key in ["SWsl", "SImg", "SHTM", "ovSF", "ovSN", "DIDr"] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        let mut duplicate = settings.clone();
        duplicate.items.push((Id::new(key), duplicate.get(key).unwrap().clone()));
        cases.push(Descriptor::new("null").with("Usng", D::Descriptor(duplicate)));
        settings.items.retain(|(k, _)| !k.is(key));
        cases.push(d);
    }
    for (key, value) in [
        ("SWsl", enm("STsl", "Other")),
        ("SWsl", enm("wrong", "SLAl")),
        ("SImg", D::Boolean(false)),
        ("SHTM", D::Boolean(true)),
        ("SHTM", D::Integer(0)),
        ("ovSF", D::Integer(1)),
        ("ovSN", D::Text(UnicodeString::new_nul(""))),
        ("ovSN", D::Text(UnicodeString::new_nul("../outside"))),
        ("ovSN", D::Integer(0)),
        ("DIDr", D::Boolean(false)),
        ("ovNC", D::List(vec![])),
        ("ovSN", D::Text(UnicodeString(vec![0xd800, 0]))),
        ("ovSN", D::Text(UnicodeString::new_nul("bad\0name"))),
        ("ovSN", D::Text(UnicodeString::new_nul("C:\\outside"))),
        ("obIA", D::Boolean(false)),
        ("obIP", D::Text(UnicodeString::new_nul(""))),
    ] {
        let mut d = base.clone();
        let Some(D::Descriptor(settings)) = d.items.iter_mut().find(|(key, _)| key.is("Usng")).map(|(_, v)| v) else { panic!("SaveForWeb") };
        settings.items.retain(|(k, _)| !k.is(key));
        settings.items.push((Id::new(key), value));
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("export"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for extra in [
        json!({"slices":"user"}),
        json!({"slices":"selected"}),
        json!({"numbers":[1]}),
        json!({"html":true}),
        json!({"imagesFolder":1}),
        json!({"imagesFolder":"../outside"}),
        json!({"path":"/out/image.png"}),
        json!({"folder":"/out/other"}),
        json!({"dir":""}),
    ] {
        let mut p = json!({"dir":"/out/slices","format":"png24"});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let native = Action::new("Recorded", vec![("file.export.saveForWebLegacy".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{p}");
    }
}

fn duplicate_into_document(source: ReferenceItem, name: &str) -> Descriptor {
    Descriptor::new("Dplc")
        .with("null", D::Reference(vec![source]))
        .with("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul(name) }]))
        .with("Vrsn", D::Integer(5))
        .with("Idnt", D::List(vec![D::Integer(42)]))
}

#[test]
fn recorded_duplicate_destination_keeps_names_and_layer_reference_forms() {
    for source in [
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ReferenceItem::Index { class: class("Lyr "), index: 1 },
        ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Ink") },
        ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") },
    ] {
        for event in [AtnEvent::Code(*b"Dplc"), AtnEvent::name("duplicate")] {
            let d = duplicate_into_document(source.clone(), "目标.psd").with("Nm  ", D::Text(UnicodeString::new_nul("Copy"))).with("MkVs", D::Boolean(false));
            let action = recorded(event, d.clone());
            assert_eq!(action.steps[0].0, "layer.duplicate");
            assert_eq!(action.steps[0].1["document"], json!({"name":"目标.psd"}));
            assert_eq!(action.steps[0].1["name"], "Copy");
            let back = saved(&action).descriptor.unwrap();
            for key in ["null", "T   ", "Nm  "] {
                assert_eq!(back.get(key), d.get(key));
            }
            assert_eq!(recorded(AtnEvent::Code(*b"Dplc"), back).steps, action.steps);
        }
    }
}

#[test]
fn recorded_duplicate_destination_preserves_legacy_same_document_recordings() {
    let source = ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") };
    let d = Descriptor::new("Dplc").with("null", D::Reference(vec![source])).with("Vrsn", D::Integer(2));
    let action = recorded(AtnEvent::Code(*b"Dplc"), d);
    assert_eq!(action.steps, [("layer.duplicate".into(), json!({}))]);
    let empty = Action::new("Recorded", vec![("layer.duplicate".into(), serde_json::Value::Null)]);
    assert_eq!(saved(&empty).event, AtnEvent::Code(*b"Dplc"));
}

#[test]
fn recorded_duplicate_destination_replays_without_switching_the_parent_document() {
    for depth in [8, 16, 32] {
        for reference_kind in ["target", "name", "index"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":24,"height":20,"name":"Source.psd","depth":depth,"background":"transparent"})).unwrap();
            s.execute("select.rect", json!({"x":2,"y":3,"width":4,"height":5})).unwrap();
            s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.execute("layer.setProps", json!({"name":"Ink"})).unwrap();
            s.execute("layer.new.layer", json!({"name":"Other"})).unwrap();
            if reference_kind == "target" {
                s.execute("layer.select", json!({"name":"Ink"})).unwrap();
            }
            let source = s.active().unwrap().doc.clone();
            let source_history = s.active().unwrap().history.past_len();
            s.execute("file.new", json!({"width":32,"height":28,"name":"Target.psd","depth":depth})).unwrap();
            let dest_history = s.active().unwrap().history.past_len();
            s.set_active(0);
            let reference = match reference_kind {
                "name" => ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Ink") },
                "index" => ReferenceItem::Index { class: class("Lyr "), index: 1 },
                _ => ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
            };
            let action =
                recorded(AtnEvent::name("duplicate"), duplicate_into_document(reference, "Target.psd").with("Nm  ", D::Text(UnicodeString::new_nul("Copy"))));
            replay_saved(&mut s, &action);
            assert_eq!(s.active_index(), Some(0), "the parent must keep acting on the source");
            assert_eq!(s.documents()[0].doc, source);
            assert_eq!(s.documents()[0].history.past_len(), source_history);
            let dest = &s.documents()[1];
            let copy = dest.doc.layers.iter().find(|layer| layer.name == "Copy").unwrap();
            assert_eq!(copy.surface().unwrap().content_bounds(), photocraft_geom::Rect::from_xywh(2, 3, 4, 5));
            assert_eq!(dest.history.past_len(), dest_history + 1);
            assert_ne!(copy.id, source.layers[0].id);
            s.set_active(1);
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.active().unwrap().doc.layers.iter().all(|layer| layer.name != "Copy"));
        }
    }
}

#[test]
fn recorded_duplicate_destination_copies_the_selection_and_continues_in_the_source() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":16,"name":"Target.psd"})).unwrap();
    s.execute("file.new", json!({"width":16,"height":16,"name":"Source.psd","background":"transparent"})).unwrap();
    s.execute("layer.setProps", json!({"name":"First"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Second"})).unwrap();
    s.execute("layer.select", json!({"name":"First","mode":"add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2);
    let source_ids = s.active().unwrap().selected_layers();
    let reference = ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") };
    let mut action = recorded(AtnEvent::name("duplicate"), duplicate_into_document(reference, "Target.psd"));
    action.steps.push(("layer.new.layer".into(), json!({"name":"After"})));
    let (bytes, _) = atn_cmds::export("Identifiers", &[&action], &[], false).unwrap();
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    s.actions.list = imported.actions;
    let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
    assert_eq!(result["ran"], 2);
    assert_eq!(s.active_index(), Some(1));
    assert!(s.documents()[1].doc.layers.iter().any(|layer| layer.name == "After"));
    let dest = &s.documents()[0];
    assert_eq!(dest.doc.layers.len(), 3);
    assert_eq!(dest.doc.layers.iter().skip(1).map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    assert!(dest.doc.layers.iter().all(|layer| !source_ids.contains(&layer.id)));
}

#[test]
fn recorded_duplicate_destination_rejects_ambiguous_and_unknown_forms() {
    let source = ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") };
    let base = duplicate_into_document(source, "Target.psd");
    for (key, value) in [
        ("T   ", D::Reference(vec![ReferenceItem::Identifier { class: class("Dcmn"), id: 42 }])),
        ("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Target.psd") }])),
        ("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString(vec![0xd800]) }])),
        ("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("") }])),
        ("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("bad\0name") }])),
        ("Vrsn", D::Integer(6)),
        ("MkVs", D::Boolean(true)),
        ("Idnt", D::List(vec![D::Text(UnicodeString::new_nul("wrong"))])),
        ("Nm  ", D::Integer(1)),
        ("unknown", D::Boolean(true)),
    ] {
        let mut d = base.clone();
        d.items.retain(|(k, _)| !k.is(key));
        d.items.push((Id::new(key), value));
        let action = recorded(AtnEvent::name("duplicate"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for p in [
        json!({"document":{"name":"Target.psd"},"inPlace":true}),
        json!({"document":1}),
        json!({"document":{"name":"Target.psd","index":1}}),
        json!({"document":{"name":""}}),
        json!({"document":{"name":"Target.psd"},"mergedOnly":true}),
    ] {
        let action = Action::new("Recorded", vec![("layer.duplicate".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&action], &[], false).is_err(), "{p}");
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"name":"Source.psd"})).unwrap();
    s.execute("file.new", json!({"width":8,"height":8,"name":"Target.psd"})).unwrap();
    s.execute("file.new", json!({"width":8,"height":8,"name":"Target.psd"})).unwrap();
    s.set_active(0);
    for name in ["Missing.psd", "Target.psd"] {
        let before: Vec<_> = s.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect();
        assert!(s.execute("layer.duplicate", json!({"document":{"name":name}})).is_err());
        assert_eq!(s.active_index(), Some(0));
        assert_eq!(s.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect::<Vec<_>>(), before);
    }
    s.close(2);
    s.set_active(0);
    for extra in [json!({"source":1}), json!({"offset":[10,20]}), json!({"documentName":"Wrong"}), json!({"name":1}), json!({"inPlace":true})] {
        let mut p = json!({"document":{"name":"Target.psd"}});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(s.execute("layer.duplicate", p.clone()).is_err(), "{p}");
        assert_eq!(s.active_index(), Some(0));
        assert_eq!(s.documents()[1].doc.layers.len(), 1);
    }
}

fn color_overlay_style(red: f64, green: f64, blue: f64) -> Descriptor {
    let overlay = Descriptor::new("SoFi")
        .with("enab", D::Boolean(true))
        .with("present", D::Boolean(true))
        .with("showInDialog", D::Boolean(true))
        .with("Md  ", enm("BlnM", "Nrml"))
        .with("Clr ", D::Descriptor(Descriptor::new("RGBC").with("Rd  ", D::Double(red)).with("Grn ", D::Double(green)).with("Bl  ", D::Double(blue))))
        .with("Opct", D::UnitFloat { unit: *b"#Prc", value: 100.0 });
    Descriptor::new("setd")
        .with(
            "null",
            D::Reference(vec![
                ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
                ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
            ]),
        )
        .with("T   ", D::Descriptor(Descriptor::new("Lefx").with("Scl ", D::UnitFloat { unit: *b"#Prc", value: 100.0 }).with("SoFi", D::Descriptor(overlay))))
}

#[test]
fn recorded_color_overlay_style_preserves_the_full_descriptor_and_edits() {
    let d = color_overlay_style(12.25, 128.0, 240.75);
    for event in [AtnEvent::Code(*b"setd"), AtnEvent::name("set")] {
        let mut action = recorded(event, d.clone());
        assert_eq!(action.steps[0].0, "layer.layerStyle.replace");
        assert_eq!(action.steps[0].1["effects"].as_array().unwrap().len(), 1);
        assert_eq!(action.steps[0].1["effects"][0]["kind"], "colorOverlay");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
        action.steps[0].1["effects"][0]["params"]["opacity"] = json!(50);
        let written = saved(&action).descriptor.unwrap();
        assert_ne!(written.to_bytes(), d.to_bytes(), "an edit must not restore stale source bytes");
        let Some(D::Descriptor(style)) = written.get("T   ") else { panic!("style") };
        let Some(D::Descriptor(overlay)) = style.get("SoFi") else { panic!("overlay") };
        assert_eq!(overlay.get("Opct"), Some(&D::UnitFloat { unit: *b"#Prc", value: 50.0 }));
        assert_eq!(enum_value(overlay, "Md  ", "BlnM"), &Id::new("Nrml"));
    }
}

#[test]
fn recorded_color_overlay_style_preserves_enabled_opacity_and_blend() {
    for enabled in [false, true] {
        let mut d = color_overlay_style(12.25, 128.0, 240.75);
        let Some(D::Descriptor(style)) = d.items.iter_mut().find(|(key, _)| key.is("T   ")).map(|(_, v)| v) else { panic!("style") };
        let Some(D::Descriptor(overlay)) = style.items.iter_mut().find(|(key, _)| key.is("SoFi")).map(|(_, v)| v) else { panic!("overlay") };
        for (key, value) in [("enab", D::Boolean(enabled)), ("Md  ", enm("BlnM", "Mltp")), ("Opct", D::UnitFloat { unit: *b"#Prc", value: 37.5 })] {
            overlay.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = value;
        }
        let action = recorded(AtnEvent::name("set"), d.clone());
        assert_eq!(action.steps[0].0, "layer.layerStyle.replace");
        let effect: photocraft_doc::Effect = serde_json::from_value(action.steps[0].1["effects"][0]["fx"].clone()).unwrap();
        let photocraft_doc::Effect::ColorOverlay { common, .. } = effect else { panic!("effect") };
        assert_eq!(common.enabled, enabled);
        assert_eq!(common.blend, photocraft_color::BlendMode::Multiply);
        assert_eq!(common.opacity, 0.375);
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

#[test]
fn recorded_color_overlay_style_replaces_old_effects_and_undoes_at_each_depth() {
    let action = recorded(AtnEvent::name("set"), color_overlay_style(0.0, 0.0, 255.0));
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({"size":1,"distance":1})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"color":"#00ff00","size":1})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        replay_saved(&mut s, &action);
        let st = s.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(layer.effects.items.len(), 1, "Set Lefx replaces the full list");
        assert!(matches!(layer.effects.items[0], photocraft_doc::Effect::ColorOverlay { .. }));
        assert_eq!(st.history.past_len(), past + 1);
        let pixel = photocraft_compose::render(&st.doc, photocraft_geom::Rect::from_xywh(3, 3, 1, 1)).px[0];
        assert_eq!(pixel, [0.0, 0.0, 1.0, 1.0]);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn recorded_color_overlay_style_keeps_unknown_or_incomplete_effects() {
    let base = color_overlay_style(12.25, 128.0, 240.75);
    let mut cases = Vec::new();
    for (key, value) in
        [("Scl ", D::UnitFloat { unit: *b"#Prc", value: 50.0 }), ("masterFXSwitch", D::Boolean(false)), ("DrSh", D::Descriptor(Descriptor::new("DrSh")))]
    {
        let mut d = base.clone();
        let Some(D::Descriptor(style)) = d.items.iter_mut().find(|(key, _)| key.is("T   ")).map(|(_, v)| v) else { panic!("style") };
        style.items.retain(|(k, _)| !k.is(key));
        style.items.push((Id::new(key), value));
        cases.push(d);
    }
    for (key, value) in [
        ("present", D::Boolean(false)),
        ("showInDialog", D::Boolean(false)),
        ("Opct", D::UnitFloat { unit: *b"#Prc", value: 101.0 }),
        ("Opct", D::UnitFloat { unit: *b"#Pxl", value: 50.0 }),
        ("Md  ", enm("BlnM", "unknown")),
        ("unknown", D::Boolean(true)),
    ] {
        let mut d = base.clone();
        let Some(D::Descriptor(style)) = d.items.iter_mut().find(|(key, _)| key.is("T   ")).map(|(_, v)| v) else { panic!("style") };
        let Some(D::Descriptor(overlay)) = style.items.iter_mut().find(|(key, _)| key.is("SoFi")).map(|(_, v)| v) else { panic!("overlay") };
        overlay.items.retain(|(k, _)| !k.is(key));
        overlay.items.push((Id::new(key), value));
        cases.push(d);
    }
    for key in ["null", "T   "] {
        let mut d = base.clone();
        d.items.push((Id::new(key), d.get(key).unwrap().clone()));
        cases.push(d);
    }
    for key in ["enab", "Md  ", "Clr ", "Opct"] {
        let mut d = base.clone();
        let Some(D::Descriptor(style)) = d.items.iter_mut().find(|(key, _)| key.is("T   ")).map(|(_, v)| v) else { panic!("style") };
        let Some(D::Descriptor(overlay)) = style.items.iter_mut().find(|(key, _)| key.is("SoFi")).map(|(_, v)| v) else { panic!("overlay") };
        let mut duplicate = overlay.clone();
        duplicate.items.push((Id::new(key), duplicate.get(key).unwrap().clone()));
        let mut duplicate_style = style.clone();
        duplicate_style.items.iter_mut().find(|(key, _)| key.is("SoFi")).unwrap().1 = D::Descriptor(duplicate);
        cases.push(Descriptor::new("null").with("null", base.get("null").unwrap().clone()).with("T   ", D::Descriptor(duplicate_style)));
        let Some(D::Descriptor(overlay)) = style.items.iter_mut().find(|(key, _)| key.is("SoFi")).map(|(_, v)| v) else { panic!("overlay") };
        overlay.items.retain(|(k, _)| !k.is(key));
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("set"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

fn history_offset(offset: i32) -> Descriptor {
    Descriptor::new("slct").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("HstS"), offset }]))
}

fn history_document(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth})).unwrap();
    for (name, color) in [("First", "#0000ff"), ("Second", "#00ff00"), ("Third", "#ff0000")] {
        s.execute("layer.new.layer", json!({"name":name})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":color})).unwrap();
    }
    assert_eq!(s.active().unwrap().history.past_len(), 6);
    s
}

#[test]
fn recorded_history_offsets_import_and_export_signed_relative_references() {
    for offset in [i32::MIN, -5, -2, -1, 1, 3, i32::MAX] {
        let d = history_offset(offset);
        for event in [AtnEvent::Code(*b"slct"), AtnEvent::name("select")] {
            let action = recorded(event, d.clone());
            assert_eq!(action.steps, vec![("history.selectState".into(), json!({"offset":offset}))]);
            assert_eq!(saved(&action).descriptor.unwrap().get("null"), d.get("null"));
        }
    }
}

#[test]
fn recorded_history_offsets_resolve_against_the_playback_position() {
    for depth in [8, 16, 32] {
        let mut s = history_document(depth);
        for (from, offset, target) in [(6, -2, 4), (6, -5, 1), (4, -2, 2), (1, 3, 4), (0, 6, 6)] {
            let action = recorded(AtnEvent::name("select"), history_offset(offset));
            s.execute("history.selectState", json!({"state":target})).unwrap();
            let expected = s.active().unwrap().doc.clone();
            let expected_layer = s.active().unwrap().active_layer;
            s.execute("history.selectState", json!({"state":from})).unwrap();
            let revision = s.active().unwrap().revision;
            replay_saved(&mut s, &action);
            let st = s.active().unwrap();
            assert_eq!(st.doc, expected, "{from} + {offset} at depth {depth}");
            assert_eq!(st.active_layer, expected_layer);
            assert_eq!(st.history.past_len(), target);
            assert_eq!(st.history.redo_labels().count(), 6 - target, "later states remain redoable");
            assert_eq!(st.revision, revision + 1, "one history jump");
        }
    }
}

#[test]
fn recorded_history_offsets_reject_invalid_native_targets_before_changing_history() {
    let mut s = history_document(8);
    s.execute("history.selectState", json!({"state":3})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let redo: Vec<_> = s.active().unwrap().history.redo_labels().map(str::to_string).collect();
    for p in [
        json!({"offset":-4}),
        json!({"offset":4}),
        json!({"offset":i64::MIN}),
        json!({"offset":i64::MAX}),
        json!({"offset":f64::MAX}),
        json!({"offset":0.5}),
        json!({"offset":"-2"}),
        json!({"offset":null}),
        json!({"offset":true}),
        json!({"offset":[]}),
        json!({"state":1,"offset":-2}),
    ] {
        assert!(s.execute("history.selectState", p.clone()).is_err(), "{p}");
        let st = s.active().unwrap();
        assert_eq!(st.doc, before, "{p}");
        assert_eq!(st.revision, revision, "{p}");
        assert_eq!(st.history.past_len(), 3, "{p}");
        assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo, "{p}");
    }
    assert_eq!(s.execute("history.selectState", json!({"offset":0})).unwrap()["changed"], false);
    assert_eq!(s.active().unwrap().revision, revision, "native zero offset is a no-op");
    assert!(Session::new().execute("history.selectState", json!({"offset":-2})).is_err());
}

#[test]
fn recorded_history_offsets_keep_unrepresented_scopes_and_export_options() {
    let d = history_offset(-2);
    let D::Reference(r) = d.get("null").unwrap() else { panic!("history reference") };
    let mut duplicate = d.clone();
    duplicate.items.push((Id::new("null"), D::Reference(r.clone())));
    let cases = [
        history_offset(0),
        d.clone().with("unknown", D::Boolean(true)),
        duplicate,
        Descriptor::new("slct")
            .with("null", D::Reference(vec![r[0].clone(), ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") }])),
        Descriptor::new("slct").with("null", D::Reference(vec![ReferenceItem::Index { class: class("HstS"), index: 2 }])),
        Descriptor::new("slct").with("null", D::Reference(vec![ReferenceItem::Identifier { class: class("HstS"), id: 2 }])),
    ];
    for d in cases {
        let action = recorded(AtnEvent::name("select"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for p in [
        json!({"state":1}),
        json!({"offset":0}),
        json!({"offset":0.5}),
        json!({"offset":i64::MAX}),
        json!({"offset":-2,"state":1}),
        json!({"offset":-2,"unknown":true}),
    ] {
        let native = Action::new("Recorded", vec![("history.selectState".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{p}");
    }
}

#[test]
fn recorded_history_offsets_continue_the_parent_only_after_a_successful_jump() {
    for (offset, succeeds) in [(-2, true), (-20, false)] {
        let mut s = history_document(8);
        let before = s.active().unwrap().doc.clone();
        let mut action = recorded(AtnEvent::name("select"), history_offset(offset));
        action.steps.push(("layer.renameLayer".into(), json!({"name":"AfterRestore"})));
        s.actions.list = vec![action];
        let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        if succeeds {
            assert_eq!(result["ran"], 2, "{result}");
            assert!(result.get("failed").is_none(), "{result}");
            let st = s.active().unwrap();
            assert_eq!(st.doc.layers.len(), 3, "return to the state before the third layer was created");
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "AfterRestore");
        } else {
            assert!(result.get("failed").is_some(), "{result}");
            assert_eq!(result["ran"], 0);
            assert_eq!(s.active().unwrap().doc, before, "an out-of-range jump must not partially undo");
        }
    }
}

fn snapshot_selection(name: &str) -> Descriptor {
    Descriptor::new("slct").with("null", D::Reference(vec![ReferenceItem::Name { class: class("SnpS"), name: UnicodeString::new_nul(name) }]))
}

#[test]
fn recorded_snapshot_selection_imports_and_exports_the_name() {
    for name in ["Checkpoint", "检查点", "  padded  ", "1"] {
        let d = snapshot_selection(name);
        for event in [AtnEvent::Code(*b"slct"), AtnEvent::name("select")] {
            let action = recorded(event, d.clone());
            assert_eq!(action.steps, vec![("history.selectSnapshot".into(), json!({"name":name}))]);
            assert_eq!(saved(&action).descriptor.unwrap().get("null"), d.get("null"));
        }
        let native = Action::new("Recorded", vec![("history.selectSnapshot".into(), json!({"name":name}))]);
        assert_eq!(saved(&native).descriptor.unwrap().get("null"), d.get("null"));
    }
}

#[test]
fn recorded_snapshot_selection_resolves_names_at_playback() {
    // Import has no document or snapshot ids to capture; the name resolves when played.
    let action = recorded(AtnEvent::name("select"), snapshot_selection("Checkpoint"));
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4,"depth":depth})).unwrap();
        s.execute("history.newSnapshot", json!({"name":"Unrelated"})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#0000ff"})).unwrap();
        let id = s.execute("history.newSnapshot", json!({"name":"Checkpoint"})).unwrap()["snapshot"].as_u64().unwrap();
        let checkpoint = s.active().unwrap().doc.clone();
        s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Later"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let states = s.active().unwrap().history.past_len();
        replay_saved(&mut s, &action);
        assert_eq!(s.active().unwrap().doc, checkpoint, "restore the named snapshot at depth {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), states + 1, "one undoable restore");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        s.execute("history.selectSnapshot", json!({"snapshot":id})).unwrap();
        assert_eq!(s.active().unwrap().doc, checkpoint, "existing id calls still work");
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("history.renameSnapshot", json!({"snapshot":id,"name":"Renamed"})).unwrap();
        let failed = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert!(failed.get("failed").is_some(), "the old name must not retain an earlier resolved id: {failed}");
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn recorded_snapshot_selection_keeps_unrepresented_references_and_options() {
    let d = snapshot_selection("Checkpoint");
    let D::Reference(r) = d.get("null").unwrap() else { panic!("snapshot reference") };
    let mut duplicate = d.clone();
    duplicate.items.push((Id::new("null"), D::Reference(r.clone())));
    let mut cases = vec![d.clone().with("unknown", D::Boolean(true)), d.clone().with("MkVs", D::Boolean(true)), duplicate];
    for reference in [
        vec![ReferenceItem::Name { class: class("HstS"), name: UnicodeString::new_nul("Checkpoint") }],
        vec![ReferenceItem::Index { class: class("SnpS"), index: 1 }],
        vec![ReferenceItem::Identifier { class: class("SnpS"), id: 1 }],
        vec![ReferenceItem::Enumerated { class: class("SnpS"), type_id: Id::new("Ordn"), value: Id::new("Trgt") }],
        vec![r[0].clone(), ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") }],
    ] {
        cases.push(Descriptor::new("slct").with("null", D::Reference(reference)));
    }
    cases.extend([snapshot_selection(""), snapshot_selection("   ")]);
    for name in [UnicodeString(vec![0xd800, 0]), UnicodeString(vec![65, 0, 0])] {
        cases.push(Descriptor::new("slct").with("null", D::Reference(vec![ReferenceItem::Name { class: class("SnpS"), name }])));
    }
    for d in cases {
        let action = recorded(AtnEvent::name("select"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
    for p in [json!({"snapshot":1}), json!({"name":"Checkpoint","snapshot":1}), json!({"name":"Checkpoint","unknown":true}), json!({"name":""})] {
        let native = Action::new("Recorded", vec![("history.selectSnapshot".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&native], &[], false).is_err(), "{p}");
    }
}

#[test]
fn recorded_snapshot_selection_rejects_missing_ambiguous_and_invalid_native_names() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    let id = s.execute("history.newSnapshot", json!({"name":"Duplicate"})).unwrap()["snapshot"].clone();
    s.execute("history.newSnapshot", json!({"name":"Duplicate"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let states = s.active().unwrap().history.past_len();
    for p in [
        json!({"name":"Duplicate"}),
        json!({"name":"Missing"}),
        json!({"name":" Duplicate "}),
        json!({"name":""}),
        json!({"name":"   "}),
        json!({"name":null}),
        json!({"name":1}),
        json!({"name":true}),
        json!({"name":[]}),
        json!({"name":"Duplicate","snapshot":id}),
    ] {
        assert!(s.execute("history.selectSnapshot", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().doc, before, "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), states, "{p}");
    }
    assert!(Session::new().execute("history.selectSnapshot", json!({"name":"Checkpoint"})).is_err());
}

fn vector_mask_selection() -> Descriptor {
    Descriptor::new("setd")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]))
        .with(
            "T   ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Path"), type_id: Id::new("Path"), value: Id::new("vectorMask") },
                ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
            ]),
        )
        .with("Vrsn", D::Integer(1))
        .with("vectorMaskParams", D::Boolean(true))
}

#[test]
fn recorded_vector_mask_selection_imports_and_exports_a_path_reference() {
    let expected = vector_mask_selection();
    for event in [AtnEvent::Code(*b"setd"), AtnEvent::name("set")] {
        let action = recorded(event, expected.clone());
        assert_eq!(action.steps, vec![("select.loadSelection".into(), json!({"channel":"vectorMask"}))]);
        let written = saved(&action).descriptor.unwrap();
        for key in ["null", "T   ", "Vrsn", "vectorMaskParams"] {
            assert_eq!(written.get(key), expected.get(key), "{key}: a vector mask is a Path, not a named Channel");
        }
    }
    for p in [json!({"channel":"vectorMask"}), json!({"channel":"vectorMask","invert":false,"operation":"new","layer":null,"document":null})] {
        let native = Action::new("Recorded", vec![("select.loadSelection".into(), p)]);
        assert_eq!(saved(&native).descriptor.unwrap().get("T   "), expected.get("T   "));
    }
}

#[test]
fn recorded_vector_mask_selection_resolves_the_active_layer_at_playback() {
    let action = recorded(AtnEvent::name("set"), vector_mask_selection());
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":40,"height":20,"depth":depth})).unwrap();
        for (name, x) in [("First", 4), ("Second", 22)] {
            s.execute("layer.new.layer", json!({"name":name})).unwrap();
            let path = json!({"subpaths":[{"knots":[[x,4],[x+12,4],[x+12,16],[x,16]]}]});
            s.execute("layer.vectorMask.add", json!({"path":path})).unwrap();
        }
        for (name, x) in [("First", 4), ("Second", 22)] {
            s.execute("layer.select", json!({"layer":name})).unwrap();
            s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":2})).unwrap();
            let before = s.active().unwrap().doc.clone();
            replay_saved(&mut s, &action);
            let selection = s.active().unwrap().doc.selection.as_ref().unwrap();
            assert_eq!(selection.content_bounds(), photocraft_geom::Rect::from_xywh(x, 4, 12, 12));
            assert!(selection.sample_channel(x + 6, 10, 0) > 0.99);
            assert!(selection.sample_channel(0, 0, 0) < 0.01, "replace the previous selection");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, before, "Undo at depth {depth}");
        }
        s.execute("layer.vectorMask.delete", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.actions.list = vec![action.clone()];
        let result = s.execute("actions.play", json!({"action":"Recorded"})).unwrap();
        assert!(result.get("failed").is_some(), "a missing vector mask must fail: {result}");
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn recorded_vector_mask_selection_keeps_unrepresented_forms() {
    let base = vector_mask_selection();
    let mut cases = vec![base.clone().with("Fthr", D::Double(2.0)), base.clone().with("AntA", D::Boolean(false))];
    for (key, values) in [("Vrsn", vec![D::Integer(2), D::Double(1.0)]), ("vectorMaskParams", vec![D::Boolean(false), D::Integer(1)])] {
        let mut missing = base.clone();
        missing.items.retain(|(k, _)| !k.is(key));
        cases.push(missing);
        for value in values {
            let mut d = base.clone();
            d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = value;
            cases.push(d);
        }
    }
    for key in ["null", "T   ", "Vrsn", "vectorMaskParams"] {
        let mut d = base.clone();
        d.items.push((Id::new(key), d.get(key).unwrap().clone()));
        cases.push(d);
    }
    let D::Reference(source) = base.get("T   ").unwrap() else { panic!("source reference") };
    for replacement in [
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Frnt") },
        ReferenceItem::Enumerated { class: class("Dcmn"), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Path"), value: Id::new("Trgt") },
        ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Source") },
    ] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(k, _)| k.is("T   ")).unwrap().1 = D::Reference(vec![source[0].clone(), replacement]);
        cases.push(d);
    }
    for (key, reference) in [
        ("null", vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }, source[1].clone()]),
        ("T   ", vec![source[0].clone()]),
        ("T   ", vec![source[0].clone(), source[1].clone(), source[1].clone()]),
        ("T   ", vec![ReferenceItem::Enumerated { class: class("Path"), type_id: Id::new("Ordn"), value: Id::new("vectorMask") }, source[1].clone()]),
    ] {
        let mut d = base.clone();
        d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = D::Reference(reference);
        cases.push(d);
    }
    for d in cases {
        let action = recorded(AtnEvent::name("set"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&action).descriptor.unwrap().to_bytes(), d.to_bytes());
    }
}

#[test]
fn recorded_vector_mask_selection_export_refuses_options_it_cannot_represent() {
    for extra in [json!({"layer":"Source"}), json!({"document":1}), json!({"invert":true}), json!({"operation":"add"}), json!({"unknown":true})] {
        let mut p = json!({"channel":"vectorMask"});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let action = Action::new("Recorded", vec![("select.loadSelection".into(), p.clone())]);
        assert!(atn_cmds::export("Identifiers", &[&action], &[], false).is_err(), "{p}");
        let (bytes, skipped) = atn_cmds::export("Identifiers", &[&action], &[], true).unwrap();
        assert_eq!(skipped.len(), 1, "{p}");
        assert!(atn::parse(&bytes).unwrap().actions[0].items.is_empty());
    }
}

fn canvas() -> Descriptor {
    Descriptor::new("null")
        .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 6.0 })
        .with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 4.0 })
        .with("Hrzn", enm("HrzL", "Left"))
        .with("Vrtc", enm("VrtL", "Top "))
}

/// Foreground, background and custom extension colours select different new border pixels.
#[test]
fn canvas_extension_identifiers_preserve_foreground_background_and_custom_color() {
    for (input, custom, canonical, expected) in [
        ("FrgC", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("Frgd", false, "FrgC", [1.0, 0.0, 0.0, 1.0]),
        ("BckC", false, "BckC", [0.0, 1.0, 0.0, 1.0]),
        ("Bckg", false, "BckC", [0.0, 1.0, 0.0, 1.0]),
        ("Clr ", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
        ("Othr", true, "Clr ", [0.0, 0.0, 1.0, 1.0]),
    ] {
        let mut d = canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", input));
        if custom {
            d = d.with("canvasExtensionColor", D::Descriptor(blue()));
        }
        let action = recorded(AtnEvent::Code(*b"CnvS"), d);
        assert_eq!(action.steps[0].0, "image.canvasSize");
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "canvasExtensionColorType", "canvasExtensionColorType"), Id::new(canonical));
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":4,"depth":depth,"background":"white"})).unwrap();
            s.execute("tools.setColors", json!({"foreground":"#ff0000","background":"#00ff00"})).unwrap();
            replay_saved(&mut s, &action);
            let pixel: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x":5,"y":1})).unwrap()).unwrap();
            assert_eq!(pixel, expected, "{input} at {depth} bits");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(4, 4));
        }
    }
    for d in [
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "futureColor")).with("canvasExtensionColor", D::Descriptor(blue())),
        canvas().with("canvasExtensionColorType", D::Integer(1)),
        canvas().with("canvasExtensionColorType", enm("otherType", "BckC")),
        canvas().with("canvasExtensionColorType", D::Enumerated { type_id: Id::new("canvasExtensionColorType"), value: Id::Str(b"BckC".to_vec()) }),
        canvas().with("canvasExtensionColorType", enm("canvasExtensionColorType", "Clr ")),
    ] {
        let action = recorded(AtnEvent::Code(*b"CnvS"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Absolute intent writes the measured code and unknown intent values remain preserved steps.
#[test]
fn absolute_colorimetric_writes_aclr_and_rejects_unknown_intents() {
    let profile = || Descriptor::new("null").with("T   ", D::Text(UnicodeString::new_nul("sRGB IEC61966-2.1")));
    for input in ["AClr", "AbsC"] {
        let action = recorded(AtnEvent::name("convertToProfile"), profile().with("Inte", enm("Inte", input)));
        assert_eq!(
            action.steps,
            vec![("edit.convertToProfile".into(), json!({"profile":"srgb","intent":"absolute","dither":false,"flatten":false,"rasterizePlaced":false}))]
        );
        assert_eq!(*enum_value(saved(&action).descriptor.as_ref().unwrap(), "Inte", "Inte"), Id::new("AClr"));
    }
    for value in [enm("Inte", "futureIntent"), D::Integer(1)] {
        let d = profile().with("Inte", value);
        let action = recorded(AtnEvent::name("convertToProfile"), d.clone());
        assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&action).descriptor, Some(d));
    }
}

/// Wrap uses opening order after tab moves, with overflow-safe offsets and valid ATN references.
#[test]
fn document_offsets_wrap_in_opening_order_and_zero_atn_offsets_stay_kept() {
    let mut s = Session::new();
    for name in ["a", "b", "c"] {
        s.execute("file.new", json!({"width":2,"height":2,"name":name})).unwrap();
    }
    s.execute("document.move", json!({"document":2,"to":0})).unwrap();
    for (offset, expected) in [(-2, "c"), (2, "a"), (5, "a"), (-5, "c"), (i64::MIN, "c"), (i64::MAX, "c"), (0, "b")] {
        s.execute("document.activate", json!({"document":2})).unwrap();
        s.execute("document.activate", json!({"offset":offset})).unwrap();
        assert_eq!(s.active().unwrap().doc.name, expected, "{offset}");
    }
    for offset in [-2, 2] {
        let d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]));
        let action = recorded(AtnEvent::Code(*b"slct"), d);
        assert_eq!(action.steps[0].0, "document.activate");
        s.execute("document.activate", json!({"document":2})).unwrap();
        let (id, p) = &action.steps[0];
        s.execute(id, p.clone()).unwrap();
        assert_eq!(s.active().unwrap().doc.name, if offset < 0 { "c" } else { "a" });
        let written = saved(&action).descriptor.unwrap();
        assert!(matches!(written.get("null"),Some(D::Reference(r)) if matches!(r.as_slice(),[ReferenceItem::Offset{offset:o,..}] if *o==offset)));
    }
    let zero = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset: 0 }]));
    let action = recorded(AtnEvent::Code(*b"slct"), zero.clone());
    assert_eq!(action.steps[0].0, atn_cmds::PHOTOSHOP_STEP);
    assert_eq!(saved(&action).descriptor, Some(zero));
    assert!(atn_cmds::export("Identifiers", &[&Action::new("Zero", vec![("document.activate".into(), json!({"offset":0}))])], &[], false).is_err());
}
