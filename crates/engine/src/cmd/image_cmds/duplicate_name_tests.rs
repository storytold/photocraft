//! #14590: Image › Duplicate names the copy as the reference application does: the source's name
//! without its extension plus " copy", counting up (" copy 2", " copy 3") across every open
//! document and taking the lowest free number; a given name is used verbatim.

use serde_json::json;

use crate::Session;

/// A session with one small document named `name`.
fn source(name: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": name, "width": 20, "height": 10, "background": "white"})).unwrap();
    s
}

fn name(s: &Session, index: usize) -> &str {
    &s.documents()[index].doc.name
}

/// Duplicates the active document with `params` and returns the copy's name from the result.
fn duplicate(s: &mut Session, params: serde_json::Value) -> String {
    let r = s.execute("image.duplicate", params).unwrap();
    let index = r["document"].as_u64().unwrap() as usize;
    assert_eq!(r["name"], json!(name(s, index)), "the result names the copy");
    name(s, index).to_string()
}

#[test]
fn the_default_name_drops_the_file_extension() {
    let mut s = source("dupspec.psd");
    assert_eq!(duplicate(&mut s, json!({})), "dupspec copy");
    let mut s = source("dupspec2.jpg");
    assert_eq!(duplicate(&mut s, json!({})), "dupspec2 copy");
    let mut s = source("oracle");
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy", "an unsaved document has no extension to drop");
}

#[test]
fn dots_that_are_not_an_extension_stay() {
    let mut s = source("v1.2");
    assert_eq!(duplicate(&mut s, json!({})), "v1.2 copy");
}

#[test]
fn taken_names_count_up_across_open_documents() {
    let mut s = source("oracle");
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy 2");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy 3");
}

#[test]
fn documents_with_the_same_base_name_share_the_numbering() {
    let mut s = source("oracle");
    s.execute("file.new", json!({"name": "oracle", "width": 20, "height": 10})).unwrap();
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy 2");
}

#[test]
fn the_lowest_free_number_is_taken_again_after_a_close() {
    let mut s = source("oracle");
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy 2");
    // Close "oracle copy"; the next copy takes its name back.
    assert_eq!(name(&s, 1), "oracle copy");
    s.close(1);
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy");
}

#[test]
fn a_copy_counts_on_from_its_own_number() {
    let mut s = source("X copy");
    assert_eq!(duplicate(&mut s, json!({})), "X copy 2");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "X copy 3");
    let mut s = source("Bar copy 7");
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 8");
    let mut s = source("Bar copy 7.psd");
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 8");
}

#[test]
fn an_empty_name_gets_the_default() {
    let mut s = source("oracle");
    duplicate(&mut s, json!({}));
    s.set_active(0);
    duplicate(&mut s, json!({}));
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({"name": ""})), "oracle copy 3");
}

#[test]
fn a_given_name_is_kept_verbatim_even_when_taken() {
    let mut s = source("oracle");
    assert_eq!(duplicate(&mut s, json!({"name": "Silent"})), "Silent");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({"name": "Silent"})), "Silent", "a given name is not numbered");
    assert_eq!(duplicate(&mut s, json!({"name": "my.file.psd"})), "my.file.psd", "a given name keeps its extension");
}

#[test]
fn merged_only_copies_use_the_same_name() {
    let mut s = source("dupspec.psd");
    assert_eq!(duplicate(&mut s, json!({"mergedOnly": true})), "dupspec copy");
}

/// Holds back a copy of the first document named `name`, as a background open does while its
/// profile prompt waits.
fn hold(s: &mut Session, name: &str) {
    let mut doc = (*s.documents()[0].doc).clone();
    doc.name = name.to_string();
    s.hold_open(doc, false);
}

#[test]
fn every_document_held_for_a_prompt_is_skipped_however_many_there_are() {
    let mut s = source("oracle");
    hold(&mut s, "oracle copy");
    for n in 2..=1100 {
        hold(&mut s, &format!("oracle copy {n}"));
    }
    assert_eq!(duplicate(&mut s, json!({})), "oracle copy 1101");
}

#[test]
fn a_copy_number_with_none_after_it_is_never_reused() {
    // The largest number has no next one, so the whole name is copied as any other name is.
    let mut s = source("Bar copy 18446744073709551615");
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 18446744073709551615 copy");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 18446744073709551615 copy 2");
    // One below counts on to the largest number, then, with that one taken, does the same.
    let mut s = source("Bar copy 18446744073709551614");
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 18446744073709551615");
    s.set_active(0);
    assert_eq!(duplicate(&mut s, json!({})), "Bar copy 18446744073709551614 copy");
}
