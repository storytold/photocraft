//! File › Package (`packageFile`), File › Import › Notes (`importAnnots`) and Image › Apply Data
//! Set (`apply` of a `dataSetClass` by name), as `docs/atn.md` measures them (#10723).

use super::descriptors::{desc, get_ref, jstr, name_ref, path_text, path_value, trimmed};
use super::fields::{Mapped, step};
use super::{Descriptor, ReferenceItem, Value, json};

/// Whether `p` sets nothing but `allowed` (a `null` counts as unset).
fn sets_only(p: &Value, allowed: &[&str]) -> bool {
    p.as_object().is_none_or(|m| m.iter().all(|(k, v)| v.is_null() || allowed.contains(&k.as_str())))
}

/// The one alias key `key` of a step that carries nothing else.
fn only_path(d: &Descriptor, key: &str) -> Option<String> {
    if d.items.len() != 1 {
        return None;
    }
    path_text(d.get(key)?)
}

/// File › Package: `T   ` is the folder the package is written into.
pub(super) fn package_from(d: &Descriptor) -> Mapped {
    only_path(d, "T   ").map_or(Mapped::Unsupported { harmless: false }, |dir| step("file.package", json!({"dir": dir})))
}

/// Only the folder: the event has no key for a package format other than the default.
pub(super) fn package_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &["dir"]).then_some(())?;
    Some(desc("null").with("T   ", path_value(jstr(p, "dir").filter(|s| !s.is_empty())?)))
}

/// File › Import › Notes: `Usng` is the file the notes come from. Only the files
/// `file.import.notes` reads map (.psd, .psb, .pcraft); notes from a PDF or FDF keep the
/// Photoshop step, so loading reports them instead of playback failing.
pub(super) fn notes_from(d: &Descriptor) -> Mapped {
    only_path(d, "Usng")
        .filter(|path| notes_file(path))
        .map_or(Mapped::Unsupported { harmless: false }, |path| step("file.import.notes", json!({"path": path})))
}

fn notes_file(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| ["psd", "psb", "pcraft"].iter().any(|ok| e.eq_ignore_ascii_case(ok)))
}

pub(super) fn notes_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &["path"]).then_some(())?;
    Some(desc("null").with("Usng", path_value(jstr(p, "path").filter(|s| notes_file(s))?)))
}

/// Image › Apply Data Set: the data set is named in `null` (`dataSetClass` by name). Any other
/// `apply`, and one with an empty name or a name that isn't valid UTF-16 (which
/// `apply_data_set_to` can't write back), keeps the Photoshop step.
pub(super) fn apply_data_set_from(d: &Descriptor) -> Mapped {
    if d.items.len() != 1 {
        return Mapped::Unsupported { harmless: false };
    }
    match get_ref(d, "null") {
        Some([ReferenceItem::Name { class, name }])
            if trimmed(&class.class_id) == "dataSetClass" && String::from_utf16(&name.0).is_ok() && !name.to_string_lossy().is_empty() =>
        {
            step("image.applyDataSet", json!({"name": name.to_string_lossy()}))
        }
        _ => Mapped::Unsupported { harmless: false },
    }
}

/// By name only: a data set chosen by index has no form in the event.
pub(super) fn apply_data_set_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &["name"]).then_some(())?;
    Some(desc("null").with("null", name_ref("dataSetClass", jstr(p, "name").filter(|s| !s.is_empty())?)))
}
