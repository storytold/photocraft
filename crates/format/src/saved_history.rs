//! The optional undo history section, `history.json` (#6053).
//!
//! ```text
//! history.json   {history_version, format_version, generator, base, current, states: [{label, active, selected, document}]}
//! ```
//!
//! Each state's `document` is a manifest document tree (`DocM`, at `format_version` and migrated
//! like `manifest.json`) whose tiles and blobs are the bundle's own content-addressed objects, so
//! pixels a step left alone are stored once. `base` is the BLAKE3 hash of the `manifest.json`
//! written with it: a bundle whose manifest was rewritten by a writer that doesn't know this
//! section (an older version re-saving a directory bundle) no longer matches, and its stale
//! history is ignored.
//!
//! The section is never required. `manifest.json` is the same with or without it, so older
//! versions open the document as before, and a history that is missing, newer than this build,
//! damaged, stale or over the load budget is dropped while the document still opens.

use std::collections::BTreeMap;
use std::sync::Arc;

use photocraft_doc::{Document, LayerId, SavedHistory, SavedState};
use serde::{Deserialize, Serialize};

use crate::convert::{self, Loader};
use crate::manifest::{DocM, FORMAT_VERSION, Hash};
use crate::{FormatError, Result};

/// The section's entry name.
pub const HISTORY: &str = "history.json";

/// Version of the section's own layout (not of the documents in it, which follow
/// [`FORMAT_VERSION`]). A newer one is ignored rather than refused.
pub const HISTORY_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct TargetM {
    #[serde(default)]
    active: Option<u64>,
    #[serde(default)]
    selected: Vec<u64>,
}

#[derive(Deserialize)]
struct StateIn {
    label: String,
    #[serde(flatten)]
    target: TargetM,
}

#[derive(Serialize)]
struct StateOut {
    label: String,
    #[serde(flatten)]
    target: TargetM,
    document: DocM,
}

#[derive(Serialize)]
struct HistoryOut<'a> {
    history_version: u32,
    format_version: u32,
    generator: String,
    base: &'a str,
    current_label: &'a str,
    current: TargetM,
    /// Oldest first.
    states: Vec<StateOut>,
}

fn target_m(active: Option<LayerId>, selected: &[LayerId]) -> TargetM {
    TargetM { active: active.map(|l| l.0), selected: selected.iter().map(|l| l.0).collect() }
}

/// What a history adds to a save: the section's bytes and the objects only it references.
pub(crate) struct Prepared {
    pub json: Vec<u8>,
    pub tiles: BTreeMap<Hash, (Arc<photocraft_raster::Tile>, photocraft_color::SampleType)>,
    pub blobs: BTreeMap<Hash, Arc<Vec<u8>>>,
}

/// The history section for a save whose manifest is `manifest`; `has_tile` and `has_blob` say
/// which objects the document itself already stores. States are taken newest first while the
/// pixels only they add fit in [`SavedHistory::max_bytes`] and the section stays small enough for
/// the default load limits to read it back. `None` when no state fits. The history must never
/// cost the document its save, so a state that can't be written ends the history there instead
/// of failing.
pub(crate) fn prepare(
    h: &SavedHistory,
    manifest: &[u8],
    hash_cache: Option<&mut std::collections::HashMap<usize, (std::sync::Weak<photocraft_raster::Tile>, Hash)>>,
    has_tile: &dyn Fn(&str) -> bool,
    has_blob: &dyn Fn(&str) -> bool,
) -> Option<Prepared> {
    if h.max_bytes == 0 {
        return None;
    }
    let mut cache = hash_cache;
    let mut tiles = BTreeMap::new();
    let mut blobs = BTreeMap::new();
    let mut spent = 0u64;
    // Headroom for labels, targets and the section's own fields.
    let max_json = (crate::LoadOptions::default().max_manifest_bytes as u64).saturating_sub(1 << 20);
    let mut json_spent = 0u64;
    let mut states = Vec::new();
    for s in h.states.iter().rev() {
        let mut c = crate::store::Collect { hash_cache: cache.as_deref_mut(), ..Default::default() };
        if convert::check_nesting(&s.doc.layers).is_err() {
            break;
        }
        let document = convert::doc_m(&s.doc, &mut c);
        if crate::finite::check(&document).is_err() {
            break;
        }
        // The section is read with the manifest's size limit: keep it within the default one.
        let Ok(json_len) = serde_json::to_vec(&document).map(|j| j.len() as u64) else { break };
        if json_spent.saturating_add(json_len) > max_json {
            break;
        }
        let mut added = 0u64;
        for (hash, (t, _)) in &c.tiles {
            if !has_tile(hash) && !tiles.contains_key(hash) {
                added = added.saturating_add(t.bytes().len() as u64);
            }
        }
        for (hash, b) in &c.blobs {
            if !has_blob(hash) && !blobs.contains_key(hash) {
                added = added.saturating_add(b.len() as u64);
            }
        }
        if spent.saturating_add(added) > h.max_bytes {
            break;
        }
        spent = spent.saturating_add(added);
        json_spent = json_spent.saturating_add(json_len);
        for (hash, t) in c.tiles {
            if !has_tile(&hash) {
                tiles.entry(hash).or_insert(t);
            }
        }
        for (hash, b) in c.blobs {
            if !has_blob(&hash) {
                blobs.entry(hash).or_insert(b);
            }
        }
        states.push(StateOut { label: s.label.clone(), target: target_m(s.active, &s.selected), document });
    }
    if states.is_empty() {
        return None;
    }
    states.reverse();
    let base = crate::store::hash_bytes(manifest);
    let out = HistoryOut {
        history_version: HISTORY_VERSION,
        format_version: FORMAT_VERSION,
        generator: format!("photocraft-format {}", env!("CARGO_PKG_VERSION")),
        base: &base,
        current_label: &h.current_label,
        current: target_m(h.current_active, &h.current_selected),
        states,
    };
    let json = serde_json::to_vec(&out).ok()?;
    crate::store::check_json_depth(&json, crate::store::MAX_MANIFEST_DEPTH + 2).ok()?;
    Some(Prepared { json, tiles, blobs })
}

fn ids(v: &[u64]) -> Vec<LayerId> {
    v.iter().map(|&i| LayerId(i)).collect()
}

/// Reads the history section for the document `doc` loaded from a manifest whose bytes hash to
/// `base`, with `loader` (the one that loaded `doc`, so states share its decoded tiles and its
/// byte budget). `Ok(None)` when there is no section or it doesn't belong to this manifest or this
/// build; `Err` when it is damaged. Either way the caller opens `doc` without it.
pub(crate) fn load(raw: Option<Vec<u8>>, base: &str, doc: &Document, loader: &mut Loader<'_>, opts: &crate::LoadOptions) -> Result<Option<SavedHistory>> {
    let Some(raw) = raw else { return Ok(None) };
    crate::store::check_json_depth(&raw, crate::store::MAX_MANIFEST_DEPTH + 2)?;
    let mut de = serde_json::Deserializer::from_slice(&raw);
    de.disable_recursion_limit();
    let mut v = <serde_json::Value as Deserialize>::deserialize(&mut de)?;
    de.end()?;
    let version = v.get("history_version").and_then(serde_json::Value::as_u64).ok_or_else(|| FormatError::corrupt("history has no history_version"))?;
    if version == 0 || version > u64::from(HISTORY_VERSION) || v.get("base").and_then(serde_json::Value::as_str) != Some(base) {
        return Ok(None);
    }
    let format_version = v.get("format_version").cloned().ok_or_else(|| FormatError::corrupt("history has no format_version"))?;
    let current_label = v.get("current_label").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    let current: TargetM = serde_json::from_value(v.get_mut("current").map(serde_json::Value::take).unwrap_or_default())?;
    let Some(serde_json::Value::Array(raw_states)) = v.get_mut("states").map(serde_json::Value::take) else {
        return Err(FormatError::corrupt("history has no states"));
    };
    let mut states = Vec::with_capacity(raw_states.len());
    for mut s in raw_states {
        let document = s.get_mut("document").map(serde_json::Value::take).ok_or_else(|| FormatError::corrupt("history state has no document"))?;
        let StateIn { label, target } = serde_json::from_value(s)?;
        // Migrated like a manifest of the history's format version.
        let mut wrapped = serde_json::json!({"format_version": format_version.clone(), "document": document});
        crate::migrate::migrate(&mut wrapped)?;
        let m = crate::store::doc_m_from_json(wrapped.get_mut("document").map(serde_json::Value::take).unwrap_or_default())?;
        crate::store::admit_canvas(m.size, opts)?;
        let state = loader.document(&m)?;
        // Undo swaps documents within one tab: the states must be the document's own, with the
        // file's ids (a remapped load can't relate them).
        if !loader.preserve_ids || state.id != doc.id {
            return Ok(None);
        }
        states.push(SavedState { label, doc: Arc::new(state), active: target.active.map(LayerId), selected: ids(&target.selected) });
    }
    if states.is_empty() {
        return Ok(None);
    }
    Ok(Some(SavedHistory { states, current_label, current_active: current.active.map(LayerId), current_selected: ids(&current.selected), max_bytes: 0 }))
}
