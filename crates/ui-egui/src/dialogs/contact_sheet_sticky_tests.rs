//! #13695: Contact Sheet II opens with every value of the last OK'd run, the folder included, also
//! after a restart. Cancel, Escape and a failed run keep the values from before, the Files list is
//! not kept, and a stored value the dialog cannot show gives its default. Drawn with the real dialog.

use std::sync::{Arc, Mutex};

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use super::contact_sheet_layout_tests::{H, fields, harness_with, image_folder, pick, shown, type_into};
use super::{COMMAND, RES_UNIT};
use crate::prefs_ui::tests::{launched_on, stored};

/// The preferences file of a launch, shared by the launches on it.
type Store = Arc<Mutex<Option<String>>>;

/// An app launched on `store` with the dialog open.
fn opened_on(store: &Store) -> H {
    harness_with(launched_on(store))
}

fn opened() -> H {
    opened_on(&Store::default())
}

/// Open the dialog again from the File menu, in the same window; its dialog was closed.
fn reopen(h: &mut H) {
    assert!(h.state().ui.dialogs.is_empty(), "the dialog was closed");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);
}

/// The values the dialog holds: the command's params and the resolution unit.
pub(super) fn values(h: &H) -> Map<String, Value> {
    fields(h).iter().filter(|(k, _)| !k.starts_with("__") || *k == RES_UNIT).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// What the preferences in memory hold for the dialog.
fn kept(h: &H) -> Option<Value> {
    h.state().session.prefs().dialogs.get(COMMAND).cloned()
}

pub(super) fn click(h: &mut H, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

pub(super) fn text_of(h: &H, role: Role, name: &str) -> String {
    h.get_by_role_and_label(role, name).accesskit_node().value().unwrap_or_default().to_string()
}

pub(super) fn ticked(h: &H, name: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, name).accesskit_node().toggled() == Some(Toggled::True)
}

/// Types `text` into the text field `name`, replacing what it held.
fn type_text(h: &mut H, name: &str, text: &str) {
    click(h, Role::TextInput, name);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// In the order `edit_everything` clicks them: unticking Include Subfolders greys Group Images by
/// Folder, and unticking the caption greys the font settings.
const CHECKBOXES: [(&str, &str); 6] = [
    ("groupByFolder", "Group Images by Folder"),
    ("includeSubfolders", "Include Subfolders"),
    ("flatten", "Flatten All Layers"),
    ("rotateForBestFit", "Rotate For Best Fit"),
    ("autoSpacing", "Use Auto-Spacing"),
    ("caption", "Use Filename as Caption"),
];

/// Changes every control from what it shows, whatever the defaults are, reading the images from
/// `dir`.
fn edit_everything(h: &mut H, dir: &std::path::Path) {
    type_text(h, "Input", &dir.to_string_lossy());
    pick(h, "Units", "pixels");
    type_into(h, "Resolution", "200");
    let other = if text_of(h, Role::ComboBox, "Resolution Unit") == "pixels/cm" { "pixels/inch" } else { "pixels/cm" };
    pick(h, "Resolution Unit", other);
    pick(h, "Mode", "Grayscale");
    pick(h, "Bit Depth", "16-bit");
    pick(h, "Place", "down first");
    type_into(h, "Columns", "2");
    type_into(h, "Rows", "3");
    type_text(h, "Font", "Serif");
    type_into(h, "Font size", "9");
    for (_, label) in CHECKBOXES {
        click(h, Role::CheckBox, label);
    }
}

/// Presses OK, which builds a sheet from the folder.
fn ok(h: &mut H) {
    let sheets = h.state().session.documents().len();
    click(h, Role::Button, "OK");
    assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
    assert_eq!(h.state().session.documents().len(), sheets + 1, "OK built a sheet");
}

/// A run with every control changed was OK'd and the dialog is open again; its values.
fn after_a_run(h: &mut H, tag: &str) -> Map<String, Value> {
    let dir = image_folder(tag);
    let defaults = values(h);
    let unit = fields(h).get(RES_UNIT).cloned();
    edit_everything(h, &dir);
    let run = values(h);
    assert_eq!(run["input"], json!(dir.to_string_lossy()));
    assert!(run.get(RES_UNIT).is_some_and(|u| Some(u) != unit.as_ref()), "the resolution unit was changed: {run:?}");
    for (key, d) in &defaults {
        assert_ne!(&run[key], d, "{key} was changed");
    }
    ok(h);
    std::fs::remove_dir_all(&dir).ok();
    reopen(h);
    run
}

#[test]
fn after_ok_the_next_open_shows_every_value_of_that_run_including_the_folder() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = opened();
        let run = after_a_run(&mut h, "sticky-ok");
        assert_eq!(values(&h), run);
        // The controls show them, not only the fields.
        let per_cm = run[RES_UNIT] == "cm";
        let combos = ["Units", "Resolution Unit", "Mode", "Bit Depth", "Place"].map(|n| text_of(&h, Role::ComboBox, n));
        assert_eq!(combos, ["pixels", if per_cm { "pixels/cm" } else { "pixels/inch" }, "Grayscale", "16-bit", "down first"].map(String::from));
        let number = |key: &str| run[key].as_f64().unwrap_or(f64::NAN);
        let resolution = number("resolution") / if per_cm { 2.54 } else { 1.0 };
        let want = [number("width"), number("height"), resolution, 2.0, 3.0, 9.0];
        let got = ["Width", "Height", "Resolution", "Columns", "Rows", "Font size"].map(|n| shown(&h, n));
        assert!(want.iter().zip(got).all(|(w, g)| (w - g).abs() < 0.01), "shown {got:?}, want {want:?}");
        assert_eq!(json!(text_of(&h, Role::TextInput, "Input")), run["input"], "the folder of the run");
        assert_eq!(text_of(&h, Role::TextInput, "Font"), "Serif");
        for (key, label) in CHECKBOXES {
            assert_eq!(json!(ticked(&h, label)), run[key], "{label}");
        }
    });
}

#[test]
fn cancel_and_escape_show_the_last_okd_values_next_time() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = opened();
        let run = after_a_run(&mut h, "sticky-cancel");
        let edit = |h: &mut H| {
            type_text(h, "Input", "/elsewhere");
            pick(h, "Units", "cm");
            pick(h, "Mode", "Lab Color");
            type_into(h, "Columns", "7");
            assert_ne!(values(h), run, "the dialog was edited");
        };
        edit(&mut h);
        click(&mut h, Role::Button, "Cancel");
        reopen(&mut h);
        assert_eq!(values(&h), run, "after Cancel");

        edit(&mut h);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        reopen(&mut h);
        assert_eq!(values(&h), run, "after Escape");
    });
}

#[test]
fn cancel_before_any_ok_keeps_the_defaults() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = opened();
        let defaults = values(&h);
        pick(&mut h, "Mode", "Lab Color");
        type_into(&mut h, "Columns", "7");
        click(&mut h, Role::Button, "Cancel");
        reopen(&mut h);
        assert_eq!(values(&h), defaults);
        assert_eq!(kept(&h), None, "nothing stored");
    });
}

/// A run the command refuses (a folder without images) keeps what the last OK'd run stored.
#[test]
fn a_failed_run_keeps_the_values_from_before() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = opened();
        let run = after_a_run(&mut h, "sticky-failed");
        let empty = std::env::temp_dir().join(format!("pc-contact-sheet-sticky-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        type_text(&mut h, "Input", &empty.to_string_lossy());
        type_into(&mut h, "Columns", "7");
        let sheets = h.state().session.documents().len();
        click(&mut h, Role::Button, "OK");
        std::fs::remove_dir_all(&empty).ok();
        assert_eq!(h.state().session.documents().len(), sheets, "no sheet was built");
        assert_eq!(kept(&h), Some(Value::Object(run)), "the stored values are those of the run before");
    });
}

/// The Files list (#13686) does not exist yet; a `files` field the dialog carries is not kept.
#[test]
fn the_files_list_is_not_kept() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = opened();
        let dir = image_folder("sticky-files");
        type_text(&mut h, "Input", &dir.to_string_lossy());
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("files".into(), json!([dir.join("a.png").to_string_lossy()]));
        ok(&mut h);
        std::fs::remove_dir_all(&dir).ok();
        let kept = kept(&h).unwrap_or_else(|| panic!("OK stored the values"));
        assert_eq!((kept.get("files"), &kept["input"]), (None, &json!(dir.to_string_lossy())), "{kept}");
        reopen(&mut h);
        assert!(!fields(&h).contains_key("files"), "the next open has no Files list");
    });
}

#[test]
fn the_values_of_the_last_ok_survive_a_restart() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let store = Store::default();
        let mut h = opened_on(&store);
        let run = after_a_run(&mut h, "sticky-restart");
        let ctx = h.ctx.clone();
        crate::prefs_ui::tick(h.state_mut(), &ctx);
        assert_eq!(stored(&store)["dialogs"][COMMAND], Value::Object(run.clone()), "written to the store");
        let h = opened_on(&store);
        assert_eq!(values(&h), run);
    });
}

#[test]
fn a_stored_value_the_dialog_cannot_show_gives_its_default() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let defaults = values(&opened());
        let open_with = |entry: Option<Value>| {
            let mut file = photocraft_engine::Session::new().prefs_value();
            if let Some(entry) = entry {
                file["dialogs"][COMMAND] = entry;
            }
            opened_on(&Arc::new(Mutex::new(Some(file.to_string()))))
        };
        let broken = [
            json!({"mode": "xyz"}),
            json!({"columns": -1}),
            json!({"columns": 101}),
            json!({"rows": 2.5}),
            json!({"depth": 12}),
            json!({"resolution": 0}),
            json!({"resolution": 10001}),
            json!({"input": 5}),
            json!({"caption": "yes"}),
            json!({"flatten": 1}),
            json!({RES_UNIT: "mm"}),
            // Units, Width and Height only together, and only when all three are good.
            json!({"units": "furlongs", "width": 3, "height": 4}),
            json!({"units": "cm", "width": 300001, "height": 4}),
            json!({"units": "cm", "width": "wide", "height": 4}),
            json!({"units": "cm", "width": 3}),
            json!("not an object"),
            json!([1, 2]),
        ];
        for entry in broken {
            assert_eq!(values(&open_with(Some(entry.clone()))), defaults, "{entry}");
        }
        assert_eq!(values(&open_with(None)), defaults, "no entry");

        // A broken value and keys the dialog does not have are skipped; the rest is used, whole
        // numbers stored as 16.0 or 3.0 included.
        let entry =
            json!({"mode": "xyz", "columns": 2, "rows": 3.0, "depth": 16.0, "units": "pixels", "width": 100, "height": 200, "files": ["a.png"], "future": 1});
        let mut want = defaults.clone();
        want.extend(
            [("columns", json!(2)), ("rows", json!(3)), ("depth", json!(16)), ("units", json!("pixels")), ("width", json!(100.0)), ("height", json!(200.0))]
                .map(|(k, v)| (k.to_string(), v)),
        );
        assert_eq!(values(&open_with(Some(entry))), want);
    });
}
