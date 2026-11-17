//! The recorded Export Layers to Files script, restricted to represented JPEG settings.
use super::descriptors::{jstr, text};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Value, json};

pub(super) const EVENT: &str = "6f1c2cf5-4a97-4e32-8f59-f5d7a087adef";
pub(super) const COMMAND: &str = "file.export.layersToFiles";
const SOURCE: &str = "_atnExportLayersDescriptor";

#[derive(PartialEq)]
struct Settings {
    dir: String,
    prefix: String,
    visible: bool,
    quality: u8,
}

fn string(d: &Descriptor, key: &str) -> Option<String> {
    let D::Text(t) = d.get(key)? else {
        return None;
    };
    let end = t.0.iter().rposition(|c| *c != 0).map_or(0, |i| i + 1);
    let s = String::from_utf16(t.0.get(..end)?).ok()?;
    (!s.contains('\0')).then_some(s)
}

/// Other file types' settings are inactive for JPEG, but must still have their recorded types.
fn recorded(d: &Descriptor) -> Option<Settings> {
    const REQUIRED: &[&str] = &["Msge", "destination", "fileNamePrefix", "visibleOnly", "FlTy", "icc", "jpegQuality"];
    const TEXT: &[&str] = &["tiffCompression", "tiffJpegQuality", "pdfEncoding", "pdfJpegQuality", "targaDepth", "bmpDepth"];
    const BOOL: &[&str] = &["psdMaxComp", "png24Transparency", "png24Interlaced", "png24Trim", "png8Transparency", "png8Interlaced", "png8Trim"];
    if !d.class_id.is("cjsp") || REQUIRED.iter().any(|key| d.items.iter().filter(|(k, _)| k.is(key)).count() != 1) {
        return None;
    }
    for (i, (key, value)) in d.items.iter().enumerate() {
        if d.items.iter().take(i).any(|(earlier, _)| earlier == key) {
            return None;
        }
        if REQUIRED.iter().any(|k| key.is(k)) {
            continue;
        }
        if !(TEXT.iter().any(|k| key.is(k)) && matches!(value, D::Text(_)) || BOOL.iter().any(|k| key.is(k)) && matches!(value, D::Boolean(_))) {
            return None;
        }
    }
    // ICC suppression has no native option yet; other file types need their own settings map.
    if !matches!(d.get("FlTy"),Some(D::Double(v)) if *v == 1.0) || !matches!(d.get("icc"), Some(D::Boolean(true))) {
        return None;
    }
    let message = string(d, "Msge")?;
    if message.is_empty() {
        return None;
    }
    let dir = string(d, "destination")?;
    if dir.is_empty() {
        return None;
    }
    let prefix = string(d, "fileNamePrefix")?;
    let D::Boolean(visible) = d.get("visibleOnly")? else {
        return None;
    };
    let quality = string(d, "jpegQuality")?;
    if quality.is_empty() || !quality.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let quality = quality.parse::<u8>().ok().filter(|q| *q <= 12)?;
    Some(Settings { dir, prefix, visible: *visible, quality })
}

pub(super) fn from(d: &Descriptor) -> Mapped {
    let Some(s) = recorded(d) else {
        return Mapped::Unsupported { harmless: false };
    };
    let mut p = json!({"dir":s.dir,"prefix":s.prefix,"visibleOnly":s.visible,"quality":s.quality,
        "format":"jpg","groups":"recursive","trim":false,"jpegFormat":"baseline"});
    // The script's message is locale-sensitive, not a disposable label. Preserve it and the
    // inactive settings even after native parameters are edited; never invent a locale marker.
    p[SOURCE] = photocraft_paint::tile::b64_encode(&d.to_bytes()).into();
    step(COMMAND, p)
}

fn native(p: &Value) -> Option<Settings> {
    let m = p.as_object()?;
    if m.keys().any(|key| !["dir", "prefix", "visibleOnly", "quality", "format", "groups", "trim", "jpegFormat", SOURCE].contains(&key.as_str()))
        || jstr(p, "format")? != "jpg"
        || jstr(p, "groups")? != "recursive"
        || p.get("trim")?.as_bool()?
        || jstr(p, "jpegFormat")? != "baseline"
    {
        return None;
    }
    let dir = jstr(p, "dir").filter(|s| !s.contains('\0'))?.to_string();
    if dir.is_empty() {
        return None;
    }
    Some(Settings {
        dir,
        prefix: jstr(p, "prefix").filter(|s| !s.contains('\0'))?.to_string(),
        visible: p.get("visibleOnly")?.as_bool()?,
        quality: u8::try_from(p.get("quality")?.as_u64()?).ok().filter(|q| *q <= 12)?,
    })
}

pub(super) fn to(_: &str, p: &Value) -> Option<Descriptor> {
    let current = native(p)?;
    let mut d = Descriptor::from_bytes(&photocraft_paint::tile::b64_decode(jstr(p, SOURCE)?)?).ok()?;
    let original = recorded(&d)?;
    if original == current {
        return Some(d);
    }
    for (key, value) in &mut d.items {
        if key.is("destination") && current.dir != original.dir {
            *value = text(&current.dir);
        } else if key.is("fileNamePrefix") && current.prefix != original.prefix {
            *value = text(&current.prefix);
        } else if key.is("visibleOnly") {
            *value = D::Boolean(current.visible);
        } else if key.is("jpegQuality") && current.quality != original.quality {
            *value = text(&current.quality.to_string());
        }
    }
    Some(d)
}
