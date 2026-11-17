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
    s.default_name_base = Some(|_, _| "Draft.v2".into());
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
    s.default_name_base = Some(|language, english| if language == "de" { format!("Fixture {english}") } else { english.into() });
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
        s.default_name_base = Some(|lang, base| format!("{lang}:{base}"));
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
            s.default_name_base = Some(|_, _| String::new());
        }
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.name, format!("Untitled-{}", if empty_service { 2 } else { 1 }));
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "Color Fill 1");
    }
}
