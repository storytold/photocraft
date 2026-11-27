//! #218: Type › Warp Text… opens with the type layer's current warp (it always started at Arc)
//! and previews the warp live on the canvas.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::text::TextWarp;
use photocraft_doc::{Document, LayerContent};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A document with one type layer, active.
fn app_with_type() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("type.create", json!({"x": 20, "y": 120, "text": "Warp", "size": 60})).unwrap();
    app
}

fn flag() -> Value {
    json!({"style": "flag", "bend": -30.0, "horizontalDistortion": 20.0, "verticalDistortion": -10.0, "orientation": "vertical"})
}

fn warp(app: &PhotocraftApp) -> Option<TextWarp> {
    let st = app.session.active().unwrap();
    warp_of(&st.doc, st.active_layer.unwrap())
}

fn warp_of(doc: &Document, id: photocraft_doc::LayerId) -> Option<TextWarp> {
    match &doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.warp.clone(),
        _ => panic!("the layer is a type layer"),
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The open dialogs drawn over `app`, as the window shows them.
pub(crate) fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::run(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let mut h = h;
    h.run_steps(4);
    h
}

fn set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// Set dialog values as the sliders and dropdowns would, and compute the canvas preview as a
/// frame of the canvas does.
fn edit_and_preview(h: &mut Harness<'_, Box<PhotocraftApp>>, id: DialogId, values: Value) {
    for (k, v) in values.as_object().unwrap() {
        set(h.state_mut(), id, k, v.clone());
    }
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::canvas::ensure_filter_preview(h.state_mut(), 0, &ctx);
}

/// Open Warp Text and return the dialog with its values.
fn open(app: &mut PhotocraftApp) -> (DialogId, DialogFields) {
    let id = crate::filter_dialog::open(app, "type.warpText").expect("Warp Text opens");
    let fields = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone();
    (id, fields)
}

/// OK as the button files it, then the passes in which the family runs the command.
fn confirm(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    crate::dialogs::testing::confirm(app, id).expect("OK is accepted");
    for _ in 0..3 {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(app, ui));
    }
}

fn no_dialog_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.is_empty()
}

fn num(f: &DialogFields, key: &str) -> f64 {
    match f.get(key) {
        Some(FieldValue::Int(i)) => *i as f64,
        Some(FieldValue::Number(n)) => *n,
        other => panic!("{key} is no number: {other:?}"),
    }
}

fn shown(f: &DialogFields) -> (&str, f64, f64, f64, &str) {
    (f.text("style").unwrap(), num(f, "bend"), num(f, "horizontalDistortion"), num(f, "verticalDistortion"), f.text("orientation").unwrap())
}

#[test]
fn warp_text_opens_with_the_type_layers_current_warp() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("flag", -30.0, 20.0, -10.0, "vertical"));
}

#[test]
fn confirming_the_warp_text_dialog_unchanged_keeps_the_layers_warp() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let before = warp(&app);
    assert_eq!(before.as_ref().map(|w| w.style.as_str()), Some("warpFlag"));
    let (id, _) = open(&mut app);
    confirm(&mut app, id);
    assert_eq!(warp(&app), before);
}

#[test]
fn warp_text_opens_at_none_for_a_layer_without_warp() {
    let mut app = app_with_type();
    let (id, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("none", 50.0, 0.0, 0.0, "horizontal"));
    confirm(&mut app, id);
    assert_eq!(warp(&app), None);
}

#[test]
fn remembered_warp_values_never_override_the_layers_warp() {
    let mut app = app_with_type();
    let (id, _) = open(&mut app);
    set(&mut app, id, "style", json!("twist"));
    set(&mut app, id, "bend", json!(80.0));
    confirm(&mut app, id);
    assert_eq!(warp(&app).map(|w| (w.style, w.value)), Some(("warpTwist".to_string(), 80.0)));
    app.run("type.create", json!({"x": 20, "y": 60, "text": "Other", "size": 30})).unwrap();
    app.run("type.warpText", flag()).unwrap();
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("flag", -30.0, 20.0, -10.0, "vertical"));
}

/// A PSD can carry a style without a short id: the dialog shows None with the layer's values.
#[test]
fn an_unknown_warp_style_opens_at_none_with_the_layers_values() {
    let mut app = app_with_type();
    let st = app.session.active_mut().unwrap();
    let id = st.active_layer.unwrap();
    let mut doc = (*st.doc).clone();
    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
        t.warp = Some(TextWarp { style: "warpCustom".into(), value: 12.0, horizontal_distortion: -5.0, vertical_distortion: 7.0, horizontal: false });
    }
    *st.doc_mut() = std::sync::Arc::new(doc);
    let (_, fields) = open(&mut app);
    assert_eq!(shown(&fields), ("none", 12.0, -5.0, 7.0, "vertical"));
}

#[test]
fn warp_text_dialog_previews_the_warp_without_editing_the_document() {
    let mut app = app_with_type();
    let (id, _) = open(&mut app);
    let (revision, original) = {
        let st = app.session.active().unwrap();
        (st.revision, st.doc.clone())
    };
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = harness(app);
    assert!(h.query_by_label("Preview").is_some(), "the dialog offers Preview");
    edit_and_preview(&mut h, id, json!({"style": "arc", "bend": 40.0}));
    let app = h.state();
    let shown = app.filter_preview.as_ref().and_then(|p| p.result.clone()).expect("a preview");
    assert_eq!(warp_of(&shown, layer).map(|w| (w.style, w.value)), Some(("warpArc".to_string(), 40.0)));
    assert_ne!(photocraft_compose::flatten(&shown).px, photocraft_compose::flatten(&original).px, "the preview shows the warp");
    let st = app.session.active().unwrap();
    assert_eq!(st.revision, revision);
    assert_eq!(warp(app), None, "the document is not edited");
}

#[test]
fn cancelling_warp_text_leaves_the_document_and_history_untouched() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let (before, history) = (warp(&app), steps(&app));
    let revision = app.session.active().unwrap().revision;
    let (id, _) = open(&mut app);
    let mut h = harness(app);
    edit_and_preview(&mut h, id, json!({"style": "wave", "bend": 70.0}));
    assert!(h.state().filter_preview.as_ref().is_some_and(|p| p.result.is_some()), "the preview is on screen");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    let app = h.state();
    assert!(no_dialog_open(app));
    assert!(app.filter_preview.is_none(), "the preview is gone");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!((warp(app), steps(app)), (before, history));
}

#[test]
fn confirming_warp_text_is_one_undoable_step() {
    let mut app = app_with_type();
    app.run("type.warpText", flag()).unwrap();
    let (before, history) = (warp(&app), steps(&app));
    let (id, _) = open(&mut app);
    let mut h = harness(app);
    edit_and_preview(&mut h, id, json!({"style": "fish", "bend": 25.0, "orientation": "horizontal"}));
    h.get_by_label("OK").click();
    h.run_steps(2);
    let app = h.state_mut();
    assert!(no_dialog_open(app));
    assert_eq!(warp(app).map(|w| (w.style, w.value, w.horizontal)), Some(("warpFish".to_string(), 25.0, true)));
    assert_eq!(steps(app), history + 1);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(|e| e.to_string()), Some("Warp Text".to_string()));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(warp(app), before);
}
