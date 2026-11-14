//! #14495: Image › Apply Data Set… opens its own small dialog, as in the reference application: a
//! "Data Set:" list of the document's sets in document order with the first one selected on every
//! open, Apply (default) and Cancel beside it. Apply, Enter and a double-click apply the selected
//! set once and close; Cancel, Escape and the close box change nothing. The dialog never writes the
//! variables or data sets, and Image › Variables › Data Sets… keeps its own page.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use serde_json::{Value, json};

const SETS: [&str; 3] = ["Zed", "Data Set 2", "Data Set 3"];

/// A document with a visibility and a pixel replacement variable on one layer, and three sets
/// (not in name order) that only set the visibility: Zed hides it, Data Set 2 shows it, Data Set 3
/// hides it. No set has an image for the pixel variable.
fn app() -> PhotocraftApp {
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

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            photocraft_ui_egui::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, photocraft_ui_egui::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Image › Apply Data Set… as the menu runs it, then a few frames.
fn open(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.applyDataSet", json!({})).unwrap();
    h.run_steps(3);
}

fn opened() -> Harness<'static, PhotocraftApp> {
    let mut h = harness(app());
    open(&mut h);
    h
}

fn index(h: &Harness<'static, PhotocraftApp>) -> Value {
    h.state().ui.dialogs.last().unwrap().fields["index"].clone()
}

fn visible(h: &Harness<'static, PhotocraftApp>) -> bool {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().visible
}

/// History states added since the test document was made (making its layer is one).
fn history(h: &Harness<'static, PhotocraftApp>) -> usize {
    let base = app().session.active().unwrap().history.past_len();
    h.state().session.active().unwrap().history.past_len() - base
}

fn variables(h: &Harness<'static, PhotocraftApp>) -> photocraft_doc::Variables {
    h.state().session.active().unwrap().doc.variables.clone()
}

#[test]
fn menu_opens_the_apply_data_set_dialog_not_the_variables_editor() {
    let h = opened();
    assert_eq!(h.state().ui.dialogs.len(), 1);
    let d = h.state().ui.dialogs.last().unwrap();
    assert_eq!(photocraft_ui_egui::dialogs::title(d), "Apply Data Set");
    assert_eq!(d.fields["__command"], "image.applyDataSet");
    assert!(!d.fields.contains_key("__variables"), "the dialog carries no editable variables");
    h.get_by_label("Data Set:");
    h.get_by_label("Apply");
    h.get_by_label("Cancel");
    for absent in ["OK", "Define", "Data Sets", "New", "Delete", "Name:", "▶"] {
        assert!(h.query_by_label(absent).is_none(), "{absent}");
    }
}

#[test]
fn variables_data_sets_page_keeps_its_own_dialog() {
    let mut h = harness(app());
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.variables.dataSets", json!({})).unwrap();
    h.run_steps(3);
    let d = h.state().ui.dialogs.last().unwrap();
    assert!(d.fields.contains_key("__variables"));
    assert_eq!(photocraft_ui_egui::dialogs::title(d), "Variables");
    h.get_by_label("OK");
    h.get_by_label("Define");
    assert!(h.query_by_label("Data Set:").is_none());
}

#[test]
fn list_shows_every_set_in_document_order_with_the_first_selected() {
    let h = opened();
    let tops: Vec<f32> = SETS.iter().map(|name| h.get_by_label(name).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "document order, not sorted: {tops:?}");
    assert_eq!(index(&h), 0);
    let selected: Vec<bool> = SETS.iter().map(|name| h.get_by_label(name).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).collect();
    assert_eq!(selected, [true, false, false]);
}

#[test]
fn first_set_is_selected_on_every_open_also_after_another_was_applied() {
    let mut h = opened();
    h.get_by_label("Data Set 3").click();
    h.run_steps(2);
    assert_eq!(index(&h), 2);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(variables(&h).active, Some(2));
    open(&mut h);
    assert_eq!(index(&h), 0);
}

#[test]
fn clicks_select_one_row_and_a_click_below_the_rows_keeps_it() {
    let mut h = opened();
    h.get_by_label("Data Set 2").click();
    h.run_steps(2);
    assert_eq!(index(&h), 1);
    for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::COMMAND] {
        h.get_by_label("Data Set 3").click_modifiers(modifiers);
        h.run_steps(2);
        assert_eq!(index(&h), 2, "{modifiers:?} selects only the clicked row");
        h.get_by_label("Zed").click_modifiers(modifiers);
        h.run_steps(2);
        assert_eq!(index(&h), 0, "{modifiers:?}");
    }
    h.get_by_label("Data Set 2").click_secondary();
    h.run_steps(2);
    assert_eq!(index(&h), 1, "right-click selects");
    assert_eq!(h.state().ui.dialogs.len(), 1, "and shows no menu");
    // The empty area under the last row.
    let below = h.get_by_label("Data Set 3").rect().center() + egui::vec2(0.0, 60.0);
    h.event(egui::Event::PointerMoved(below));
    h.event(egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: below, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    assert_eq!(index(&h), 1);
    assert!(visible(&h), "selecting applies nothing");
    assert_eq!(history(&h), 0);
}

/// Apply, Enter and a double-click each apply the selected set once and close the dialog.
#[test]
fn apply_enter_and_double_click_apply_the_selected_set_once() {
    for way in ["button", "enter", "double-click"] {
        let mut h = opened();
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
                // Two clicks in a row on a row that isn't selected yet, pressed and released
                // in separate frames as a mouse does.
                let at = h.get_by_label("Data Set 3").rect().center();
                h.hover_at(at);
                h.run_steps(40);
                for _ in 0..2 {
                    for pressed in [true, false] {
                        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                        h.step();
                    }
                }
            }
        }
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "{way}: closes");
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
    let mut h = opened();
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(!visible(&h));
    assert_eq!(variables(&h).active, Some(0));
}

#[test]
fn cancel_escape_and_the_close_box_change_nothing() {
    for way in ["cancel", "escape", "close"] {
        if way == "close" && cfg!(target_os = "macos") {
            continue; // No close box on macOS.
        }
        let mut h = opened();
        let before = variables(&h);
        let revision = h.state().session.active().unwrap().revision;
        h.get_by_label("Data Set 3").click();
        h.run_steps(2);
        match way {
            "cancel" => h.get_by_label("Cancel").click(),
            "escape" => h.key_press(egui::Key::Escape),
            _ => h.get_by_label("Close").click(),
        }
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "{way}");
        assert!(visible(&h), "{way}");
        assert_eq!(history(&h), 0, "{way}");
        assert_eq!(variables(&h), before, "{way}");
        assert_eq!(h.state().session.active().unwrap().revision, revision, "{way}");
    }
}

#[test]
fn holding_alt_keeps_cancel() {
    let mut h = opened();
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none());
    h.get_by_label("Cancel");
}

fn control(h: &mut Harness<'static, PhotocraftApp>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    let reply = match handle(h.state_mut(), &ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    };
    h.run_steps(2);
    reply
}

#[test]
fn control_channel_lists_sets_and_applies_the_selected_one() {
    let mut h = opened();
    let inspected = control(&mut h, "ui.inspect", json!({}));
    let dialogs = inspected["result"]["dialogs"].as_array().unwrap();
    let d = dialogs.iter().find(|d| d["fields"]["__command"] == "image.applyDataSet").expect("listed in ui.inspect");
    assert_eq!(d["fields"]["index"], 0);
    let id = d["id"].clone();

    let set = control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": 1}));
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(index(&h), 1);
    let confirmed = control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
    assert_eq!(confirmed["ok"], true, "{confirmed}");
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(variables(&h).active, Some(1));
    assert_eq!(history(&h), 1);

    open(&mut h);
    let id = h.state().ui.dialogs.last().unwrap().id;
    let cancelled = control(&mut h, "ui.dialog.cancel", json!({"dialog": id}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(history(&h), 1);
}

/// `ui.dialog.set` can store anything in `index`: a bad one is an error, never a panic or a
/// different set.
#[test]
fn bad_index_from_the_control_channel_is_an_error() {
    for bad in [json!(99), json!(3), json!(-1), json!("Zed"), json!(1.5), Value::Null, json!(u64::MAX), json!(4_294_967_296_u64)] {
        let mut h = opened();
        let id = h.state().ui.dialogs.last().unwrap().id;
        control(&mut h, "ui.dialog.set", json!({"dialog": id, "field": "index", "value": bad}));
        h.run_steps(2);
        let confirmed = control(&mut h, "ui.dialog.confirm", json!({"dialog": id}));
        assert_ne!(confirmed["ok"], true, "{bad}: {confirmed}");
        assert_eq!(variables(&h).active, None, "{bad}: no set was applied");
        assert!(visible(&h), "{bad}");
        assert_eq!(history(&h), 0, "{bad}");
    }
}

/// A document without data sets (the menu item is disabled then, but the control channel can open
/// it on any document) shows an empty list, and Apply only reports why.
#[test]
fn no_data_sets_shows_an_empty_list_and_apply_reports_it() {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    a.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut h = harness(a);
    let id = photocraft_ui_egui::panels::apply_data_set_ui::open(h.state_mut());
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.last().unwrap().id, id);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert!(!h.state().ui.status.is_empty(), "the failure is reported");
}

/// Every theme marks the selected row, also those without a row colour of their own.
#[test]
fn the_selected_row_is_visible_in_every_theme() {
    for kind in photocraft_ui_egui::theme::ThemeKind::ALL {
        let t = photocraft_ui_egui::theme::Tokens::for_kind(kind);
        let fill = photocraft_ui_egui::panels::apply_data_set_ui::selected_fill(&t);
        assert!(fill.a() > 0, "{kind:?}: the selected row has a fill");
        assert_ne!(fill, t.field, "{kind:?}: it differs from the list background");
    }
}
