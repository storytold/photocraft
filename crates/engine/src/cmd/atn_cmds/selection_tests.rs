use super::*;

/// An independently constructed selection-property reference, as recorded in Group F.
fn selection() -> D {
    D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }])
}

/// Bounds stay in their recorded Top/Left/Bottom/Right order and pixel-double units.
fn rectangle(bounds: [f64; 4], ellipse: bool) -> Descriptor {
    let mut shape = desc(if ellipse { "Elps" } else { "Rctn" });
    for (key, value) in ["Top ", "Left", "Btom", "Rght"].into_iter().zip(bounds) {
        shape = shape.with(key, px(value));
    }
    let mut d = desc("null").with("null", selection()).with("T   ", D::Descriptor(shape));
    if ellipse {
        d = d.with("AntA", D::Boolean(true));
    }
    d
}

/// One real action file, shared by descriptor and engine playback witnesses.
fn file(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Group F"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event, dictionary: b"Step".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Both recorded event forms retain explicit example settings and descriptor key/type order.
#[test]
fn group_f_load_save_preserves_recorded_descriptors() {
    let mut cases = vec![
        ("AddT", Some("addTo"), rectangle([30.0, 30.0, 90.0, 90.0], false)),
        ("AddT", Some("addTo"), rectangle([30.0, 30.0, 90.0, 90.0], true)),
        ("SbtF", Some("subtractFrom"), rectangle([30.0, 30.0, 60.0, 60.0], false)),
        ("IntW", Some("interfaceWhite"), rectangle([30.0, 30.0, 120.0, 120.0], false)),
        ("PstI", Some("pasteInto"), desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        ("PstO", Some("pasteOutside"), desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        ("Fade", Some("fade"), desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Mltp"))),
        ("", Some("linkSelectedLayers"), desc("null").with("null", target_ref("Lyr "))),
        ("Dstr", Some("distort"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
        ("RmvL", Some("removeLayerMask"), desc("null")),
        ("DfnP", Some("definePattern"), desc("null")),
        ("MrgL", None, desc("null")),
    ];
    // Opposite-signed fractions can lose an endpoint through x+(right-x); preserve it explicitly.
    for mode in ["AddT", "SbtF", "IntW"] {
        cases.push((mode, None, rectangle([-0.1, -0.1, 0.3, 0.2], false)));
    }
    let mut ellipse = rectangle([1.5, 2.5, 8.5, 10.5], true);
    ellipse.items.last_mut().unwrap().1 = D::Boolean(false);
    cases.push(("AddT", Some("addTo"), ellipse));
    cases.push(("Fade", Some("fade"), desc("null").with("Opct", pct(25.5)).with("Md  ", enm("BlnM", "Scrn"))));
    for edge in ["AdCH", "AdRg", "AdTp", "AdCV", "AdBt"] {
        cases.push(("Dstr", Some("distort"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", edge))));
    }
    for (id, name, d) in cases {
        let mut events = Vec::new();
        if !id.is_empty() {
            events.push(AtnEvent::Code(id.as_bytes().try_into().unwrap()));
        }
        if let Some(name) = name {
            events.push(AtnEvent::name(name));
        }
        for event in events {
            let mut s = Session::new();
            let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&file(event.clone(),d.clone()))})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]), "{id}");
            let saved = s.execute("actions.save", json!({"action":"Step"})).unwrap();
            let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let back = atn::parse(&bytes).unwrap();
            assert_eq!(back.actions[0].items[0].descriptor, Some(d.clone()), "{id}");
            assert_eq!(back.actions[0].items[0].event.id(), if id.is_empty() { name.unwrap() } else { id });
        }
    }
}

/// Fade writes native defaults and clamps only its documented opacity range.
#[test]
fn fade_defaults_and_opacity_limits_are_explicit() {
    let native = export_item("edit.fade", &json!({}), "S").unwrap().descriptor.unwrap();
    assert_eq!(native, desc("null").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml")));
    for (opacity, expected) in [(-5.0, 0.0), (0.0, 0.0), (100.0, 100.0), (105.0, 100.0)] {
        let bytes = file(AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(opacity)).with("Md  ", enm("BlnM", "Nrml")));
        let loaded = import(&bytes).unwrap();
        assert!(loaded.unsupported.is_empty());
        assert_eq!(loaded.actions[0].steps[0].1["opacity"].as_f64(), Some(expected));
        let saved = export("Group F", &[&loaded.actions[0]], &[], false).unwrap().0;
        let d = atn::parse(&saved).unwrap().actions[0].items[0].descriptor.clone().unwrap();
        assert_eq!(d, desc("null").with("Opct", pct(expected)).with("Md  ", enm("BlnM", "Nrml")));
    }
}

/// Native exports keep their current event; only imported old mask/merge steps select old IDs.
#[test]
fn group_f_legacy_origin_is_allowlisted_and_does_not_replace_native_events() {
    for (id, old, current) in [("layer.layerMask.delete", "RmvL", "Dlt "), ("layer.mergeLayers", "MrgL", "Mrg2")] {
        assert_eq!(export_item(id, &json!({}), "S").unwrap().event.id(), current);
        let params = json!({"_atnEvent":old});
        assert_eq!(export_item(id, &params, "S").unwrap().event.id(), old);
        assert!(export_item(id, &json!({"_atnEvent":"arbitraryEvent"}), "S").is_none());
    }
    assert!(export_item("edit.definePattern", &json!({"_atnEvent":"MrgL"}), "S").is_none());
    assert!(export_item("select.rect", &json!({"x":0,"y":0,"width":1,"height":1,"mode":"replace","_atnBounds":[0,0,1,1]}), "S").is_none());
    assert!(export_item("select.rect", &json!({"x":0,"y":0,"width":2,"height":1,"mode":"add","_atnBounds":[0,0,1,1]}), "S").is_none());
}

/// Missing settings, unknown enums, repeated keys and wrong targets stay kept with their flags.
#[test]
fn group_f_unsupported_settings_are_not_silently_replaced() {
    let mut wrong_ref = rectangle([1.0, 2.0, 3.0, 4.0], false);
    wrong_ref.items[0].1 = target_ref("Lyr ");
    let mut cases = vec![
        (*b"AddT", desc("null")),
        (*b"AddT", wrong_ref),
        (*b"PstI", desc("null").with("AntA", enm("Annt", "Unknown")).with("As  ", D::Class(class("Pxel")))),
        (*b"PstO", desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Other")))),
        (*b"Fade", desc("null").with("Opct", pct(50.0))),
        (*b"Fade", desc("null").with("Opct", px(50.0)).with("Md  ", enm("BlnM", "Nrml"))),
        (*b"Dstr", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "Unknown"))),
        (*b"RmvL", desc("null").with("apply", D::Boolean(true))),
    ];
    for (event, d) in [
        (*b"AddT", rectangle([1.0, 2.0, 3.0, 4.0], true)),
        (*b"PstI", desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        (*b"Fade", desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Nrml"))),
        (*b"Dstr", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
    ] {
        for item in &d.items {
            let mut repeated = d.clone();
            repeated.items.push(item.clone());
            cases.push((event, repeated));
        }
    }
    for key in ["Top ", "Left", "Btom", "Rght"] {
        let mut d = rectangle([1.0, 2.0, 3.0, 4.0], false);
        let D::Descriptor(shape) = &mut d.items[1].1 else { panic!("rectangle fixture must contain a shape") };
        shape.items.push((code(key), px(9.0)));
        cases.push((*b"AddT", d));
    }
    for (event, d) in cases {
        let mut original = atn::parse(&file(AtnEvent::Code(event), d)).unwrap();
        original.actions[0].items[0].enabled = false;
        original.actions[0].items[0].with_dialog = true;
        original.actions[0].items[0].dialog_options = 2;
        let loaded = import(&atn::write(&original).unwrap()).unwrap();
        assert_eq!(loaded.unsupported.len(), 1);
        assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
        let saved = export("Group F", &[&loaded.actions[0]], &[], false).unwrap().0;
        assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0], original.actions[0].items[0]);
    }
    for (id, p) in [
        ("edit.pasteSpecial.pasteInto", json!({"center":[1,2]})),
        ("edit.pasteSpecial.pasteOutside", json!({"target":"mask"})),
        ("layer.linkLayers", json!({"layer":"Other"})),
        ("layer.distribute.leftEdges", json!({"layer":7})),
        ("edit.definePattern", json!({"name":"Frozen name"})),
        ("edit.fade", json!({"mode":"unknown"})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add","feather":2})),
    ] {
        assert!(export_item(id, &p, "S").is_none(), "{id}: {p}");
    }
}

/// Execute the action name assigned by the loader, including collision-safe renaming.
fn play(s: &mut Session, event: AtnEvent, d: Descriptor) -> Value {
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&file(event,d))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap()
}

/// A small image with real pixels; all witnesses operate through the public command boundary.
fn image(depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#4080c0"})).unwrap();
    s
}

/// Compare the actual stored selection, rather than only the translated command params.
#[test]
fn selection_combinations_play_on_an_existing_mask_and_undo() {
    for depth in [8, 16, 32] {
        for (event, mode, ellipse) in [(*b"AddT", "add", false), (*b"AddT", "add", true), (*b"SbtF", "subtract", false), (*b"IntW", "intersect", false)] {
            let mut actual = image(depth);
            let mut direct = image(depth);
            for s in [&mut actual, &mut direct] {
                s.execute("select.rect", json!({"x":1,"y":1,"width":4,"height":4})).unwrap();
            }
            let mask = |s: &Session| s.active().unwrap().doc.selection.as_ref().map(|m| m.read_region(s.active().unwrap().doc.bounds()));
            let before = mask(&actual);
            let result = play(&mut actual, AtnEvent::Code(event), rectangle([3.0, 3.0, 7.0, 7.0], ellipse));
            assert!(result.get("failed").is_none(), "{result}");
            let mut p = json!({"x":3,"y":3,"width":4,"height":4,"mode":mode});
            if ellipse {
                p["ellipse"] = json!(true);
                p["antiAlias"] = json!(true);
            }
            direct.execute("select.rect", p).unwrap();
            assert_eq!(mask(&actual), mask(&direct), "{depth}/{mode}/{ellipse}");
            assert_ne!(mask(&actual), before, "the action changes the existing mask");
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(mask(&actual), before);
        }
    }
}

/// Clipboard-driven paste creates the same layer and mask through imported and direct commands.
#[test]
fn paste_actions_use_the_clipboard_and_selection_and_undo() {
    for (event, id) in [(*b"PstI", "edit.pasteSpecial.pasteInto"), (*b"PstO", "edit.pasteSpecial.pasteOutside")] {
        let setup = || {
            let mut s = image(32);
            s.execute("select.all", json!({})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            s.execute("select.rect", json!({"x":2,"y":2,"width":3,"height":3})).unwrap();
            s
        };
        let mut actual = setup();
        let mut direct = setup();
        let before = actual.active().unwrap().doc.layers.len();
        let d = desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")));
        let result = play(&mut actual, AtnEvent::Code(event), d);
        assert!(result.get("failed").is_none(), "{result}");
        direct.execute(id, json!({})).unwrap();
        let state = |s: &Session| {
            let st = s.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
            (st.doc.layers.len(), layer.surface().unwrap().read_region(st.doc.bounds()), layer.mask.as_ref().unwrap().surface.read_region(st.doc.bounds()))
        };
        assert_eq!(actual.active().unwrap().doc.layers.len(), before + 1);
        assert_eq!(state(&actual), state(&direct));
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(actual.active().unwrap().doc.layers.len(), before);
    }
}

/// Zero-opacity Fade restores the preceding pixels; no previous edit fails without mutation.
#[test]
fn fade_action_consumes_the_previous_edit_and_fails_without_it() {
    for depth in [8, 16, 32] {
        let mut s = image(depth);
        let before = s.execute("document.pixel", json!({"x":3,"y":3})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        assert_ne!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
        let result = play(&mut s, AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(0.0)).with("Md  ", enm("BlnM", "Nrml")));
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let result = play(&mut s, AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Mltp")));
    assert_eq!(result["failed"]["id"], "edit.fade");
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

/// The Link event uses the specified existing toggle, including a second playback and Undo.
#[test]
fn link_action_targets_the_selected_layers() {
    let mut s = image(8);
    s.execute("layer.new.layer", json!({"name":"One"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Two"})).unwrap();
    s.execute("layer.select", json!({"layer":"One"})).unwrap();
    s.execute("layer.select", json!({"layer":"Two","mode":"add"})).unwrap();
    let linked = |s: &Session| s.active().unwrap().doc.layers.iter().filter(|l| l.link_group.is_some()).count();
    for expected in [2, 0] {
        let result = play(&mut s, AtnEvent::name("linkSelectedLayers"), desc("null").with("null", target_ref("Lyr ")));
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(linked(&s), expected);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(linked(&s), 2);
}

/// Distinct sized/placed layers exercise each recorded distribution edge and center.
#[test]
fn distribute_actions_match_the_requested_native_direction_and_undo() {
    for (value, edge) in [
        ("AdLf", "leftEdges"),
        ("AdCH", "horizontalCenters"),
        ("AdRg", "rightEdges"),
        ("AdTp", "topEdges"),
        ("AdCV", "verticalCenters"),
        ("AdBt", "bottomEdges"),
    ] {
        let setup = || {
            let mut s = image(8);
            for (name, x, y, size) in [("One", 0, 0, 1), ("Two", 4, 3, 2), ("Three", 6, 6, 1)] {
                s.execute("layer.new.layer", json!({"name":name})).unwrap();
                s.execute("select.rect", json!({"x":x,"y":y,"width":size,"height":size})).unwrap();
                s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
                s.execute("select.deselect", json!({})).unwrap();
            }
            s.execute("layer.select", json!({"layer":"One"})).unwrap();
            for name in ["Two", "Three"] {
                s.execute("layer.select", json!({"layer":name,"mode":"add"})).unwrap();
            }
            s
        };
        let mut actual = setup();
        let mut direct = setup();
        let bounds = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| (l.name.clone(), l.surface().map(|v| v.content_bounds()))).collect::<Vec<_>>();
        let before = bounds(&actual);
        let result = play(&mut actual, AtnEvent::Code(*b"Dstr"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", value)));
        assert!(result.get("failed").is_none(), "{result}");
        direct.execute(&format!("layer.distribute.{edge}"), json!({})).unwrap();
        assert_eq!(bounds(&actual), bounds(&direct), "{edge}");
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(&actual), before);
    }
}

/// Legacy steps perform real edits and keep their native Undo behavior.
#[test]
fn old_mask_and_merge_actions_execute_their_mapped_commands() {
    let mut s = image(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let has_mask = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.is_some()
    };
    assert!(has_mask(&s));
    let result = play(&mut s, AtnEvent::Code(*b"RmvL"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert!(!has_mask(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(has_mask(&s));
    s.execute("layer.new.layer", json!({"name":"Overlay"})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    s.execute("select.allLayers", json!({})).unwrap();
    let before = s.active().unwrap().doc.layers.len();
    let result = play(&mut s, AtnEvent::Code(*b"MrgL"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
}

/// The empty Define Pattern event creates the selected region without changing canvas pixels.
#[test]
fn define_pattern_action_uses_the_selected_region() {
    let mut s = image(8);
    s.execute("select.rect", json!({"x":1,"y":2,"width":3,"height":2})).unwrap();
    let before = s.execute("document.pixel", json!({"x":3,"y":3})).unwrap();
    let count = s.patterns.items.len();
    let result = play(&mut s, AtnEvent::Code(*b"DfnP"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.patterns.items.len(), count + 1);
    let p = s.patterns.items.last().unwrap();
    assert_eq!((p.width, p.height), (3, 2));
    assert!(!p.surface.format().alpha);
    assert_eq!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
}
