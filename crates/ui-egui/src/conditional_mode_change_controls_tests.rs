//! #13676: File › Automate › Conditional Mode Change… ticks any set of source modes, with All and
//! None, and names the modes in full. Drawn with the real dialog window; OK runs the command.

use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::COMMAND;
use crate::PhotocraftApp;
use crate::warp_text_dialog_tests::harness;

/// The checkboxes as Photoshop's two columns show them, row by row.
const LEFT: [&str; 4] = ["Bitmap", "Grayscale", "Duotone", "Indexed Color"];
const RIGHT: [&str; 4] = ["RGB Color", "CMYK Color", "Lab Color", "Multichannel"];
const ALL_IDS: [&str; 8] = ["bitmap", "grayscale", "duotone", "indexed", "rgb", "cmyk", "lab", "multichannel"];

/// An RGB document with the dialog opened from the File menu.
fn opened() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    let id = crate::menus::invoke(&mut app, &egui::Context::default(), COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    (harness(app), id)
}

fn field(h: &Harness<'_, PhotocraftApp>, id: u64, key: &str) -> Value {
    h.state().ui.dialogs.iter().find(|d| d.id == id).map_or(Value::Null, |d| d.fields[key].clone())
}

fn checked(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn click(h: &mut Harness<'_, PhotocraftApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

fn rect(h: &Harness<'_, PhotocraftApp>, role: Role, label: &str) -> egui::Rect {
    h.get_by_role_and_label(role, label).rect()
}

fn mode(h: &Harness<'_, PhotocraftApp>) -> ColorMode {
    h.state().session.active().unwrap().doc.mode
}

fn steps(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.entries().len()
}

/// Open the Mode: list and pick `label` from it.
fn pick_target(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_role(Role::ComboBox).click();
    h.run_steps(3);
    let entry = h.query_all_by_label(label).find(|n| n.accesskit_node().role() != Role::CheckBox && n.accesskit_node().role() != Role::ComboBox);
    entry.unwrap_or_else(|| panic!("{label} in the open Mode: list")).click();
    h.run_steps(3);
}

#[test]
fn source_mode_has_eight_checkboxes_in_two_columns_with_all_and_none_under_them() {
    let (h, _) = opened();
    for label in ["Source Mode", "Target Mode", "Mode:"] {
        assert!(h.query_by_label(label).is_some(), "{label} in the dialog");
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
    assert!(h.get_by_role(Role::ComboBox).rect().top() > all.bottom(), "the Mode: list is below the Source Mode group");
    // Today's default ("any") ticks every source, and none is greyed for an RGB document.
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| checked(&h, l)));
    assert!(LEFT.into_iter().chain(RIGHT).all(|l| !h.get_by_role_and_label(Role::CheckBox, l).accesskit_node().is_disabled()));
    for raw in ["Any", "Rgb", "From", "To"] {
        assert!(h.query_by_label(raw).is_none(), "no raw {raw} control");
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
    // Until #13681 adds its alert, OK with nothing ticked converts nothing and closes.
    let before = steps(&h);
    click(&mut h, Role::Button, "None");
    click(&mut h, Role::Button, "OK");
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Rgb, before));
}

#[test]
fn the_mode_list_names_the_targets_in_full_and_escape_closes_only_the_list() {
    let (mut h, id) = opened();
    let before = steps(&h);
    h.get_by_role(Role::ComboBox).click();
    h.run_steps(3);
    let mut rows: Vec<(f32, String)> = h
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label().map(|l| (n.rect().top(), l.to_string())))
        // "Close" is the dialog's × box, drawn outside macOS.
        .filter(|(_, l)| !["All", "None", "OK", "Cancel", "Close"].contains(&l.as_str()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let names: Vec<&str> = rows.iter().map(|(_, l)| l.as_str()).collect();
    // #13678: all eight modes, in Photoshop's order.
    assert_eq!(names, ["Bitmap", "Grayscale", "Duotone", "Indexed Color", "RGB Color", "CMYK Color", "Lab Color", "Multichannel"]);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "Escape on the open list keeps the dialog");
    assert_eq!(field(&h, id, "to"), json!("rgb"));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "a second Escape cancels");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Rgb, before), "the document is unchanged");
}

#[test]
fn enter_sends_the_ticked_sources_in_the_list_order_and_converts() {
    let (mut h, id) = opened();
    click(&mut h, Role::Button, "None");
    for label in ["Lab Color", "RGB Color", "Grayscale"] {
        click(&mut h, Role::CheckBox, label);
    }
    assert_eq!(field(&h, id, "from"), json!(["grayscale", "rgb", "lab"]));
    pick_target(&mut h, "Lab Color");
    assert_eq!(field(&h, id, "to"), json!("lab"));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Enter is OK");
    assert_eq!(mode(&h), ColorMode::Lab);
}

#[test]
fn ok_leaves_a_document_whose_mode_is_not_ticked() {
    let (mut h, _) = opened();
    let before = steps(&h);
    click(&mut h, Role::CheckBox, "RGB Color");
    pick_target(&mut h, "Grayscale");
    click(&mut h, Role::Button, "OK");
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Rgb, before), "RGB Color unticked: nothing converts");
}

/// `ui.dialog.set` can still give `from` as "any", in a list or as one mode, and `to` in any
/// spelling the command takes: the boxes and the list show it, and OK passes it on unchanged.
#[test]
fn values_set_over_the_control_channel_show_and_reach_the_command() {
    let every: Vec<&str> = LEFT.into_iter().chain(RIGHT).collect();
    for (from, ticked, to, shown, converted) in [
        (json!("any"), every.as_slice(), "gray", "Grayscale", ColorMode::Grayscale),
        (json!(["any"]), every.as_slice(), "RGB", "RGB Color", ColorMode::Rgb),
        (json!(["lab", "ANY"]), every.as_slice(), "cmyk", "CMYK Color", ColorMode::Cmyk),
        (json!("gray"), &["Grayscale"][..], "cmyk", "CMYK Color", ColorMode::Rgb),
        (json!("rgb"), &["RGB Color"][..], "Lab", "Lab Color", ColorMode::Lab),
        (json!("rgb"), &["RGB Color"][..], "indexed", "Indexed Color", ColorMode::Indexed),
    ] {
        let (mut h, id) = opened();
        let ctx = h.ctx.clone();
        for (key, value) in [("from", from.clone()), ("to", json!(to))] {
            let (req, _rx) = crate::ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": key, "value": value}));
            let crate::control::Outcome::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("ui.dialog.set replies at once") };
            assert_eq!(reply["ok"], json!(true), "ui.dialog.set {key}: {reply}");
        }
        h.run_steps(3);
        for l in &every {
            assert_eq!(checked(&h, l), ticked.contains(l), "{from}: {l}");
        }
        assert_eq!(h.get_by_role(Role::ComboBox).accesskit_node().value().as_deref(), Some(shown), "to {to}");
        assert_eq!((field(&h, id, "from"), field(&h, id, "to")), (from.clone(), json!(to)), "drawing keeps the values");
        click(&mut h, Role::Button, "OK");
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(mode(&h), converted, "{from} to {to}");
    }
}
