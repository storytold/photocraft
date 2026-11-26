use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::*;
use crate::canvas::testing::{ToolEvent, tool_event};

/// A document whose active layer is a `kind` layer over a white Background.
pub(super) fn app_with(kind: Kind) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // Strokes and filters finish in the pass that asks for them, so the history counts are settled at once.
    app.background_jobs = false;
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    match kind {
        Kind::Type => app.run("type.create", json!({"x": 10, "y": 40, "text": "Hello"})).map(|_| ()),
        Kind::Shape => app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 60, 40], "fill": "#ff0000"})).map(|_| ()),
        Kind::SmartObject => {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
            app.run("layer.smartObjects.convertToSmartObject", json!({})).map(|_| ())
        }
        Kind::Fill => app.run("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).map(|_| ()),
    }
    .unwrap();
    // A new fill layer targets its mask (Photoshop); painting its content means targeting the
    // layer itself, as a click on its thumbnail does.
    app.ui.mask_target = false;
    app.sync_mask_targets();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app
}

pub(super) fn active_content(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

pub(super) fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

pub(super) fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

/// The open rasterize prompt.
pub(super) fn prompt(app: &PhotocraftApp) -> Option<DialogId> {
    alerts::prompt_tagged::<Rasterize>(app)
}

/// Answers the open prompt `id` with its default button, as `ui.dialog.confirm` does: the answer
/// acts inside the call.
pub(super) fn answer(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// Picks `choice` ("proceed", "alternate", "cancel") on the prompt `id`; the error says the prompt has
/// no such button.
pub(super) fn choose(app: &mut PhotocraftApp, id: DialogId, choice: &str) -> Result<(), String> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "choice", json!(choice))
}

/// The open parameter dialog of `filter`.
pub(super) fn filter_dialog(app: &PhotocraftApp, filter: &str) -> Option<DialogId> {
    app.framework.dialogs.get::<HostedParamDialog>().filter(|d| d.key() == Some(filter)).map(|d| d.id())
}

pub(super) const BLUR: &str = "filter.blur.gaussianBlur";

pub(super) fn menu(app: &mut PhotocraftApp, id: &str) -> Result<serde_json::Value, String> {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, id, json!({}))
}

/// #8806: OK on the prompt rasterizes and adds one history state; the held click is not
/// replayed, and the next click paints with no prompt.
#[test]
fn ok_rasterizes_without_replaying_the_click() {
    for kind in Kind::ALL {
        for tool in [Tool::Brush, Tool::Pencil] {
            let mut app = app_with(kind);
            app.ui.tool = tool;
            let before = past(&app);
            click(&mut app, 30.0, 20.0);
            let id = prompt(&app).unwrap_or_else(|| panic!("{kind:?}/{tool:?}: no prompt"));
            assert_eq!(past(&app), before, "nothing happens until the answer");
            answer(&mut app, id);
            assert_eq!(active_content(&app), "Pixel");
            assert_eq!(past(&app), before + 1, "{kind:?}/{tool:?}: rasterize only, the held click is not replayed");
            // The held click left no paint; the next click paints.
            let st = app.session.active().unwrap();
            let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(30, 20);
            assert!(c[2] < 0.5 || c[0] > 0.5, "{kind:?}/{tool:?}: nothing painted by the held click: {c:?}");
            click(&mut app, 30.0, 20.0);
            assert!(prompt(&app).is_none());
            assert_eq!(past(&app), before + 2, "{kind:?}/{tool:?}: the next click paints");
            // One undo takes the paint back; the layer stays rasterized.
            app.session.undo();
            assert_eq!(active_content(&app), "Pixel");
            app.session.undo();
            assert_eq!(active_content(&app), kind_name(kind));
        }
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Type => "Type",
        Kind::Shape => "Shape",
        Kind::SmartObject => "Smart Object",
        Kind::Fill => "Fill",
    }
}

#[test]
fn cancel_does_nothing() {
    let mut app = app_with(Kind::Type);
    app.ui.tool = Tool::Brush;
    let before = (past(&app), app.session.active().unwrap().doc.clone());
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).unwrap();
    // A second press while it is up doesn't stack another prompt.
    click(&mut app, 40.0, 20.0);
    assert_eq!(app.framework.dialogs.open_dialogs().len(), 1);
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(prompt(&app).is_none());
    assert_eq!(past(&app), before.0);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before.1) || *app.session.active().unwrap().doc == *before.1);
    assert_eq!(active_content(&app), "Type");
}

#[test]
fn only_pixel_painting_asks() {
    let mut app = app_with(Kind::Type);
    let ctx = app.ctx().clone();
    for tool in [Tool::Move, Tool::RectMarquee, Tool::Eyedropper, Tool::Hand, Tool::Type] {
        app.ui.tool = tool;
        click(&mut app, 30.0, 20.0);
        assert!(prompt(&app).is_none(), "{tool:?} must not ask");
        app.ui.text_edit = None;
    }
    for tool in [Tool::Eraser, Tool::PaintBucket, Tool::Smudge] {
        app.ui.tool = tool;
        click(&mut app, 30.0, 20.0);
        let id = prompt(&app).unwrap_or_else(|| panic!("{tool:?} asks"));
        assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    }
    // ⌥-click with the Clone Stamp sets the source.
    app.ui.tool = Tool::CloneStamp;
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 20.0 }, Modifiers::ALT);
    assert!(prompt(&app).is_none());
    // With a source set, a plain click paints, so it asks.
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).expect("the Clone Stamp with a source asks");
    assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    // Painting the layer's mask needs no rasterizing.
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    app.ui.tool = Tool::Brush;
    click(&mut app, 30.0, 20.0);
    assert!(prompt(&app).is_none());
    assert!(app.session.active().unwrap().doc.layers.iter().any(|l| matches!(l.content, LayerContent::Text(_))));
}

/// A prompt whose layer was rasterized meanwhile reports the error instead of panicking.
#[test]
fn a_stale_prompt_fails_gracefully() {
    let mut app = app_with(Kind::Type);
    app.ui.tool = Tool::Brush;
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).unwrap();
    app.run("layer.rasterize.type", json!({})).unwrap();
    let before = past(&app);
    answer(&mut app, id);
    assert!(app.ui.status_error, "OK reports why nothing happened: {}", app.ui.status);
    assert_eq!(past(&app), before);
}

/// Review round: the prompt names its layer by number, and a document read back from a file keeps the
/// layer ids stored in it, so two open documents can hold the same number. A brush press in one
/// document whose question is answered while another document is active writes in neither of them.
#[test]
fn a_document_switch_while_the_prompt_is_up_rasterizes_nothing() {
    let mut app = app_with(Kind::Type);
    app.ui.tool = Tool::Brush;
    let mine = app.session.active_index().unwrap();
    let text = app.session.active().unwrap().active_layer.unwrap();
    click(&mut app, 30.0, 20.0);
    let id = prompt(&app).expect("the press opened the prompt");
    // A second document made from the bytes of the first: its layers keep the ids stored in the file,
    // so the same number names a Type layer there too, and that layer is active there.
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    let twin = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(twin.layer(text).map(|l| l.content.kind_name()), Some("Type"), "both documents hold a Type layer {}", text.0);
    app.session.add_document(twin, None);
    let other = app.session.active_index().unwrap();
    assert_ne!(other, mine, "the second document is the active one now");
    assert_eq!(app.session.documents()[other].active_layer, Some(text), "the other document's active layer carries the same id");
    let (before_other, before_mine) = (app.session.documents()[other].revision, app.session.documents()[mine].revision);
    answer(&mut app, id);
    assert!(app.ui.status_error, "OK says why nothing ran: {}", app.ui.status);
    // Both documents are read live, after the answer.
    assert_eq!(app.session.documents()[other].revision, before_other, "no history step in the document that is active now");
    assert_eq!(app.session.documents()[other].doc.layer(text).unwrap().content.kind_name(), "Type", "its Type layer stays editable");
    assert_eq!(app.session.documents()[mine].revision, before_mine, "the document the question was about is untouched");
    assert_eq!(app.session.documents()[mine].doc.layer(text).unwrap().content.kind_name(), "Type", "and keeps its text");
}

/// The Filter menu's prompt keeps its question inside the document it was opened for for the same
/// reason: OK would otherwise rasterize or convert a same-id layer of another open document and run
/// the filter on that one.
#[test]
fn the_filter_prompt_refuses_a_document_that_is_no_longer_active() {
    let mut app = app_with(Kind::Type);
    let mine = app.session.active_index().unwrap();
    let text = app.session.active().unwrap().active_layer.unwrap();
    menu(&mut app, BLUR).unwrap();
    let id = prompt(&app).expect("the filter asks first");
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    let twin = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(twin.layer(text).map(|l| l.content.kind_name()), Some("Type"), "both documents hold a Type layer {}", text.0);
    app.session.add_document(twin, None);
    let other = app.session.active_index().unwrap();
    let (before_other, before_mine) = (app.session.documents()[other].revision, app.session.documents()[mine].revision);
    answer(&mut app, id);
    assert!(app.ui.status_error, "OK says why nothing ran: {}", app.ui.status);
    assert_eq!(app.session.documents()[other].revision, before_other, "no history step in the document that is active now");
    assert_eq!(app.session.documents()[other].doc.layer(text).unwrap().content.kind_name(), "Type", "its Type layer stays editable");
    assert!(filter_dialog(&app, BLUR).is_none(), "Gaussian Blur did not open on the other document");
    assert_eq!(app.session.documents()[mine].revision, before_mine, "the document the question was about is untouched");
    assert_eq!(app.session.documents()[mine].doc.layer(text).unwrap().content.kind_name(), "Type", "and keeps its text");
}

#[test]
fn the_control_channel_sees_and_confirms_the_prompt() {
    let mut app = app_with(Kind::Shape);
    let ctx = crate::canvas::testing::ctx(&app);
    app.ui.tool = Tool::Pencil;
    // The canvas shows once before a pointer event can be mapped onto the document.
    crate::canvas::testing::frame(&mut app);
    let call = |app: &mut PhotocraftApp, method: &str, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Reply::Done(v) => v,
            _ => panic!("deferred"),
        }
    };
    call(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 30, "y": 20}, {"kind": "up", "x": 30, "y": 20}]}));
    // `ui.pointer` queues its events for the tool host: the passes that take them come after the call.
    for _ in 0..8 {
        crate::canvas::testing::frame(&mut app);
    }
    let listed = call(&mut app, "ui.inspect", json!({}))["result"]["dialogs"][0]["id"].clone();
    assert!(!listed.is_null(), "ui.inspect lists the prompt");
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": listed}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(active_content(&app), "Pixel");
}

/// #4303: Filter › Gaussian Blur… on a type layer is live and asks to rasterize first.
#[test]
fn a_filter_on_a_type_layer_asks_then_opens_on_the_rasterized_layer() {
    let mut app = app_with(Kind::Type);
    assert!(!app.session.is_enabled(BLUR), "the engine itself still needs pixels");
    assert!(crate::menus::is_enabled(&app, BLUR));
    let before = past(&app);
    menu(&mut app, BLUR).unwrap();
    let id = prompt(&app).expect("the prompt");
    assert_eq!(alerts::prompt_tag::<Rasterize>(&app).map(|tag| tag.kind), Some(Kind::Type));
    assert!(filter_dialog(&app, BLUR).is_none(), "the filter waits for the answer");
    assert_eq!((past(&app), active_content(&app)), (before, "Type"));
    answer(&mut app, id);
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 1);
    let blur = filter_dialog(&app, BLUR).expect("Gaussian Blur opens on the rasterized layer");
    answer(&mut app, blur);
    assert_eq!(past(&app), before + 2, "rasterize, then blur");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some(BLUR));
}

#[test]
fn cancelling_the_filter_prompt_keeps_the_type_layer() {
    let mut app = app_with(Kind::Type);
    let before = (past(&app), app.session.active().unwrap().doc.clone());
    menu(&mut app, BLUR).unwrap();
    // The menu again while it is up doesn't stack another prompt.
    menu(&mut app, BLUR).unwrap();
    assert_eq!(app.framework.dialogs.open_dialogs().len(), 1);
    let id = prompt(&app).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(&mut app, &ctx, id).unwrap();
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(past(&app), before.0);
    assert_eq!(*app.session.active().unwrap().doc, *before.1);
    assert_eq!(active_content(&app), "Type");
}

/// Convert to Smart Object keeps the text editable and applies the filter as a smart filter.
#[test]
fn convert_to_smart_object_applies_a_smart_filter() {
    let mut app = app_with(Kind::Type);
    menu(&mut app, BLUR).unwrap();
    let id = prompt(&app).unwrap();
    choose(&mut app, id, "alternate").unwrap();
    answer(&mut app, id);
    assert_eq!(active_content(&app), "Smart Object");
    let blur = filter_dialog(&app, BLUR).expect("Gaussian Blur opens on the Smart Object");
    answer(&mut app, blur);
    let st = app.session.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a Smart Object") };
    assert_eq!(sm.smart_filters.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(), [BLUR]);
}

/// A filter without a dialog runs straight after rasterizing.
#[test]
fn a_filter_without_a_dialog_runs_after_rasterizing() {
    let mut app = app_with(Kind::Type);
    let before = past(&app);
    menu(&mut app, "filter.blur.blurMore").unwrap();
    let id = prompt(&app).unwrap();
    answer(&mut app, id);
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(past(&app), before + 2);
    assert_eq!(active_content(&app), "Pixel");
}

#[test]
fn filters_stay_grey_where_rasterizing_would_not_help() {
    // A locked type layer can't be rasterized either.
    let mut app = app_with(Kind::Type);
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(!crate::menus::is_enabled(&app, BLUR));
    // Only type layers ask: a shape layer's filters stay grey, as before.
    let app = app_with(Kind::Shape);
    assert!(!crate::menus::is_enabled(&app, BLUR));
    // Non-filter commands a type layer greys are untouched.
    let app = app_with(Kind::Type);
    assert_eq!(crate::menus::is_enabled(&app, "edit.fill"), app.session.is_enabled("edit.fill"));
}

/// The filter prompt offers Convert to Smart Object only where the filter runs as a smart filter
/// (Vanishing Point edits pixels only), and Rasterize opens the filter on the rasterized layer.
#[test]
fn the_filter_prompt_offers_convert_rasterize_and_cancel() {
    // (filter, offers Convert to Smart Object, what shows the filter opened)
    type Opened = fn(&PhotocraftApp) -> bool;
    let table: [(&str, bool, Opened); 2] =
        [(BLUR, true, |app| filter_dialog(app, BLUR).is_some()), ("filter.vanishingPoint", false, |app| crate::distort_ui::modal_workspace(app).is_some())];
    for (filter, convert, opened) in table {
        let mut app = app_with(Kind::Type);
        menu(&mut app, filter).unwrap();
        let id = prompt(&app).unwrap_or_else(|| panic!("{filter}: no prompt"));
        assert_eq!(choose(&mut app, id, "alternate").is_ok(), convert, "{filter}: Convert to Smart Object");
        assert!(choose(&mut app, id, "cancel").is_ok(), "{filter}: Cancel");
        choose(&mut app, id, "proceed").unwrap();
        answer(&mut app, id);
        assert_eq!(active_content(&app), "Pixel", "{filter}");
        assert!(opened(&app), "{filter}: opens on the rasterized layer");
    }
}

/// The filters with their own window or mode open on the rasterized layer, or on the Smart Object.
#[test]
fn filters_with_their_own_window_open_after_the_prompt() {
    // (filter, runs on a Smart Object as a smart filter)
    let filters = [("filter.liquify", true), ("filter.filterGallery", true), ("filter.cameraRaw", true), ("filter.vanishingPoint", false)];
    for (filter, smart) in filters {
        for choice in ["proceed", "alternate"].into_iter().filter(|choice| smart || *choice == "proceed") {
            let mut app = app_with(Kind::Type);
            assert!(crate::menus::is_enabled(&app, filter), "{filter}");
            menu(&mut app, filter).unwrap();
            assert!(crate::distort_ui::modal_workspace(&app).is_none(), "{filter}: waits for the answer");
            let id = prompt(&app).unwrap_or_else(|| panic!("{filter}: no prompt"));
            choose(&mut app, id, choice).unwrap();
            answer(&mut app, id);
            assert!(!app.ui.status_error, "{filter}/{choice}: {}", app.ui.status);
            assert_eq!(active_content(&app), if choice == "proceed" { "Pixel" } else { "Smart Object" }, "{filter}");
            assert!(crate::distort_ui::modal_workspace(&app).is_some(), "{filter}/{choice}: its window opens");
        }
    }
}

/// Last Filter asks only when there is a filter to repeat, then repeats it on the rasterized layer.
#[test]
fn last_filter_on_a_type_layer_needs_a_filter_to_repeat() {
    let mut app = app_with(Kind::Type);
    assert!(!crate::menus::is_enabled(&app, "filter.lastFilter"), "nothing to repeat yet");
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("filter.blur.blurMore", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let text = st.doc.layers.iter().find(|l| matches!(l.content, LayerContent::Text(_))).unwrap().id;
    app.run("layer.select", json!({"layer": text.0})).unwrap();
    assert!(crate::menus::is_enabled(&app, "filter.lastFilter"));
    let before = past(&app);
    menu(&mut app, "filter.lastFilter").unwrap();
    let id = prompt(&app).expect("the prompt");
    answer(&mut app, id);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(active_content(&app), "Pixel");
    assert_eq!(past(&app), before + 2, "rasterize, then Blur More again");
    assert_eq!(app.session.journal.iter().filter(|(id, _)| id == "filter.blur.blurMore").count(), 2);
}

/// Every Filter menu item live on a pixel layer is live on a type layer and gets there through the
/// prompt (Flame, Tree, Picture Frame already run on a type layer: they render on a new layer).
#[test]
fn every_filter_live_on_pixels_asks_on_a_type_layer() {
    let ids: Vec<&str> = crate::menu_catalog::CATALOG.iter().filter(|e| e.3.starts_with("filter.")).map(|e| e.3).collect();
    let mut pixels = app_with(Kind::Type);
    pixels.run("layer.rasterize.type", json!({})).unwrap();
    let mut asked = 0;
    for id in ids {
        let mut app = app_with(Kind::Type);
        if !crate::menus::is_enabled(&pixels, id) || app.session.is_enabled(id) {
            continue;
        }
        assert!(crate::menus::is_enabled(&app, id), "{id}");
        menu(&mut app, id).unwrap();
        let d = prompt(&app).unwrap_or_else(|| panic!("{id}: no prompt"));
        answer(&mut app, d);
        assert!(!app.ui.status_error, "{id}: {}", app.ui.status);
        assert_eq!(active_content(&app), "Pixel", "{id}");
        asked += 1;
    }
    assert!(asked > 40, "{asked} filters");
}
