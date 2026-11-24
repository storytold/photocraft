//! #14495: Image › Apply Data Set… opens its own small dialog, as in the reference application: a
//! "Data Set:" list of the document's sets in document order with the first one selected on every
//! open, Apply (default) and Cancel beside it. Apply, Enter and a double-click apply the selected
//! set once and close; Cancel and Escape change nothing. The dialog never writes the variables or
//! data sets, and Image › Variables › Data Sets… keeps its own page.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply};
use serde_json::{Value, json};

type H = Harness<'static, Box<PhotocraftApp>>;

const SETS: [&str; 3] = ["Zed", "Data Set 2", "Data Set 3"];

/// A document with a visibility and a pixel replacement variable on one layer, and three sets
/// (not in name order) that only set the visibility: Zed hides it, Data Set 2 shows it, Data Set 3
/// hides it. No set has an image for the pixel variable.
fn app() -> Box<PhotocraftApp> {
    #[cfg(not(target_arch = "wasm32"))]
    photocraft_raster::spill::use_test_volume_space();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap().0;
    app.run(
        "image.variables.define",
        json!({"defs": [
            {"name": "badge", "layer": layer, "type": "visibility"},
            {"name": "photo", "layer": layer, "type": "pixelReplacement", "method": "fit"},
        ]}),
    )
    .unwrap();
    let sets: Vec<Value> = SETS
        .iter()
        .zip([false, true, false])
        .map(|(name, on)| json!({"name": name, "values": [{"variable": "badge", "kind": "visibility", "value": on}]}))
        .collect();
    app.run("image.variables.dataSets", json!({"dataSets": sets})).unwrap();
    app
}

fn harness(app: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            photocraft_ui_egui::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// One control request, as a transport sends it.
fn control(h: &mut H, method: &str, params: Value) -> Value {
    let (request, _) = ControlRequest::new(method, params);
    let ctx = h.ctx.clone();
    let reply = match photocraft_ui_egui::control::handle(h.state_mut(), &ctx, &request) {
        Reply::Done(value) => value,
        _ => panic!("{method} did not answer at once"),
    };
    h.run_steps(2);
    reply
}

fn refused(reply: &Value) -> bool {
    reply.get("error").is_some() || reply["ok"] == json!(false)
}

/// Image › Apply Data Set… as the menu runs it, then a few frames; the dialog's id.
fn open(h: &mut H) -> String {
    let ctx = h.ctx.clone();
    let reply = photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
    h.run_steps(3);
    reply["dialog"].as_str().expect("the menu opens a dialog").to_string()
}

fn opened() -> (H, String) {
    let mut h = harness(app());
    let id = open(&mut h);
    (h, id)
}

/// The open dialogs as `ui.inspect` lists them.
fn dialogs(h: &mut H) -> Vec<Value> {
    control(h, "ui.inspect", json!({}))["result"]["dialogs"].as_array().cloned().unwrap_or_default()
}

/// The Apply Data Set dialog's fields, `Null` when it is not open.
fn fields(h: &mut H) -> Value {
    dialogs(h).into_iter().find(|d| d["fields"]["__command"] == "image.applyDataSet").map_or(Value::Null, |d| d["fields"].clone())
}

fn index(h: &mut H) -> Value {
    fields(h)["index"].clone()
}

fn visible(h: &H) -> bool {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().visible
}

/// History states added since the test document was made (making its layer is one).
fn history(h: &H) -> usize {
    let base = app().session.active().unwrap().history.past_len();
    h.state().session.active().unwrap().history.past_len() - base
}

fn variables(h: &H) -> photocraft_doc::Variables {
    h.state().session.active().unwrap().doc.variables.clone()
}

/// A double-click at `at` as a mouse sends it: press, release, press, release, one pass each,
/// 50 ms apart. A harness pass moves the clock on by 0.25 s, so two presses a pass or more apart
/// are more than the 0.3 s the framework allows between the clicks of a double-click; these passes
/// carry their own times, and the clock then runs on as before.
fn double_click(h: &mut H, at: egui::Pos2) {
    let start = h.ctx.input(|i| i.time);
    for (n, pressed) in [true, false, true, false].into_iter().enumerate() {
        h.input_mut().time = Some(start + 0.05 * (n as f64 + 1.0));
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.step();
    }
    h.input_mut().time = None;
}

#[test]
fn menu_opens_the_apply_data_set_dialog_not_the_variables_editor() {
    let (mut h, _) = opened();
    let listed = dialogs(&mut h);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["fields"]["__command"], "image.applyDataSet");
    assert!(listed[0]["fields"].get("defs").is_none(), "the dialog carries no editable variables");
    h.get_by_label("Apply");
    h.get_by_label("Cancel");
    assert!(h.query_all_by_label("Data Set:").count() > 0);
    for absent in ["OK", "Define", "Data Sets", "New", "Delete", "Next Data Set"] {
        assert!(h.query_by_label(absent).is_none(), "{absent}");
    }
}

#[test]
fn variables_data_sets_page_keeps_its_own_dialog() {
    let mut h = harness(app());
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.variables.dataSets", json!({})).unwrap();
    h.run_steps(3);
    let listed = dialogs(&mut h);
    assert_eq!(listed.len(), 1);
    assert!(listed[0]["fields"].get("defs").is_some());
    h.get_by_label("OK");
    h.get_by_label("Define");
    assert!(h.query_by_label("Data Set:").is_none());
}

#[test]
fn list_shows_every_set_in_document_order_with_the_first_selected() {
    let (mut h, _) = opened();
    let tops: Vec<f32> = SETS.iter().map(|name| h.get_by_label(name).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "document order, not sorted: {tops:?}");
    assert_eq!(index(&mut h), 0);
}

#[test]
fn first_set_is_selected_on_every_open_also_after_another_was_applied() {
    let (mut h, _) = opened();
    h.get_by_label("Data Set 3").click();
    h.run_steps(2);
    assert_eq!(index(&mut h), 2);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(!h.state().dialog_open());
    assert_eq!(variables(&h).active, Some(2));
    open(&mut h);
    assert_eq!(index(&mut h), 0);
}

#[test]
fn clicks_select_one_row_and_a_click_below_the_rows_keeps_it() {
    let (mut h, _) = opened();
    h.get_by_label("Data Set 2").click();
    h.run_steps(2);
    assert_eq!(index(&mut h), 1);
    for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::COMMAND] {
        h.get_by_label("Data Set 3").click_modifiers(modifiers);
        h.run_steps(2);
        assert_eq!(index(&mut h), 2, "{modifiers:?} selects only the clicked row");
        h.get_by_label("Zed").click_modifiers(modifiers);
        h.run_steps(2);
        assert_eq!(index(&mut h), 0, "{modifiers:?}");
    }
    h.get_by_label("Data Set 2").click();
    h.run_steps(2);
    // The empty area under the last row.
    let below = h.get_by_label("Data Set 3").rect().center() + egui::vec2(0.0, 60.0);
    h.event(egui::Event::PointerMoved(below));
    h.event(egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert_eq!(index(&mut h), 1);
    assert!(visible(&h), "selecting applies nothing");
    assert_eq!(history(&h), 0);
}

/// Apply, Enter and a double-click each apply the selected set once and close the dialog.
#[test]
fn apply_enter_and_double_click_apply_the_selected_set_once() {
    for way in ["button", "enter", "double-click"] {
        let (mut h, _) = opened();
        let before = variables(&h);
        match way {
            "button" | "enter" => {
                h.get_by_label("Data Set 3").click();
                h.run_steps(2);
                if way == "button" {
                    h.get_by_label("Apply").click();
                } else {
                    h.key_press(egui::Key::Enter);
                }
            }
            _ => {
                // Two clicks in a row on a row that isn't selected yet.
                let at = h.get_by_label("Data Set 3").rect().center();
                h.hover_at(at);
                h.run_steps(40);
                double_click(&mut h, at);
            }
        }
        h.run_steps(3);
        assert!(!h.state().dialog_open(), "{way}: closes");
        assert!(h.state().ui.status.is_empty(), "{way}: {}", h.state().ui.status);
        assert!(!visible(&h), "{way}: Data Set 3 hides the layer");
        assert_eq!(history(&h), 1, "{way}: one history state");
        let after = variables(&h);
        assert_eq!(after.active, Some(2), "{way}");
        assert_eq!((after.defs, after.data_sets), (before.defs, before.data_sets), "{way}: the variables and data sets stay as they were");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert!(visible(&h), "{way}: one undo restores the layer");
    }
}

/// The first set applies with nothing clicked: Zed hides the layer.
#[test]
fn apply_right_after_open_applies_the_first_set() {
    let (mut h, _) = opened();
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(!visible(&h));
    assert_eq!(variables(&h).active, Some(0));
}

#[test]
fn cancel_and_escape_change_nothing() {
    for way in ["cancel", "escape"] {
        let (mut h, _) = opened();
        let before = variables(&h);
        let revision = h.state().session.active().unwrap().revision;
        h.get_by_label("Data Set 3").click();
        h.run_steps(2);
        if way == "cancel" {
            h.get_by_label("Cancel").click();
        } else {
            h.key_press(egui::Key::Escape);
        }
        h.run_steps(3);
        assert!(!h.state().dialog_open(), "{way}");
        assert!(visible(&h), "{way}");
        assert_eq!(history(&h), 0, "{way}");
        assert_eq!(variables(&h), before, "{way}");
        assert_eq!(h.state().session.active().unwrap().revision, revision, "{way}");
    }
}

#[test]
fn holding_alt_keeps_cancel() {
    let (mut h, _) = opened();
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none());
    h.get_by_label("Cancel");
}

#[test]
fn control_channel_lists_sets_and_applies_the_selected_one() {
    let (mut h, id) = opened();
    assert_eq!(index(&mut h), 0);
    let set = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 1}));
    assert!(!refused(&set), "{set}");
    assert_eq!(index(&mut h), 1);
    let confirmed = control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
    assert!(!refused(&confirmed), "{confirmed}");
    assert!(!h.state().dialog_open());
    assert_eq!(variables(&h).active, Some(1));
    assert_eq!(history(&h), 1);

    let id = open(&mut h);
    let cancelled = control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert!(!refused(&cancelled), "{cancelled}");
    assert!(!h.state().dialog_open());
    assert_eq!(history(&h), 1);
}

/// `ui.dialog.set` takes only a row of the list: a bad index is refused, never a panic or a
/// different set, and the selection stays.
#[test]
fn bad_index_from_the_control_channel_is_refused() {
    for bad in [json!(99), json!(3), json!(-1), json!("Zed"), json!(1.5), Value::Null, json!(u64::MAX), json!(4_294_967_296_u64)] {
        let (mut h, id) = opened();
        let reply = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": bad}));
        assert!(refused(&reply), "{bad}: {reply}");
        assert_eq!(index(&mut h), 0, "{bad}");
        assert_eq!(variables(&h).active, None, "{bad}: no set was applied");
        assert!(visible(&h), "{bad}");
        assert_eq!(history(&h), 0, "{bad}");
    }
}

/// A document without data sets (the menu item is disabled then, but the control channel can open
/// it on any document) shows an empty list, and Apply only reports why.
#[test]
fn no_data_sets_shows_an_empty_list_and_apply_reports_it() {
    #[cfg(not(target_arch = "wasm32"))]
    photocraft_raster::spill::use_test_volume_space();
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    a.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut h = harness(a);
    photocraft_ui_egui::panels::apply_data_set_ui::open(h.state_mut());
    h.run_steps(3);
    assert!(h.state().dialog_open());
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(!h.state().dialog_open());
    assert!(!h.state().ui.status.is_empty(), "the failure is reported");
}
