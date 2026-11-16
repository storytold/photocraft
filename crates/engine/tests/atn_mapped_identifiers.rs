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
fn recorded_transform_zero_points_keep_nonzero_or_unrepresentable_offsets() {
    for value in [
        D::UnitFloat { unit: *b"#Rlt", value: 1.0 },
        D::UnitFloat { unit: *b"#Rlt", value: -2.842_170_943_040_401e-14 },
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
