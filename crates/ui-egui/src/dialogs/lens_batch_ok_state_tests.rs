//! #13986: as in the reference application, OK in the batch Lens Correction dialog stays greyed while Source Files is
//! empty or the profile is None, and `ui.dialog.confirm` refuses the same cases. The correction checkboxes do not
//! count.

use craft::ui::dialog::DialogId;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::lens_batch_layout_tests::{H, opened, set, source_folder};

/// Whether the dialog's OK takes a click.
fn ok_enabled(h: &H) -> bool {
    !h.get_by_role_and_label(Role::Button, "OK").accesskit_node().is_disabled()
}

fn is_open(h: &H, id: DialogId) -> bool {
    h.state().framework.dialogs.open_dialogs().iter().any(|d| d.id == id)
}

#[test]
fn ok_is_greyed_without_a_source_file_or_with_no_profile() {
    let (mut h, id) = opened();
    assert!(!ok_enabled(&h), "a fresh dialog lists no source");
    let dir = source_folder();
    set(&mut h, id, "input", json!({"files": [dir.path().join("a.png")], "useFolder": false}));
    assert!(ok_enabled(&h), "a listed file and the Auto profile");
    set(&mut h, id, "profile", json!("none"));
    assert!(!ok_enabled(&h), "no profile");
    set(&mut h, id, "profile", json!("generic"));
    assert!(ok_enabled(&h), "the Generic profile");
    for key in ["correctDistortion", "correctCA", "correctVignette", "autoScale"] {
        set(&mut h, id, key, json!(false));
    }
    assert!(ok_enabled(&h), "the corrections do not count");
    set(&mut h, id, "input", json!({"files": [], "useFolder": false}));
    assert!(!ok_enabled(&h), "the list emptied again");
    // A folder in Folder mode is a source too.
    set(&mut h, id, "input", json!({"files": [], "folder": dir.path().to_string_lossy(), "useFolder": true}));
    assert!(ok_enabled(&h), "a folder");
    let empty = tempfile::tempdir().unwrap();
    std::fs::write(empty.path().join("notes.txt"), "not an image").unwrap();
    set(&mut h, id, "input", json!({"files": [], "folder": empty.path().to_string_lossy(), "useFolder": true}));
    assert!(!ok_enabled(&h), "a folder without images");
    let refused = crate::dialogs::testing::confirm(h.state_mut(), id);
    h.run_steps(3);
    assert!(refused.is_err() && is_open(&h, id), "confirm refuses it too: {refused:?}");
}

#[test]
fn confirm_refuses_what_the_greyed_ok_does_and_keeps_the_dialog() {
    let (mut h, id) = opened();
    for (input, profile) in [(json!({"files": [], "useFolder": false}), "auto"), (json!({"files": ["/tmp/a.png"], "useFolder": false}), "none")] {
        set(&mut h, id, "input", input.clone());
        set(&mut h, id, "profile", json!(profile));
        let answer = crate::dialogs::testing::confirm(h.state_mut(), id);
        h.run_steps(3);
        assert!(answer.is_err(), "{input} {profile}: {answer:?}");
        assert!(is_open(&h, id), "{input} {profile}: the dialog stays open");
    }
}
