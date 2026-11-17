//! Save for Web's static image filename components, separately from inactive HTML naming.
use super::descriptors::{enm, get_bool, get_enum, text};
use super::steps_doc::{web_keys, web_slice_images_folder};
use super::{D, Descriptor, Value, json};

pub(super) const KEYS: &[&str] = &["ovFN", "ovNC", "ovCM", "ovCW", "ovCU"];
const COMPONENTS: &[(&str, &str)] = &[
    ("NC00", "document"),
    ("NC01", "slice"),
    ("NC08", "serial"),
    ("NC09", "serial2"),
    ("NC10", "letter"),
    ("NC11", "LETTER"),
    ("NC19", "underscore"),
    ("NC20", "hyphen"),
    ("NC21", "space"),
    ("NC24", "none"),
];

fn exact_text(value: &D) -> Option<String> {
    let D::Text(value) = value else { return None };
    if value.0.len() > 4097 {
        return None;
    }
    let s = String::from_utf16(value.0.strip_suffix(&[0]).unwrap_or(&value.0)).ok()?;
    (s.len() <= 4096 && !s.contains('\0')).then_some(s)
}

fn format_extension(format: &str) -> Option<&'static str> {
    Some(match format {
        "PN24" | "PN8" | "png24" | "png8" => "png",
        "JPEG" | "jpeg" => "jpg",
        "GIFf" | "gif" => "gif",
        "WBMP" | "wbmp" => "wbmp",
        _ => return None,
    })
}

/// Only a plain filename with the represented format's extension is understood here. In
/// particular, do not reinterpret an absolute/relative destination hidden in this field.
fn file_name_supported(name: &str, ext: &str) -> bool {
    let Some((base, extension)) = name.rsplit_once('.') else { return false };
    !base.is_empty()
        && extension.eq_ignore_ascii_case(ext)
        && name.len() <= 255
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

pub(super) fn from(u: &Descriptor) -> Option<Value> {
    if web_slice_images_folder(u).is_none() || get_bool(u, "ovCW") != Some(true) {
        return None;
    }
    // Both recorded Macintosh choices are inert for the represented Windows-compatible,
    // static image output. The original flag stays in the descriptor carrier.
    get_bool(u, "ovCM")?;
    let unix = get_bool(u, "ovCU")?;
    let ext = format_extension(&get_enum(u, "Fmt ")?)?;
    let D::List(parts) = u.get("ovNC")? else { return None };
    if parts.len() != 9 {
        return None;
    }
    let mut components = Vec::with_capacity(8);
    let mut extension = None;
    for part in parts {
        let D::Descriptor(part) = part else { return None };
        if !part.class_id.is("SCnc") || !web_keys(part, &["ncTp", "ncSt"]) {
            return None;
        }
        let Some(D::Enumerated { type_id, value }) = part.get("ncTp") else { return None };
        if !type_id.is("STnc") {
            return None;
        }
        if value.is("NC25") {
            let literal = exact_text(part.get("ncSt")?)?;
            // A slash asks the reference exporter for a different path (and can silently
            // produce no file). Do not turn it into an unrelated portable filename.
            if literal.chars().any(|c| c.is_control() || c == '/') {
                return None;
            }
            components.push(json!({"text":literal}));
        } else {
            if part.get("ncSt").is_some() {
                return None;
            }
            if value.is("NC22") {
                extension = Some("lower");
            } else if value.is("NC23") {
                extension = Some("upper");
            } else if value.is("NC02") || value.is("NC06") {
                components.push(json!("none"));
            } else {
                components.push(json!(COMPONENTS.iter().find(|(code, _)| value.is(code))?.1));
            }
        }
    }
    // A missing extension component has additional collision rules not represented here.
    let extension = extension?;
    while components.last().is_some_and(|part| part == "none") {
        components.pop();
    }
    if components.is_empty() {
        components.push(json!("none"));
    }
    let mut naming = json!({"components":components,"extension":extension,"unix":unix,"windows":true,"wholeImage":true});
    if let Some(value) = u.get("ovFN") {
        let filename = exact_text(value)?;
        if !file_name_supported(&filename, ext) {
            return None;
        }
        naming["fileName"] = json!(filename);
    }
    Some(naming)
}

pub(super) fn to(naming: &Value, format: &str) -> Option<Descriptor> {
    let object = naming.as_object()?;
    if object.keys().any(|key| !["components", "extension", "unix", "windows", "wholeImage", "fileName"].contains(&key.as_str()))
        || object.get("windows")?.as_bool() != Some(true)
        || object.get("wholeImage")?.as_bool() != Some(true)
    {
        return None;
    }
    let unix = match object.get("unix") {
        None => false,
        Some(Value::Bool(v)) => *v,
        _ => return None,
    };
    let extension = match object.get("extension") {
        None => "lower",
        Some(Value::String(v)) => v.as_str(),
        _ => return None,
    };
    let code = match extension {
        "lower" => "NC22",
        "upper" => "NC23",
        _ => return None,
    };
    let components = object.get("components")?.as_array()?;
    if components.is_empty() || components.len() > 8 {
        return None;
    }
    let mut parts = Vec::with_capacity(9);
    for component in components {
        let part = if let Some(value) = component.as_str() {
            Descriptor::new("SCnc").with("ncTp", enm("STnc", COMPONENTS.iter().find(|(_, name)| *name == value)?.0))
        } else {
            let object = component.as_object()?;
            if object.len() != 1 {
                return None;
            }
            let literal = object.get("text")?.as_str()?;
            if literal.len() > 4096 || literal.chars().any(|c| c.is_control() || c == '/') {
                return None;
            }
            Descriptor::new("SCnc").with("ncTp", enm("STnc", "NC25")).with("ncSt", text(literal))
        };
        parts.push(D::Descriptor(part));
    }
    while parts.len() < 8 {
        parts.push(D::Descriptor(Descriptor::new("SCnc").with("ncTp", enm("STnc", "NC24"))));
    }
    parts.push(D::Descriptor(Descriptor::new("SCnc").with("ncTp", enm("STnc", code))));
    let mut d = Descriptor::new("SaveForWeb")
        .with("ovNC", D::List(parts))
        .with("ovCM", D::Boolean(false))
        .with("ovCW", D::Boolean(true))
        .with("ovCU", D::Boolean(unix));
    if let Some(value) = object.get("fileName") {
        let name = value.as_str()?;
        if !file_name_supported(name, format_extension(format)?) {
            return None;
        }
        d = d.with("ovFN", text(name));
    }
    Some(d)
}
