use craft::ui::canvas::ShieldColor;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use craft::ui::runtime::requests::ColorRequest;
use serde_json::{Value, json};

use super::*;
use crate::color_picker_ui::{PickWindow, answer_for, hex, open_for_request, open_for_target, open_for_window, target, testing};

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 12})).unwrap();
    app
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// One pass of the dialog host alone, with no taker: a request the dialogs filed stays for the test to take.
fn host_pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.framework.dialogs.run(ui));
    // No renderer takes the pass's texture uploads.
    output.textures_delta.clear();
}

/// The values hosted dialog `id` holds, in their JSON form.
fn values(app: &PhotocraftApp, id: DialogId) -> Value {
    serde_json::to_value(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the dialog is open").fields()).unwrap()
}

/// The colour hosted dialog `id` holds under `key`, as `"#rrggbb"`.
fn held_color(app: &PhotocraftApp, id: DialogId, key: &str) -> Option<String> {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id)?.fields().color(key).map(hex)
}

/// The Picker's colour changes, as a drag in it or `ui.dialog.set` changes it, and a frame passes.
fn pick(app: &mut PhotocraftApp, picker: DialogId, color: &str) {
    testing::set_color(app, picker, color);
    sync(app);
}

/// A hosted Edit › Fill on Color…, with its Color Picker open over it as its colour row opens it.
struct FillPicker {
    fill: DialogId,
    picker: DialogId,
    /// The key of the colour row the picker answers.
    key: String,
}

/// Opens Edit › Fill, chooses Contents › Color, takes the colour row's request and opens its picker child.
fn fill_with_picker(app: &mut PhotocraftApp) -> FillPicker {
    let ctx = app.ctx().clone();
    let fill = crate::fill_ui::open(app);
    // The Color row shows once Contents is Color.
    app.framework.dialogs.set_json_field(&ctx, fill, "contents", json!("color")).unwrap();
    app.framework.dialogs.set_json_field(&ctx, fill, "color", json!([32.0 / 255.0, 160.0 / 255.0, 64.0 / 255.0])).unwrap();
    host_pass(app);
    // The request the Color row's press files: its key and the colour the row holds.
    let request = ColorRequest::new("color", craft::theme::Srgba::from_rgb_array([32.0 / 255.0, 160.0 / 255.0, 64.0 / 255.0]));
    let key = request.key.clone();
    let picker = open_for_request(app, fill, request);
    assert_eq!(top_of(app), Some(picker), "the picker opens over the Fill dialog");
    FillPicker { fill, picker, key }
}

fn top_of(app: &PhotocraftApp) -> Option<DialogId> {
    crate::color_picker_ui::top(app)
}

/// An agent's control call, as `ui.dialog.*` and `ui.set` arrive.
fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = app.ctx().clone();
    let (req, _rx) = crate::control::ControlRequest::new(method, params);
    let crate::control::Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("{method}: an immediate reply") };
    r
}

/// The colour the dialog `id` previews on its row `key`, as `"#rrggbb"`.
fn previewed(app: &PhotocraftApp, id: DialogId, key: &str) -> Option<String> {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id)?;
    dialog.previewed_colors().iter().find(|(row, _)| row == key).map(|(_, color)| hex([color.r, color.g, color.b]))
}

/// #9886: the Fill dialog's colour row follows the picker before OK, Cancel puts back what it showed, and nothing
/// reaches the document's History or the dialog's values.
#[test]
fn a_dialog_colour_follows_the_picker_and_cancel_restores_it() {
    let mut app = app();
    let FillPicker { fill, picker, key } = fill_with_picker(&mut app);
    let before = values(&app, fill);
    for color in ["#336699", "#ff0000", "#00ff00"] {
        testing::set_color(&mut app, picker, color);
        assert!(sync(&mut app), "{color}: the row shows the new colour");
        assert!(!sync(&mut app), "{color}: nothing new on a second pass");
        assert_eq!(previewed(&app, fill, &key).as_deref(), Some(color), "{color}: the row previews the picker's colour");
        assert_eq!(values(&app, fill), before, "the row's values are never written while previewing");
    }
    assert_eq!(previewed_dialogs(&app), vec![fill]);
    assert_eq!(past(&app), 0, "previewing makes no history step");
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    assert_eq!(values(&app, fill), before, "Cancel leaves the dialog as it was");
    assert!(previewed_dialogs(&app).is_empty());
    assert!(app.color_picker_live.is_empty());
    assert!(answer_for(&mut app, fill).is_none(), "Cancel answers nothing: {key}");
    assert_eq!(past(&app), 0);
}

/// The picker's OK only answers the row, still without a history step; the dialog's own OK then makes exactly one,
/// with the picked colour.
#[test]
fn ok_keeps_the_colour_and_the_dialog_makes_one_history_step() {
    let mut app = app();
    let FillPicker { fill, picker, key } = fill_with_picker(&mut app);
    pick(&mut app, picker, "#ff0000");
    pick(&mut app, picker, "#336699");
    crate::dialogs::testing::confirm(&mut app, picker).unwrap();
    assert!(app.color_picker_live.is_empty());
    assert_eq!(past(&app), 0, "the picker's OK only answers the dialog's colour row");
    let (answered_key, color) = answer_for(&mut app, fill).expect("OK answers the row");
    assert_eq!((answered_key.as_str(), rgb_hex(color).as_str()), (key.as_str(), "#336699"));
    assert!(answer_for(&mut app, fill).is_none(), "an answer is taken once");
    // The dialog takes the answer into its values as its pass does.
    app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(fill).expect("Fill is open").answer_color(key.clone(), color);
    host_pass(&mut app);
    assert_eq!(held_color(&app, fill, &key).as_deref(), Some("#336699"));

    crate::dialogs::testing::confirm(&mut app, fill).unwrap();
    assert_eq!(past(&app), 1, "one step for the Fill");
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(3, 3);
    let want = [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0];
    assert!((0..3).all(|i| (c[i] - want[i]).abs() < 1.5 / 255.0), "{c:?}");
}

/// Back on the colour it opened on, the row shows the value it held; a picker that never changes leaves the parent
/// alone.
#[test]
fn the_opening_colour_shows_the_original_value() {
    let mut app = app();
    let FillPicker { fill, picker, .. } = fill_with_picker(&mut app);
    let before = values(&app, fill);
    assert!(!sync(&mut app), "nothing to show yet");
    assert_eq!(values(&app, fill), before);
    pick(&mut app, picker, "#808080");
    assert_eq!(previewed_dialogs(&app), vec![fill]);
    // The opening colour is the one the row held: showing it again is the row's own value.
    testing::set_color(&mut app, picker, "#20a040");
    assert!(sync(&mut app), "back on the opening colour the preview ends");
    assert!(!sync(&mut app));
    assert_eq!(values(&app, fill), before);
    assert_eq!(held_color(&app, fill, "color").as_deref(), Some("#20a040"));
}

/// A picker closed without Cancel (a document closed, a host cancel) never leaves its colour behind, and a closed
/// parent ends the preview without an error or a panic.
#[test]
fn a_picker_or_parent_closed_any_other_way_ends_the_preview() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let FillPicker { fill, picker, .. } = fill_with_picker(&mut app);
    let before = values(&app, fill);
    pick(&mut app, picker, "#336699");
    // Closed by the host with no taker run: only `sync` can notice it.
    assert!(app.framework.dialogs.cancel_by_id(&ctx, picker));
    assert!(sync(&mut app), "the closed picker's colour is taken out of the row");
    assert_eq!(values(&app, fill), before);
    assert!(app.color_picker_live.is_empty());

    // A cancelled dialog takes its picker with it, leaving no picker whose OK has no target.
    let FillPicker { fill, picker, .. } = fill_with_picker(&mut app);
    pick(&mut app, picker, "#336699");
    crate::dialogs::testing::cancel(&mut app, fill).unwrap();
    assert!(app.framework.dialogs.get_by_id::<ColorPickerDialog>(picker).is_none(), "the picker closes with its dialog");
    assert!(target(&app, picker).is_none(), "and its target is dropped");
    assert!(app.color_picker_live.is_empty(), "the preview ends with its dialog");
    assert_eq!(past(&app), 0);
}

/// The dialog under a picker can't be confirmed by automation with a colour the picker still only shows.
#[test]
fn the_dialog_waits_for_its_picker() {
    let mut app = app();
    let FillPicker { fill, picker, .. } = fill_with_picker(&mut app);
    pick(&mut app, picker, "#336699");
    assert_eq!(previewed_dialogs(&app), vec![fill]);
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": fill.value().to_string()}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(past(&app), 0);
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(fill).is_some(), "the dialog stays open");
}

/// Automation can't edit a dialog under a previewing picker: Cancel would put the edit back.
#[test]
fn the_dialog_under_a_picker_refuses_control_edits() {
    let mut app = app();
    let FillPicker { fill, picker, .. } = fill_with_picker(&mut app);
    pick(&mut app, picker, "#336699");
    let before = values(&app, fill);
    let set = json!({"dialog": fill.value().to_string(), "field": "opacity", "value": 50});
    let r = call(&mut app, "ui.dialog.set", set.clone());
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(values(&app, fill), before);
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    let r = call(&mut app, "ui.dialog.set", set);
    assert_eq!(r["ok"], true, "{r}");
}

/// The crop shield's colour choice and custom colour.
fn shield(app: &PhotocraftApp) -> (ShieldColor, [u8; 3]) {
    let s = &app.ui.tool_options.crop.shield;
    (s.color, s.custom_color)
}

/// Opens the picker for the Crop shield's custom colour on the colour it holds.
fn shield_picker(app: &mut PhotocraftApp) -> DialogId {
    let custom = app.ui.tool_options.crop.shield.custom_color.map(|v| f32::from(v) / 255.0);
    open_for_target(app, "cropShield", "Color Picker (Shield Color)", custom).expect("the shield picker opens")
}

/// A shield colour set elsewhere (`ui.set cropShield`) while its picker is open is what Cancel restores.
#[test]
fn the_crop_shield_keeps_a_colour_set_while_its_picker_is_open() {
    // Cancel puts the shield back to what it held before the preview took it, unless something else set it since.
    let mut app = app();
    let picker = shield_picker(&mut app);
    pick(&mut app, picker, "#336699");
    app.ui.tool_options.crop.shield.custom_color = [1, 2, 3];
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    assert_eq!(shield(&app), (ShieldColor::Custom, [1, 2, 3]));

    let picker = shield_picker(&mut app);
    app.ui.tool_options.crop.shield.color = ShieldColor::MatchCanvas;
    pick(&mut app, picker, "#ff0000");
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    assert_eq!(shield(&app), (ShieldColor::MatchCanvas, [1, 2, 3]), "Cancel puts back the choice made while it was open");
}

/// The Crop shield's custom colour follows its picker; Cancel restores the colour and the Match Canvas choice, OK
/// keeps the custom one.
#[test]
fn the_crop_shield_colour_follows_its_picker() {
    let mut app = app();
    app.ui.tool_options.crop.shield.custom_color = [10, 20, 30];
    let before = shield(&app);
    assert_eq!(before.0, ShieldColor::MatchCanvas);
    let picker = shield_picker(&mut app);
    pick(&mut app, picker, "#336699");
    assert_eq!(shield(&app), (ShieldColor::Custom, [0x33, 0x66, 0x99]), "shown before OK");
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    assert_eq!(shield(&app), before);

    let picker = shield_picker(&mut app);
    pick(&mut app, picker, "#ff0000");
    pick(&mut app, picker, "#336699");
    crate::dialogs::testing::confirm(&mut app, picker).unwrap();
    assert_eq!(shield(&app), (ShieldColor::Custom, [0x33, 0x66, 0x99]));
    assert!(app.color_picker_live.is_empty());
    assert_eq!(past(&app), 0);
}

/// Preferences › Apply, like OK, waits for an open colour picker: it would otherwise save the colour the picker
/// previews and may still cancel.
#[test]
fn preferences_apply_waits_for_an_open_picker() {
    let mut app = app();
    crate::prefs_ui::open_preferences(&mut app, "guidesGridAndSlices");
    assert!(crate::prefs_ui::window_open(&app), "Preferences opens");
    let before = app.session.prefs().guides_grid_and_slices.guide_color.clone();
    let request = ColorRequest::new("guidesGridAndSlices.guideColor", Srgba::from_rgb_array([0.0, 1.0, 0.0]));
    let picker = open_for_window(&mut app, PickWindow::Preferences, request);
    pick(&mut app, picker, "#ff8000");
    assert!(crate::prefs_ui::apply_window(&mut app).is_err(), "Apply refuses while the picker is open");
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, before, "nothing is saved");
    crate::dialogs::testing::cancel(&mut app, picker).unwrap();
    assert!(crate::prefs_ui::apply_window(&mut app).is_ok(), "Apply works once the picker is closed");
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, before, "Cancel left the colour as it was");
}
