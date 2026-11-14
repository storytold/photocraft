//! #13981: the batch Lens Correction dialog shows its controls in the reference application's four
//! groups with the reference captions, Correction Options in two columns and no Alt+Reset.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(super) type H = egui_kittest::Harness<'static, PhotocraftApp>;

/// The whole app's dialogs with the batch Lens Correction open from the menu, without a document
/// (the command needs none).
pub(super) fn harness() -> H {
    harness_with(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()))
}

/// [`harness`] over `app` (open documents, a remembered state).
pub(super) fn harness_with(app: PhotocraftApp) -> H {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, super::COMMAND, json!({})).unwrap();
    h.run_steps(4);
    h
}

pub(super) fn fields(h: &H) -> &Map<String, Value> {
    &h.state().ui.dialogs.last().expect("the dialog is open").fields
}

/// Replaces the text of the text field `name` with `text`.
pub(super) fn type_into(h: &mut H, name: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, name).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
}

/// Sets the field `field` of the open dialog to `value` through `ui.dialog.set`, as scripts do.
pub(super) fn set(h: &mut H, field: &str, value: Value) {
    let dialog = h.state().ui.dialogs.last().expect("the dialog is open").id;
    let (req, _rx) = crate::ControlRequest::new("ui.dialog.set", json!({"dialog": dialog, "field": field, "value": value}));
    let ctx = h.ctx.clone();
    assert!(matches!(crate::control::handle(h.state_mut(), &ctx, &req), crate::control::Outcome::Done(_)));
    h.run_steps(2);
}

/// An entry of the open dropdown list (not the dropdown, which shows the current entry).
pub(super) fn entry<'a>(h: &'a H, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label).rfind(|n| n.accesskit_node().role() != Role::ComboBox).unwrap_or_else(|| panic!("no entry {label}"))
}

/// Opens the dropdown `name` and picks `label`.
pub(super) fn pick(h: &mut H, name: &str, label: &str) {
    h.get_by_role_and_label(Role::ComboBox, name).click();
    h.run_steps(3);
    entry(h, label).click();
    h.run_steps(3);
}

fn top(h: &H, role: Role, label: &str) -> f32 {
    h.get_by_role_and_label(role, label).rect().top()
}

/// A fresh folder `pc-lens-batch-<name>-<pid>` holding one source, `a.png`.
pub(super) fn fixture(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-lens-batch-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut image = photocraft_engine::Session::new();
    image.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    let png = photocraft_io::export(&image.active().unwrap().doc, "a.png", &Default::default()).unwrap().bytes;
    std::fs::write(dir.join("a.png"), png).unwrap();
    dir
}

const GROUPS: [&str; 4] = ["Source Files", "Destination Folder", "Lens Correction Profile", "Correction Options"];

#[test]
fn the_four_groups_follow_the_reference_order() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        let title = |label: &str| h.get_by_label(label).rect().top();
        for pair in GROUPS.windows(2) {
            assert!(title(pair[0]) < title(pair[1]), "group order: {pair:?}");
        }
        // Each group's controls sit below its title and above the next group's.
        let groups: [&[(Role, &str)]; 4] = [
            &[(Role::ComboBox, "Use"), (Role::Button, "Browse…")],
            &[(Role::ComboBox, "File Type"), (Role::TextInput, "Output"), (Role::Button, "Choose…")],
            &[(Role::ComboBox, "Profile")],
            &[(Role::CheckBox, "Geometric Distortion"), (Role::CheckBox, "Auto Scale Image"), (Role::ComboBox, "Edge")],
        ];
        for (i, controls) in groups.iter().enumerate() {
            for &(role, label) in *controls {
                let y = top(&h, role, label);
                assert!(title(GROUPS[i]) < y, "{label} is below {}", GROUPS[i]);
                if let Some(next) = GROUPS.get(i + 1) {
                    assert!(y < title(next), "{label} is above {next}");
                }
            }
        }
    });
}

#[test]
fn the_controls_carry_the_reference_captions() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        for (role, label) in [
            (Role::ComboBox, "File Type"),
            (Role::Button, "Choose…"),
            (Role::CheckBox, "Geometric Distortion"),
            (Role::CheckBox, "Chromatic Aberration"),
            (Role::CheckBox, "Vignette"),
            (Role::CheckBox, "Auto Scale Image"),
            (Role::Label, "Edge:"),
            (Role::ComboBox, "Edge"),
        ] {
            assert!(h.query_by_role_and_label(role, label).is_some(), "{label}");
        }
        for generated in ["Correct c a", "Correct distortion", "Correct vignette", "Auto scale", "Format"] {
            assert!(h.query_by_label(generated).is_none(), "the generated label {generated} is gone");
        }
    });
}

#[test]
fn correction_options_sit_in_two_columns() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        let rect = |role, label| h.get_by_role_and_label(role, label).rect();
        let left = ["Geometric Distortion", "Chromatic Aberration", "Vignette"].map(|l| rect(Role::CheckBox, l));
        for pair in left.windows(2) {
            assert!(pair[0].top() < pair[1].top() && (pair[0].left() - pair[1].left()).abs() < 1.0, "the left column: {pair:?}");
        }
        let (scale, caption, edge) = (rect(Role::CheckBox, "Auto Scale Image"), rect(Role::Label, "Edge:"), rect(Role::ComboBox, "Edge"));
        // A checkbox's node is its label, right of the box: Edge: starts left of it.
        let left_edge = left.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
        assert!(left_edge < caption.left() && caption.left() < scale.left(), "the right column starts right of the left one");
        assert!((scale.top() - left[0].top()).abs() < 1.0, "both columns start on the same row");
        assert!(
            scale.bottom() <= caption.top() && caption.right() <= edge.left() && (caption.center().y - edge.center().y).abs() < 2.0,
            "Edge: and its dropdown follow Auto Scale Image"
        );
    });
}

#[test]
fn edge_lists_the_reference_entries_in_order_with_the_stored_values() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        h.get_by_role_and_label(Role::ComboBox, "Edge").click();
        h.run_steps(3);
        let order = ["Edge Extension", "Transparency", "Black Color", "White Color"];
        let ys: Vec<f32> = order.iter().map(|l| entry(&h, l).rect().top()).collect();
        assert!(ys.windows(2).all(|p| p[0] < p[1]), "Edge lists {order:?}, got the tops {ys:?}");
        h.get_by_role_and_label(Role::ComboBox, "Edge").click();
        h.run_steps(3);
        for (label, value) in [("Edge Extension", "edgeExtension"), ("Black Color", "black"), ("White Color", "white"), ("Transparency", "transparency")] {
            pick(&mut h, "Edge", label);
            assert_eq!(fields(&h)["edge"], value, "{label}");
        }
    });
}

#[test]
fn alt_leaves_cancel_as_cancel_and_escape_cancels() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let dir = fixture("escape");
        let output = dir.join("out");
        set(&mut h, "input", json!([dir.join("a.png")]));
        type_into(&mut h, "Output", &output.to_string_lossy());
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.run_steps(2);
        assert!(h.query_by_role_and_label(Role::Button, "Cancel").is_some(), "Alt keeps Cancel");
        assert!(h.query_by_label("Reset").is_none(), "no Reset under Alt");
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        let ran = output.exists();
        std::fs::remove_dir_all(&dir).ok();
        assert!(h.state().ui.dialogs.is_empty(), "Escape closes the dialog");
        assert!(!ran, "Escape runs nothing");
    });
}

/// OK sends the params the form sent before the layout: the same keys.
#[test]
fn ok_runs_the_command_with_the_same_params() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        assert_eq!(
            crate::filter_dialog::params_of(fields(&h)),
            json!({"input": [], "output": "./corrected", "format": "psd", "profile": "auto", "correctDistortion": true, "correctVignette": false, "correctCA": false, "autoScale": true, "edge": "transparency"})
        );

        let dir = fixture("ok");
        let (input, output) = (json!([dir.join("a.png")]), dir.join("out").to_string_lossy().into_owned());
        set(&mut h, "input", input.clone());
        type_into(&mut h, "Output", &output);
        pick(&mut h, "File Type", "PNG");
        pick(&mut h, "Profile", "None");
        pick(&mut h, "Edge", "White Color");
        for label in ["Geometric Distortion", "Chromatic Aberration", "Vignette", "Auto Scale Image"] {
            h.get_by_role_and_label(Role::CheckBox, label).click();
            h.run_steps(2);
        }
        assert_eq!(
            crate::filter_dialog::params_of(fields(&h)),
            json!({"input": input, "output": output, "format": "png", "profile": "none", "correctDistortion": false, "correctVignette": true, "correctCA": true, "autoScale": false, "edge": "white"})
        );
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let written = dir.join("out").join("a.png").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        assert!(written, "OK runs the batch into the destination folder");
    });
}

/// Browse… picks the source files and Choose… the destination folder.
#[test]
fn browse_picks_the_source_and_choose_the_destination() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
        let answers = vec![Some(FileDialogAnswer::Paths(vec!["/tmp/sources/a.png".into()])), Some(FileDialogAnswer::FolderTo("/tmp/results".into()))];
        let (show, asked) = crate::file_dialog::fake(answers);
        let mut h = harness_with(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() }));
        let click = |h: &mut H, button: &str| {
            h.get_by_role_and_label(Role::Button, button).click();
            h.run_steps(2);
            let ctx = h.ctx.clone();
            h.state_mut().poll_file_dialog(&ctx, None);
            h.run_steps(2);
        };
        click(&mut h, "Browse…");
        assert_eq!((&fields(&h)["input"], &fields(&h)["output"]), (&json!(["/tmp/sources/a.png"]), &json!("./corrected")));
        click(&mut h, "Choose…");
        assert_eq!((&fields(&h)["input"], &fields(&h)["output"]), (&json!(["/tmp/sources/a.png"]), &json!("/tmp/results")));
        assert!(matches!(asked.borrow().as_slice(), [FileDialogRequest::Open { multiple: true, .. }, FileDialogRequest::Folder { .. }]));
    });
}

/// A field `ui.dialog.set` leaves without a value of its type shows its default, and OK sends what
/// the dialog shows.
#[test]
fn a_field_set_to_null_shows_and_sends_its_default() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        for field in ["autoScale", "edge"] {
            set(&mut h, field, Value::Null);
        }
        let ticked = h.get_by_role_and_label(Role::CheckBox, "Auto Scale Image").accesskit_node().toggled();
        assert_eq!(ticked, Some(egui::accesskit::Toggled::True), "Auto Scale Image shows its default");
        assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Edge").accesskit_node().value().as_deref(), Some("Transparency"));
        let params = crate::filter_dialog::params_of(fields(&h));
        assert_eq!((&params["autoScale"], &params["edge"]), (&json!(true), &json!("transparency")), "OK sends what is shown");
    });
}

/// A list of files set through `ui.dialog.set` stays the `input` OK sends, as the batch command
/// takes a folder or a list of files.
#[test]
fn a_file_list_set_as_input_is_kept_and_sent() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let dir = fixture("list");
        let files = json!([dir.join("a.png").to_string_lossy()]);
        set(&mut h, "input", files.clone());
        set(&mut h, "output", json!(dir.join("out").to_string_lossy()));
        set(&mut h, "format", json!("png"));
        let kept = crate::filter_dialog::params_of(fields(&h))["input"].clone();
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let written = dir.join("out").join("a.png").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(kept, files, "the drawn dialog keeps the list");
        assert!(written, "OK runs the batch over the listed file");
    });
}
