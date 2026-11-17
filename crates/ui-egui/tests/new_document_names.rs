//! New Document's suggested name must be the name actually passed to the engine.
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
    s.untitled_name_base = Some(|_| "Draft.v2".into());
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
    s.untitled_name_base = Some(|language| if language == "de" { "Fixture" } else { "Untitled" }.into());
    s.prefs.edit(|p| p.interface.language = "de".into());
    s.actions.list.push(Action::new(
        "Place",
        vec![
            ("file.new".into(), json!({"width":4,"height":4})),
            ("document.activate".into(), json!({"offset":-1})),
            ("layer.duplicate".into(), json!({"document":{"name":"Fixture-1"}})),
        ],
    ));
    let result = s.execute("file.automate.batch", json!({"action":"Place","input":[input],"output":dir.path().join("out")})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
}
