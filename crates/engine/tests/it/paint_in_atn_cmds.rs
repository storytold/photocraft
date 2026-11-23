//! Tests that run paint commands, moved from the engine-core `atn_cmds` tests: 18 test(s).

mod fade_step_tests {
    use crate::atn_fixtures::*;
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString};
    use photocraft_engine::Session;
    use photocraft_engine::atn_cmds::*;
    use serde_json::{Value, json};

    /// One action with a single recorded Fade step.
    fn fade_file(descriptor: Descriptor) -> Vec<u8> {
        atn::write(&AtnFile {
            name: UnicodeString::new_nul("Fade"),
            actions: vec![AtnAction {
                name: UnicodeString::new_nul("Step"),
                items: vec![AtnItem { event: AtnEvent::Code(*b"Fade"), dictionary: b"Fade".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap()
    }

    /// The command, opacity and mode a recorded Fade step imports as.
    fn imported(descriptor: Descriptor) -> (String, f64, String) {
        let loaded = import(&fade_file(descriptor)).unwrap();
        assert!(loaded.unsupported.is_empty());
        let (id, params) = {
            let step = &loaded.actions[0].steps[0];
            (&step.command, &step.params)
        };
        (id.clone(), params["opacity"].as_f64().unwrap(), params["mode"].as_str().unwrap().to_owned())
    }

    /// An 8 x 8 image, inverted so there is an edit to fade.
    fn inverted(depth: u8) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#4080c0"})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        s
    }

    /// The pixel a recorded Fade step leaves after the Invert it fades.
    fn played(depth: u8, descriptor: Descriptor) -> Value {
        let mut s = inverted(depth);
        let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&fade_file(descriptor))})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]));
        let result = s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        s.execute("document.pixel", json!({"x":3,"y":3})).unwrap()
    }

    /// The pixel `edit.fade` leaves when run directly with these params.
    fn faded(depth: u8, params: Value) -> Value {
        let mut s = inverted(depth);
        s.execute("edit.fade", params).unwrap();
        s.execute("document.pixel", json!({"x":3,"y":3})).unwrap()
    }

    /// A step with no keys plays at 100 % Normal; with one key, the other takes its default.
    #[test]
    fn fade_step_missing_keys_take_fixed_defaults() {
        assert_eq!(imported(desc("null")), ("edit.fade".into(), 100.0, "Normal".into()));
        assert_eq!(imported(desc("null").with("Opct", pct(40.0))), ("edit.fade".into(), 40.0, "Normal".into()));
        assert_eq!(imported(desc("null").with("Md  ", enm("BlnM", "Mltp"))), ("edit.fade".into(), 100.0, "Multiply".into()));
        for depth in [8, 16, 32] {
            assert_eq!(played(depth, desc("null")), faded(depth, json!({"opacity":100,"mode":"normal"})));
            assert_eq!(played(depth, desc("null").with("Md  ", enm("BlnM", "Mltp"))), faded(depth, json!({"opacity":100,"mode":"multiply"})));
        }
    }

    /// Opacity less than 100 out of range wraps modulo 100 and fractions round; whole values in range
    /// are unchanged, and unmeasured values further out keep the clamp.
    #[test]
    fn fade_step_opacity_wraps_and_rounds() {
        for (recorded, expected) in [
            (150.0, 50.0),
            (-5.0, 95.0),
            (50.5, 51.0),
            (0.0, 0.0),
            (37.0, 37.0),
            (100.0, 100.0),
            (-0.4, 0.0),
            (199.0, 99.0),
            (200.0, 100.0),
            (1e9, 100.0),
            (-100.0, 0.0),
            (-105.0, 0.0),
        ] {
            let (id, opacity, _) = imported(desc("null").with("Opct", pct(recorded)).with("Md  ", enm("BlnM", "Nrml")));
            assert_eq!(id, "edit.fade");
            assert_eq!(opacity, expected, "{recorded}");
            assert!(opacity.is_sign_positive(), "{recorded}");
        }
        for depth in [8, 16, 32] {
            let recorded = desc("null").with("Opct", pct(150.0)).with("Md  ", enm("BlnM", "Nrml"));
            assert_eq!(played(depth, recorded), faded(depth, json!({"opacity":50,"mode":"normal"})));
        }
    }
}

mod render_tests {
    use crate::atn_fixtures::*;
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    use photocraft_engine::Session;
    use serde_json::json;

    /// A real action file with one step, used by both descriptor and playback checks.
    fn file(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
        atn::write(&AtnFile {
            name: UnicodeString::new_nul("Render set"),
            actions: vec![AtnAction {
                name: UnicodeString::new_nul("Render"),
                items: vec![AtnItem { event, dictionary: b"Render".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap()
    }

    /// Measured descriptor order and types, independent of the converter's field construction.
    fn fibers(variance: i32, strength: i32, seed: i32) -> Descriptor {
        desc("null").with("Vrnc", D::Integer(variance)).with("Strg", D::Integer(strength)).with("RndS", D::Integer(seed))
    }

    /// The measured center is a Point object with two doubles, not pixel or percent units.
    fn flare(brightness: i32, center: [f64; 2], lens: &str) -> Descriptor {
        desc("null")
            .with("Brgh", D::Integer(brightness))
            .with("FlrC", D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(center[0])).with("Vrtc", D::Double(center[1]))))
            .with("Lns ", enm("Lns ", lens))
    }

    /// Imported playback uses current tool colors and the same filter path as direct commands.
    #[test]
    fn render_actions_match_direct_filters_and_undo() {
        let cases = [
            (*b"Clds", desc("null"), "filter.render.clouds", json!({})),
            (*b"DfrC", desc("null"), "filter.render.differenceClouds", json!({})),
            (*b"Fbrs", fibers(23, 6, -7), "filter.render.fibers", json!({"variance": 23, "strength": 6, "seed": -7})),
            (
                *b"LnsF",
                flare(240, [5.0, -5.0], "Nkn1"),
                "filter.render.lensFlare",
                json!({"brightness": 240, "centerX": 5.0, "centerY": -5.0, "lens": "prime105"}),
            ),
            // Just right of the canvas: the dialog's 0..1 range does not clamp a recorded centre.
            (*b"LnsF", flare(100, [1.4, 0.25], "Zm"), "filter.render.lensFlare", json!({"brightness": 100, "centerX": 1.4, "centerY": 0.25, "lens": "zoom"})),
        ];
        for single_state in [true, false] {
            for mode in ["rgb", "gray"] {
                for depth in [8, 16, 32] {
                    for (event, descriptor, id, params) in &cases {
                        let setup = || {
                            let mut s = Session::new();
                            s.execute("file.new", json!({"width": 8, "height": 6, "mode": mode, "depth": depth, "background": "transparent"})).unwrap();
                            s.execute("edit.fill", json!({"contents": "color", "color": "#304050"})).unwrap();
                            s.execute("tools.setColors", json!({"foreground": "#fe1020", "background": "#1050e0"})).unwrap();
                            s
                        };
                        let mut action = setup();
                        let mut direct = setup();
                        action.execute("actions.setHistory", json!({"singleState": single_state})).unwrap();
                        let before_doc = action.active().unwrap().doc.clone();
                        let history = action.active().unwrap().history.past_len();
                        let before = action.execute("document.pixel", json!({"x": 3, "y": 2})).unwrap();
                        action
                            .execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&file(AtnEvent::Code(*event), descriptor.clone()))}))
                            .unwrap();
                        let played = action.execute("actions.play", json!({"action": "Render"})).unwrap();
                        assert!(played.get("failed").is_none(), "{played}");
                        direct.execute(id, params.clone()).unwrap();
                        let surface = |s: &Session| {
                            let st = s.active().unwrap();
                            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
                        };
                        assert_eq!(surface(&action), surface(&direct), "{mode}/{depth}/{id}");
                        let after_doc = action.active().unwrap().doc.clone();
                        let changed = after_doc.as_ref() != before_doc.as_ref();
                        let added = usize::from(!single_state || changed);
                        assert_eq!(action.active().unwrap().history.past_len(), history + added, "{mode}/{depth}/{id}, singleState={single_state}");
                        // An unchanged grouped action preserves the setup row instead of adding one.
                        if added > 0 {
                            action.execute("edit.undo", json!({})).unwrap();
                        } else {
                            assert!(std::sync::Arc::ptr_eq(&action.active().unwrap().doc, &before_doc));
                        }
                        assert_eq!(action.active().unwrap().doc.as_ref(), before_doc.as_ref(), "{mode}/{depth}/{id} undo");
                        assert_eq!(action.execute("document.pixel", json!({"x": 3, "y": 2})).unwrap(), before);
                        if added > 0 {
                            action.execute("edit.redo", json!({})).unwrap();
                            assert_eq!(action.active().unwrap().doc.as_ref(), after_doc.as_ref(), "{mode}/{depth}/{id} redo");
                        }
                    }
                }
            }
        }
    }
}

mod selection_tests {
    use crate::atn_fixtures::*;
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, ReferenceItem, UnicodeString, Value as D};
    use photocraft_engine::Session;
    use serde_json::{Value, json};

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
            let bounds =
                |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| (l.name.clone(), l.surface().map(|v| v.content_bounds()))).collect::<Vec<_>>();
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
}

mod tests {
    use crate::atn_fixtures::*;
    use craft_psd::atn::{self};
    use photocraft_engine::Session;
    use photocraft_engine::atn_cmds::*;
    use serde_json::Value;

    use photocraft_engine::actions_cmds::ActionStep;

    use photocraft_engine::actions_cmds::ActionsExt;

    use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};

    use craft_psd::descriptor::{Descriptor, Id, ReferenceItem, UnicodeString, Value as D};

    use serde_json::json;

    fn item(event: &str, label: &str, d: Option<Descriptor>) -> AtnItem {
        let event = match event.as_bytes() {
            [a, b, c, dd] => AtnEvent::Code([*a, *b, *c, *dd]),
            _ => AtnEvent::name(event),
        };
        AtnItem { event, dictionary: label.as_bytes().to_vec(), descriptor: d, ..Default::default() }
    }

    fn b64(bytes: &[u8]) -> String {
        photocraft_paint::tile::b64_encode(bytes)
    }

    fn channel_ord(value: &str) -> D {
        D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: code(value) }])
    }

    fn fsel(t: D) -> Descriptor {
        desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with("T   ", t)
    }

    fn set_of(name: &str, actions: Vec<(&str, Vec<AtnItem>)>) -> String {
        let file = AtnFile {
            name: UnicodeString::new_nul(name),
            expanded: false,
            actions: actions.into_iter().map(|(n, items)| AtnAction { name: UnicodeString::new_nul(n), items, ..Default::default() }).collect(),
            warnings: vec![],
            ..Default::default()
        };
        b64(&atn::write(&file).unwrap())
    }

    /// The step shapes of the TrueGrit Texture Supply sets (Distress Press, Debaser, Transparency,
    /// Halftone Zine Machine), rebuilt here as Photoshop records them, import and play: the
    /// distressing filters land on the targeted layer mask, Remove White leaves the white
    /// transparent, nudges in points follow the resolution, and Step Backward undoes a step.
    #[test]
    fn distressing_and_remove_white_shapes_import_and_play() {
        let ord = |cls: &str, ty: &str, v: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new(ty), value: code(v) }]);
        let black = D::Descriptor(desc("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0)));
        let fill = desc("null")
            .with("null", class_ref("contentLayer"))
            .with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(desc("solidColorLayer").with("Clr ", black)))));
        let refine = desc("null")
            .with("refineEdgeBorderRadius", px(0.0))
            .with("refineEdgeBorderContrast", pct(100.0))
            .with("refineEdgeSmooth", long(2.0))
            .with("refineEdgeFeatherRadius", px(0.0))
            .with("refineEdgeChoke", pct(0.0))
            .with("refineEdgeAutoRadius", D::Boolean(false))
            .with("refineEdgeDecontaminate", D::Boolean(false))
            .with("refineEdgeOutput", enm("refineEdgeOutput", "refineEdgeOutputUserMask"));
        let distress = vec![
            item("setd", "Set", Some(fsel(channel_ord("Trsp")))),
            item("Hd  ", "Hide", Some(desc("null").with("null", D::List(vec![target_ref("Lyr ")])))),
            item("Mk  ", "Make", Some(fill)),
            item("slct", "Select", Some(desc("null").with("null", channel_ord("Msk")).with("MkVs", D::Boolean(false)))),
            item("refineSelectionEdge", "Select and Mask", Some(refine)),
            item("Rple", "Ripple", Some(desc("null").with("Amnt", long(50.0)).with("RplS", enm("RplS", "Sml")))),
            item("Crst", "Crystallize", Some(desc("null").with("ClSz", long(3.0)))),
            item("GEfc", "Filter Gallery", Some(desc("null").with("GEfk", enm("GEft", "Spt")).with("SprR", long(7.0)).with("Smth", long(15.0)))),
            item("slct", "Select", Some(desc("null").with("null", ord("Chnl", "Ordn", "Trgt")).with("MkVs", D::Boolean(false)))),
            item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
        ];
        let bckg = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") }]);
        let remove_white = vec![
            item("CnvM", "Convert Mode", Some(desc("null").with("T   ", D::Class(class("Grys"))))),
            item(
                "setd",
                "Set",
                Some(desc("null").with("null", bckg).with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml"))))),
            ),
            item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
            item("Dlt ", "Delete", None),
            item("setd", "Set", Some(fsel(enm("Ordn", "None")))),
        ];
        let pt = |v: f64| D::UnitFloat { unit: *b"#Rlt", value: v };
        let nudge = vec![item(
            "move",
            "Move",
            Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(desc("Ofst").with("Hrzn", pt(0.36)).with("Vrtc", pt(-0.36))))),
        )];
        let undo = vec![item("Invs", "Inverse", None), item("slct", "Select", Some(desc("null").with("null", ord("HstS", "Ordn", "Prvs"))))];
        let data = set_of("TrueGrit", vec![("Distress", distress), ("Remove White", remove_white), ("Nudge", nudge), ("Undo", undo)]);

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 80, "resolution": 200})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 80, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let r = s.execute("actions.load", json!({"data": data})).unwrap();
        // Only the re-selection of the targeted channel stays a Photoshop step, switched off.
        let unsupported: Vec<&str> = r["unsupported"].as_array().unwrap().iter().map(|u| u["action"].as_str().unwrap()).collect();
        assert_eq!(unsupported, ["Distress"], "{r}");

        let r = s.execute("actions.play", json!({"action": "Distress"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        let st = s.active().unwrap();
        let fill = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let mask = &fill.mask.as_ref().expect("the fill layer is masked by the ink").surface;
        // The rectangle's straight top edge is roughened: pixels along it differ.
        let edge: Vec<f32> = (20..100).map(|x| mask.sample_channel(x, 20, 0)).collect();
        assert!(edge.iter().any(|v| *v < 0.5) && edge.iter().any(|v| *v > 0.5), "{edge:?}");
        assert!(mask.sample_channel(60, 40, 0) > 0.9 && mask.sample_channel(5, 5, 0) < 0.1);
        assert!(st.doc.selection.is_some(), "the targeted mask loads as the selection");
        assert_eq!(st.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::LayerMask);

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#202020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("actions.load", json!({"data": data})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.mode, photocraft_color::ColorMode::Grayscale);
        let l = d.layers.last().unwrap();
        assert!(!d.is_background(l.id), "Layer From Background");
        let surf = l.surface().unwrap();
        assert!(surf.sample_channel(2, 2, 1) < 0.01, "white is cleared");
        // #202020 is about 22 in the default (Dot Gain 20 %-compatible) working gray.
        let ink = surf.sample_channel(20, 15, 1);
        assert!((ink - (1.0 - 22.0 / 255.0)).abs() < 0.02, "the ink keeps its darkness as opacity: {ink}");
        // An image that opened as a normal layer has no Background: the step leaves it as it is.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"contents": "white"})).unwrap();
        s.execute("actions.load", json!({"data": data})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        let surf = s.active().unwrap().doc.layers.last().unwrap().surface().unwrap().clone();
        assert!(surf.sample_channel(2, 2, 1) < 0.01, "white is cleared without a Background");

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "resolution": 200})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        s.execute("actions.load", json!({"data": data})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        assert!(s.execute("actions.play", json!({"action": "Nudge"})).unwrap().get("failed").is_none());
        let st = s.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert_eq!((b.x0, b.y0), (11, 9), "0.36 pt is 1 px at 200 ppi");
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        let before = s.active().unwrap().doc.selection.clone();
        assert!(s.execute("actions.play", json!({"action": "Undo"})).unwrap().get("failed").is_none());
        assert_eq!(s.active().unwrap().doc.selection, before, "Step Backward undid the Inverse");
    }

    /// Remove White on art in its own layer above a white Background: the Background becomes a
    /// normal layer, but the art layer stays targeted, so the white is cleared from the art.
    #[test]
    fn remove_white_clears_the_targeted_layer_not_the_converted_background() {
        let ord = |cls: &str, ty: &str, v: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new(ty), value: code(v) }]);
        let bckg = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") }]);
        let steps = vec![
            item("CnvM", "Convert Mode", Some(desc("null").with("T   ", D::Class(class("Grys"))))),
            item(
                "setd",
                "Set",
                Some(desc("null").with("null", bckg).with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml"))))),
            ),
            item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
            item("Dlt ", "Delete", None),
            item("setd", "Set", Some(fsel(enm("Ordn", "None")))),
        ];
        let data = set_of("TrueGrit", vec![("Remove White", steps)]);
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        let art = s.execute("layer.new.layer", json!({"name": "Art"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("edit.fill", json!({"contents": "white"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("actions.load", json!({"data": data})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        let st = s.active().unwrap();
        assert_eq!(st.active_layer.map(|l| l.0), Some(art), "the art layer stays targeted");
        let art = st.doc.layer(photocraft_doc::LayerId(art)).unwrap().surface().unwrap();
        assert!(art.sample_channel(2, 2, 1) < 0.01, "the art's white is cleared");
        assert!(art.sample_channel(20, 15, 1) > 0.99, "the black stays");
        let bottom = st.doc.layers.first().unwrap();
        assert!(!st.doc.is_background(bottom.id), "the Background became a normal layer");
        assert!(bottom.surface().unwrap().sample_channel(2, 2, 1) > 0.99, "and kept its pixels");
    }

    /// #6745: a recorded Camera Raw Filter step (`Adobe Camera Raw Filter`, the layout of 62 real
    /// recordings) imports as `filter.cameraRaw` with its sliders, colour mixer, grading wheels,
    /// vignette style and point curves, and plays. One that uses the Calibration panel (or noise
    /// settings the command lacks) stays a Photoshop step rather than replaying a different look.
    #[test]
    fn camera_raw_filter_steps_import_as_filter_camera_raw() {
        let int = |v: i32| D::Integer(v);
        // Every step carries the version keys a recording has (a verified process version).
        let cr = || desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVN", int(5)).with("PrVe", int(184_549_376));
        let mut d = cr()
            .with("WBal", enm("WBal", "Cst "))
            .with("Temp", int(12))
            .with("Tint", int(-4))
            .with("Ex12", D::Double(0.35))
            .with("Cr12", int(10))
            .with("Hi12", int(-30))
            .with("Sh12", int(25))
            .with("Cl12", int(8))
            .with("Vibr", int(15))
            .with("PC_1", int(20))
            .with("PC_2", int(50))
            .with("PC_3", int(80))
            .with("STSH", int(210))
            .with("STSS", int(12))
            .with("CgSL", int(0))
            .with("CgBl", int(60))
            .with("STB ", int(-10))
            .with("Shrp", int(40))
            .with("ShpR", D::Double(1.2))
            .with("LNR ", int(20))
            .with("LNRC", int(0))
            .with("CNR ", int(25))
            .with("CNRS", int(50))
            .with("PCVA", int(-20))
            .with("PCVS", int(2))
            .with("Crv ", D::List([0, 0, 64, 50, 255, 255].map(int).to_vec()))
            .with("CrvR", D::List([0, 0, 255, 255].map(int).to_vec()));
        for (k, v) in ["R", "O", "Y", "G", "A", "B", "P", "M"].iter().zip(1..) {
            d = d.with(&format!("HA_{k}"), int(v));
        }
        d = d.with("SA_O", int(-15));
        let calibrated = cr().with("Ex12", D::Double(0.2)).with("RHue", int(8));
        let smoothed = cr().with("CNR ", int(25)).with("CNRS", int(70));
        let auto_wb = cr().with("WBal", enm("WBal", "Atmt")).with("Temp", int(5));
        let partial_splits = cr().with("PC_1", int(20));
        let odd_curve = cr().with("Crv ", int(3));
        // Review of #8714: values that would replay differently, or fail to play, stay Photoshop steps.
        let reversed_splits = cr().with("PC_1", int(80)).with("PC_2", int(50)).with("PC_3", int(20));
        let split_past_100 = cr().with("PC_1", int(20)).with("PC_2", int(50)).with("PC_3", int(120));
        let one_point_curve = cr().with("Crv ", D::List([0, 0].map(int).to_vec()));
        let unordered_curve = cr().with("CrvG", D::List([0, 0, 128, 90, 64, 50, 255, 255].map(int).to_vec()));
        let text_contrast = cr().with("Cr12", D::Text(UnicodeString::new_nul("ten")));
        let text_hue = cr().with("HA_R", D::Text(UnicodeString::new_nul("red")));
        let text_grade = cr().with("STSH", D::Text(UnicodeString::new_nul("blue")));
        let exposure_past_5 = cr().with("Ex12", D::Double(6.0));
        let negative_sharpening = cr().with("Shrp", int(-10));
        // CodeRabbit on batch 11: a process version the PSD mapping hasn't verified renders differently.
        let unknown_version = desc("null")
            .with("CrVe", D::Text(UnicodeString::new_nul("16.1.1")))
            .with("PrVN", int(4))
            .with("PrVe", int(184_549_376))
            .with("Ex12", D::Double(0.2));
        let no_version_number =
            desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
        let mismatched_pair = desc("null")
            .with("CrVe", D::Text(UnicodeString::new_nul("16.1.1")))
            .with("PrVN", int(6))
            .with("PrVe", int(184_549_376))
            .with("Ex12", D::Double(0.2));
        let no_camera_raw_version = desc("null").with("PrVN", int(5)).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
        let items = vec![
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(d)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(calibrated)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(smoothed)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(auto_wb)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(partial_splits)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(odd_curve)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(reversed_splits)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(split_past_100)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(one_point_curve)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(unordered_curve)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_contrast)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_hue)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_grade)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(exposure_past_5)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(negative_sharpening)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(unknown_version)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(no_version_number)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(mismatched_pair)),
            item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(no_camera_raw_version)),
        ];
        let file = AtnFile {
            name: UnicodeString::new_nul("Looks"),
            expanded: false,
            actions: vec![AtnAction { name: UnicodeString::new_nul("Warm"), items, ..Default::default() }],
            warnings: vec![],
            ..Default::default()
        };
        let imp = import(&atn::write(&file).unwrap()).unwrap();
        let steps = &imp.actions[0].steps;
        let ActionStep { command: id, params: p } = &steps[0];
        assert_eq!(id, "filter.cameraRaw");
        // The recorded descriptor rides along, so Save Actions can write the step back.
        let mut p = p.clone();
        assert!(p.as_object_mut().unwrap().remove("__atnDescriptor").is_some(), "{p}");
        assert_eq!(
            p,
            json!({
                "temperature": 12, "tint": -4, "exposure": 0.35, "contrast": 10, "highlights": -30, "shadows": 25,
                "clarity": 8, "vibrance": 15, "curveSplits": [20, 50, 80],
                "hslHue": [1, 2, 3, 4, 5, 6, 7, 8], "hslSat": [0, -15, 0, 0, 0, 0, 0, 0],
                "gradeShadows": {"hue": 210, "sat": 12, "lum": 0}, "gradeBlending": 60, "gradeBalance": -10,
                "sharpenAmount": 40, "sharpenRadius": 1.2, "noiseLuminance": 20, "noiseColor": 25,
                "vignetteAmount": -20, "vignetteStyle": "colorPriority", "pointCurve": [[0, 0], [64, 50], [255, 255]]
            })
        );
        for (k, ActionStep { command: id, .. }) in steps.iter().enumerate().skip(1) {
            assert_eq!(id, PHOTOSHOP_STEP, "step {k}");
        }
        assert_eq!(imp.unsupported.iter().map(|u| u.step).collect::<Vec<_>>(), (1..=18).collect::<Vec<_>>());
        // The imported step plays.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s.execute(id, p.clone()).unwrap();
    }

    /// Review of #8714: a Camera Raw Filter step that was loaded saves back as the recorded step,
    /// byte for byte, as it did while it was a kept Photoshop step; once edited (or when PhotoCraft
    /// recorded it) Save Actions refuses it as before. The loaded step plays.
    #[test]
    fn loaded_camera_raw_filter_steps_save_back_unchanged() {
        let recorded = desc("null")
            .with("CrVe", D::Text(UnicodeString::new_nul("16.1.1")))
            .with("PrVN", D::Integer(5))
            .with("PrVe", D::Integer(184_549_376))
            .with("WBal", enm("WBal", "Cst "))
            .with("Ex12", D::Double(0.35))
            .with("Cr12", D::Integer(10))
            .with("Crv ", D::List([0, 0, 64, 50, 255, 255].map(D::Integer).to_vec()));
        let file = one_action(vec![item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(recorded.clone()))]);
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
        assert_eq!(s.actions().list.actions()[0].steps[0].command, "filter.cameraRaw");
        let saved_step = |r: &Value| {
            let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
            saved.actions[0].items.first().map(|i| (i.event.clone(), i.dictionary.clone(), i.descriptor.clone()))
        };
        // Strict save, and the Actions panel's save (which skips what can't be written). The step's
        // dictionary is stored action data, so it keeps Photoshop's wording and not the menu label
        // ("RAW Develop Filter"): a loaded step reads the same in Photoshop's Actions panel.
        for skip in [false, true] {
            let r = s.execute("actions.save", json!({"action": "A", "skipUnsupported": skip})).unwrap();
            assert_eq!(r["skipped"], json!([]), "{r}");
            let expected = (AtnEvent::name("Adobe Camera Raw Filter"), b"Camera Raw Filter".to_vec(), Some(recorded.clone()));
            assert_eq!(saved_step(&r), Some(expected));
        }
        // It plays.
        s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        assert!(played.get("failed").is_none(), "{played}");
        // Edited, it no longer matches what was recorded.
        let recorded = s.actions().list.actions()[0].steps[0].clone();
        let mut edited = recorded.clone();
        edited.params["exposure"] = json!(1.5);
        s.actions_mut().list.replace_step(0, 0, edited).unwrap();
        let e = s.execute("actions.save", json!({"action": "A"})).unwrap_err().to_string();
        assert!(e.contains("filter.cameraRaw"), "{e}");
        // Recorded by PhotoCraft: no descriptor to write back.
        let mut plain = recorded;
        plain.params.as_object_mut().unwrap().remove("__atnDescriptor");
        s.actions_mut().list.replace_step(0, 0, plain).unwrap();
        let r = s.execute("actions.save", json!({"action": "A", "skipUnsupported": true})).unwrap();
        assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "{r}");
    }

    fn one_action(items: Vec<AtnItem>) -> AtnFile {
        AtnFile {
            name: UnicodeString::new_nul("Saves"),
            expanded: false,
            actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
            warnings: vec![],
            ..Default::default()
        }
    }

    /// A Pattern Fill step whose `Ptrn` holds `id` and `name`, as the reference application records it.
    fn pattern_fill_step(id: &str, name: &str) -> AtnItem {
        let t = desc("patternLayer").with("Ptrn", D::Descriptor(desc("Ptrn").with("Idnt", text(id)).with("Nm  ", text(name))));
        item(
            "Mk  ",
            "Make",
            Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))),
        )
    }

    /// Defines a 4×4 pattern of `color` named `name` in `s`'s library from its active document.
    fn define_pattern(s: &mut Session, color: &str, name: &str) -> String {
        s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        s.execute("edit.definePattern", json!({"name": name})).unwrap()["pattern"].as_str().unwrap().to_string()
    }

    fn pixel(s: &mut Session) -> Vec<f64> {
        let v = s.execute("document.pixel", json!({"x": 1, "y": 1})).unwrap();
        v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect()
    }

    /// A Pattern Fill step whose `Ptrn` names both an installed id and a display name shared by
    /// another pattern plays the pattern with that id; a foreign id falls back to the name.
    #[test]
    fn pattern_fill_prefers_an_installed_id_over_a_colliding_name() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        let mut define = |color: &str| {
            s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
            s.execute("edit.definePattern", json!({"name": "Twin"})).unwrap()["pattern"].as_str().unwrap().to_string()
        };
        let red = define("#ff0000");
        let blue = define("#0000ff");
        assert_ne!(red, blue);
        s.patterns.items.retain(|p| p.id == red || p.id == blue);
        s.patterns.items.sort_by_key(|p| p.id != blue);
        let fill_step = |id: &str| {
            let t = desc("patternLayer").with("Ptrn", D::Descriptor(desc("Ptrn").with("Idnt", text(id)).with("Nm  ", text("Twin"))));
            item(
                "Mk  ",
                "Make",
                Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))),
            )
        };
        let data = set_of("Pat", vec![("Installed", vec![fill_step(&red)]), ("Foreign", vec![fill_step("not-installed-id")])]);
        let r = s.execute("actions.load", json!({"data": data})).unwrap();
        assert!(r["unsupported"].as_array().is_none_or(|u| u.is_empty()), "{r}");
        for (action, want) in [("Installed", [1.0, 0.0, 0.0, 1.0]), ("Foreign", [0.0, 0.0, 1.0, 1.0])] {
            s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
            let r = s.execute("actions.play", json!({"action": action})).unwrap();
            assert!(r.get("failed").is_none(), "{r}");
            let v = s.execute("document.pixel", json!({"x": 1, "y": 1})).unwrap();
            let got: Vec<f64> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect();
            assert_eq!(got, want, "{action}");
        }
    }

    /// The recorded id is looked up by id alone, in the document and then the library, before any
    /// name: a document pattern *named* like the id does not beat the library pattern with that id,
    /// and a foreign id that happens to be another pattern's name falls back to the recorded name.
    /// An id and a name of the same pattern play it.
    #[test]
    fn pattern_fill_ids_beat_names_across_document_and_library() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        let red = define_pattern(&mut s, "#ff0000", "Red");
        define_pattern(&mut s, "#0000ff", "Blue");
        let green = define_pattern(&mut s, "#00ff00", &red);
        let data = set_of(
            "Pat",
            vec![
                ("IdWins", vec![pattern_fill_step(&red, "Blue")]),
                ("NameFallback", vec![pattern_fill_step("Red", "Blue")]),
                ("Same", vec![pattern_fill_step(&red, "Red")]),
            ],
        );
        s.execute("actions.load", json!({"data": data})).unwrap();
        // Only the playback document holds the green pattern named like the red one's id.
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": green})).unwrap();
        s.execute("pattern.delete", json!({"pattern": green})).unwrap();
        assert!(s.active().unwrap().doc.patterns.iter().any(|p| p.id == green && p.name == red));
        assert_eq!(pixel(&mut s), [0.0, 1.0, 0.0, 1.0]);
        for (action, want) in [("IdWins", [1.0, 0.0, 0.0, 1.0]), ("NameFallback", [0.0, 0.0, 1.0, 1.0]), ("Same", [1.0, 0.0, 0.0, 1.0])] {
            let r = s.execute("actions.play", json!({"action": action})).unwrap();
            assert!(r.get("failed").is_none(), "{action}: {r}");
            assert_eq!(pixel(&mut s), want, "{action}");
        }
    }

    /// CodeRabbit on batch 11: when the recorded id is installed nowhere, the recorded name is looked
    /// up as a name only. A pattern whose id equals that name text doesn't beat the one named so.
    #[test]
    fn pattern_fill_name_fallback_matches_names_not_ids() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        let red = define_pattern(&mut s, "#ff0000", "Red");
        // Green is named like the red pattern's id.
        define_pattern(&mut s, "#00ff00", &red);
        let data = set_of("Pat", vec![("NameIsAnId", vec![pattern_fill_step("5e1c0000-aaaa-4bbb-8ccc-000000000000", &red)])]);
        s.execute("actions.load", json!({"data": data})).unwrap();
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        let r = s.execute("actions.play", json!({"action": "NameIsAnId"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert_eq!(pixel(&mut s), [0.0, 1.0, 0.0, 1.0]);
    }

    /// A recorded pattern installed neither by id nor by name fails with an error that names the
    /// pattern, from the imported step and from the Pattern Fill dialog's command alike.
    #[test]
    fn pattern_fill_missing_everywhere_names_the_pattern() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        define_pattern(&mut s, "#ff0000", "Red");
        let data = set_of("Pat", vec![("Missing", vec![pattern_fill_step("5e1c0000-aaaa-4bbb-8ccc-000000000000", "My Wood")])]);
        s.execute("actions.load", json!({"data": data})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Missing"})).unwrap();
        let err = r["failed"]["error"].as_str().unwrap_or_default();
        assert!(err.contains("\"My Wood\""), "{r}");
        s.execute("layer.newFillLayer.pattern", json!({"pattern": "Red"})).unwrap();
        let e = s.execute("layer.fill.pattern.set", json!({"pattern": "5e1c0000-aaaa-4bbb-8ccc-000000000000", "patternName": "My Wood"})).unwrap_err();
        assert!(e.to_string().contains("\"My Wood\""), "{e}");
    }
}
