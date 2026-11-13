//! #13676: File › Automate › Conditional Mode Change… ticks any set of source modes, with All and
//! None, and names the modes in full. Drawn with the real dialog window; OK runs the command.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::COMMAND;
use crate::PhotocraftApp;
use crate::warp_text_dialog_tests::harness;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The checkboxes as Photoshop's two columns show them, row by row.
const LEFT: [&str; 4] = ["Bitmap", "Grayscale", "Duotone", "Indexed Color"];
const RIGHT: [&str; 4] = ["RGB Color", "CMYK Color", "Lab Color", "Multichannel"];
const ALL_IDS: [&str; 8] = ["bitmap", "grayscale", "duotone", "indexed", "rgb", "cmyk", "lab", "multichannel"];

/// An RGB document with the dialog opened from the File menu.
fn opened() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (harness(app), id)
}

fn is_open(h: &H, id: DialogId) -> bool {
    h.state().framework.dialogs.open_dialogs().iter().any(|d| d.id == id)
}

fn field(h: &H, id: DialogId, key: &str) -> Value {
    h.state()
        .framework
        .dialogs
        .get_by_id::<HostedParamDialog>(id)
        .and_then(|d| d.fields().get(key).and_then(|v| serde_json::to_value(v).ok()))
        .unwrap_or(Value::Null)
}

fn checked(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn click(h: &mut H, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

fn rect(h: &H, role: Role, label: &str) -> egui::Rect {
    h.get_by_role_and_label(role, label).rect()
}

fn mode(h: &H) -> ColorMode {
    h.state().session.active().unwrap().doc.mode
}

fn steps(h: &H) -> usize {
    h.state().session.active().unwrap().history.entries().len()
}

/// OK as the default button files it; the family runs the command in the passes after it.
fn ok(h: &mut H, id: DialogId) {
    try_ok(h, id).expect("OK is accepted");
}

/// OK as [`ok`] does, with the refusal of a held OK as the error.
pub(super) fn try_ok(h: &mut H, id: DialogId) -> Result<(), String> {
    let answer = crate::dialogs::testing::confirm(h.state_mut(), id);
    h.run_steps(3);
    answer
}

fn set(h: &mut H, id: DialogId, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(3);
}

#[test]
fn source_mode_has_eight_checkboxes_in_two_columns_with_all_and_none_under_them() {
    let (h, _) = opened();
    for label in ["Source Mode", "Target Mode", "Mode"] {
        assert!(h.query_all_by_label(label).next().is_some(), "{label} in the dialog");
    }
    let mut above = f32::MIN;
    for (left, right) in LEFT.into_iter().zip(RIGHT) {
        let (l, r) = (rect(&h, Role::CheckBox, left), rect(&h, Role::CheckBox, right));
        assert!(l.right() < r.left() && (l.center().y - r.center().y).abs() < 1.0, "{left} and {right} share a row: {l:?} {r:?}");
        assert!(l.top() > above, "{left} is below the row above it");
        above = l.bottom();
    }
    let (all, none) = (rect(&h, Role::Button, "All"), rect(&h, Role::Button, "None"));
    assert!(
        all.top() > above && (all.center().y - none.center().y).abs() < 1.0 && all.right() < none.left(),
        "All, None under the checkboxes: {all:?} {none:?}"
    );
    assert!(
        h.query_all_by_label("Target Mode").map(|n| n.rect().top()).fold(f32::MAX, f32::min) > all.bottom(),
        "the Target Mode section is below the Source Mode one"
    );
    // Today's default ("any") ticks every source, and none is greyed for an RGB document.
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| checked(&h, l)));
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| !h.get_by_role_and_label(Role::CheckBox, l).accesskit_node().is_disabled()));
    for raw in ["Any", "From", "To"] {
        assert!(h.query_all_by_label(raw).next().is_none(), "no raw {raw} control");
    }
}

#[test]
fn none_clears_and_all_ticks_every_source() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "None");
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| !checked(&h, l)), "None clears all eight");
    assert_eq!(field(&h, id, "from"), json!([]));
    click(&mut h, Role::Button, "All");
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| checked(&h, l)), "All ticks all eight");
    assert_eq!(field(&h, id, "from"), json!(ALL_IDS));
}

#[test]
fn the_mode_list_names_the_targets_in_full() {
    let rows = super::rows();
    let to = rows.iter().find(|p| p.key == "to").expect("a Mode list");
    let craft::ui::patterns::form_dialogs::ParamKind::Choice { options, default, .. } = &to.kind else { panic!("Mode is a list") };
    let names: Vec<&str> = options.iter().map(|(_, label)| label.as_str()).collect();
    // #13678: all eight modes, in Photoshop's order.
    assert_eq!(names, ["Bitmap", "Grayscale", "Duotone", "Indexed Color", "RGB Color", "CMYK Color", "Lab Color", "Multichannel"]);
    assert_eq!(default.as_deref(), Some("rgb"));
    assert_eq!(to.label.as_str(), "Mode");
}

#[test]
fn ticked_sources_go_to_the_command_in_the_list_order_and_convert() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "None");
    for label in ["Lab Color", "RGB Color", "Grayscale"] {
        click(&mut h, Role::CheckBox, label);
    }
    assert_eq!(field(&h, id, "from"), json!(["grayscale", "rgb", "lab"]));
    set(&mut h, id, "to", json!("lab"));
    ok(&mut h, id);
    assert!(!is_open(&h, id));
    assert_eq!(mode(&h), ColorMode::Lab);
}

#[test]
fn ok_leaves_a_document_whose_mode_is_not_ticked() {
    let (mut h, id) = opened();
    let before = steps(&h);
    click(&mut h, Role::CheckBox, "RGB Color");
    set(&mut h, id, "to", json!("grayscale"));
    assert!(try_ok(&mut h, id).is_err(), "OK is refused");
    assert!(is_open(&h, id), "the dialog stays under its alert (#13681)");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Rgb, before), "RGB Color unticked: nothing converts");
}

/// `ui.dialog.set` can still give `from` as "any", in a list or as one mode: the boxes show it, and OK passes it on
/// unchanged.
#[test]
fn values_set_over_the_control_channel_show_and_reach_the_command() {
    let every: Vec<&str> = LEFT.into_iter().chain(RIGHT).collect();
    for (from, ticked, to, converted) in [
        (json!("any"), every.as_slice(), "grayscale", ColorMode::Grayscale),
        (json!(["any"]), every.as_slice(), "rgb", ColorMode::Rgb),
        (json!(["lab", "ANY"]), every.as_slice(), "cmyk", ColorMode::Cmyk),
        (json!("gray"), &["Grayscale"][..], "cmyk", ColorMode::Rgb),
        (json!("rgb"), &["RGB Color"][..], "lab", ColorMode::Lab),
        (json!("rgb"), &["RGB Color"][..], "indexed", ColorMode::Indexed),
    ] {
        let (mut h, id) = opened();
        set(&mut h, id, "from", from.clone());
        set(&mut h, id, "to", json!(to));
        for l in &every {
            assert_eq!(checked(&h, l), ticked.contains(l), "{from}: {l}");
        }
        assert_eq!(field(&h, id, "from"), from, "drawing keeps the value");
        let _ = try_ok(&mut h, id);
        if converted == ColorMode::Indexed {
            // #13678: the Indexed Color dialog asks for its options first.
            let (key, target) = open_dialog(&h).expect("a dialog is open");
            assert_eq!(key, "image.mode.indexedColor");
            ok(&mut h, target);
        }
        // Nothing to convert keeps the dialog under its alert (#13681).
        assert_eq!(is_open(&h, id), converted == ColorMode::Rgb, "{from} to {to}");
        assert_eq!(mode(&h), converted, "{from} to {to}");
    }
}

/// A 50% gray RGB document with the dialog opened from the File menu.
fn opened_gray() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20, "background": "#808080"})).unwrap();
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (harness(app), id)
}

/// The one open dialog: its key (the command it runs) and id.
fn open_dialog(h: &H) -> Option<(String, DialogId)> {
    let dialogs = &h.state().framework.dialogs;
    let [d] = dialogs.open_dialogs().try_into().ok()?;
    let key = dialogs.get_by_id::<HostedParamDialog>(d.id).and_then(|dialog| craft::ui::dialog::StackDialog::key(dialog).map(str::to_owned))?;
    Some((key, d.id))
}

fn history(h: &H) -> Vec<String> {
    h.state().session.active().unwrap().history.entries()
}

/// #13678: OK with Bitmap, Duotone or Indexed Color as the target opens that mode's own dialog,
/// as Image › Mode does, and nothing converts yet. Its OK converts with the options it shows.
#[test]
fn ok_opens_the_target_modes_dialog_whose_ok_converts_with_its_options() {
    for (target, command, key, option) in [
        ("bitmap", "image.mode.bitmap", "method", "threshold"),
        ("duotone", "image.mode.duotone", "type", "monotone"),
        ("indexed", "image.mode.indexedColor", "palette", "web"),
    ] {
        let (mut h, id) = opened_gray();
        let before = history(&h);
        set(&mut h, id, "to", json!(target));
        ok(&mut h, id);
        let (open, dialog) = open_dialog(&h).unwrap_or_else(|| panic!("{target}: a dialog is open"));
        assert_eq!(open, command, "{target}: its own dialog is open");
        assert_eq!((mode(&h), history(&h)), (ColorMode::Rgb, before.clone()), "{target}: nothing converted yet");
        set(&mut h, dialog, key, json!(option));
        ok(&mut h, dialog);
        assert!(open_dialog(&h).is_none(), "{target}: its OK closes it");
        let d = &h.state().session.active().unwrap().doc;
        match target {
            "bitmap" => {
                // 50% Threshold leaves an even gray one colour; Diffusion Dither would mix both.
                let buf = photocraft_compose::flatten(d);
                let px = &buf.px;
                assert_eq!(d.mode, ColorMode::Bitmap);
                assert!(px.iter().all(|p| p[0] == px[0][0]), "50% Threshold, not Diffusion Dither");
            }
            "duotone" => {
                assert_eq!(d.mode, ColorMode::Duotone);
                assert_eq!(d.duotone.as_ref().map(|t| t.inks.len()), Some(1), "Monotone: one ink");
            }
            _ => {
                assert_eq!(d.mode, ColorMode::Indexed);
                // Selective keeps the one gray and the forced Black and White; Web is its 216 colours.
                assert!(d.color_table.as_ref().is_some_and(|t| t.colors.len() >= 216), "Web palette");
            }
        }
        assert_eq!(history(&h).len(), before.len() + if target == "indexed" { 1 } else { 2 }, "{target}: {:?}", history(&h));
    }
}

/// #13678: Cancel in the target mode's dialog leaves the document and its History as they were.
#[test]
fn cancel_in_the_target_modes_dialog_leaves_the_document_as_it_was() {
    for target in ["bitmap", "duotone", "indexed"] {
        let (mut h, id) = opened_gray();
        let before = history(&h);
        set(&mut h, id, "to", json!(target));
        ok(&mut h, id);
        let (open, dialog) = open_dialog(&h).unwrap_or_else(|| panic!("{target}: a dialog is open"));
        assert_ne!(open, COMMAND, "{target}: its own dialog is open");
        crate::dialogs::testing::cancel(h.state_mut(), dialog).expect("Cancel");
        h.run_steps(3);
        assert!(open_dialog(&h).is_none(), "{target}: Cancel closes it");
        assert_eq!((mode(&h), history(&h)), (ColorMode::Rgb, before), "{target}: the document and History are unchanged");
    }
}
