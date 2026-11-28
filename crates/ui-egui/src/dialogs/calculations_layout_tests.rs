//! #14526: Image ▸ Calculations opens the reference application's layout: a Source 1 group (its
//! Channel and Invert rows), a Source 2 group, then Blending, Opacity, Scale and Offset for Add and
//! Subtract only, and Result listing New Document, New Channel and Selection with New Channel
//! chosen. Opacity is a whole percent whose arrow opens a slider; OK sends `scale` and `offset` for
//! the two modes that show them and today's values for every other Blending.
//!
//! The dialog is opened as its menu item opens it, on a solid red document, and read through its
//! own drawn rows. The Source and Layer dropdowns (#14520) and the Mask group (#14528) are not here:
//! this file only pins that their places in the order are held.

use craft::ui::dialog::{DialogId, StackDialog as _};
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;

use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{fields, set, top};

const CALCULATIONS: &str = "image.calculations";
const APPLY_IMAGE: &str = "image.applyImage";

/// A 4x3 RGB document with a Background, its layer filled solid red: the Red channel reads 1 and the
/// Green channel 0, so the dialog's sources give values a test can name.
fn app_with_document() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 4, "height": 3})).expect("a document");
    app.run("edit.fill", json!({"color": "#ff0000"})).expect("fill the layer solid red");
    app.sync_views();
    app
}

/// The dialog `command` as its menu item opens it, drawn on the app's own context.
fn dialog(command: &str) -> (Harness<'static, Box<PhotocraftApp>>, DialogId) {
    let mut app = app_with_document();
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, command, json!({})).expect("the command opens its dialog");
    let id = top(&app);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(900.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    (h, id)
}

fn calculations() -> (Harness<'static, Box<PhotocraftApp>>, DialogId) {
    dialog(CALCULATIONS)
}

/// The top edges of everything drawn with the caption `label`, in the dialog's own order.
fn tops(h: &Harness<'_, Box<PhotocraftApp>>, label: &str) -> Vec<f32> {
    let mut list: Vec<f32> = h.query_all_by_label(label).map(|node| node.rect().top()).collect();
    list.sort_by(f32::total_cmp);
    list
}

/// The top edge of the single row captioned `label`.
fn top_of(h: &Harness<'_, Box<PhotocraftApp>>, label: &str) -> f32 {
    tops(h, label).into_iter().next().unwrap_or_else(|| panic!("a row captioned {label}"))
}

/// The number field of the row `label`, as the dialog draws it.
fn field_of<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(Role::SpinButton, label)
}

/// Writes a row of the open form the way its control would, and lets the next passes redraw the rows.
fn write(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, key: &str, value: serde_json::Value) {
    set(h.state_mut(), id, key, value).unwrap_or_else(|error| panic!("the Calculations form takes {key}: {error}"));
    h.run_steps(3);
}

/// The messages of the alerts standing over the dialog.
fn messages(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|open| open.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Types `text` into the number field of the row `label` and leaves it with Tab: Enter would answer
/// the whole dialog with OK. The field is emptied first (backspaces, which do nothing once the text
/// is gone), so the typed digits are the whole value.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, text: &str) {
    let at = field_of(h, label).rect().center();
    type_at(h, at, text);
}

/// The same, clicked at the right edge of the row's node: for a row that draws a slider beside its
/// number field the node spans both, and a click in the middle moves the slider.
fn type_into_at_edge(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, text: &str) {
    let row = field_of(h, label).rect();
    type_at(h, egui::pos2(row.right() - 4.0, row.center().y), text);
}

/// Clicks at `at`, clears the field and types `text`, then leaves it with Tab.
fn type_at(h: &mut Harness<'static, Box<PhotocraftApp>>, at: egui::Pos2, text: &str) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(1);
    h.key_press(egui::Key::End);
    h.run_steps(1);
    for _ in 0..8 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(1);
    // One text event for the whole value: typed character by character, the field writes its own
    // number back between the keys, so a mark like "." never reaches the draft.
    h.event(egui::Event::Text(text.into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// Clicks OK on the alert standing over the dialog, not on the dialog's own OK.
fn dismiss_alert(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.get_by_role(Role::AlertDialog).get_by_label("OK").click();
    h.run_steps(4);
}

#[test]
fn the_rows_follow_the_reference_order() {
    let (h, _) = calculations();
    let source1 = top_of(&h, "Source 1");
    let source2 = top_of(&h, "Source 2");
    let channel = tops(&h, "Channel");
    let invert = tops(&h, "Invert");
    assert_eq!(channel.len(), 2, "each source group holds one Channel row");
    assert_eq!(invert.len(), 2, "each source group holds one Invert row");
    assert!(source1 < channel[0] && channel[0] < invert[0] && invert[0] < source2, "Source 1, its Channel, its Invert, then Source 2");
    assert!(source2 < channel[1] && channel[1] < invert[1], "and the same three rows under Source 2");
    let blending = top_of(&h, "Blending");
    let opacity = top_of(&h, "Opacity");
    let result = top_of(&h, "Result");
    assert!(invert[1] < blending, "the two groups come before Blending");
    assert!(blending < opacity && opacity < result, "Blending, then Opacity, then Result last");
    assert_eq!(h.query_all_by_role_and_label(Role::SpinButton, "Scale").count(), 0, "no Scale row with Blending Multiply");
    assert_eq!(h.query_all_by_role_and_label(Role::SpinButton, "Offset").count(), 0, "no Offset row with Blending Multiply");
}

#[test]
fn scale_and_offset_rows_stand_between_opacity_and_result_for_add_and_subtract() {
    let (mut h, id) = calculations();
    for mode in ["add", "subtract"] {
        write(&mut h, id, "blending", json!(mode));
        let scale = field_of(&h, "Scale").rect();
        let offset = field_of(&h, "Offset").rect();
        let opacity = top_of(&h, "Opacity");
        let result = top_of(&h, "Result");
        assert!(opacity < scale.top() && scale.top() < offset.top() && offset.top() < result, "{mode}: Scale and Offset under Opacity, above Result");
    }
    write(&mut h, id, "blending", json!("multiply"));
    assert_eq!(h.query_all_by_role_and_label(Role::SpinButton, "Scale").count(), 0, "Multiply takes the rows away again");
    assert_eq!(h.query_all_by_role_and_label(Role::SpinButton, "Offset").count(), 0, "and the Blending row alone changes nothing else");
}

#[test]
fn the_opacity_row_is_a_whole_percent_field_whose_arrow_opens_a_slider() {
    let (mut h, id) = calculations();
    assert_eq!(field_of(&h, "Opacity").accesskit_node().numeric_value(), Some(100.0), "Opacity starts at 100");
    let row = field_of(&h, "Opacity").rect();
    let in_row = |node: &egui_kittest::Node<'_>| (node.rect().center().y - row.center().y).abs() < 8.0;
    assert!(h.query_all_by_label("%").any(|node| in_row(&node)), "the percent sign stands in the Opacity row");
    assert_eq!(h.query_all_by_role(Role::Slider).filter(|node| in_row(node)).count(), 0, "no slider is drawn in the row");
    let at = h
        .query_all_by_role_and_label(Role::Button, "Slider")
        .find(|node| in_row(node))
        .expect("the Opacity row carries the popup-slider arrow")
        .rect()
        .center();
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert_eq!(h.query_all_by_role(Role::Slider).count(), 1, "the arrow opened exactly one slider");
    let track = h.query_all_by_role(Role::Slider).next().expect("the open slider").rect();
    let drag_to = |h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32| {
        let point = egui::pos2(x, track.center().y);
        h.hover_at(point);
        h.run_steps(1);
        h.drag_at(point);
        h.run_steps(1);
        h.drop_at(point);
        h.run_steps(3);
    };
    drag_to(&mut h, track.right() - 1.0);
    assert_eq!(fields(h.state(), id).number("opacity"), Some(100.0), "the slider's top end is 100 %");
    drag_to(&mut h, track.left() + 1.0);
    assert_eq!(fields(h.state(), id).number("opacity"), Some(0.0), "and its bottom end is 0 %");
}

/// Inside the range the field works in whole percents: a typed 56 stands as 56 %, and nothing stands
/// over the dialog.
#[test]
fn the_opacity_field_keeps_whole_percents_without_an_alert() {
    let (mut h, id) = calculations();
    type_into(&mut h, "Opacity", "56");
    assert_eq!(fields(h.state(), id).number("opacity"), Some(56.0), "a typed 56 stands");
    assert!(messages(&h).is_empty(), "a value inside 0..100 raises nothing: {:?}", messages(&h));
    assert_eq!(field_of(&h, "Opacity").accesskit_node().numeric_value(), Some(56.0), "the field shows the whole percent");
}

/// A typed value carrying more digits than the row keeps settles on that row's own grid, as measured:
/// Opacity 55.6 % becomes 56 %, Scale 1.23456 becomes 1.235 and Offset 10.6 becomes 11. Which decimal
/// mark a field takes follows the number format of the machine the test runs on (the hosted rows read
/// through the framework's own codec, which follows the OS region rather than the list
/// `i18n::decimal_separator` reports), so both marks are tried and the row must end at `want`. Each
/// row starts at a value other than `want`, so a declined mark cannot pass by leaving it behind.
#[test]
fn a_typed_value_with_too_many_digits_settles_on_the_rows_digits() {
    let (mut h, id) = calculations();
    write(&mut h, id, "blending", json!("add"));
    settle(&mut h, id, &NumberRow { label: "Opacity", key: "opacity", from: 100.0, text: "55.6", want: 56.0, slider_row: false });
    settle(&mut h, id, &NumberRow { label: "Scale", key: "scale", from: 1.0, text: "1.23456", want: 1.235, slider_row: true });
    settle(&mut h, id, &NumberRow { label: "Offset", key: "offset", from: 0.0, text: "10.6", want: 11.0, slider_row: true });
    assert!(messages(&h).is_empty(), "settling inside the bounds raises no alert: {:?}", messages(&h));
}

/// One row of the settling test: its caption and field key, the value to start from, the text to
/// type, the value the row must end at, and whether the row draws a slider beside its field (then
/// the click goes to the field's right edge, so it cannot move that slider).
struct NumberRow<'a> {
    label: &'a str,
    key: &'a str,
    from: f64,
    text: &'a str,
    want: f64,
    slider_row: bool,
}

/// Types `text` (with both decimal marks in turn, the row first set to `from`) into the number field
/// of the row and asserts it ends at `want`.
fn settle(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, row: &NumberRow<'_>) {
    for mark in [",", "."] {
        set(h.state_mut(), id, row.key, json!(row.from)).unwrap_or_else(|error| panic!("the {} row starts at {}: {error}", row.label, row.from));
        h.run_steps(2);
        let text = row.text.replace('.', mark);
        if row.slider_row {
            type_into_at_edge(h, row.label, &text);
        } else {
            type_into(h, row.label, &text);
        }
        if fields(h.state(), id).number(row.key) == Some(row.want) {
            return;
        }
    }
    panic!("neither decimal mark settled {} {} to {}: {:?}", row.label, row.text, row.want, fields(h.state(), id).number(row.key));
}

/// Past the range the nearest end is kept: a typed 150 % becomes 100 %. The measured reference shows
/// no alert here; the pinned framework raises one range alert for a typed value past a row's range
/// and the app's hosted-dialog host opens it for every parameter dialog
/// (`dialogs::answer_requests`), which is what Create Ellipse and the other dialogs do. A silent
/// clamp for this row alone needs a switch in craft's number row, asked on the issue; this test pins
/// the shared behaviour the dialog has today.
#[test]
fn a_typed_opacity_past_the_range_keeps_the_nearest_percent() {
    let (mut h, id) = calculations();
    type_into(&mut h, "Opacity", "150");
    assert_eq!(fields(h.state(), id).number("opacity"), Some(100.0), "150 % becomes 100 %");
    assert_eq!(h.query_all_by_role(Role::AlertDialog).count(), 1, "the dialog's standard range alert stands once");
    assert!(messages(&h).iter().any(|m| m.contains("Closest value inserted")), "{:?}", messages(&h));
    dismiss_alert(&mut h);
    assert!(messages(&h).is_empty(), "OK closes the alert");
    assert_eq!(fields(h.state(), id).number("opacity"), Some(100.0), "the closest end stays");
    assert!(set(h.state_mut(), id, "opacity", json!(-5)).is_err(), "a value below 0 never reaches the row");
    assert_eq!(fields(h.state(), id).number("opacity"), Some(100.0), "and the field keeps what it had");
    assert!(h.state().framework.dialogs.get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(id).is_some(), "the dialog stayed open throughout");
}

#[test]
fn the_result_row_lists_new_document_then_new_channel_then_selection() {
    let (mut h, id) = calculations();
    assert_eq!(fields(h.state(), id).text("result"), Some("newChannel"), "New Channel is chosen while the dialog opens");
    h.get_by_role_and_label(Role::ComboBox, "Result").click();
    h.run_steps(2);
    let list = h.query_all_by_role(Role::ListBox).last().expect("the open Result list").rect();
    let entry_top = |name: &str| {
        h.query_all_by_label(name)
            .filter(|node| list.contains(node.rect().center()))
            .map(|node| node.rect().top())
            .min_by(f32::total_cmp)
            .unwrap_or_else(|| panic!("the Result list holds a {name} entry"))
    };
    let (document, channel, selection) = (entry_top("New Document"), entry_top("New Channel"), entry_top("Selection"));
    assert!(document < channel && channel < selection, "New Document above New Channel above Selection: {document}, {channel}, {selection}");
    h.query_all_by_label("New Document").find(|node| list.contains(node.rect().center())).expect("the New Document entry").click();
    h.run_steps(2);
    assert_eq!(fields(h.state(), id).text("result"), Some("newDocument"), "the entry keeps the param id of today");
    assert_eq!(h.query_all_by_role_and_label(Role::SpinButton, "Scale").count(), 0, "changing Result shows no other row");
}

/// The number OK would send for `key`, or None while the form holds no value there.
fn sent(params: &serde_json::Value, key: &str) -> Option<f64> {
    params.get(key).and_then(serde_json::Value::as_f64)
}

#[test]
fn ok_with_add_sends_the_scale_and_offset_the_rows_hold() {
    let (mut h, id) = calculations();
    write(&mut h, id, "blending", json!("add"));
    write(&mut h, id, "scale", json!(1.5));
    write(&mut h, id, "offset", json!(10));
    let params = crate::filter_dialog::command_params(CALCULATIONS, fields(h.state(), id));
    assert_eq!(params["blending"], json!("add"), "OK sends the Blending row");
    assert_eq!(sent(&params, "scale"), Some(1.5), "OK sends Scale 1.5");
    assert_eq!(sent(&params, "offset"), Some(10.0), "OK sends Offset 10");
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK runs the command");
    h.run_steps(3);
    let (name, ran) = h.state().session.journal.iter().rev().find(|(name, _)| name == CALCULATIONS).expect("Calculations ran");
    assert_eq!(name, CALCULATIONS);
    assert_eq!((sent(ran, "scale"), sent(ran, "offset")), (Some(1.5), Some(10.0)), "{ran}");
    assert_eq!(h.state().session.active().expect("the document").doc.channels.len(), 1, "the default Result adds one channel");
}

/// Any other Blending keeps what OK sends today: Scale 1 and Offset 0 at most, never a value the
/// user typed while the rows were up.
#[test]
fn another_blending_sends_todays_scale_and_offset() {
    let (mut h, id) = calculations();
    write(&mut h, id, "blending", json!("add"));
    write(&mut h, id, "scale", json!(1.5));
    write(&mut h, id, "offset", json!(10));
    write(&mut h, id, "blending", json!("screen"));
    let params = crate::filter_dialog::command_params(CALCULATIONS, fields(h.state(), id));
    assert_eq!(params["blending"], json!("screen"));
    assert_eq!(sent(&params, "scale"), Some(1.0), "the hidden Scale row is back at the command's own 1: {params}");
    assert_eq!(sent(&params, "offset"), Some(0.0), "and the hidden Offset row at its 0: {params}");
    assert_eq!(sent(&params, "opacity"), Some(100.0), "the other rows are unchanged");
}

/// The two rows carry the reference's bounds, sliders and digit counts: Scale 1..2 with a drawn
/// slider whose ends are 1 and 2 and at most three decimals in the field, Offset -255..255 with a
/// drawn slider centred at 0 whose ends are -255 and 255 and whole numbers in the field. Automation
/// cannot write past a bound: the row refuses it.
#[test]
fn the_scale_and_offset_rows_keep_their_bounds_and_digits() {
    let (mut h, id) = calculations();
    write(&mut h, id, "blending", json!("add"));
    assert_eq!(field_of(&h, "Scale").accesskit_node().numeric_value(), Some(1.0), "Scale starts at 1");
    assert_eq!(field_of(&h, "Offset").accesskit_node().numeric_value(), Some(0.0), "Offset starts at 0");
    for bad in [0.0, 3.0] {
        assert!(set(h.state_mut(), id, "scale", json!(bad)).is_err(), "Scale refuses {bad}: its bound is 1..2");
    }
    for bad in [-300.0, 256.0] {
        assert!(set(h.state_mut(), id, "offset", json!(bad)).is_err(), "Offset refuses {bad}: its bound is -255..255");
    }
    let field_scale = field_of(&h, "Scale").rect();
    h.query_all_by_role(Role::Slider).find(|n| (n.rect().center().y - field_scale.center().y).abs() < 8.0).expect("the Scale row draws its slider");
    let field_offset = field_of(&h, "Offset").rect();
    h.query_all_by_role(Role::Slider).find(|n| (n.rect().center().y - field_offset.center().y).abs() < 8.0).expect("the Offset row draws its slider");
    // Dragging each slider across its track gives the reference's bounds, and the value the row keeps
    // shows its digits: Scale at most three of them, Offset whole numbers.
    let drag_slider = |h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, along: f32| {
        let row = field_of(h, label).rect();
        let track = h.query_all_by_role(Role::Slider).find(|n| (n.rect().center().y - row.center().y).abs() < 8.0).expect("the slider of that row").rect();
        let point = egui::pos2(track.left() + 1.0 + (track.width() - 2.0) * along, track.center().y);
        h.hover_at(point);
        h.run_steps(1);
        h.drag_at(point);
        h.run_steps(1);
        h.drop_at(point);
        h.run_steps(3);
    };
    drag_slider(&mut h, "Scale", 1.0);
    assert_eq!(fields(h.state(), id).number("scale"), Some(2.0), "the Scale slider's top end is 2");
    drag_slider(&mut h, "Scale", 0.0);
    assert_eq!(fields(h.state(), id).number("scale"), Some(1.0), "and its bottom end is 1");
    drag_slider(&mut h, "Scale", 0.75);
    let scale = fields(h.state(), id).number("scale").expect("the Scale row holds a number");
    assert_eq!((scale * 1000.0).round() / 1000.0, scale, "Scale keeps at most three decimals: {scale}");
    drag_slider(&mut h, "Offset", 1.0);
    assert_eq!(fields(h.state(), id).number("offset"), Some(255.0), "the Offset slider's top end is 255");
    drag_slider(&mut h, "Offset", 0.0);
    assert_eq!(fields(h.state(), id).number("offset"), Some(-255.0), "and its bottom end is -255");
    drag_slider(&mut h, "Offset", 0.5);
    let offset = fields(h.state(), id).number("offset").expect("the Offset row holds a number");
    assert_eq!(offset.fract(), 0.0, "Offset keeps whole numbers: {offset}");
    assert_eq!(offset, 0.0, "and its track is centred at 0");
    assert_eq!(h.query_all_by_role(Role::AlertDialog).count(), 0, "the sliders stay inside their bounds, so nothing alerts");
}

/// Neighbour case: Apply Image shares the channel-form opener and keeps its own rows - Scale and
/// Offset there do not wait for Add or Subtract, and its Preserve Transparency and Mask Channel rows
/// stay.
#[test]
fn apply_image_keeps_its_own_rows() {
    let (h, _) = dialog(APPLY_IMAGE);
    assert_eq!(field_of(&h, "Scale").accesskit_node().numeric_value(), Some(1.0), "Apply Image shows Scale with Blending Multiply");
    assert_eq!(field_of(&h, "Offset").accesskit_node().numeric_value(), Some(0.0), "and Offset too");
    assert!(h.query_by_role_and_label(Role::CheckBox, "Preserve Transparency").is_some(), "its Preserve Transparency row stays");
    assert!(h.query_by_role_and_label(Role::ComboBox, "Mask Channel").is_some(), "and its Mask Channel row");
    assert!(h.query_all_by_label("Source 1").next().is_none(), "Apply Image gains no source groups from this change");
}

/// Neighbour case: the live Preview of #14530 is untouched by the new rows - the box is still there,
/// on while the dialog opens, and OK still runs the same command.
#[test]
fn the_form_keeps_its_preview_box_on() {
    let (h, id) = calculations();
    let dialog = h.state().framework.dialogs.get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(id).expect("the form is open");
    assert_eq!(dialog.key(), Some(CALCULATIONS));
    assert!(dialog.is_previewing(), "the Preview box is still there, on");
}
