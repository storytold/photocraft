//! Explicit image naming for Save for Web. The ordinary exporter keeps its existing defaults.
use super::{Document, Result, SliceKind, Value, bad, slices};
use std::collections::{HashMap, HashSet};

const CMD: &str = "file.export.saveForWebLegacy";

fn invalid(message: &str) -> super::EngineError {
    bad(CMD, format!("fileNaming: {message}"))
}

fn text(value: &Value) -> Result<&str> {
    value.as_str().filter(|s| s.len() <= 4096 && !s.contains('\0')).ok_or_else(|| invalid("expected text of at most 4096 bytes without NUL"))
}

/// Build every output name before the exporter commits a floating selection or writes a file.
/// Slice names retain resolved slice numbers; file serials count image slices only.
pub(super) fn plan(p: &Value, doc: &Document, ext: &str) -> Result<Option<Vec<String>>> {
    let Some(value) = p.get("fileNaming").filter(|v| !v.is_null()) else { return Ok(None) };
    if p.get("dir").and_then(Value::as_str).is_none_or(str::is_empty)
        || ["path", "folder", "numbers"].iter().any(|key| p.get(key).is_some_and(|v| !v.is_null()))
        || p.get("html").is_some_and(|v| !v.is_null() && v != false)
        || p.get("slices").is_some_and(|v| !v.is_null() && v != "all")
    {
        return Err(invalid("requires Images Only, all slices and a dir destination"));
    }
    let value = value.as_object().ok_or_else(|| invalid("expected an object"))?;
    if value.keys().any(|k| !["name", "components", "extension", "unix"].contains(&k.as_str())) {
        return Err(invalid("unknown setting"));
    }
    let components = value
        .get("components")
        .and_then(Value::as_array)
        .filter(|c| (1..=9).contains(&c.len()))
        .ok_or_else(|| invalid("components must contain 1 to 9 entries"))?;
    let extension = match value.get("extension") {
        None => "lower",
        Some(v) => text(v)?,
    };
    let extension = match extension {
        "lower" => ext.to_ascii_lowercase(),
        "upper" => ext.to_ascii_uppercase(),
        "none" => String::new(),
        _ => return Err(invalid("unknown extension choice")),
    };
    let unix = match value.get("unix") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| invalid("unix must be boolean"))?,
    };
    let base = match value.get("name") {
        Some(v) => text(v)?,
        None => doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(base, _)| base),
    };
    if base.len() > 4096 || base.contains('\0') {
        return Err(invalid("document name is too long or contains NUL"));
    }
    for part in components {
        match part {
            Value::String(s)
                if matches!(s.as_str(), "document" | "slice" | "serial" | "serial2" | "letter" | "LETTER" | "underscore" | "hyphen" | "space" | "none") => {}
            Value::Object(o) if o.len() == 1 && o.contains_key("text") => {
                text(o.get("text").ok_or_else(|| invalid("missing text"))?)?;
            }
            _ => return Err(invalid("unknown component")),
        }
    }
    // The resolver copies stored names into visible pieces. Reject oversized names before
    // that copy, not only after concatenating a filename from them.
    if doc.slices.list.len() > 100_000 || doc.slices.list.iter().any(|slice| slice.name.len() > 4096) {
        return Err(invalid("too many slices or an oversized slice name"));
    }
    let all = slices::resolve(doc);
    // Repeated components can generate a distinct long string per slice. Bound that extra
    // allocation separately from the document's existing slice storage.
    if all.len() > 100_000 {
        return Err(invalid("too many output names"));
    }
    let mut used = HashSet::new();
    let mut next_serial = HashMap::new();
    let mut names = Vec::with_capacity(all.len());
    let mut file_serial = 0u64;
    for slice in &all {
        if slice.kind == SliceKind::NoImage {
            names.push(String::new());
            continue;
        }
        file_serial += 1; // `all` is capped above; no-image cells never consume a file serial.
        let mut stem = String::new();
        for part in components {
            if let Some(literal) = part.get("text") {
                stem.push_str(text(literal)?);
                continue;
            }
            match part.as_str().unwrap_or_default() {
                "document" => stem.push_str(base),
                "slice" => {
                    if slice.id.and_then(|id| doc.slices.get(id)).is_some_and(|s| !s.name.is_empty()) {
                        stem.push_str(&slice.name);
                    } else if all.len() == 1 {
                        stem.push_str(base);
                    } else {
                        stem.push_str(&format!("{base}_{:02}", slice.number));
                    }
                }
                "serial" => stem.push_str(&file_serial.to_string()),
                "serial2" => stem.push_str(&format!("{file_serial:02}")),
                "letter" | "LETTER" => stem.push_str(&crate::file_cmds::serial_letters(file_serial, part == "LETTER")),
                "underscore" => stem.push('_'),
                "hyphen" => stem.push('-'),
                "space" => stem.push(' '),
                _ => {}
            }
        }
        if unix {
            stem = stem.replace(' ', "-");
        }
        if stem.is_empty() {
            stem.push('_');
        }
        let suffix = if extension.is_empty() { String::new() } else { format!(".{extension}") };
        let mut name = format!("{stem}{suffix}");
        // Remember the next suffix for each base: identical components across many slices
        // must not rescan all earlier suffixes on every image.
        let key = name.to_lowercase();
        let mut serial = next_serial.get(&key).copied().unwrap_or(2usize);
        while !used.insert(name.to_lowercase()) {
            name = format!("{stem}-{serial:02}{suffix}");
            serial = serial.checked_add(1).ok_or_else(|| invalid("too many duplicate names"))?;
        }
        next_serial.insert(key, serial);
        // Do not interpret text components as paths, Windows streams/devices or oversized names.
        if name.encode_utf16().count() > 255
            || name.len() > 255
            || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            || super::assets::asset_path_components(&name).is_err()
        {
            return Err(invalid("generated name is not a portable filename"));
        }
        names.push(name);
    }
    Ok(Some(names))
}
