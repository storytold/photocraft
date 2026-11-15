use super::*;

fn new_document(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
}

fn test_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    new_document(&mut app);
    app
}

fn fields(app: &mut PhotocraftApp, dialog: u64) -> &mut Map<String, Value> {
    &mut app.ui.dialogs.iter_mut().find(|d| d.id == dialog).unwrap().fields
}

fn set(app: &mut PhotocraftApp, dialog: u64, key: &str, value: Value) {
    let f = fields(app, dialog);
    let selected = f["selected"].as_str().unwrap().to_owned();
    set_param(f, &selected, key, value);
}

fn params(app: &mut PhotocraftApp, dialog: u64) -> Value {
    effects_of(fields(app, dialog)).iter().find(|e| e["kind"] == "satin").unwrap()["params"].clone()
}

#[test]
fn a_new_satin_starts_from_the_last_confirmed_settings() {
    let mut app = test_app();
    let first = open(&mut app, Some("satin")).unwrap();
    set(&mut app, first, "opacity", json!(60));
    set(&mut app, first, "size", json!(15));
    crate::dialogs::confirm(&mut app, first).unwrap();

    new_document(&mut app);
    let next = open(&mut app, Some("satin")).unwrap();
    assert_eq!(params(&mut app, next)["opacity"].as_f64(), Some(60.0));
    assert_eq!(params(&mut app, next)["size"].as_f64(), Some(15.0));
}

#[test]
fn opening_and_adding_satin_carry_all_committed_controls() {
    let mut app = test_app();
    let first = open(&mut app, Some("satin")).unwrap();
    for (key, value) in [
        ("opacity", json!(72)),
        ("blend", json!("Screen")),
        ("color", json!("#4488cc")),
        ("angle", json!(42)),
        ("distance", json!(28)),
        ("size", json!(17)),
        ("contour", json!("Gaussian")),
        ("antiAlias", json!(false)),
        ("invert", json!(false)),
    ] {
        set(&mut app, first, key, value);
    }
    crate::dialogs::confirm(&mut app, first).unwrap();
    let existing = open(&mut app, Some("satin")).unwrap();
    let expected = params(&mut app, existing);
    crate::dialogs::cancel(&mut app, existing).unwrap();

    new_document(&mut app);
    let next = open(&mut app, Some(BLENDING)).unwrap();
    add_instance(fields(&mut app, next), "satin");
    assert_eq!(params(&mut app, next), expected);

    add_instance(fields(&mut app, next), "dropShadow");
    let f = fields(&mut app, next);
    let shadow = entry(f, f["selected"].as_str().unwrap()).unwrap();
    assert_eq!(shadow["params"]["opacity"], defaults("dropShadow")["opacity"]);
}

#[test]
fn cancel_failure_and_an_unrelated_ok_leave_the_last_satin_in_place() {
    let mut app = test_app();
    let committed = open(&mut app, Some("satin")).unwrap();
    set(&mut app, committed, "size", json!(15));
    crate::dialogs::confirm(&mut app, committed).unwrap();

    new_document(&mut app);
    let canceled = open(&mut app, Some("satin")).unwrap();
    set(&mut app, canceled, "size", json!(24));
    let preview = fields(&mut app, canceled).clone();
    preview_document(&app.session.active().unwrap().doc, &app.session.patterns, &preview).unwrap();
    crate::dialogs::cancel(&mut app, canceled).unwrap();

    let failed = open(&mut app, Some("satin")).unwrap();
    let f = fields(&mut app, failed);
    let selected = f["selected"].as_str().unwrap().to_owned();
    entry_mut(f, &selected).unwrap()["params"] = Value::Null;
    assert!(crate::dialogs::confirm(&mut app, failed).is_err());

    app.run("layer.layerStyle.satin", json!({"size": 23})).unwrap();
    let existing = open(&mut app, Some(BLENDING)).unwrap();
    assert_eq!(params(&mut app, existing)["size"].as_f64(), Some(23.0));
    crate::dialogs::confirm(&mut app, existing).unwrap();

    let disabled = open(&mut app, Some("satin")).unwrap();
    let f = fields(&mut app, disabled);
    let selected = f["selected"].as_str().unwrap().to_owned();
    entry_mut(f, &selected).unwrap()["on"] = json!(false);
    crate::dialogs::confirm(&mut app, disabled).unwrap();

    new_document(&mut app);
    let next = open(&mut app, Some("satin")).unwrap();
    assert_eq!(params(&mut app, next)["size"].as_f64(), Some(15.0));
}

#[test]
fn editing_an_existing_satin_is_remembered_even_after_switching_pages() {
    let mut app = test_app();
    app.run("layer.layerStyle.satin", json!({"size": 23})).unwrap();
    let dialog = open(&mut app, Some("satin")).unwrap();
    set(&mut app, dialog, "size", json!(31));
    set(&mut app, dialog, "opacity", json!(140));
    fields(&mut app, dialog).insert("selected".into(), json!(BLENDING));
    crate::dialogs::confirm(&mut app, dialog).unwrap();

    new_document(&mut app);
    let next = open(&mut app, Some("satin")).unwrap();
    assert_eq!(params(&mut app, next)["size"].as_f64(), Some(31.0));
    assert_eq!(params(&mut app, next)["opacity"].as_f64(), Some(100.0));
}

#[test]
fn reset_and_restart_use_the_user_default() {
    use egui_kittest::{Harness, kittest::Queryable};

    let mut app = test_app();
    app.run("layer.layerStyle.makeDefault", json!({"kind": "satin", "params": {"size": 40}})).unwrap();
    let original = open(&mut app, Some("satin")).unwrap();
    assert_eq!(params(&mut app, original)["size"].as_f64(), Some(40.0));
    set(&mut app, original, "size", json!(15));
    crate::dialogs::confirm(&mut app, original).unwrap();

    let mut restarted = test_app();
    restarted.session.load_presets_json(app.session.presets.to_json(&app.session));
    let fresh = open(&mut restarted, Some("satin")).unwrap();
    assert_eq!(params(&mut restarted, fresh)["size"].as_f64(), Some(40.0));

    new_document(&mut app);
    let next = open(&mut app, Some("satin")).unwrap();
    assert_eq!(params(&mut app, next)["size"].as_f64(), Some(15.0));
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    h.get_by_label("Reset to Default").click();
    h.run_steps(3);
    assert_eq!(params(h.state_mut(), next)["size"].as_f64(), Some(40.0));
    crate::dialogs::cancel(h.state_mut(), next).unwrap();
    let again = open(h.state_mut(), Some("satin")).unwrap();
    assert_eq!(params(h.state_mut(), again)["size"].as_f64(), Some(15.0));
}
