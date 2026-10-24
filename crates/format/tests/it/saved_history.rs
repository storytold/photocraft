//! Undo history saved in the bundle (`history.json`, #6053): round trips, the size budget, and
//! every way a history can be unusable without the document failing to open.

use std::sync::Arc;

use crate::common;
use common::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerId, SavedHistory, SavedState, Size};
use photocraft_format::*;

const HISTORY: &str = "history.json";

/// A document after three steps, and its history: Open → Brush Tool → New Layer.
fn edited() -> (Document, SavedHistory) {
    let mut open = Document::with_background("h.pcraft", Size::new(600, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    scribble(open.layers[0].surface_mut().unwrap(), 3, false);
    let bg = open.layers[0].id;
    let mut brushed = open.clone();
    scribble(brushed.layers[0].surface_mut().unwrap(), 7, false);
    let mut layered = brushed.clone();
    let mut top = Layer::raster("Layer 1", layered.pixel_format());
    scribble(top.surface_mut().unwrap(), 9, false);
    let top_id = top.id;
    layered.layers.push(top);
    let state = |label: &str, doc: &Document, id: LayerId| SavedState { label: label.into(), doc: Arc::new(doc.clone()), active: Some(id), selected: vec![id] };
    let history = SavedHistory {
        states: vec![state("Open", &open, bg), state("Brush Tool", &brushed, bg)],
        current_label: "New Layer".into(),
        current_active: Some(top_id),
        current_selected: vec![top_id],
        max_bytes: u64::MAX,
    };
    (layered, history)
}

fn with_history(doc: &Document, history: SavedHistory) -> Document {
    let mut d = doc.clone();
    d.saved_history = Some(Arc::new(history));
    d
}

/// `doc` without the history it carries, for comparing documents.
fn bare(doc: &Document) -> Document {
    let mut d = doc.clone();
    d.saved_history = None;
    d
}

fn assert_same_history(back: &SavedHistory, saved: &SavedHistory) {
    assert_eq!(back.current_label, saved.current_label);
    assert_eq!(back.current_active, saved.current_active);
    assert_eq!(back.current_selected, saved.current_selected);
    assert_eq!(back.states.len(), saved.states.len());
    for (b, s) in back.states.iter().zip(&saved.states) {
        assert_eq!(b.label, s.label);
        assert_eq!(b.active, s.active);
        assert_eq!(b.selected, s.selected);
        assert_eq!(*b.doc, *s.doc, "state `{}`", s.label);
        assert!(b.doc.saved_history.is_none());
    }
}

#[test]
fn zip_round_trip_keeps_the_history() {
    let (doc, history) = edited();
    let bytes = save_to_bytes(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap();
    let back = load_from_bytes(&bytes).unwrap();
    assert_eq!(bare(&back), doc);
    assert_same_history(back.saved_history.as_deref().expect("history restored"), &history);
}

#[test]
fn directory_round_trip_keeps_the_history() {
    let (doc, history) = edited();
    let dir = temp_dir("history-dir").join("h.pcraft");
    PcraftWriter::new().save_dir(&with_history(&doc, history.clone()), &dir, &SaveOptions::default()).unwrap();
    let back = load_path(&dir).unwrap();
    assert_eq!(bare(&back), doc);
    assert_same_history(back.saved_history.as_deref().expect("history restored"), &history);
}

#[test]
fn reopened_states_share_unchanged_tiles_with_the_document() {
    let (doc, history) = edited();
    let back = load_from_bytes(&save_to_bytes(&with_history(&doc, history), &SaveOptions::default()).unwrap()).unwrap();
    let h = back.saved_history.clone().unwrap();
    // The background was not touched by New Layer: its tiles are the same copy-on-write tiles.
    let current = back.layers[0].surface().unwrap();
    let before = h.states[1].doc.layers[0].surface().unwrap();
    let shared = current.tiles().filter(|(at, t)| before.tiles().any(|(b, u)| b == *at && Arc::ptr_eq(t, u))).count();
    assert!(shared > 0 && shared == current.tiles().count(), "unchanged tiles load once and are shared");
}

#[test]
fn the_manifest_is_the_same_with_or_without_history() {
    // Older versions read only manifest.json and the objects it names: they open the same document.
    let (doc, history) = edited();
    let with = save_to_bytes(&with_history(&doc, history), &SaveOptions::default()).unwrap();
    let without = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let manifest = |b: &[u8]| zip::ZipReader::new(b).unwrap().read_by_name("manifest.json", 1 << 30).unwrap();
    assert_eq!(manifest(&with), manifest(&without));
    assert!(zip::ZipReader::new(&with).unwrap().find(HISTORY).is_some());
}

#[test]
fn no_history_writes_no_history_section() {
    let (doc, _) = edited();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert!(zip::ZipReader::new(&bytes).unwrap().find(HISTORY).is_none());
    assert!(load_from_bytes(&bytes).unwrap().saved_history.is_none());
}

#[test]
fn the_budget_keeps_the_newest_states_that_fit() {
    let (doc, mut history) = edited();
    // Brush Tool repainted Open's background tiles: those are the only pixels no later state
    // holds. Brush Tool's background is the document's own, so a budget below one tile keeps
    // Brush Tool and drops Open.
    history.max_bytes = 1000;
    let back = load_from_bytes(&save_to_bytes(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap()).unwrap();
    let h = back.saved_history.expect("the newest state fits");
    assert_eq!(h.states.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Brush Tool"]);
    assert_eq!(*h.states[0].doc, *history.states[1].doc);
    assert_eq!(h.current_label, "New Layer");

    // A budget of 0 writes no states, so no history at all.
    history.max_bytes = 0;
    let bytes = save_to_bytes(&with_history(&doc, history), &SaveOptions::default()).unwrap();
    assert!(zip::ZipReader::new(&bytes).unwrap().find(HISTORY).is_none());
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

/// Saves `doc` with `history` as a directory bundle, lets `damage` change it, and reopens it.
fn reopen_damaged(damage: impl FnOnce(&std::path::Path)) -> Document {
    let (doc, history) = edited();
    let dir = temp_dir("history-damage").join("h.pcraft");
    PcraftWriter::new().save_dir(&with_history(&doc, history), &dir, &SaveOptions::default()).unwrap();
    damage(&dir);
    let back = load_path(&dir).expect("the document opens whatever happened to its history");
    assert_eq!(bare(&back), doc);
    back
}

fn edit_history_json(dir: &std::path::Path, f: impl FnOnce(&mut serde_json::Value)) {
    let path = dir.join(HISTORY);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn garbage_history_opens_without_history() {
    let back = reopen_damaged(|dir| std::fs::write(dir.join(HISTORY), b"{ not json").unwrap());
    assert!(back.saved_history.is_none());
}

#[test]
fn missing_history_tiles_open_without_history() {
    let back = reopen_damaged(|dir| {
        // Open's background tiles are held by the history alone: delete every tile the manifest
        // does not name.
        let manifest = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        for e in std::fs::read_dir(dir.join("tiles")).unwrap() {
            let e = e.unwrap();
            let hash = e.file_name().to_string_lossy().trim_end_matches(".zst").to_string();
            if !manifest.contains(&hash) {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
    });
    assert!(back.saved_history.is_none());
}

#[test]
fn history_of_another_manifest_is_ignored() {
    // An older version re-saving the bundle rewrites manifest.json and leaves history.json behind.
    let back = reopen_damaged(|dir| {
        let path = dir.join("manifest.json");
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        v["generator"] = "photocraft-format 0.0.1".into();
        std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    });
    assert!(back.saved_history.is_none());
}

#[test]
fn a_newer_history_version_is_ignored() {
    let back = reopen_damaged(|dir| edit_history_json(dir, |v| v["history_version"] = 99.into()));
    assert!(back.saved_history.is_none());
}

#[test]
fn a_state_that_does_not_decode_drops_the_history() {
    let back = reopen_damaged(|dir| edit_history_json(dir, |v| v["states"][0]["document"]["size"] = "huge".into()));
    assert!(back.saved_history.is_none());
}

/// A past state with a number beyond f32's range would load as infinity that no later save can
/// write back (#6862), so it drops the history like any state that doesn't decode.
#[test]
fn a_state_with_an_out_of_range_number_drops_the_history() {
    let back = reopen_damaged(|dir| edit_history_json(dir, |v| v["states"][0]["document"]["layers"][0]["opacity"] = serde_json::json!(1e39)));
    assert!(back.saved_history.is_none());
}

/// The smallest `max_total_bytes` the bundle `bytes` loads in with `states` states of history.
fn smallest_load_budget(bytes: &[u8], states: usize) -> u64 {
    let loads = |budget: u64| {
        load_from_bytes_with(bytes, &LoadOptions { max_total_bytes: budget, ..Default::default() })
            .is_ok_and(|d| d.saved_history.as_ref().map_or(0, |h| h.states.len()) == states)
    };
    let mut budget = 1u64;
    while !loads(budget) {
        budget *= 2;
        assert!(budget < 1 << 40);
    }
    let mut lo = budget / 2;
    while lo + 1 < budget {
        let mid = (lo + budget) / 2;
        if loads(mid) {
            budget = mid;
        } else {
            lo = mid;
        }
    }
    budget
}

#[test]
fn history_beyond_the_load_budget_opens_without_history() {
    let (doc, history) = edited();
    let with = save_to_bytes(&with_history(&doc, history), &SaveOptions::default()).unwrap();
    // The smallest budget the document alone loads in: the history can't fit beside it.
    let budget = smallest_load_budget(&save_to_bytes(&doc, &SaveOptions::default()).unwrap(), 0);
    let back = load_from_bytes_with(&with, &LoadOptions { max_total_bytes: budget, ..Default::default() }).unwrap();
    assert_eq!(bare(&back), doc);
    assert!(back.saved_history.is_none());
}

#[test]
fn a_history_saved_at_the_load_budget_reopens_with_it() {
    // The history preference allows more than the loader reads back (64 GiB against 16 GiB, and
    // the document's own pixels count too): the save keeps only the states a loader with that
    // budget can reopen, so the newest ones survive instead of the whole history being dropped.
    let (doc, history) = edited();
    let doc_bytes = smallest_load_budget(&save_to_bytes(&doc, &SaveOptions::default()).unwrap(), 0);
    let with_all = save_to_bytes(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap();
    let all_bytes = smallest_load_budget(&with_all, history.states.len());
    // Room beside the document for Brush Tool's pixels (none: they are the document's own) but
    // not for Open's repainted background.
    let budget = all_bytes - 1;
    assert!(budget > doc_bytes);
    let save_at = |budget: u64| {
        let mut w = PcraftWriter::new();
        w.set_history_load_budget(budget);
        w.save_zip(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap().0
    };
    let bytes = save_at(budget);
    let back = load_from_bytes_with(&bytes, &LoadOptions { max_total_bytes: budget, ..Default::default() }).unwrap();
    assert_eq!(bare(&back), doc);
    let h = back.saved_history.expect("the states that fit the load budget reopen");
    assert_eq!(h.states.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Brush Tool"]);

    // A budget the history fits in keeps all of it; one the document alone fills keeps none.
    let back = load_from_bytes_with(&save_at(all_bytes), &LoadOptions { max_total_bytes: all_bytes, ..Default::default() }).unwrap();
    assert_same_history(back.saved_history.as_deref().expect("the whole history fits"), &history);
    let bytes = save_at(doc_bytes);
    assert!(zip::ZipReader::new(&bytes).unwrap().find(HISTORY).is_none());

    // The default writer leaves room within the default loader's budget.
    assert_eq!(PcraftWriter::new().history_load_budget(), LoadOptions::default().max_total_bytes);
}

#[test]
fn remapped_ids_open_without_history() {
    // Targets and states are matched to the document by id: without the file's ids they can't be.
    let (doc, history) = edited();
    let bytes = save_to_bytes(&with_history(&doc, history), &SaveOptions::default()).unwrap();
    let back = load_from_bytes_with(&bytes, &LoadOptions { preserve_ids: false, ..Default::default() }).unwrap();
    assert!(back.saved_history.is_none());
}

#[test]
fn a_directory_save_without_history_removes_the_old_one() {
    let (doc, history) = edited();
    let dir = temp_dir("history-stale").join("h.pcraft");
    let mut w = PcraftWriter::new();
    w.save_dir(&with_history(&doc, history), &dir, &SaveOptions::default()).unwrap();
    assert!(dir.join(HISTORY).exists());
    w.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
    assert!(!dir.join(HISTORY).exists());
    let back = load_path(&dir).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn incremental_zip_saves_keep_and_drop_the_history() {
    let (doc, history) = edited();
    let mut w = PcraftWriter::new();
    let (first, _) = w.save_zip(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap();
    let (second, _) = w.save_zip(&doc, &SaveOptions::default()).unwrap();
    let (third, _) = w.save_zip(&with_history(&doc, history.clone()), &SaveOptions::default()).unwrap();
    assert!(load_from_bytes(&first).unwrap().saved_history.is_some());
    assert!(load_from_bytes(&second).unwrap().saved_history.is_none());
    assert_same_history(load_from_bytes(&third).unwrap().saved_history.as_deref().unwrap(), &history);
}
