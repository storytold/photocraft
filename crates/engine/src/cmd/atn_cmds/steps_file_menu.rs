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

/// File › Import › Notes: `Usng` is the file the notes come from.
pub(super) fn notes_from(d: &Descriptor) -> Mapped {
    only_path(d, "Usng").map_or(Mapped::Unsupported { harmless: false }, |path| step("file.import.notes", json!({"path": path})))
}

pub(super) fn notes_to(_: &str, p: &Value) -> Option<Descriptor> {
    sets_only(p, &["path"]).then_some(())?;
    Some(desc("null").with("Usng", path_value(jstr(p, "path").filter(|s| !s.is_empty())?)))
}

/// Image › Apply Data Set: the data set is named in `null` (`dataSetClass` by name). Any other
/// `apply`, and one with an empty name (which `apply_data_set_to` can't write back), keeps the
/// Photoshop step.
pub(super) fn apply_data_set_from(d: &Descriptor) -> Mapped {
    if d.items.len() != 1 {
        return Mapped::Unsupported { harmless: false };
    }
    match get_ref(d, "null") {
        Some([ReferenceItem::Name { class, name }]) if trimmed(&class.class_id) == "dataSetClass" && !name.to_string_lossy().is_empty() => {
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
