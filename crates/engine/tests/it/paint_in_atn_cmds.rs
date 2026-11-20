//! Tests that run painting commands, moved from the engine-core `atn_cmds` tests: 27 test(s).

mod fade_step_tests {
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
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    use photocraft_engine::Session;
    use photocraft_engine::atn_cmds::*;
    use serde_json::json;

    use photocraft_engine::actions_cmds::ActionsExt;

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
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Class, Descriptor, ReferenceItem, UnicodeString, Value as D};
    use photocraft_engine::Session;
    use photocraft_engine::atn_cmds::*;
    use serde_json::{Value, json};

    use photocraft_engine::actions_cmds::ActionsExt;

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

mod stylize_tests {
    use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    use photocraft_engine::Session;
    use photocraft_engine::atn_cmds::*;
    use serde_json::{Value, json};

    use photocraft_engine::actions_cmds::ActionStep;

    use photocraft_engine::actions_cmds::ActionsExt;

    struct Fixture {
        code: &'static str,
        name: &'static str,
        id: &'static str,
        default: Descriptor,
        defaults: Value,
        changed: Descriptor,
        changes: Value,
    }

    /// Independent descriptor fixtures spell out measured key order, types, values and defaults.
    fn fixtures() -> Vec<Fixture> {
        vec![
            Fixture {
                code: "Extr",
                name: "extrude",
                id: "filter.stylize.extrude",
                default: desc("null")
                    .with("ExtS", D::Integer(30))
                    .with("ExtD", D::Integer(30))
                    .with("ExtF", D::Boolean(false))
                    .with("ExtM", D::Boolean(false))
                    .with("ExtT", enm("ExtT", "Blks"))
                    .with("ExtR", enm("ExtR", "Rndm")),
                defaults: json!({"size":30,"depth":30,"solidFrontFaces":false,"maskIncompleteBlocks":false,"type":"blocks","depthMode":"random"}),
                changed: desc("null")
                    .with("ExtS", D::Integer(42))
                    .with("ExtD", D::Integer(91))
                    .with("ExtF", D::Boolean(true))
                    .with("ExtM", D::Boolean(true))
                    .with("ExtT", enm("ExtT", "Pyrm"))
                    .with("ExtR", enm("ExtR", "LvlB")),
                changes: json!({"size":42,"depth":91,"solidFrontFaces":true,"maskIncompleteBlocks":true,"type":"pyramids","depthMode":"levelBased"}),
            },
            Fixture {
                code: "TrcC",
                name: "traceContour",
                id: "filter.stylize.traceContour",
                default: desc("null").with("Lvl ", D::Integer(128)).with("Edg ", enm("CntE", "Upr ")),
                defaults: json!({"level":128,"edge":"upper"}),
                changed: desc("null").with("Lvl ", D::Integer(43)).with("Edg ", enm("CntE", "Lwr ")),
                changes: json!({"level":43,"edge":"lower"}),
            },
            Fixture {
                code: "Wnd ",
                name: "wind",
                id: "filter.stylize.wind",
                default: desc("null").with("WndM", enm("WndM", "Wnd ")).with("Drct", enm("Drct", "Left")),
                defaults: json!({"method":"wind","direction":"fromRight"}),
                changed: desc("null").with("WndM", enm("WndM", "Stgr")).with("Drct", enm("Drct", "Rght")),
                changes: json!({"method":"stagger","direction":"fromLeft"}),
            },
            Fixture {
                code: "Tls ",
                name: "tiles",
                id: "filter.stylize.tiles",
                default: desc("null").with("TlNm", D::Integer(10)).with("TlOf", D::Integer(10)).with("FlCl", enm("FlCl", "FlBc")),
                defaults: json!({"count":10,"maxOffset":10,"fill":"background"}),
                changed: desc("null").with("TlNm", D::Integer(17)).with("TlOf", D::Integer(23)).with("FlCl", enm("FlCl", "FlFr")),
                changes: json!({"count":17,"maxOffset":23,"fill":"foreground"}),
            },
            Fixture {
                code: "ClrH",
                name: "colorHalftone",
                id: "filter.pixelate.colorHalftone",
                default: desc("null")
                    .with("Rds ", D::Integer(8))
                    .with("Ang1", D::Integer(108))
                    .with("Ang2", D::Integer(162))
                    .with("Ang3", D::Integer(90))
                    .with("Ang4", D::Integer(45)),
                defaults: json!({"maxRadius":8,"channel1":108,"channel2":162,"channel3":90,"channel4":45}),
                changed: desc("null")
                    .with("Rds ", D::Integer(12))
                    .with("Ang1", D::Integer(-10000))
                    .with("Ang2", D::Integer(10000))
                    .with("Ang3", D::Integer(-721))
                    .with("Ang4", D::Integer(1001)),
                changes: json!({"maxRadius":12,"channel1":-10000,"channel2":10000,"channel3":-721,"channel4":1001}),
            },
            Fixture {
                code: "Mztn",
                name: "mezzotint",
                id: "filter.pixelate.mezzotint",
                default: desc("null").with("MztT", enm("MztT", "FnDt")),
                defaults: json!({"type":"fineDots"}),
                changed: desc("null").with("MztT", enm("MztT", "LngS")),
                changes: json!({"type":"longStrokes"}),
            },
            Fixture {
                code: "Msc ",
                name: "mosaic",
                id: "filter.pixelate.mosaic",
                default: desc("null").with("ClSz", px(2.0)),
                defaults: json!({"cellSize":2}),
                changed: desc("null").with("ClSz", px(31.5)),
                changes: json!({"cellSize":31.5}),
            },
            Fixture {
                code: "Pntl",
                name: "pointillize",
                id: "filter.pixelate.pointillize",
                default: desc("null").with("ClSz", D::Integer(5)),
                defaults: json!({"cellSize":5}),
                changed: desc("null").with("ClSz", D::Integer(19)),
                changes: json!({"cellSize":19}),
            },
        ]
    }

    /// Load an actual action file through the command boundary, including flag and name handling.
    pub(super) fn load_items(items: Vec<AtnItem>) -> Session {
        let file = AtnFile {
            name: UnicodeString::new_nul("Group D"),
            actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
            ..Default::default()
        };
        let bytes = atn::write(&file).unwrap();
        let mut s = Session::new();
        s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
        s
    }

    /// Save through the real command and parse its bytes so descriptor order and types stay visible.
    pub(super) fn saved_items(s: &mut Session) -> Vec<AtnItem> {
        let result = s.execute("actions.save", json!({"action":"A"})).unwrap();
        assert_eq!(result["skipped"], json!([]));
        let bytes = photocraft_paint::tile::b64_decode(result["data"].as_str().unwrap()).unwrap();
        atn::parse(&bytes).unwrap().actions.remove(0).items
    }

    /// A measured four-character event with the supplied independent descriptor.
    fn code_item(code: &str, descriptor: Descriptor) -> AtnItem {
        AtnItem { event: AtnEvent::Code(code.as_bytes().try_into().unwrap()), descriptor: Some(descriptor), ..Default::default() }
    }

    /// Trace levels are whole numbers 0..255: imports clamp to that range and play like the clamped direct command.
    #[test]
    fn trace_levels_clamp_on_import_and_play_at_the_boundary() {
        for level in [i32::MIN, -1, 0, 128, 255, 256, 300, i32::MAX] {
            let clamped = level.clamp(0, 255);
            for (edge, name) in [("Lwr ", "lower"), ("Upr ", "upper")] {
                let descriptor = desc("null").with("Lvl ", D::Integer(level)).with("Edg ", enm("CntE", edge));
                let mut item = code_item("TrcC", descriptor);
                item.with_dialog = true;
                let mut imported = load_items(vec![item]);
                assert_eq!(imported.actions().list.actions()[0].steps[0].command, "filter.stylize.traceContour", "{level}/{name}");
                assert_eq!(imported.actions().list.actions()[0].steps[0].params["level"], clamped);
                let saved = saved_items(&mut imported);
                let expected = desc("null").with("Lvl ", D::Integer(clamped)).with("Edg ", enm("CntE", edge));
                assert_eq!(saved[0].descriptor.as_ref().unwrap().to_bytes(), expected.to_bytes());
                assert!(saved[0].with_dialog);
                for depth in [8, 16, 32] {
                    let mut direct = Session::new();
                    prepare_pattern(&mut imported, depth, "rgb");
                    prepare_pattern(&mut direct, depth, "rgb");
                    let played = imported.execute("actions.play", json!({"action":"A"})).unwrap();
                    assert!(played.get("failed").is_none(), "{level}/{name}/{depth}: {played}");
                    direct.execute("filter.stylize.traceContour", json!({"level":level.clamp(0,255),"edge":name})).unwrap();
                    assert_eq!(pixels(&imported), pixels(&direct), "{level}/{name}/{depth}");
                }
            }
        }
        for level in [0, 128, 255] {
            assert!(export_item("filter.stylize.traceContour", &json!({"level":level}), "Group D").is_some(), "{level}");
        }
        for level in [json!(-1), json!(256), json!(255.5), json!(2147483648.0)] {
            assert!(export_item("filter.stylize.traceContour", &json!({"level":level}), "Group D").is_none(), "{level}");
        }
    }

    /// Imported actions match native pixels with either grouped or per-step Undo and Redo.
    #[test]
    fn imported_group_d_playback_matches_direct_commands_across_modes_and_depths() {
        let items: Vec<_> = fixtures()
            .into_iter()
            .map(|f| code_item(f.code, f.changed))
            .chain(["FndE", "Slrz", "Fct ", "Frgm"].map(|code| code_item(code, desc("null"))))
            .collect();
        for single_state in [true, false] {
            for (depth, mode) in [(8, "rgb"), (16, "rgb"), (32, "rgb"), (8, "gray"), (8, "cmyk"), (16, "lab")] {
                let mut imported = load_items(items.clone());
                imported.execute("actions.setHistory", json!({"singleState": single_state})).unwrap();
                imported.actions_mut().list.set_step(0, 10, false).unwrap();
                imported.actions_mut().list.set_dialog(0, 0, true).unwrap();
                let steps = imported.actions().list.actions()[0].steps.clone();
                let mut direct = Session::new();
                for s in [&mut imported, &mut direct] {
                    prepare_pattern(s, depth, mode);
                }
                let before = pixels(&imported);
                let before_doc = imported.active().unwrap().doc.clone();
                let history = imported.active().unwrap().history.past_len();
                let result = imported.execute("actions.play", json!({"action":"A"})).unwrap();
                assert!(result.get("failed").is_none(), "{depth}/{mode}: {result}");
                assert_eq!(result["ran"], 11);
                for (index, ActionStep { command: id, params }) in steps.into_iter().enumerate() {
                    if index != 10 {
                        direct.execute(&id, params).unwrap();
                    }
                }
                assert_eq!(pixels(&imported), pixels(&direct), "{depth}/{mode}");
                let after_doc = imported.active().unwrap().doc.clone();
                if single_state {
                    assert_eq!(imported.active().unwrap().history.past_len(), history + 1);
                } else {
                    assert_eq!(imported.active().unwrap().history.past_len(), direct.active().unwrap().history.past_len());
                }
                let added = imported.active().unwrap().history.past_len() - history;
                assert!(added > 0);
                for _ in 0..added {
                    imported.execute("edit.undo", json!({})).unwrap();
                }
                assert_eq!(pixels(&imported), before, "{depth}/{mode} undo");
                assert_eq!(imported.active().unwrap().doc.as_ref(), before_doc.as_ref(), "{depth}/{mode} undo");
                for _ in 0..added {
                    imported.execute("edit.redo", json!({})).unwrap();
                }
                assert_eq!(imported.active().unwrap().doc.as_ref(), after_doc.as_ref(), "{depth}/{mode} redo");
                let saved = saved_items(&mut imported);
                assert!(!saved[10].enabled);
                assert!(saved[0].with_dialog);
            }
        }
    }

    /// A nonuniform image and nondefault tool colors expose differences hidden by flat inputs.
    fn prepare_pattern(s: &mut Session, depth: u32, mode: &str) {
        s.execute("file.new", json!({"width":32,"height":24,"depth":depth,"mode":mode,"background":"#102030"})).unwrap();
        s.execute("select.rect", json!({"x":3,"y":2,"width":14,"height":11})).unwrap();
        s.execute("edit.fill", json!({"color":"#ee9922"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("tools.setColors", json!({"foreground":"#e83a91","background":"#148fca"})).unwrap();
    }

    fn pixels(s: &Session) -> Vec<[f32; 4]> {
        photocraft_compose::render(&s.active().unwrap().doc, photocraft_geom::Rect::new(0, 0, 32, 24)).into_px()
    }

    /// Saving omitted native params preserves their actual command behavior, not Photoshop's defaults.
    #[test]
    fn native_and_measured_defaults_stay_distinct_through_playback() {
        use photocraft_algo::FilterParams;
        let mosaic = load_items(vec![code_item("Msc ", desc("null"))]);
        let measured = &mosaic.actions().list.actions()[0].steps[0].params;
        assert!(matches!(photocraft_engine::filters::params_for("filter.pixelate.mosaic",&json!({})),Some(FilterParams::Mosaic{cell_size}) if cell_size==10.0));
        assert!(matches!(photocraft_engine::filters::params_for("filter.pixelate.mosaic",measured),Some(FilterParams::Mosaic{cell_size}) if cell_size==2.0));
        let trace = load_items(vec![code_item("TrcC", desc("null"))]);
        let measured = &trace.actions().list.actions()[0].steps[0].params;
        assert!(matches!(
            photocraft_engine::filters_ext::params_for("filter.stylize.traceContour", &json!({})),
            Some(FilterParams::TraceContour { upper: false, .. })
        ));
        assert!(matches!(
            photocraft_engine::filters_ext::params_for("filter.stylize.traceContour", measured),
            Some(FilterParams::TraceContour { upper: true, .. })
        ));
        for id in ["filter.pixelate.mosaic", "filter.stylize.traceContour"] {
            let item = export_item(id, &json!({}), "Group D").unwrap();
            let mut imported = load_items(vec![item]);
            let mut native = Session::new();
            prepare_pattern(&mut imported, 8, "rgb");
            prepare_pattern(&mut native, 8, "rgb");
            native.execute(id, json!({})).unwrap();
            let result = imported.execute("actions.play", json!({"action":"A"})).unwrap();
            assert!(result.get("failed").is_none(), "{id}: {result}");
            assert_eq!(pixels(&imported), pixels(&native), "{id}");
        }
    }
}

mod tests {
    use craft_psd::atn::{self};
    use craft_psd::descriptor::Class;
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

    fn color_lookup_case() -> Value {
        let bytes = b"LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let d = desc("null")
            .with("lookupType", enm("colorLookupType", "3DLUT"))
            .with("Nm  ", text("identity.cube"))
            .with("Dthr", D::Boolean(false))
            .with("profile", D::RawData(Vec::new()))
            .with("LUTFormat", enm("LUTFormatType", "LUTFormatCUBE"))
            .with("LUT3DFileData", D::RawData(Vec::new()))
            .with("LUT3DFileName", text("identity.cube"));
        json!({"data":b64(bytes),"dataEncoding":"base64","lutFormat":"cube","fileName":"identity.cube","dither":false,"__atnColorLookup":b64(&d.to_bytes())})
    }

    /// Keep the starting counter nested in naming rules, as recorded by the Batch dialog.
    fn batch_descriptor(component: &str) -> Descriptor {
        let rules = desc("fileNamingRules")
            .with("fileNamingComponents", D::List(vec![D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", component)))]))
            .with("Strt", D::Integer(7))
            .with("Mcnt", D::Boolean(true))
            .with("Win ", D::Boolean(false))
            .with("unix", D::Boolean(true));
        desc("Btch")
            .with("null", path_value("/input"))
            .with("Fldr", D::Boolean(false))
            .with(
                "Usng",
                D::Reference(vec![
                    ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("Child") },
                    ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul("S") },
                ]),
            )
            .with("T   ", path_value("/output"))
            .with("Ovrd", D::Boolean(false))
            .with("fileNamingRules", D::Descriptor(rules))
    }

    /// Preserve the recorded template in the generic registry round-trip witness.
    fn batch_case() -> Value {
        json!({"input":"/input","output":"/output","action":"Child","set":"S","flatOutput":true,"includeSubfolders":false,"overrideSaveAs":false,"stopOnError":true,"includeNonImages":true,
        "namingComponents":[{"component":"serial3"}],"startingSerial":7,"compatibility":{"macintosh":true,"windows":false,"unix":true},
        "__atnBatch":b64(&batch_descriptor("threeDigit").to_bytes())})
    }

    /// Save and Close replaces the folder path while retaining its recorded naming metadata.
    fn batch_save_and_close_case() -> Value {
        let mut d = batch_descriptor("threeDigit");
        d.items.retain(|(key, _)| !key.is("T   "));
        let d = d.with("SvAn", D::Boolean(true));
        let mut p = batch_case();
        p.as_object_mut().unwrap().remove("output");
        p.as_object_mut().unwrap().remove("flatOutput");
        p["destination"] = json!("saveAndClose");
        p["__atnBatch"] = json!(b64(&d.to_bytes()));
        p
    }

    /// Every exported command has a canonical import, including recorded full-stack navigation.
    #[test]
    /// Each mapped command retains its parameters through its corresponding ATN descriptor.
    fn every_mapped_command_exports_and_imports_back() {
        photocraft_engine::install().unwrap();
        let cases: Vec<(&str, Value)> = vec![
            (
                "edit.perspectiveWarp",
                json!({"planes":[{"src":[[4,4],[16,4],[16,20],[4,20]],"dst":[[5,5],[17,4],[18,20],[4,19]]}],
                "target":"pixels","__atnPerspectiveWarpRect":[0,0,32,24]}),
            ),
            (
                "paint.gradient",
                json!({"from":[1,1],"to":[6,2],"style":"linear","method":"classic","smoothness":100,
            "stops":[[0,"#0020ff"],[1,"#ffe000"]],"transparency":[],"opacity":100,"mode":"Normal","reverse":false,"dither":false}),
            ),
            ("filter.render.clouds", json!({})),
            ("filter.render.differenceClouds", json!({})),
            ("filter.render.fibers", json!({"variance": 20, "strength": 7, "seed": -123})),
            ("filter.render.lensFlare", json!({"brightness": 180, "centerX": 5, "centerY": -5, "lens": "moviePrime"})),
            ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add"})),
            ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add","ellipse":true,"antiAlias":false})),
            ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"subtract"})),
            ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"intersect"})),
            ("edit.pasteSpecial.pasteInto", json!({})),
            ("edit.pasteSpecial.pasteOutside", json!({})),
            (
                "edit.contentAwareFill",
                json!({"sampling":"rectangular","sampleAllLayers":true,"colorAdaptation":"default",
                "rotationAdaptation":"none","scale":true,"mirror":false,"output":"new"}),
            ),
            ("edit.fade", json!({"opacity":50,"mode":"Multiply"})),
            ("layer.new.frameFromLayers", json!({"layer": "Photo", "name": "Frame 1"})),
            ("layer.linkLayers", json!({})),
            ("layer.new.artboard", json!({"rect": [10, 20, 100, 50], "name": "Hero"})),
            ("layer.new.artboardFromLayers", json!({"name": "Cards"})),
            ("layer.smartObjects.stackMode.range", json!({})),
            ("layer.smartObjects.stackMode.entropy", json!({})),
            ("layer.smartObjects.stackMode.kurtosis", json!({})),
            ("layer.smartObjects.stackMode.maximum", json!({})),
            ("layer.smartObjects.stackMode.median", json!({})),
            ("layer.smartObjects.stackMode.minimum", json!({})),
            ("layer.smartObjects.stackMode.mean", json!({})),
            ("layer.smartObjects.stackMode.skewness", json!({})),
            ("layer.smartObjects.stackMode.standardDeviation", json!({})),
            ("layer.smartObjects.stackMode.summation", json!({})),
            ("layer.smartObjects.stackMode.variance", json!({})),
            ("layer.smartObjects.stackMode.none", json!({})),
            ("layer.distribute.leftEdges", json!({})),
            ("layer.distribute.horizontalCenters", json!({})),
            ("layer.distribute.rightEdges", json!({})),
            ("layer.distribute.topEdges", json!({})),
            ("layer.distribute.verticalCenters", json!({})),
            ("layer.distribute.bottomEdges", json!({})),
            ("edit.definePattern", json!({})),
            ("layer.layerMask.delete", json!({"_atnEvent":"RmvL"})),
            ("layer.mergeLayers", json!({"_atnEvent":"MrgL"})),
            ("filter.stylize.findEdges", json!({})),
            ("filter.stylize.solarize", json!({})),
            ("filter.pixelate.facet", json!({})),
            ("filter.pixelate.fragment", json!({})),
            (
                "filter.stylize.extrude",
                json!({"size":42,"depth":91,"solidFrontFaces":true,"maskIncompleteBlocks":true,"type":"pyramids","depthMode":"levelBased"}),
            ),
            ("filter.stylize.traceContour", json!({"level":43,"edge":"lower"})),
            ("filter.stylize.wind", json!({"method":"stagger","direction":"fromLeft"})),
            ("filter.stylize.tiles", json!({"count":17,"maxOffset":23,"fill":"foreground"})),
            ("filter.pixelate.colorHalftone", json!({"maxRadius":12,"channel1":-10000,"channel2":10000,"channel3":-721,"channel4":1001})),
            ("filter.pixelate.mezzotint", json!({"type":"longStrokes"})),
            ("filter.pixelate.mosaic", json!({"cellSize":31.5})),
            ("filter.pixelate.pointillize", json!({"cellSize":19})),
            ("filter.blur.gaussianBlur", json!({"radius": 3.5})),
            ("filter.sharpen.unsharpMask", json!({"amount": 120, "radius": 1.5, "threshold": 3})),
            ("filter.noise.addNoise", json!({"amount": 12.5, "distribution": "gaussian", "monochromatic": true, "seed": 7})),
            ("filter.noise.median", json!({"radius": 2})),
            ("filter.stylize.emboss", json!({"angle": 135, "height": 3, "amount": 100})),
            (
                "image.calculations",
                json!({"source1":{"channel":{"color":"red"}},"source2":{"channel":{"color":"green"}},"blending":"add","scale":1,"offset":0}),
            ),
            ("image.adjustments.colorLookup", color_lookup_case()),
            ("file.automate.batch", batch_case()),
            ("file.automate.batch", batch_save_and_close_case()),
            ("filter.other.custom", json!({"kernel":[0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0],"scale":1,"offset":0})),
            ("filter.render.fibers", json!({"variance":64,"strength":1,"seed":-1})),
            ("filter.blur.shapeBlur", json!({"radius":5,"shapeName":"Recorded Square","shapeId":"original-fixture-square","shapeOriginType":9})),
            ("filter.blur.motionBlur", json!({"angle": 45, "distance": 20})),
            ("filter.blur.smartBlur", json!({"radius": 2.5, "threshold": 40, "quality": "medium", "mode": "overlayEdge"})),
            ("filter.other.offset", json!({"horizontal": 10, "vertical": -5, "undefinedAreas": "wrap"})),
            ("filter.blur.blur", json!({})),
            (
                "file.automate.createDroplet",
                json!({"path": "/d/Sharpen.pcdroplet", "action": "Sharpen", "set": "Tools", "output": "/d/out", "overrideOpen": true}),
            ),
            ("filter.distort.displace", json!({"horizontal":-999,"vertical":999,"fit":"stretch","undefinedAreas":"repeat","mapPath":LOCAL_MAP})),
            ("filter.distort.shear", json!({"points": [[0,0], [0.5,0.25], [1,0]], "undefinedAreas": "repeat"})),
            ("filter.distort.pinch", json!({"amount": -30})),
            ("filter.distort.polarCoordinates", json!({"mode": "polarToRectangular"})),
            ("filter.distort.spherize", json!({"amount": -50, "mode": "verticalOnly"})),
            ("filter.distort.twirl", json!({"angle": -200})),
            (
                "filter.distort.wave",
                json!({"type": "square", "generators": 3, "wavelengthMin": 20, "wavelengthMax": 200, "amplitudeMin": 4, "amplitudeMax": 40,
                "undefinedAreas": "wrap", "seed": 4_000_000_000_u32}),
            ),
            ("filter.distort.zigZag", json!({"amount": -20, "ridges": 12, "style": "aroundCenter"})),
            ("filter.video.deInterlace", json!({"eliminate": "evenFields", "createBy": "duplication"})),
            ("filter.video.ntscColors", json!({})),
            ("filter.other.hsbHsl", json!({"inputMode": "hsl", "rowOrder": "rgb"})),
            (
                "filter.other.custom",
                json!({"kernel": [0, 0, 0, 0, 0, 0, -1, -1, -1, 0, 0, -1, 9, -1, 0, 0, -1, -1, -1, 0, 0, 0, 0, 0, 0], "scale": 3, "offset": -10}),
            ),
            ("filter.blur.average", json!({})),
            ("layer.matting.removeBlackMatte", json!({})),
            ("layer.matting.removeWhiteMatte", json!({})),
            (
                "filter.stylize.oilPaint",
                json!({"lighting": false, "stylization": 6.5, "cleanliness": 3, "scale": 2, "bristleDetail": 7, "angle": 45, "shine": 2}),
            ),
            ("filter.sharpen.smartSharpen", json!({"amount": 150, "radius": 2.5, "reduceNoise": 0})),
            ("select.focusArea", json!({})),
            ("select.sky", json!({"sampleAllLayers": false})),
            ("layer.removeBackground", json!({})),
            ("channel.split", json!({})),
            ("channel.mergeSpot", json!({})),
            ("image.trap", json!({"width": 3})),
            ("image.adjustments.matchColor", json!({"luminance": 120, "intensity": 80, "fade": 25})),
            (
                "image.adjustments.levels",
                json!({"inBlack": 5, "inWhite": 250, "gamma": 0.9, "outBlack": 0, "outWhite": 255, "red": {"inBlack": 2, "inWhite": 255, "gamma": 1, "outBlack": 0, "outWhite": 255}}),
            ),
            ("image.adjustments.curves", json!({"points": [[0, 0], [128, 140], [255, 255]], "blue": [[0, 10], [255, 245]]})),
            (
                "image.adjustments.hueSaturation",
                json!({"colorize": false, "hue": 10, "saturation": -20, "lightness": 0, "reds": {"hue": 5, "saturation": 0, "lightness": 0, "range": [315, 345, 15, 45]}}),
            ),
            ("image.adjustments.brightnessContrast", json!({"brightness": 10, "contrast": -5, "legacy": false})),
            (
                "image.adjustments.shadowsHighlights",
                json!({"shadowAmount": 60, "shadowTone": 40, "shadowRadius": 25, "highlightAmount": 10, "highlightTone": 50, "highlightRadius": 30, "blackClip": 0.02, "whiteClip": 0.05, "midtone": 15, "color": 20}),
            ),
            ("image.adjustments.threshold", json!({"level": 100})),
            ("image.adjustments.posterize", json!({"levels": 4})),
            ("image.adjustments.invert", json!({})),
            ("image.adjustments.colorBalance", json!({"shadows": [1, 2, 3], "midtones": [0, 0, 0], "highlights": [-5, 0, 5], "preserveLuminosity": true})),
            ("image.adjustments.vibrance", json!({"vibrance": 30, "saturation": 5})),
            ("image.adjustments.exposure", json!({"exposure": 0.5, "offset": 0.01, "gamma": 1.1})),
            ("image.adjustments.photoFilter", json!({"color": "#ec8a00", "density": 25, "preserveLuminosity": true})),
            (
                "image.adjustments.blackWhite",
                json!({"reds": 40, "yellows": 60, "greens": 40, "cyans": 60, "blues": 20, "magentas": 80, "tint": true, "tintColor": "#e4d5b0"}),
            ),
            ("image.adjustments.channelMixer", json!({"red": [100, 0, 0, 0], "green": [0, 90, 10, 0], "blue": [0, 0, 100, 5], "monochrome": false})),
            ("image.adjustments.selectiveColor", json!({"method": "absolute", "reds": [10, 0, -5, 0], "yellows": [0, 5, 0, 0], "neutrals": [0, 0, 0, 3]})),
            ("select.subject", json!({"sampleAllLayers": false})),
            ("select.colorRange", json!({"select": "midtones", "fuzziness": 30, "tonalRange": [90, 160], "invert": true})),
            ("select.colorRange", json!({"select": "skinTones", "fuzziness": 40})),
            ("select.colorRange", json!({"select": "highlights", "tonalRange": 200})),
            ("select.colorRange", json!({"select": "reds"})),
            ("layer.createClippingMask", json!({})),
            ("layer.releaseClippingMask", json!({"layer": "Shadow"})),
            ("layer.layerMask.revealAll", json!({})),
            ("layer.layerMask.hideAll", json!({})),
            ("layer.layerMask.revealSelection", json!({})),
            ("layer.layerMask.hideSelection", json!({})),
            ("tools.setColors", json!({"foreground": "#336699"})),
            ("tools.setColors", json!({"background": "#ffffff"})),
            ("image.adjustments.desaturate", json!({})),
            ("image.autoTone", json!({})),
            ("image.imageSize", json!({"width": 800, "height": 600, "resolution": 72, "resample": "bicubicSharper", "scaleStyles": true})),
            ("image.canvasSize", json!({"width": 100, "height": 50, "relative": true, "anchor": "topLeft", "extensionColor": "white"})),
            ("image.imageRotation.90cw", json!({})),
            ("image.rotation.arbitrary", json!({"angle": 12.5, "direction": "ccw"})),
            ("edit.transform.flipHorizontal", json!({})),
            ("image.imageRotation.flipCanvasVertical", json!({})),
            ("image.crop", json!({"x": 10, "y": 20, "width": 100, "height": 50, "deleteCroppedPixels": true})),
            ("image.trim", json!({"basedOn": "transparent", "top": true, "bottom": true, "left": false, "right": true})),
            ("image.mode.grayscale", json!({})),
            ("image.mode.bits16", json!({})),
            ("layer.duplicate", json!({"name": "copy"})),
            // Duplicate into a new document is a `Mk`, not a `Dplc` (the three shapes of
            // a recorded Duplicate Layer with Destination: New, plus a named layer).
            ("layer.duplicate", json!({"document": "new"})),
            ("layer.duplicate", json!({"document": "new", "documentName": "test new name"})),
            ("layer.duplicate", json!({"document": "new", "name": "Laer name change"})),
            ("layer.duplicate", json!({"layer": "Shadow", "document": "new", "documentName": "Doc"})),
            ("image.duplicate", json!({"mergedOnly": true})),
            ("layer.new.layer", json!({"name": "Ink", "opacity": 0.5, "mode": "Multiply"})),
            ("type.convertToShape", json!({})),
            ("type.createWorkPath", json!({})),
            (
                "path.set",
                json!({"name":"work","path":{"fillRule":"evenodd","inverted":false,"subpaths":[{"closed":false,"op":"combine","knots":[
                    {"anchor":[1,2],"in":[1,2],"out":[3,4],"smooth":false},
                    {"anchor":[8,9],"in":[6,7],"out":[8,9],"smooth":false}
                ]}]}}),
            ),
            ("filter.blur.shapeBlur", json!({})),
            ("filter.blur.shapeBlur", json!({"radius": 10})),
            ("layer.new.group", json!({"name": "G"})),
            ("layer.groupLayers", json!({})),
            ("layer.linkLayers", json!({"linked": false})),
            ("layer.linkLayers", json!({"linked": false, "_atnNoTarget": true})),
            ("layer.arrange.reverse", json!({})),
            ("layer.arrange.reverse", json!({"_atnNoTarget": true})),
            ("layer.selectLinkedLayers", json!({})),
            ("layer.layerStyle.clear", json!({})),
            ("layer.rasterize.allLayers", json!({})),
            ("layer.videoLayers.newBlankVideoLayer", json!({})),
            ("select.isolateLayers", json!({})),
            ("type.loadDefaultTypeStyles", json!({})),
            ("type.saveDefaultTypeStyles", json!({})),
            ("image.analysis.recordMeasurements", json!({})),
            ("image.analysis.setMeasurementScale", json!({"preset": "custom", "pixelLength": 120, "logicalLength": 2.5, "units": "mm"})),
            ("layer.videoLayers.insertBlankFrame", json!({})),
            ("file.automate.conditionalModeChange", json!({"from": ["grayscale", "rgb"], "to": "cmyk"})),
            ("layer.videoLayers.duplicateFrame", json!({})),
            ("layer.videoLayers.deleteFrame", json!({})),
            ("file.automate.cropAndStraightenPhotos", json!({})),
            ("file.automate.cropAndStraightenPhotos", json!({"_atnNoTarget": true})),
            ("file.automate.fitImage", json!({"width": 800, "height": 600, "dontEnlarge": true})),
            ("layer.new.artboardFromGroup", json!({})),
            ("layer.new.artboardFromGroup", json!({"background":"white","color":[1,1,1],"preset":"","guideIDs":[],"changeBackground":1})),
            (
                "layer.artboard.set",
                json!({"background":"custom","color":[0.25,0.5,0.75],"recordedPreset":"","guideIDs":[],"recordedRect":[2,2,6,6],"changeSizes":8,"changeBackground":1}),
            ),
            ("layer.new.layerViaCopy", json!({})),
            ("layer.newFillLayer.solidColor", json!({"color": "#336699"})),
            ("layer.newAdjustmentLayer.hueSaturation", json!({})),
            ("layer.newAdjustmentLayer.threshold", json!({"level": 77})),
            ("file.new", json!({"width": 640, "height": 480, "resolution": 72, "mode": "rgb", "depth": 8, "background": "transparent", "name": "Doc"})),
            ("select.all", json!({})),
            ("select.deselect", json!({})),
            ("select.loadSelection", json!({"channel": "transparency"})),
            ("select.loadSelection", json!({"channel": "Alpha 1"})),
            ("select.rect", json!({"x": 1, "y": 2, "width": 30, "height": 40})),
            ("select.rect", json!({"x": 1, "y": 2, "width": 30, "height": 40, "ellipse": true})),
            ("layer.setProps", json!({"name": "Renamed", "opacity": 0.4, "blend": "Linear Light"})),
            ("layer.setProps", json!({"layer": "Shadow", "visible": false})),
            ("layer.setLabelColor", json!({"color": "red"})),
            ("layer.showOnly", json!({"layer": "Solo"})),
            ("layer.select", json!({"layer": "Shadow", "mode": "add"})),
            ("layer.select", json!({"layer": "Shadow", "mode": "range"})),
            ("layer.select", json!({"layer": {"index": 2}})),
            ("layer.moveTo", json!({"target": {"index": 0}})),
            ("layer.moveTo", json!({"layer": {"index": 2}, "target": {"index": 4}})),
            ("layer.selectAbove", json!({})),
            ("layer.selectBottom", json!({})),
            ("layer.selectTop", json!({})),
            ("layer.selectBelow", json!({})),
            ("layer.addBelowToSelection", json!({})),
            ("layer.addAboveToSelection", json!({})),
            ("layer.delete", json!({})),
            ("edit.clear", json!({})),
            ("layer.hideLayers", json!({"layer": "Background"})),
            ("layer.showLayers", json!({})),
            ("layer.translate", json!({"dx": 3, "dy": -4})),
            ("layer.arrange.sendToBack", json!({})),
            ("layer.flattenImage", json!({})),
            ("layer.stampVisible", json!({})),
            ("layer.mergeVisible", json!({})),
            ("layer.matting.defringe", json!({"width": 2})),
            ("layer.align.leftEdges", json!({"to": "canvas"})),
            ("select.inverse", json!({})),
            ("select.modify.feather", json!({"radius": 4})),
            ("select.modify.contract", json!({"radius": 1})),
            ("edit.fill", json!({"contents": "color", "color": "#ff0080", "opacity": 50, "mode": "Multiply", "preserveTransparency": true})),
            ("edit.fill", json!({"contents": "foreground", "opacity": 100, "mode": "Normal"})),
            ("edit.stroke", json!({"width": 3, "location": "outside", "opacity": 100, "mode": "Normal", "color": "#000000", "preserveTransparency": false})),
            ("edit.copyMerged", json!({})),
            ("edit.pasteSpecial.pasteInPlace", json!({})),
            ("edit.paste", json!({})),
            ("edit.purge.histories", json!({})),
            ("edit.checkSpelling", json!({"allLayers": true})),
            ("edit.checkSpelling", json!({"allLayers": false})),
            ("tools.defaultColors", json!({})),
            (
                "edit.convertToProfile",
                json!({"profile": "display-p3", "intent": "perceptual", "bpc": false, "dither": false, "flatten": false, "rasterizePlaced": false}),
            ),
            ("edit.assignProfile", json!({"profile": "srgb"})),
            ("file.close", json!({})),
            ("file.save", json!({})),
            ("layer.layerMask.delete", json!({})),
            ("select.lasso", json!({"points": [[1, 2], [30, 2], [15, 40]]})),
            ("select.editInQuickMaskMode", json!({"on": true})),
            ("select.editInQuickMaskMode", json!({"on": false})),
            ("edit.transform", json!({"dx": 10, "dy": -5, "scaleX": 50, "scaleY": 75, "rotate": 12.5, "interpolation": "bicubic"})),
            ("edit.transform", json!({"layer": "Shadow", "dx": 0, "dy": 0, "scaleX": 200})),
            ("select.transformSelection", json!({"dx": 3, "dy": 4, "scaleX": 110, "scaleY": 110, "rotate": -30})),
            (
                "image.applyImage",
                json!({"source": {"channel": "composite", "layer": "High-Frequency"}, "blending": "subtract", "scale": 2, "offset": 128, "preserveTransparency": true}),
            ),
            ("image.applyImage", json!({"source": {"channel": "red", "layer": "merged", "invert": true}, "blending": "multiply", "opacity": 50})),
            (
                "file.export.saveForWebLegacy",
                json!({"path": "/tmp/out/banner.png", "format": "png24", "interlaced": false, "transparency": true, "embedIcc": false, "progressive": false, "optimized": true, "matte": "#ffffff"}),
            ),
            (
                "file.export.saveForWebLegacy",
                json!({"folder": "/tmp/out", "format": "png24", "interlaced": false, "transparency": true, "embedIcc": false, "progressive": false, "optimized": true, "matte": "#ffffff"}),
            ),
            (
                "file.export.saveForWebLegacy",
                json!({"path": "C:\\shots\\a.jpg", "format": "jpeg", "interlaced": false, "transparency": false, "embedIcc": true, "progressive": true, "optimized": true, "quality": 80, "matte": "none"}),
            ),
            ("image.adjustments.gradientMap", json!({"stops": [[0, "#000000"], [0.5, "#ff8000"], [1, "#ffffff"]], "reverse": true, "dither": false})),
            ("image.adjustments.gradientMap", json!({"stops": [[0,[0.5,0,0]],[1,[0,1,0]]], "__atnGrMp":{"name":"Recorded","colors":[[127.5,0,0],[0,255,0]]}})),
            ("image.adjustments.replaceColor", json!({"labMinimum":[40,30,40],"labMaximum":[60,35,45],"fuzziness":40,"hue":30,"saturation":0,"lightness":0})),
            ("layer.newAdjustmentLayer.gradientMap", json!({"stops": [[0, "#102030"], [1, "#f0e0d0"]], "reverse": false, "dither": true})),
            ("layer.smartObjects.convertToSmartObject", json!({})),
            ("layer.smartObjects.editContents", json!({})),
            ("layer.smartObjects.updateModifiedContent", json!({})),
            ("layer.smartObjects.newSmartObjectViaCopy", json!({})),
            ("layer.smartObjects.convertToLayers", json!({})),
            ("layer.smartObjects.convertToEmbedded", json!({})),
            ("layer.smartObjects.updateAllModifiedContent", json!({})),
            ("layer.smartObjects.exportContents", json!({"path": "/tmp/contents.png"})),
            ("layer.smartObjects.convertToLinked", json!({"path": "/tmp/linked.png"})),
            ("layer.smartObjects.relinkToFile", json!({"path": "/tmp/relinked.png"})),
            ("layer.smartObjects.replaceContents", json!({"path": "/tmp/replacement.png"})),
            ("paint.bucket", json!({"x": 10, "y": 20, "tolerance": 32, "antiAlias": true, "contiguous": false, "opacity": 100})),
            ("image.imageSize", json!({"percent": 200, "resample": "bicubicAutomatic", "scaleStyles": true})),
            ("file.placeEmbedded", json!({"path": "/tmp/logo.png", "scale": 50})),
            ("file.placeLinked", json!({"path": "/tmp/logo.png"})),
            ("file.openAs", json!({"path": "/tmp/a.psd"})),
            ("file.saveAs", json!({"path": "/tmp/out/a.png"})),
            ("file.saveACopy", json!({"path": "/tmp/out/a.tif"})),
            ("actions.conditional", json!({"if": "hasLayers", "then": "FS8-Layers", "else": "FS8-No-Layers", "set": "S"})),
            ("actions.conditional", json!({"if": "landscape", "then": "Wide", "else": null, "set": "S"})),
            ("tools.setColors", json!({"foreground": "#102030"})),
            ("tools.setColors", json!({"background": "#ffffff"})),
            ("view.fitOnScreen", json!({})),
            ("view.zoomIn", json!({})),
            ("layer.createClippingMask", json!({})),
            ("layer.ungroupLayers", json!({})),
            ("layer.layerStyle.copyLayerStyle", json!({})),
            ("layer.layerStyle.scaleEffects", json!({"scale": 200})),
            ("layer.layerStyle.createLayer", json!({})),
            ("layer.layerStyle.pasteLayerStyle", json!({"allowGroups": true})),
            ("layer.setExpanded", json!({"all": true, "expanded": false})),
            ("layerComp.apply", json!({"comp": "Original"})),
            ("actions.play", json!({"action": "Other", "set": "S"})),
            (STOP_MESSAGE, json!({"message": "Hi", "allowContinue": false})),
            ("document.activate", json!({"offset": -1})),
            ("document.activate", json!({"document": {"index": 2}})),
            // The TrueGrit Texture Supply sets (Debaser, Distress Press, Halftone Zine Machine).
            ("filter.distort.ripple", json!({"amount": 50, "size": "medium"})),
            ("filter.pixelate.crystallize", json!({"cellSize": 5})),
            ("filter.gallery.spatter", json!({"sprayRadius": 7, "smoothness": 15})),
            ("filter.gallery.accentedEdges", json!({"edgeWidth": 14, "edgeBrightness": 50, "smoothness": 15})),
            ("filter.gallery.angledStrokes", json!({"directionBalance": 100, "strokeLength": 50, "sharpness": 10})),
            ("filter.gallery.basRelief", json!({"detail": 15, "smoothness": 15, "light": "bottomLeft"})),
            ("filter.gallery.chalkCharcoal", json!({"charcoalArea": 20, "chalkArea": 20, "strokePressure": 5})),
            ("filter.gallery.charcoal", json!({"charcoalThickness": 7, "detail": 0, "lightDarkBalance": 100})),
            ("filter.gallery.chrome", json!({"detail": 10, "smoothness": 10})),
            ("filter.gallery.coloredPencil", json!({"pencilWidth": 24, "strokePressure": 15, "paperBrightness": 50})),
            (
                "filter.gallery.conteCrayon",
                json!({"foregroundLevel": 15, "backgroundLevel": 15, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
            ),
            ("filter.gallery.craquelure", json!({"crackSpacing": 100, "crackDepth": 10, "crackBrightness": 10})),
            ("filter.gallery.crosshatch", json!({"strokeLength": 50, "sharpness": 20, "strength": 3})),
            ("filter.gallery.cutout", json!({"numberOfLevels": 8, "edgeSimplicity": 10, "edgeFidelity": 3})),
            ("filter.gallery.darkStrokes", json!({"balance": 10, "blackIntensity": 10, "whiteIntensity": 10})),
            ("filter.gallery.diffuseGlow", json!({"graininess": 10, "glowAmount": 20, "clearAmount": 20})),
            ("filter.gallery.dryBrush", json!({"brushSize": 10, "brushDetail": 10, "texture": 3})),
            ("filter.gallery.filmGrain", json!({"grain": 20, "highlightArea": 20, "intensity": 0})),
            ("filter.gallery.fresco", json!({"brushSize": 10, "brushDetail": 10, "texture": 3})),
            ("filter.gallery.glass", json!({"distortion": 20, "smoothness": 15, "texture": "canvas", "scaling": 200, "invert": true})),
            ("filter.gallery.glowingEdges", json!({"edgeWidth": 14, "edgeBrightness": 20, "smoothness": 15})),
            ("filter.gallery.graphicPen", json!({"strokeLength": 1, "lightDarkBalance": 100, "strokeDirection": "horizontal"})),
            ("filter.gallery.halftonePattern", json!({"size": 12, "contrast": 50, "patternType": "circle"})),
            ("filter.gallery.inkOutlines", json!({"strokeLength": 50, "darkIntensity": 50, "lightIntensity": 50})),
            ("filter.gallery.mosaicTiles", json!({"tileSize": 100, "groutWidth": 15, "lightenGrout": 10})),
            ("filter.gallery.neonGlow", json!({"glowSize": 24, "glowBrightness": 50, "glowColor": "#336699"})),
            ("filter.gallery.notePaper", json!({"imageBalance": 50, "graininess": 20, "relief": 25})),
            ("filter.gallery.oceanRipple", json!({"rippleSize": 15, "rippleMagnitude": 20})),
            ("filter.gallery.paintDaubs", json!({"brushSize": 50, "sharpness": 40, "brushType": "darkRough"})),
            ("filter.gallery.paletteKnife", json!({"strokeSize": 50, "strokeDetail": 1, "softness": 10})),
            ("filter.gallery.patchwork", json!({"squareSize": 10, "relief": 25})),
            ("filter.gallery.photocopy", json!({"detail": 24, "darkness": 50})),
            ("filter.gallery.plaster", json!({"imageBalance": 50, "smoothness": 15, "light": "bottomLeft"})),
            ("filter.gallery.plasticWrap", json!({"highlightStrength": 20, "detail": 15, "smoothness": 15})),
            ("filter.gallery.posterEdges", json!({"edgeThickness": 10, "edgeIntensity": 10, "posterization": 6})),
            ("filter.gallery.reticulation", json!({"density": 50, "foregroundLevel": 50, "backgroundLevel": 50})),
            (
                "filter.gallery.roughPastels",
                json!({"strokeLength": 40, "strokeDetail": 20, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
            ),
            ("filter.gallery.smudgeStick", json!({"strokeLength": 10, "highlightArea": 20, "intensity": 0})),
            ("filter.gallery.sponge", json!({"brushSize": 10, "definition": 25, "smoothness": 15})),
            ("filter.gallery.sprayedStrokes", json!({"strokeLength": 20, "sprayRadius": 25, "strokeDirection": "horizontal"})),
            ("filter.gallery.stainedGlass", json!({"cellSize": 50, "borderThickness": 20, "lightIntensity": 10})),
            ("filter.gallery.stamp", json!({"lightDarkBalance": 50, "smoothness": 50})),
            ("filter.gallery.sumiE", json!({"strokeWidth": 15, "strokePressure": 15, "contrast": 40})),
            ("filter.gallery.texturizer", json!({"texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true})),
            ("filter.gallery.tornEdges", json!({"imageBalance": 50, "smoothness": 15, "contrast": 25})),
            (
                "filter.gallery.underpainting",
                json!({"brushSize": 40, "textureCoverage": 40, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
            ),
            ("filter.gallery.waterPaper", json!({"fiberLength": 50, "brightness": 100, "contrast": 100})),
            ("filter.gallery.watercolor", json!({"brushDetail": 14, "shadowIntensity": 10, "texture": 3})),
            ("filter.gallery.grain", json!({"intensity": 100, "contrast": 100, "grainType": "contrasty"})),
            (
                "select.refineEdge",
                json!({"radius": 0, "smartRadius": false, "smooth": 2, "feather": 0.5, "contrast": 15, "shiftEdge": -100, "decontaminate": false, "output": "layerMask"}),
            ),
            ("channel.target", json!({"channel": "mask"})),
            ("channel.target", json!({"channel": "composite"})),
            ("edit.undo", json!({})),
            ("edit.redo", json!({})),
            ("layer.select", json!({"layer": "YELLOW", "mode": "remove"})),
            ("layer.select", json!({"layer": "Or paste photocopy texture here", "mode": "range"})),
            ("layer.translate", json!({"dx": 0.36, "dy": -1.08, "units": "pt"})),
            ("select.loadSelection", json!({"channel": "targeted"})),
            ("layer.newFillLayer.pattern", json!({"pattern": "Debaser-Distressed-Dark", "scale": 50, "angle": 0, "link": true})),
            ("layer.fill.pattern.set", json!({"pattern": "1 -R2.jpg"})),
            ("layer.newFillLayer.pattern", json!({"pattern": "b7a3c1d2-0000-4000-8000-000000000001", "patternName": "Twin", "scale": 50})),
            ("image.mode.bitmap", json!({"method": "threshold"})),
            ("image.imageSize", json!({"resolution": 1200, "resample": "preserveDetails", "keepPrintSize": true, "scaleStyles": true})),
            ("layer.new.layerFromBackground", json!({"ifPresent": true, "select": false})),
            // COPYCAT, AJ_Actions2023 and the Line and Color tutorial set.
            ("layer.smartFilter.deleteFilterMask", json!({"ifPresent": true})),
            ("layer.smartFilter.blendingOptions", json!({"index": 1, "opacity": 0.5, "blend": "Darken"})),
            ("layer.layerStyle.blendingOptions", json!({"blendClippedLayersAsGroup": false, "transparencyShapesLayer": false})),
            ("layer.layerStyle.blendingOptions", json!({"blendIf": [{"channel": "gray", "thisLayer": [0, 30, 200, 255], "underlying": [0, 0, 255, 255]}]})),
            (
                "layer.layerStyle.blendingOptions",
                json!({"opacity": 40, "fillOpacity": 70, "blend": "Multiply", "blendIf": [{"channel": "blue", "thisLayer": [5, 5, 250, 250]}]}),
            ),
            ("layer.createClippingMask", json!({})),
            ("layer.lockLayers", json!({"transparency": true})),
            ("filter.stylize.diffuse", json!({"mode": "normal", "seed": 321654})),
            ("history.clear", json!({})),
            ("history.newSnapshot", json!({"name": "CopyCat - action result", "from": "fullDocument", "replaceExisting": true})),
            ("layerComp.apply", json!({"comp": "Comp 1"})),
            ("layerComp.apply", json!({"comp": {"index": 2}})),
            ("layerComp.apply", json!({"comp": {"id": 7}})),
            ("layerComp.apply", json!({"comp": {"ordinal": "Nxt"}})),
        ];
        for (id, params) in &cases {
            let item = export_item(id, params, "S").unwrap_or_else(|| panic!("{id} does not export"));
            if *id == "file.close" {
                // Close exports "don't save" and imports as a plain close; the Save answer is tested below.
                assert_eq!(item.event.id(), "Cls ");
                continue;
            }
            let e = event_for(&item.event).unwrap_or_else(|| panic!("{id} exported as an unknown event {}", item.event.id()));
            assert_eq!(item.descriptor.is_none(), *id == photocraft_engine::atn_cmds::steps_stack::NONE, "{id}: unexpected descriptor presence");
            let descriptor = item.descriptor.unwrap_or_default();
            match (e.from)(&descriptor) {
                Mapped::Steps(steps) => {
                    assert_eq!(steps.len(), 1, "{id}: {steps:?}");
                    let navigation = match *id {
                        "layer.selectAbove" => Some(("Frwr", false)),
                        "layer.selectBelow" => Some(("Bckw", false)),
                        "layer.selectTop" => Some(("Frnt", false)),
                        "layer.selectBottom" => Some(("Back", false)),
                        "layer.addAboveToSelection" => Some(("Frwr", true)),
                        "layer.addBelowToSelection" => Some(("Bckw", true)),
                        _ => None,
                    };
                    if let Some((ordinal, add)) = navigation {
                        let mut expected = json!({"layer": {"ordinal": ordinal}});
                        if add {
                            expected["mode"] = json!("add");
                        }
                        assert_eq!(steps, vec![("layer.select".to_string(), expected)]);
                        continue;
                    }
                    assert_eq!(steps[0].0, *id, "{id} came back as another command");
                    let mut back = steps[0].1.clone();
                    if *id == "paint.gradient" {
                        // Executable values match; independent public tests verify the source carrier.
                        back.as_object_mut().unwrap().remove("__atnClassicGradient");
                    }
                    if *id == "path.set" {
                        // The imported step retains the original descriptor alongside its playable path.
                        back.as_object_mut().unwrap().remove(photocraft_engine::atn_cmds::steps_path::SOURCE).unwrap();
                    }
                    if *id == "edit.perspectiveWarp" {
                        back.as_object_mut().unwrap().remove("__atnPerspectiveWarp");
                    }
                    normalise(&mut back);
                    if *id == "actions.play" || *id == "actions.conditional" {
                        back["set"] = json!("S");
                    }
                    assert_eq!(back, *params, "{id}");
                }
                Mapped::Unsupported { .. } => panic!("{id} does not import back"),
            }
        }
        // Partial native backgrounds need no recorded preset, guide IDs or last-used color.
        for (background, imported_background) in [("white", "white"), ("black", "black"), ("transparent", "transparent"), ("custom", "white")] {
            let item = export_item("layer.new.artboardFromGroup", &json!({"background":background}), "S").unwrap();
            let Mapped::Steps(steps) = (event_for(&item.event).unwrap().from)(item.descriptor.as_ref().unwrap()) else { panic!("Artboard did not import") };
            assert_eq!(
                steps,
                [("layer.new.artboardFromGroup".into(), json!({"background":imported_background,"color":[1.0,1.0,1.0],"preset":"","guideIDs":[]}))]
            );
            assert_eq!(export_item(&steps[0].0, &steps[0].1, "S").unwrap().descriptor, item.descriptor);
        }
        // Every command the table claims to export has a case above.
        let covered: std::collections::BTreeSet<&str> = cases.iter().map(|(id, _)| *id).collect();
        let mut missing: Vec<&str> = EVENTS.iter().flat_map(|e| e.ids.iter().copied()).filter(|id| !covered.contains(id)).collect();
        missing.sort_unstable();
        let allowed = [
            "edit.transform.flipVertical",
            "edit.transform.rotate180",
            "edit.transform.rotate90Ccw",
            "edit.transform.rotate90Cw",
            "image.autoColor",
            "image.autoContrast",
            "image.imageRotation.180",
            "image.imageRotation.90ccw",
            "image.imageRotation.flipCanvasHorizontal",
            "image.mode.bits32",
            "image.mode.bits8",
            "image.mode.cmyk",
            "image.mode.lab",
            "image.mode.multichannel",
            "image.mode.rgb",
            "layer.align.bottomEdges",
            "layer.align.horizontalCenters",
            "layer.align.rightEdges",
            "layer.align.topEdges",
            "layer.align.verticalCenters",
            "layer.arrange.bringForward",
            "layer.arrange.bringToFront",
            "layer.arrange.sendBackward",
            "layer.mergeLayers",
            "layer.new.layerViaCut",
            "layer.newAdjustmentLayer.blackWhite",
            "layer.newAdjustmentLayer.brightnessContrast",
            "layer.newAdjustmentLayer.channelMixer",
            "layer.newAdjustmentLayer.colorBalance",
            "layer.newAdjustmentLayer.curves",
            "layer.newAdjustmentLayer.exposure",
            "layer.newAdjustmentLayer.invert",
            "layer.newAdjustmentLayer.levels",
            "layer.newAdjustmentLayer.photoFilter",
            "layer.newAdjustmentLayer.posterize",
            "layer.newAdjustmentLayer.selectiveColor",
            "layer.newAdjustmentLayer.vibrance",
            "layer.rasterize.layer",
            "layer.renameLayer",
            "layer.setAdjustment",
            "select.allLayers",
            "select.deselectLayers",
            "select.grow",
            "select.modify.border",
            "select.modify.expand",
            "select.modify.smooth",
            "select.reselect",
            "select.similar",
            "view.actualPixels",
            "view.zoomOut",
            "tools.swapColors",
            "edit.cut",
            "edit.copy",
            "edit.purge.all",
            "edit.purge.clipboard",
            "edit.purge.undo",
            "file.newFromClipboard",
            "file.revert",
            "image.adjustments.equalize",
            "image.revealAll",
            "filter.blur.blurMore",
            "filter.blur.boxBlur",
            "filter.blur.radialBlur",
            "filter.blur.surfaceBlur",
            "filter.noise.despeckle",
            "filter.noise.dustAndScratches",
            "filter.other.highPass",
            "filter.other.maximum",
            "filter.other.minimum",
            "filter.sharpen.sharpen",
            "filter.sharpen.sharpenEdges",
            "filter.sharpen.sharpenMore",
            // Saves back only as the descriptor it was loaded from (loaded_camera_raw_filter_steps_save_back_unchanged).
            "filter.cameraRaw",
            // Load with the recorded descriptor riding along (gradient_fill_steps_export_and_import_back).
            "gradient.fill.set",
            "layer.newFillLayer.gradient",
        ];
        let unexpected: Vec<&str> = missing.iter().copied().filter(|id| !allowed.contains(id)).collect();
        assert!(unexpected.is_empty(), "add round-trip cases for {unexpected:?}");
        // Every command the table names exists.
        // (Save is the desktop shell's: recorded by it, replayed by it, written by Batch.)
        for id in EVENTS.iter().flat_map(|e| e.ids.iter()) {
            assert!(
                photocraft_engine::commands::find(id).is_some()
                    || photocraft_engine::actions_cmds::shell_save_command(id)
                    || photocraft_engine::actions_cmds::shell_view_command(id),
                "the .atn table names an unknown command `{id}`"
            );
        }
    }

    /// `paint.bucket` exported to .atn and imported back → its params, or None when it doesn't export.
    fn bucket_round_trip(params: Value) -> Option<Value> {
        let item = export_item("paint.bucket", &params, "S")?;
        let e = event_for(&item.event)?;
        match (e.from)(item.descriptor.as_ref()?) {
            Mapped::Steps(steps) => steps.first().map(|(id, p)| {
                assert_eq!(id, "paint.bucket");
                p.clone()
            }),
            _ => None,
        }
    }

    /// A Paint Bucket step with an explicit colour keeps it through .atn (#7199): replayed on a fresh
    /// document, whose foreground is black, it still fills red. A colour .atn can't hold makes the
    /// step not export rather than fill with the foreground.
    #[test]
    fn bucket_keeps_an_explicit_color_through_atn() {
        let fill = |params: &Value| {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ffffff"})).unwrap();
            s.execute("paint.bucket", params.clone()).unwrap();
            let d = s.active().unwrap();
            d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(2, 2)
        };
        let red = json!({"x": 2, "y": 2, "antiAlias": false, "color": "#ff0000"});
        let back = bucket_round_trip(red.clone()).unwrap();
        assert_eq!(back["color"], "#ff0000", "{back}");
        assert_eq!(fill(&back), fill(&red));
        assert_eq!(fill(&back), vec![1.0, 0.0, 0.0, 1.0]);
        // The colour as an array of channels.
        assert_eq!(bucket_round_trip(json!({"x": 1, "y": 1, "color": [0.0, 0.5, 1.0]})).unwrap()["color"], "#0080ff");
        // No colour, or one the command can't read: the foreground, as before.
        for p in [json!({"x": 1, "y": 1}), json!({"x": 1, "y": 1, "color": "red"})] {
            let back = bucket_round_trip(p.clone()).unwrap();
            assert!(back.get("color").is_none() && back.get("contents").is_none(), "{p} → {back}");
        }
        // A pattern fill ignores the colour, also when named with the older `source`.
        for key in ["contents", "source"] {
            let back = bucket_round_trip(json!({"x": 1, "y": 1, key: "pattern", "color": "#ff0000"})).unwrap();
            assert_eq!(back["contents"], "pattern", "{key}: {back}");
            assert!(back.get("color").is_none(), "{key}: {back}");
        }
        // Translucent or out-of-range colours have no .atn form.
        for c in [json!("#ff000080"), json!([1.0, 0.0, 0.0, 0.5]), json!([2.0, 0.0, 0.0])] {
            assert!(bucket_round_trip(json!({"x": 1, "y": 1, "color": c})).is_none(), "{c}");
        }
    }

    fn channel_ord(value: &str) -> D {
        D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: code(value) }])
    }

    fn fsel(t: D) -> Descriptor {
        desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with("T   ", t)
    }

    /// #7103: Photoshop records switching to another open document as a `select` of `Dcmn` by
    /// offset (previous -1, next +1). The template action of the report, which copies from the
    /// source image and pastes into the template opened after it, imports with no Photoshop steps,
    /// plays across the two documents and saves back the same shape. With no previous document
    /// open, playback stops at the switch instead of pasting into the wrong document.
    #[test]
    fn select_document_by_offset_imports_plays_and_saves_back() {
        let doc = |offset: i32| desc("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]));
        // A recorded document id (`DocI`) means nothing in another session: ignored, not written back.
        let copy_in = vec![
            item("slct", "Select", Some(doc(-1).with("DocI", long(35.0)))),
            item("setd", "Set", Some(fsel(enm("Ordn", "Al  ")))),
            item("copy", "Copy", None),
            item("slct", "Select", Some(doc(1))),
            item("past", "Paste", None),
        ];
        let data = set_of("Template", vec![("Copy In", copy_in)]);

        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "name": "source"})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8, "name": "template", "background": "transparent"})).unwrap();
        // Reorder tabs before playback: the source remains the previous *opened* document.
        s.execute("document.move", json!({"document": 1, "to": 0})).unwrap();
        let r = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(r["unsupported"], json!([]), "{r}");
        let steps = &s.actions().list.actions()[0].steps;
        assert_eq!(steps[0], ActionStep::new("document.activate", json!({"offset": -1})));
        assert_eq!(steps[3], ActionStep::new("document.activate", json!({"offset": 1})));

        let pixel = |s: &mut Session| -> Vec<f32> { serde_json::from_value(s.execute("document.pixel", json!({"x": 4, "y": 4})).unwrap()).unwrap() };
        assert_eq!(pixel(&mut s), [0.0; 4]);
        let r = s.execute("actions.play", json!({"action": "Copy In"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert_eq!(s.active().unwrap().doc.name, "template");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "the source's pixels are pasted on a new layer");
        assert_eq!(pixel(&mut s), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.documents()[1].doc.layers.len(), 1, "the source is only copied from");

        let first = s.actions().list.actions().to_vec();
        let r = s.execute("actions.save", json!({"set": "Template"})).unwrap();
        assert_eq!(r["skipped"], json!([]));
        let saved = photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap();
        let file = atn::parse(&saved).unwrap();
        let offsets: Vec<i32> = file.actions[0]
            .items
            .iter()
            .filter_map(|i| match get_ref(i.descriptor.as_ref()?, "null")? {
                [ReferenceItem::Offset { class, offset }] if trimmed(&class.class_id) == "Dcmn" => Some(*offset),
                _ => None,
            })
            .collect();
        assert_eq!(offsets, [-1, 1]);
        assert!(file.actions[0].items[0].descriptor.as_ref().unwrap().get("DocI").is_none());
        let mut t = Session::new();
        t.execute("actions.load", json!({"data": b64(&saved)})).unwrap();
        assert_eq!(t.actions().list.actions(), first);
        // An absolute tab index has no Photoshop equivalent: it stays out of a strict save.
        assert!(export_item("document.activate", &json!({"document": 0}), "S").is_none());
        assert!(export_item("document.activate", &json!({"document": 0, "offset": 1}), "S").is_none());

        let mut one = Session::new();
        one.execute("file.new", json!({"width": 8, "height": 8, "name": "template"})).unwrap();
        one.execute("actions.load", json!({"data": data})).unwrap();
        let r = one.execute("actions.play", json!({"action": "Copy In"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert_eq!(one.active().unwrap().doc.layers.len(), 2, "offsets wrap back to the only open document");
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
        let pt = |v: f64| D::UnitFloat { unit: PT, value: v };
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

    /// A local absolute map path on this platform: playback may read only such a path.
    #[cfg(windows)]
    const LOCAL_MAP: &str = r"C:\máp.psd";

    #[cfg(not(windows))]
    const LOCAL_MAP: &str = "/máp.psd";

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

    /// The first saved step of `action` (strict save).
    fn saved_first(s: &mut Session) -> (AtnEvent, Option<Descriptor>) {
        let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
        assert_eq!(r["skipped"], json!([]), "{r}");
        let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
        let i = &saved.actions[0].items[0];
        (i.event.clone(), i.descriptor.clone())
    }

    fn blend_if_entry(channel: &str, this: [i32; 4], under: Option<[i32; 4]>) -> D {
        let mut d = desc("Blnd").with("Chnl", channel_ref(Some(channel)));
        for (k, v) in ["SrcB", "Srcl", "SrcW", "Srcm"].into_iter().zip(this) {
            d = d.with(k, D::Integer(v));
        }
        if let Some(u) = under {
            for (k, v) in ["DstB", "Dstl", "DstW", "Dstt"].into_iter().zip(u) {
                d = d.with(k, D::Integer(v));
            }
        }
        D::Descriptor(d)
    }

    fn layer_set(t: Descriptor) -> AtnItem {
        item("setd", "Set", Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(t))))
    }

    fn blend_layers(depth: u8) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
        s
    }

    fn active_info(s: &Session) -> Value {
        let st = s.active().unwrap();
        photocraft_engine::inspect::layer(st.doc.layer(st.active_layer.unwrap()).unwrap())
    }

    /// #10771: Blend If loads as `layer.layerStyle.blendingOptions` and plays (it used to be a
    /// silent no-op), on 8- and 16-bit documents, Gray and a colour channel, split sliders.
    #[test]
    fn blend_if_imports_and_plays() {
        let blnd = D::List(vec![blend_if_entry("gray", [0, 30, 200, 255], Some([10, 10, 240, 250])), blend_if_entry("red", [20, 40, 255, 255], None)]);
        for depth in [8, 16] {
            let mut s = blend_layers(depth);
            let loaded = s
                .execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(desc("Lyr ").with("Blnd", blnd.clone()))])).unwrap())}))
                .unwrap();
            assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
            let mut expected = json!({"blendIf": [{"channel": "gray", "thisLayer": [0, 30, 200, 255], "underlying": [10, 10, 240, 250]}, {"channel": "red", "thisLayer": [20, 40, 255, 255]}]});
            normalise(&mut expected);
            let ActionStep { command: id, params: p } = &s.actions().list.actions()[0].steps[0];
            assert_eq!((id.as_str(), p), ("layer.layerStyle.blendingOptions", &expected));
            let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
            assert!(played.get("failed").is_none(), "{played}");
            let b = active_info(&s)["blendIf"].clone();
            assert_eq!(b[0], json!({"channel": 0, "thisLayer": [0, 30, 200, 255], "underlying": [10, 10, 240, 250]}), "{b}");
            assert_eq!(b[1], json!({"channel": 1, "thisLayer": [20, 40, 255, 255], "underlying": [0, 0, 255, 255]}), "{b}");
            // It saves back as the same `Blnd`.
            let (_, d) = saved_first(&mut s);
            assert_eq!(get_obj(d.as_ref().unwrap(), "T   ").unwrap().get("Blnd"), Some(&blnd));
        }
    }

    /// #10771: the Blending Options dialog's OK (opacity, fill, mode and Blend If in one Set) is one
    /// step that applies them all; a Set that also renames splits into setProps and Blend If.
    #[test]
    fn blend_if_with_the_dialog_options_is_one_step_and_a_rename_splits() {
        let blnd = D::List(vec![blend_if_entry("gray", [0, 0, 128, 255], None)]);
        let dialog = desc("Lyr ").with("Opct", pct(40.0)).with("fillOpacity", pct(70.0)).with("Md  ", enm("BlnM", "Mltp")).with("Blnd", blnd.clone());
        let mut s = blend_layers(8);
        s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(dialog.clone())])).unwrap())})).unwrap();
        let steps = &s.actions().list.actions()[0].steps;
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert_eq!(steps[0].command, "layer.layerStyle.blendingOptions");
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let info = active_info(&s);
        assert!((info["opacity"].as_f64().unwrap() - 0.4).abs() < 1e-3, "{info}");
        assert_eq!(info["blend"], json!("Multiply"), "{info}");
        assert_eq!(info["blendIf"][0]["thisLayer"], json!([0, 0, 128, 255]), "{info}");
        let (_, d) = saved_first(&mut s);
        assert_eq!(get_obj(d.as_ref().unwrap(), "T   "), Some(&dialog));
        // With a rename the Set is not the dialog's: the name and opacity go through setProps.
        let mut s = blend_layers(8);
        s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(desc("Lyr ").with("Nm  ", text("Renamed")).with("Opct", pct(40.0)).with("Blnd", blnd))])).unwrap())}))
        .unwrap();
        let ids: Vec<&str> = s.actions().list.actions()[0].steps.iter().map(|st| st.command.as_str()).collect();
        assert!(ids.contains(&"layer.setProps") && ids.contains(&"layer.layerStyle.blendingOptions"), "{ids:?}");
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let info = active_info(&s);
        assert_eq!(info["name"], json!("Renamed"));
        assert_eq!(info["blendIf"][0]["thisLayer"], json!([0, 0, 128, 255]), "{info}");
    }

    /// #10771: the Layer Style dialog's OK on a layer without effects also records the effects'
    /// scale (`Lefx` = {Scl}). With Blend If it is still the dialog's one step and saves back with it.
    #[test]
    fn blend_if_with_the_recorded_effects_scale_saves_back_unchanged() {
        let blnd = D::List(vec![blend_if_entry("gray", [10, 20, 200, 230], Some([0, 0, 255, 255]))]);
        let dialog = desc("Lyr ").with("Opct", pct(80.0)).with("Blnd", blnd.clone()).with("Lefx", D::Descriptor(desc("Lefx").with("Scl ", pct(100.0))));
        let mut s = blend_layers(8);
        s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(dialog.clone())])).unwrap())})).unwrap();
        let steps = &s.actions().list.actions()[0].steps;
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert_eq!(steps[0].command, "layer.layerStyle.blendingOptions");
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let info = active_info(&s);
        assert!((info["opacity"].as_f64().unwrap() - 0.8).abs() < 1e-3, "{info}");
        assert_eq!(info["blendIf"][0]["thisLayer"], json!([10, 20, 200, 230]), "{info}");
        let (_, d) = saved_first(&mut s);
        assert_eq!(get_obj(d.as_ref().unwrap(), "T   "), Some(&dialog));
        // A `Lefx` with an effect in it is not the scale alone: the step is not the dialog's.
        let with_fx =
            desc("Lyr ").with("Blnd", blnd).with("Lefx", D::Descriptor(desc("Lefx").with("Scl ", pct(100.0)).with("DrSh", D::Descriptor(desc("DrSh")))));
        let imp = import(&atn::write(&one_action(vec![layer_set(with_fx)])).unwrap()).unwrap();
        assert!(imp.actions[0].steps.iter().all(|st| st.params.get("__atnEffectsScale").is_none()), "{:?}", imp.actions[0].steps);
        // Nor can it play without the shadow: it stays the recorded step.
        assert_eq!(imp.actions[0].steps.iter().map(|st| st.command.as_str()).collect::<Vec<_>>(), [PHOTOSHOP_STEP], "{:?}", imp.actions[0].steps);
    }
}
