//! Generated names must be stored in the document, so exact action references can use them.
use photocraft_engine::Session;
use photocraft_ui_egui::{
    PhotocraftApp, Services,
    control::{ControlRequest, Outcome, handle},
};
use serde_json::{Value, json};

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _reply) = ControlRequest::new(method, params);
    let Outcome::Done(reply) = handle(app, ctx, &request) else { panic!("{method} did not finish") };
    assert_eq!(reply["ok"], true, "{method}: {reply}");
    reply["result"].clone()
}

fn create(app: &mut PhotocraftApp, ctx: &egui::Context, expected: &str) {
    call(app, ctx, "ui.menu.invoke", json!({"id":"file.new"}));
    let dialog = app.ui.dialogs.last_mut().unwrap();
    assert_eq!(dialog.fields["name"], expected);
    dialog.fields.insert("width".into(), json!(4));
    dialog.fields.insert("height".into(), json!(4));
    let id = dialog.id;
    call(app, ctx, "ui.dialog.confirm", json!({"dialog":id}));
    assert_eq!(app.session.active().unwrap().doc.name, expected);
}

#[test]
fn new_document_suggests_unique_default_names() {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    let ctx = egui::Context::default();
    for expected in ["Untitled-1", "Untitled-2", "Untitled-3"] {
        create(&mut app, &ctx, expected);
    }
    app.session.close(0).unwrap();
    create(&mut app, &ctx, "Untitled-4");
    app.session.close(2).unwrap();
    create(&mut app, &ctx, "Untitled-4");
    while !app.session.documents().is_empty() {
        app.session.close(0).unwrap();
    }
    create(&mut app, &ctx, "Untitled-1");
}

#[test]
fn localized_default_names_are_persisted_without_editing_the_name_field() {
    use photocraft_ui_egui::i18n::{Lang, tr};
    for lang in Lang::all() {
        let language = lang.code();
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.prefs.edit(|p| p.interface.language = language.into());
        let ctx = egui::Context::default();
        let base = tr(lang, "Untitled");
        create(&mut app, &ctx, &format!("{base}-1"));
        app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, format!("{base}-2"));
        app.session.execute("select.all", json!({})).unwrap();
        app.session.execute("edit.copy", json!({})).unwrap();
        app.session.execute("file.newFromClipboard", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, format!("{base}-3"));
    }
}

#[test]
fn reserved_stems_legacy_names_and_extreme_numbers_do_not_collide() {
    use photocraft_engine::file_cmds::untitled_document_name_with;
    let mut s = Session::new();
    for (names, expected) in [
        (vec!["Untitled", "Untitled-3.psd"], "Untitled-4"),
        (vec!["Untitled-1", "Untitled-18446744073709551615.psb"], "Untitled-2"),
        (vec!["Untitled-4294967295"], "Untitled-4294967296"),
        (vec!["Untitled-0", "Other-8", "Untitled-not-a-number"], "Untitled-1"),
    ] {
        assert_eq!(untitled_document_name_with(&s, names.iter().copied()), expected);
    }
    s.default_name_base = Some(|_, _, _| "Draft.v2".into());
    assert_eq!(untitled_document_name_with(&s, ["Draft.v2-1"].into_iter()), "Draft.v2-2");
    assert_eq!(untitled_document_name_with(&s, ["Draft.v2-1", "Draft.v2-3.psd"].into_iter()), "Draft.v2-4");
}

#[test]
fn language_changes_only_affect_future_default_names_and_custom_names_stay_literal() {
    use photocraft_ui_egui::i18n::{Lang, tr};
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    create(&mut app, &ctx, "Untitled-1");
    for language in ["ja", "de", "zh-hans", "en"] {
        app.session.prefs.edit(|p| p.interface.language = language.into());
        let before: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
        // Explicit names may resemble another language's default and still remain literal.
        for name in ["Untitled-7", "未标题-9", "Custom.psd"] {
            app.session.execute("file.new", json!({"width":4,"height":4,"name":name})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.name, name);
            app.session.close(app.session.documents().len() - 1).unwrap();
        }
        let base = tr(Lang::from_pref(language), "Untitled");
        let expected = if base == "Untitled" { "Untitled-2".to_string() } else { format!("{base}-1") };
        create(&mut app, &ctx, &expected);
        assert_eq!(app.session.documents().iter().take(before.len()).map(|d| d.doc.name.clone()).collect::<Vec<_>>(), before);
        app.session.close(app.session.documents().len() - 1).unwrap();
    }
}

#[test]
fn templates_reserve_the_name_of_an_open_new_document_dialog() {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    app.services.import = Some(Box::new(|name, bytes, options| {
        photocraft_io::import_with_options(name, bytes, options).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
    }));
    let mut source = Session::new();
    source.execute("file.new", json!({"width":4,"height":4})).unwrap();
    let bytes = photocraft_io::export(&source.active().unwrap().doc, "template.psd", &Default::default()).unwrap().bytes;
    let ctx = egui::Context::default();
    call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"file.new"}));
    assert_eq!(app.ui.dialogs.last().unwrap().fields["name"], "Untitled-1");
    for expected in ["Untitled-2", "Untitled-3"] {
        app.open_file("template.psdt", &bytes).unwrap();
        assert_eq!(app.session.active().unwrap().doc.name, expected);
        assert!(app.session.active().unwrap().path.is_none());
    }
}

/// Saved bytes exercise Photoshop Make, document selection and exact named duplication together.
#[test]
fn unnamed_atn_documents_can_be_targeted_by_the_localized_default_name() {
    use photocraft_engine::{actions_cmds::Action, atn_cmds};
    use photocraft_ui_egui::i18n::{Lang, tr};
    for language in ["en", "de", "ja", "zh-hans", "zh-hant"] {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.prefs.edit(|p| p.interface.language = language.into());
        app.session.execute("file.new", json!({"width":4,"height":4,"name":"Source","background":"#ff0000"})).unwrap();
        let source = app.session.active().unwrap().doc.id;
        let name = format!("{}-1", tr(Lang::from_pref(language), "Untitled"));
        let action = Action::new(
            "Place",
            vec![
                ("file.new".into(), json!({"width":4,"height":4})),
                ("document.activate".into(), json!({"offset":-1})),
                ("layer.duplicate".into(), json!({"document":{"name":name},"name":"Placed"})),
            ],
        );
        let (bytes, skipped) = atn_cmds::export("Naming", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty());
        assert!(imported.actions[0].steps[0].1.get("name").is_none());
        app.session.actions.list = imported.actions;
        let result = app.session.execute("actions.play", json!({"action":"Place"})).unwrap();
        assert!(result.get("failed").is_none(), "{language}: {result}");
        assert_eq!(app.session.active().unwrap().doc.id, source);
        let target = app.session.documents().iter().find(|d| d.doc.name == name).unwrap();
        assert_eq!(target.doc.layers.len(), 2);
        assert!(target.doc.layers.iter().any(|l| l.name == "Placed"));
    }
}

#[test]
fn batch_scratch_sessions_keep_the_default_name_language() {
    use photocraft_engine::actions_cmds::Action;
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"name":"Input"})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "input.png", &Default::default()).unwrap().bytes;
    let input = dir.path().join("input.png");
    std::fs::write(&input, bytes).unwrap();
    // The engine receives a language service, not a dependency on any particular catalog.
    s.default_name_base = Some(|language, _, english| if language == "de" { format!("Fixture {english}") } else { english.into() });
    s.prefs.edit(|p| p.interface.language = "de".into());
    s.actions.list.push(Action::new(
        "Place",
        vec![
            ("file.new".into(), json!({"width":4,"height":4})),
            ("document.activate".into(), json!({"offset":-1})),
            ("layer.duplicate".into(), json!({"document":{"name":"Fixture Untitled-1"}})),
            ("layer.newFillLayer.solidColor".into(), json!({"color":"#e9ebee"})),
            ("layer.hideLayers".into(), json!({"layer":"Fixture Color Fill 1"})),
        ],
    ));
    let result = s.execute("file.automate.batch", json!({"action":"Place","input":[input],"output":dir.path().join("out")})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
}

/// An unnamed Make must create the actual name a later named Hide references.
#[test]
fn unnamed_atn_fill_layers_can_be_targeted_by_the_localized_default_name() {
    use photocraft_engine::{actions_cmds::Action, atn_cmds};
    use photocraft_ui_egui::i18n::{Lang, tr};
    for lang in Lang::all() {
        for (cmd, base, params) in [
            ("layer.newFillLayer.solidColor", "Color Fill", json!({"color":"#e9ebee"})),
            ("layer.newFillLayer.gradient", "Gradient Fill", json!({"from":"#000000","to":"#ffffff"})),
            ("layer.newFillLayer.pattern", "Pattern Fill", json!({"pattern":"Checkerboard"})),
        ] {
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            app.session.prefs.edit(|p| p.interface.language = lang.code().into());
            app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
            let name = format!("{} 1", tr(lang, base));
            let action = Action::new(
                "Fill",
                vec![(cmd.into(), params), ("layer.hideLayers".into(), json!({"layer":name})), ("layer.new.layer".into(), json!({"name":"After"}))],
            );
            let (bytes, skipped) = atn_cmds::export("Naming", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty());
            let imported = atn_cmds::import(&bytes).unwrap();
            assert!(imported.unsupported.is_empty());
            assert!(imported.actions[0].steps[0].1.get("name").is_none());
            app.session.actions.list = imported.actions;
            let result = app.session.execute("actions.play", json!({"action":"Fill"})).unwrap();
            assert!(result.get("failed").is_none(), "{}: {result}", lang.code());
            let st = app.session.active().unwrap();
            let fill = st.doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2;
            assert!(!fill.visible);
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "After");
        }
    }
}

#[test]
fn fill_names_keep_numbering_explicit_names_and_preview_consistent() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.default_name_base = Some(|lang, _, base| format!("{lang}:{base}"));
        s.prefs.edit(|p| p.interface.language = "de".into());
        s.execute("file.new", json!({"width":4,"height":4,"depth":depth})).unwrap();
        for (cmd, base, params) in [
            ("layer.newFillLayer.solidColor", "Color Fill", json!({"color":"#e9ebee"})),
            ("layer.newFillLayer.gradient", "Gradient Fill", json!({"from":"#000000","to":"#ffffff"})),
            ("layer.newFillLayer.pattern", "Pattern Fill", json!({"pattern":"Checkerboard"})),
        ] {
            for n in [1, 2] {
                s.execute(cmd, params.clone()).unwrap();
                let st = s.active().unwrap();
                assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, format!("de:{base} {n}"));
            }
            let before = s.active().unwrap().doc.clone();
            s.undo();
            s.redo();
            assert_eq!(*s.active().unwrap().doc, *before);
        }
        // A live Gradient and a preset use the same sequence as Layer > New Fill Layer.
        let p = json!({"from":[0,0],"to":[4,4]});
        let shown = photocraft_engine::gradient_fill_cmds::new_layer(&s, &s.active().unwrap().doc, &p).unwrap();
        assert_eq!(shown.name, "de:Gradient Fill 3");
        s.execute("gradient.fill.create", p).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().name, shown.name);
        s.execute("gradient.presets.apply", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().name, "de:Gradient Fill 4");
        s.execute("layer.new.layer", json!({"name":"Literal"})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let params = json!({"pattern":"Checkerboard"});
        let preview = photocraft_engine::pattern_fill_cmds::preview(&s, "layer.newFillLayer.pattern", &params).unwrap();
        s.execute("layer.newFillLayer.pattern", params).unwrap();
        assert_eq!(*s.active().unwrap().doc, preview);
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().name, "Literal");
        s.prefs.edit(|p| p.interface.language = "ja".into());
        s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "ja:Color Fill 1");
        assert!(st.doc.walk().iter().any(|(_, _, l)| l.name == "de:Color Fill 1"));
    }
}

#[test]
fn absent_or_empty_name_translations_keep_english_defaults() {
    let mut s = Session::new();
    s.prefs.edit(|p| p.interface.language = "zh-hans".into());
    for empty_service in [false, true] {
        if empty_service {
            s.default_name_base = Some(|_, _, _| String::new());
        }
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.name, format!("Untitled-{}", if empty_service { 2 } else { 1 }));
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Color Fill 1");
    }
}

#[test]
fn generated_backgrounds_can_be_targeted_by_recorded_localized_names() {
    use photocraft_engine::{actions_cmds::Action, atn_cmds};
    use photocraft_ui_egui::i18n::{Lang, tr};
    for lang in Lang::all() {
        for depth in [8, 16, 32] {
            for command in ["file.new", "layer.flattenImage"] {
                let mut app = PhotocraftApp::new(Session::new(), Services::default());
                app.session.prefs.edit(|p| p.interface.language = lang.code().into());
                app.session.execute("file.new", json!({"width":4,"height":4,"depth":depth,"background":"transparent"})).unwrap();
                let name = tr(lang, "Background");
                let params = if command == "file.new" { json!({"width":4,"height":4,"depth":depth}) } else { json!({}) };
                let action = Action::new(
                    "Background",
                    vec![(command.into(), params), ("layer.select".into(), json!({"layer":name})), ("layer.new.layer".into(), json!({"name":"After"}))],
                );
                let (bytes, skipped) = atn_cmds::export("Naming", &[&action], &[], false).unwrap();
                assert!(skipped.is_empty(), "{command}: {skipped:?}");
                let imported = atn_cmds::import(&bytes).unwrap();
                assert!(imported.unsupported.is_empty());
                app.session.actions.list = imported.actions;
                let result = app.session.execute("actions.play", json!({"action":"Background"})).unwrap();
                assert!(result.get("failed").is_none(), "{} {depth} {command}: {result}", lang.code());
                let st = app.session.active().unwrap();
                let bg = st.doc.background().unwrap();
                assert_eq!(bg.name, name);
                assert!(bg.locks.transparency && bg.locks.position);
                assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "After");
            }
        }
    }
}

#[test]
fn background_conversion_localizes_new_names_and_undo_restores_the_original() {
    use photocraft_ui_egui::i18n::{Lang, tr};
    for lang in Lang::all() {
        for depth in [8, 16, 32] {
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            app.session.prefs.edit(|p| p.interface.language = lang.code().into());
            app.session.execute("file.new", json!({"width":4,"height":4,"depth":depth,"background":"transparent"})).unwrap();
            app.session.execute("layer.renameLayer", json!({"name":"Literal"})).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            app.session.execute("layer.new.backgroundFromLayer", json!({})).unwrap();
            let converted = app.session.active().unwrap().doc.clone();
            assert_eq!(converted.background().unwrap().name, tr(lang, "Background"));
            app.session.undo();
            assert_eq!(*app.session.active().unwrap().doc, *before);
            app.session.redo();
            assert_eq!(*app.session.active().unwrap().doc, *converted);
            app.session.prefs.edit(|p| p.interface.language = "en".into());
            assert_eq!(*app.session.active().unwrap().doc, *converted, "language changes do not rename existing layers");
        }
    }
}

#[test]
fn imported_background_names_remain_literal() {
    use photocraft_ui_egui::i18n::Lang;
    let mut source = Session::new();
    source.execute("file.new", json!({"width":4,"height":4})).unwrap();
    let bytes = photocraft_io::export(&source.active().unwrap().doc, "source.psd", &Default::default()).unwrap().bytes;
    for lang in Lang::all() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.prefs.edit(|p| p.interface.language = lang.code().into());
        app.services.import = Some(Box::new(|name, bytes, options| {
            photocraft_io::import_with_options(name, bytes, options).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        }));
        app.open_file("source.psd", &bytes).unwrap();
        assert_eq!(app.session.active().unwrap().doc.background().unwrap().name, "Background");
    }
}

fn naming_session() -> Session {
    let mut s = Session::new();
    s.default_name_base = Some(|language, _, key| format!("{language}[{key}]"));
    s.prefs.edit(|p| p.interface.language = "de".into());
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    s
}

fn active_name(s: &Session) -> &str {
    let st = s.active().unwrap();
    &st.doc.layer(st.active_layer.unwrap()).unwrap().name
}

#[test]
fn generated_svg_names_follow_the_session_and_preserve_source_ids() {
    use photocraft_engine::{edit_cmds::Clip, smart_cmds};
    let options = naming_session().import_options();
    let grouped =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><g opacity="0.5"><path id="Shape" d="M0 0H4V4H0Z"/><path d="M4 4H8V8H4Z"/></g></svg>"#;
    let imported = photocraft_io::import_with_options("source.svg", grouped, &options).unwrap();
    let names: Vec<_> = imported.document.walk().into_iter().map(|(_, _, l)| l.name.as_str()).collect();
    assert_eq!(names, ["de[Group] 1", "Shape", "de[Shape] 1"]);
    let collapsed = photocraft_io::svg::import_svg_bounded_with_names("source.svg", grouped, 64, &options.generated_layer_names).unwrap();
    assert_eq!(collapsed.document.layers[0].name, "de[Background]");
    for source_id in ["", " id=\"Shape\""] {
        let bytes = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><path{source_id} d="M0 0H4V4H0Z"/></svg>"#).into_bytes();
        let expected = if source_id.is_empty() { "de[Shape] 1" } else { "Shape" };
        let mut s = naming_session();
        let options = s.import_options();
        let imported = photocraft_io::import_with_options("source.svg", &bytes, &options).unwrap();
        assert_eq!(imported.document.layers[0].name, expected);
        for imported in [
            photocraft_io::svg::import_svg_bounded_with_names("source.svg", &bytes, 64, &options.generated_layer_names).unwrap(),
            photocraft_io::svg::import_svg_bounded_images_with_names("source.svg", &bytes, 64, &options.generated_layer_names).unwrap(),
        ] {
            assert_eq!(imported.document.layers[0].name, expected);
        }
        for operation in ["paste", "editContents", "convertToLayers"] {
            s = naming_session();
            s.clipboard = Some(Clip::from_svg(&bytes).unwrap());
            s.execute("edit.paste", json!({"as":"shape"})).unwrap();
            assert_eq!(active_name(&s), expected);
            if operation != "paste" {
                s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
                let id = s.active().unwrap().active_layer.unwrap();
                smart_cmds::replace_contents_bytes(&mut s, id, "source.svg", bytes.clone()).unwrap();
                s.execute(&format!("layer.smartObjects.{operation}"), json!({})).unwrap();
                assert_eq!(active_name(&s), expected, "{operation}");
            }
        }
    }
}

#[test]
fn other_generated_layer_bases_use_the_same_language_service() {
    for (command, params, expected) in [
        ("layer.new.layer", json!({}), "de[Layer] 1"),
        ("layer.new.group", json!({}), "de[Group] 1"),
        ("layer.newAdjustmentLayer.levels", json!({}), "de[Levels] 1"),
        ("layer.newAdjustmentLayer.curves", json!({}), "de[Curves] 1"),
        ("shape.create", json!({"kind":"rect","rect":[0,0,4,4]}), "de[Rectangle] 1"),
        ("layer.new.artboard", json!({}), "de[Artboard] 1"),
        ("filter.render.flame", json!({"newLayer":true}), "de[Flame…]"),
        ("filter.render.tree", json!({"newLayer":true,"leavesAmount":0}), "de[Tree…]"),
        ("filter.render.pictureFrame", json!({"newLayer":true}), "de[Picture Frame…]"),
    ] {
        let mut s = naming_session();
        s.execute(command, params).unwrap();
        assert_eq!(active_name(&s), expected, "{command}");
    }
}

#[test]
fn implicit_background_conversions_localize_layer_zero() {
    for (command, params) in [
        ("layer.new.layerFromBackground", json!({})),
        ("image.canvasSize", json!({"width":10,"height":10,"extensionColor":"transparent"})),
        ("image.crop", json!({"x":0,"y":0,"width":8,"height":8,"deleteCroppedPixels":false})),
    ] {
        let mut s = naming_session();
        s.execute(command, params).unwrap();
        assert_eq!(active_name(&s), "de[Layer] 0", "{command}");
    }
}

#[test]
fn duplicate_names_localize_only_the_generated_suffix() {
    let mut s = naming_session();
    s.execute("layer.new.layer", json!({"name":"Literal"})).unwrap();
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(active_name(&s), "Literal de[copy]");
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(active_name(&s), "Literal de[copy] 2");
    s.execute("layer.duplicate", json!({"name":"Custom copy"})).unwrap();
    assert_eq!(active_name(&s), "Custom copy");
}

#[test]
fn localized_default_dialog_names_are_the_values_committed() {
    use photocraft_ui_egui::i18n::{Lang, tr_ctx};
    for lang in Lang::all() {
        for (command, key) in [("layer.new.layer", "Layer"), ("layer.new.group", "Group"), ("layer.duplicate", "copy")] {
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            app.session.prefs.edit(|p| p.interface.language = lang.code().into());
            app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
            app.session.execute("layer.new.layer", json!({"name":"Literal"})).unwrap();
            let ctx = egui::Context::default();
            call(&mut app, &ctx, "ui.menu.invoke", json!({"id":command}));
            let dialog = app.ui.dialogs.last().unwrap();
            let base = tr_ctx(lang, "layerName", key);
            let expected = if command == "layer.duplicate" { format!("Literal {base}") } else { format!("{base} 1") };
            assert_eq!(dialog.fields["name"], expected, "{} {command}", lang.code());
            let id = dialog.id;
            call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":id}));
            assert_eq!(active_name(&app.session), expected);
        }
    }
}

#[test]
fn localized_empty_layer_detection_agrees_with_pattern_preview() {
    let mut s = naming_session();
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let p = json!({"pattern":"Checkerboard"});
    let preview = photocraft_engine::pattern_fill_cmds::preview(&s, "layer.newFillLayer.pattern", &p).unwrap();
    let result = s.execute("layer.newFillLayer.pattern", p).unwrap();
    assert_eq!(result["layer"], id, "the unnamed empty pixel layer itself becomes the fill");
    assert_eq!(active_name(&s), "de[Pattern Fill] 1");
    assert_eq!(*s.active().unwrap().doc, preview);
}

#[test]
fn localized_type_copy_names_follow_text_but_explicit_names_remain_literal() {
    let mut s = naming_session();
    s.execute("type.create", json!({"x":0,"y":4,"text":"Title"})).unwrap();
    s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(active_name(&s), "Title de[copy]");
    s.execute("type.setStyle", json!({"size":10})).unwrap();
    assert_eq!(active_name(&s), "Title de[copy]", "a style edit keeps the copy name");
    s.execute("type.edit", json!({"text":"Levels"})).unwrap();
    assert_eq!(active_name(&s), "Levels", "text content is never translated");
    s.execute("type.edit", json!({"name":"Literal","text":"Other"})).unwrap();
    s.execute("type.edit", json!({"text":"Again"})).unwrap();
    assert_eq!(active_name(&s), "Literal");
    s.execute("type.create", json!({"x":0,"y":4,"text":""})).unwrap();
    assert_eq!(active_name(&s), "de[Type Layer]");
}

#[test]
fn recorded_background_properties_target_the_converted_id_in_every_language() {
    use photocraft_color::BlendMode;
    use photocraft_engine::{actions_cmds::Action, atn_cmds};
    use photocraft_ui_egui::i18n::Lang;
    for lang in Lang::all() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.prefs.edit(|p| p.interface.language = lang.code().into());
        app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        let background = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.new.layer", json!({"name":"Selected"})).unwrap();
        let selected = app.session.active().unwrap().active_layer;
        let p = json!({"ifPresent":true,"select":false,"name":"Converted","opacity":0.5,"blend":"Multiply"});
        let action = Action::new("Convert", vec![("layer.new.layerFromBackground".into(), p.clone())]);
        let (bytes, skipped) = atn_cmds::export("Naming", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let loaded = atn_cmds::import(&bytes).unwrap();
        assert!(loaded.unsupported.is_empty());
        assert_eq!(loaded.actions[0].steps, action.steps);
        let (again, _) = atn_cmds::export("Naming", &[&loaded.actions[0]], &[], false).unwrap();
        assert_eq!(atn_cmds::import(&again).unwrap().actions[0].steps, action.steps);
        let before = app.session.active().unwrap().doc.clone();
        app.session.execute("layer.new.layerFromBackground", p).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.active_layer, selected);
        let layer = st.doc.layer(background).unwrap();
        assert_eq!((layer.name.as_str(), layer.opacity, layer.blend), ("Converted", 0.5, BlendMode::Multiply));
        app.session.undo();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.session.actions.list = loaded.actions;
        let result = app.session.execute("actions.play", json!({"action":"Convert"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(app.session.active().unwrap().active_layer, selected);
    }
    let mut s = naming_session();
    let before = s.active().unwrap().doc.clone();
    for p in [json!({"name":17}), json!({"opacity":-1}), json!({"blend":"invalid"})] {
        assert!(s.execute("layer.new.layerFromBackground", p).is_err());
        assert_eq!(*s.active().unwrap().doc, *before);
    }
}

#[test]
fn recorded_background_properties_keep_the_existing_editable_layer_case() {
    let mut s = naming_session();
    s.execute("layer.new.layerFromBackground", json!({})).unwrap();
    let zero = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name":"Selected"})).unwrap();
    let selected = s.active().unwrap().active_layer;
    let before = s.active().unwrap().doc.clone();
    let p = json!({"ifPresent":true,"select":false,"name":"Converted","opacity":0.5});
    s.execute("layer.new.layerFromBackground", p.clone()).unwrap();
    assert_eq!(s.active().unwrap().active_layer, selected);
    assert_eq!(s.active().unwrap().doc.layer(zero).unwrap().name, "Converted");
    s.undo();
    assert_eq!(*s.active().unwrap().doc, *before);
    s.execute("layer.new.layer", json!({"name":"de[Layer] 0"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("layer.new.layerFromBackground", p).is_err(), "an ambiguous name cannot choose a different layer");
    assert_eq!(*s.active().unwrap().doc, *before);
}

#[test]
fn decoder_generated_names_are_localized_without_renaming_layered_file_content() {
    use photocraft_io::{ExportOptions, import_with_options};
    let mut s = naming_session();
    let opts = s.import_options();
    s.execute("file.new", json!({"width":4,"height":4,"background":"transparent"})).unwrap();
    let source = &s.active().unwrap().doc;
    for (ext, expected) in [("jpg", "de[Background]"), ("png", "de[Layer] 0"), ("tiff", "de[Layer] 0")] {
        let file = format!("flat.{ext}");
        let bytes = photocraft_io::export(source, &file, &ExportOptions::default()).unwrap().bytes;
        let loaded = import_with_options(&file, &bytes, &opts).unwrap();
        assert_eq!(loaded.document.layers[0].name, expected, "{ext}");
        if ext == "png" {
            let mut stepper = photocraft_io::PngImport::with_names(&file, &std::sync::Arc::new(bytes), opts.generated_layer_names.clone()).unwrap().unwrap();
            let loaded = loop {
                if let Some(loaded) = stepper.step(&mut || true).unwrap() {
                    break loaded;
                }
            };
            assert_eq!(loaded.document.layers[0].name, expected);
        }
    }
    let mut flattened = source.as_ref().clone();
    flattened.layers.clear();
    let bytes = photocraft_io::export(&flattened, "flat.psd", &ExportOptions::default()).unwrap().bytes;
    let flat = import_with_options("flat.psd", &bytes, &opts).unwrap();
    assert_eq!(flat.document.layers[0].name, "de[Background]", "a PSD with no saved layers generates a new name");
    let dir = tempfile::tempdir().unwrap();
    let exr = dir.path().join("flat.exr");
    let bytes = photocraft_io::export(source, "flat.exr", &ExportOptions::default()).unwrap().bytes;
    std::fs::write(&exr, bytes).unwrap();
    let opened = photocraft_io::import_file("flat.exr", &exr, &opts, &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(opened.document.layers[0].name, "de[Background]", "streamed EXR uses the same policy");
    s.execute("layer.renameLayer", json!({"name":"Layer 0"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Background"})).unwrap();
    for ext in ["psd", "tiff"] {
        let file = format!("saved.{ext}");
        let export = ExportOptions { tiff_layers: true, ..Default::default() };
        let bytes = photocraft_io::export(&s.active().unwrap().doc, &file, &export).unwrap().bytes;
        let loaded = import_with_options(&file, &bytes, &opts).unwrap();
        assert_eq!(loaded.document.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Layer 0", "Background"]);
        if ext == "psd" {
            let renamed = import_with_options("misleading.png", &bytes, &opts).unwrap();
            assert_eq!(
                renamed.document.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
                loaded.document.layers.iter().map(|l| &l.name).collect::<Vec<_>>(),
                "content detection preserves saved names"
            );
        }
    }
}

#[test]
fn generated_name_templates_do_not_reinterpret_user_text() {
    let s = naming_session();
    assert_eq!(s.default_names().format("{name}'s {effect}", &[("name", "Literal {effect}"), ("effect", "Glow")]), "de[Literal {effect}'s Glow]");
    let mut s = s;
    s.default_name_base = Some(|_, context, key| format!("{context}:{key}"));
    assert_eq!(s.default_names().numbered_in("videoFrame", "Frame", 1), "videoFrame:Frame 1");
    assert_eq!(s.default_names().numbered("Frame", 1), "layerName:Frame 1");
}

#[test]
fn reorder_copy_preview_and_commit_use_the_same_suffix() {
    let mut s = naming_session();
    let layer = s.execute("layer.new.layer", json!({"name":"Literal"})).unwrap()["layer"].clone();
    let target = s.execute("layer.new.layer", json!({"name":"Target"})).unwrap()["layer"].clone();
    let params = json!({"layers":[layer],"target":target,"position":"above","copy":true});
    let preview = photocraft_engine::layer_multi_cmds::preview_move_with_names(&s.active().unwrap().doc, &params, true, &s.default_names()).unwrap();
    s.execute("layer.moveTo", params).unwrap();
    let names = |doc: &photocraft_doc::Document| doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&s.active().unwrap().doc), names(&preview));
    assert!(preview.layers.iter().any(|l| l.name == "Literal de[copy]"));
}

#[test]
fn type_auto_names_survive_switching_interface_languages() {
    use photocraft_ui_egui::i18n::Lang;
    for lang in Lang::all() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.prefs.edit(|p| p.interface.language = lang.code().into());
        app.session.execute("file.new", json!({"width":8,"height":8})).unwrap();
        app.session.execute("type.create", json!({"x":0,"y":4,"text":"Title"})).unwrap();
        app.session.execute("layer.duplicate", json!({})).unwrap();
        let copy = active_name(&app.session).to_string();
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.session.execute("type.setStyle", json!({"size":10})).unwrap();
        assert_eq!(active_name(&app.session), copy);
        app.session.execute("type.edit", json!({"text":"Revised"})).unwrap();
        assert_eq!(active_name(&app.session), "Revised", "{} copy", lang.code());
        app.session.execute("type.edit", json!({"name":copy,"text":"Title"})).unwrap();
        app.session.execute("type.edit", json!({"text":"Literal"})).unwrap();
        assert_eq!(active_name(&app.session), copy, "an explicit copy-looking name remains literal");
        app.session.prefs.edit(|p| p.interface.language = lang.code().into());
        app.session.execute("type.create", json!({"x":0,"y":4,"text":""})).unwrap();
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.session.execute("type.edit", json!({"text":"Filled"})).unwrap();
        assert_eq!(active_name(&app.session), "Filled", "{} empty type layer", lang.code());
        app.session.prefs.edit(|p| p.interface.language = lang.code().into());
        app.session.execute("type.create", json!({"x":0,"y":4,"text":"Title copy"})).unwrap();
        app.session.execute("layer.duplicate", json!({})).unwrap();
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.session.execute("layer.duplicate", json!({})).unwrap();
        app.session.execute("type.edit", json!({"text":"Rewritten"})).unwrap();
        assert_eq!(active_name(&app.session), "Rewritten", "{} copy markers must not consume literal text", lang.code());
    }
}
