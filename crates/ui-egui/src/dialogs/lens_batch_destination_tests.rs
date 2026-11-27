//! #13982: the batch Lens Correction destination starts empty, and the first file added to an empty Source Files list
//! sets it to `results` in that file's folder, as the reference dialog does. A destination already set is kept.

use serde_json::{Value, json};

use super::lens_batch_layout_tests::{field, opened, set, source_folder};

fn results(dir: &std::path::Path) -> Value {
    json!(dir.join("results").to_string_lossy())
}

#[test]
fn the_destination_is_empty_until_the_first_file_is_listed() {
    let (mut h, id) = opened();
    assert_eq!(field(&h, id, "output"), json!(""));
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    set(&mut h, id, "input", json!({"files": [first.path().join("a.png"), second.path().join("b.png")], "useFolder": false}));
    assert_eq!(field(&h, id, "output"), results(first.path()), "results next to the first file");
    // More files, or another first file, keep it.
    set(&mut h, id, "input", json!({"files": [second.path().join("b.png")], "useFolder": false}));
    assert_eq!(field(&h, id, "output"), results(first.path()));
}

#[test]
fn emptying_the_list_and_listing_again_keeps_a_destination_that_was_set() {
    let (mut h, id) = opened();
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    set(&mut h, id, "input", json!({"files": [first.path().join("a.png")], "useFolder": false}));
    set(&mut h, id, "input", json!({"files": [], "useFolder": false}));
    set(&mut h, id, "input", json!({"files": [second.path().join("b.png")], "useFolder": false}));
    assert_eq!(field(&h, id, "output"), results(first.path()), "the destination is set, so the new first file keeps it");
    // Cleared by hand, the next first file of an empty list sets it again.
    set(&mut h, id, "output", json!(""));
    set(&mut h, id, "input", json!({"files": [], "useFolder": false}));
    set(&mut h, id, "input", json!({"files": [second.path().join("b.png")], "useFolder": false}));
    assert_eq!(field(&h, id, "output"), results(second.path()));
}

/// A destination chosen or set through `ui.dialog.set`, as scripts do, stays when the list gets its first files.
#[test]
fn a_destination_set_first_is_kept() {
    let (mut h, id) = opened();
    let (sources, chosen) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    set(&mut h, id, "output", json!(chosen.path().to_string_lossy()));
    set(&mut h, id, "input", json!({"files": [sources.path().join("a.png")], "useFolder": false}));
    assert_eq!(field(&h, id, "output"), json!(chosen.path().to_string_lossy()));
}

/// OK writes into the `results` folder the first file gave, creating it.
#[test]
fn ok_creates_the_results_folder() {
    let (mut h, id) = opened();
    let dir = source_folder();
    set(&mut h, id, "input", json!({"files": [dir.path().join("a.png")], "useFolder": false}));
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(dir.path().join("results").join("a.psd").exists(), "OK wrote into <folder>/results");
}

/// A script that sets the list and confirms in the same frame, with no pass between: OK still sends the `results`
/// folder, so the batch doesn't refuse a missing `output`.
#[test]
fn ok_right_after_a_script_set_the_list_writes_into_results() {
    let (mut h, id) = opened();
    let dir = source_folder();
    let ctx = h.ctx.clone();
    let input = json!({"files": [dir.path().join("a.png")], "useFolder": false});
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "input", input).unwrap();
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(dir.path().join("results").join("a.psd").exists(), "OK wrote into <folder>/results");
}
