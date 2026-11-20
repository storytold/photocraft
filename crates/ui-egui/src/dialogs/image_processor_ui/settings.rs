//! XML presets for the Image Processor dialog. The element names and order follow the measured settings file.

use std::borrow::Cow;
use std::fmt::Write;

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use craft_xml::{Doctype, Event, Limits, Node, Options};

pub(super) const MAX_BYTES: usize = 256 * 1024;

const ROOT: &str = "ImageProcessor";
// XML has only an open-documents/folder switch. A processing instruction preserves PhotoCraft's
// third choice without adding an element to the reference format.
const CURRENT_SOURCE: &str = r#"source="current""#;

#[derive(Clone, Copy)]
enum Kind {
    OpenDocuments,
    Bool(bool),
    Text,
    Int(i64),
    Quality,
}

use Kind::{Bool, Int, OpenDocuments, Quality, Text};

// Fields without a dialog key use the current processor behavior. Load validates those settings too.
const ELEMENTS: &[(&str, Option<&str>, Kind)] = &[
    ("version", None, Int(1)),
    ("useopen", Some("source"), OpenDocuments),
    ("includesub", Some("includeSubfolders"), Bool(false)),
    ("source", Some("input"), Text),
    ("open", None, Bool(false)),
    ("saveinsame", None, Bool(false)),
    ("dest", Some("output"), Text),
    ("jpeg", Some("jpeg/enabled"), Bool(true)),
    ("psd", Some("psd/enabled"), Bool(false)),
    ("tiff", Some("tiff/enabled"), Bool(false)),
    ("lzw", Some("tiff/lzw"), Bool(false)),
    ("converticc", Some("jpeg/convertToSrgb"), Bool(false)),
    ("q", Some("jpeg/quality"), Quality),
    ("max", Some("psd/maximizeCompatibility"), Bool(true)),
    ("jpegresize", Some("jpeg/resize"), Bool(false)),
    ("jpegw", Some("jpeg/width"), Int(30_000)),
    ("jpegh", Some("jpeg/height"), Int(30_000)),
    ("psdresize", Some("psd/resize"), Bool(false)),
    ("psdw", Some("psd/width"), Int(30_000)),
    ("psdh", Some("psd/height"), Int(30_000)),
    ("tiffresize", Some("tiff/resize"), Bool(false)),
    ("tiffw", Some("tiff/width"), Int(30_000)),
    ("tiffh", Some("tiff/height"), Int(30_000)),
    ("runaction", None, Bool(false)),
    ("actionset", None, Text),
    ("action", None, Text),
    ("info", None, Text),
    ("icc", None, Bool(true)),
    ("keepstructure", None, Bool(true)),
];

fn invalid(tag: &str) -> String {
    crate::i18n::fmt(tl!("Invalid Image Processor setting: {setting}"), &[("setting", tag)])
}

fn too_large() -> String {
    tl!("The Image Processor settings file is too large.").into()
}

pub(super) fn encode(fields: &DialogFields) -> Result<Vec<u8>, String> {
    let mut xml = String::from('\u{feff}');
    let _ = writeln!(xml, "<{ROOT}>");
    if fields.text("source") == Some("current") {
        let _ = writeln!(xml, "\t<?PhotoCraft {CURRENT_SOURCE}?>");
    }
    for &(tag, field, kind) in ELEMENTS {
        let field = field.unwrap_or_default();
        let _ = write!(xml, "\t<{tag}>");
        match kind {
            OpenDocuments => xml.push_str(if fields.text(field) == Some("open") { "true" } else { "false" }),
            Bool(default) => {
                let _ = write!(xml, "{}", fields.bool(field).unwrap_or(default));
            }
            Int(max) => {
                let value = fields.int(field).unwrap_or(1);
                if !(1..=max).contains(&value) {
                    return Err(invalid(tag));
                }
                let _ = write!(xml, "{value}");
            }
            Quality => {
                let value = fields.number(field).unwrap_or(5.0);
                if !(1.0..=12.0).contains(&value) {
                    return Err(invalid(tag));
                }
                let _ = write!(xml, "{value}");
            }
            Text => {
                let text = fields.text(field).unwrap_or_default();
                if text.len() > MAX_BYTES {
                    return Err(too_large());
                }
                if !text.chars().all(craft_xml::is_xml_char) {
                    return Err(invalid(tag));
                }
                // Keep the preset's one element per line even for paths containing line breaks.
                xml.push_str(&craft_xml::esc(text).replace('\n', "&#10;"));
            }
        }
        let _ = writeln!(xml, "</{tag}>");
    }
    let _ = writeln!(xml, "</{ROOT}>");
    if xml.len() > MAX_BYTES {
        return Err(too_large());
    }
    Ok(xml.into_bytes())
}

pub(super) fn decode(bytes: &[u8]) -> Result<DialogFields, String> {
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| tl!("Image Processor settings must be UTF-8.").to_string())?;
    let options = Options { doctype: Doctype::Reject, pis: true, ..Options::default() }.strict().with_limits(Limits {
        max_input: MAX_BYTES,
        max_depth: 32,
        max_nodes: 4096,
        ..Limits::default()
    });
    if text.contains('&') {
        validate_references(text.strip_prefix('\u{feff}').unwrap_or(text), &options)?;
    }
    let root = options.parse(bytes).map_err(xml_error)?;
    if root.name != ROOT || root.attr("xmlns").is_some_and(|ns| !ns.is_empty()) {
        return Err(tl!("This file does not contain Image Processor settings.").into());
    }
    let current = root.children.iter().any(|node| matches!(node, Node::Pi { target, data } if target == "PhotoCraft" && data == CURRENT_SOURCE));
    let mut seen = std::collections::HashSet::new();
    let mut updates = DialogFields::new();
    for node in &root.children {
        let child = match node {
            Node::El(child) if child.attr("xmlns").is_none_or(str::is_empty) => child,
            Node::Text(text) if !text.trim().is_empty() => return Err(invalid(ROOT)),
            _ => continue,
        };
        let tag = child.name.as_str();
        let Some(&(_, field, kind)) = ELEMENTS.iter().find(|(name, _, _)| *name == tag) else { continue };
        if !seen.insert(tag) || child.children.iter().any(|node| matches!(node, Node::El(_))) {
            return Err(invalid(tag));
        }
        let value = parse_value(kind, child.text(), current).ok_or_else(|| invalid(tag))?;
        if let Some(field) = field {
            updates.set(field, value);
        }
    }
    Ok(updates)
}

fn xml_error(error: craft_xml::Error) -> String {
    crate::i18n::fmt(tl!("Could not read Image Processor settings: {error}"), &[("error", &error.to_string())])
}

/// The shared reader accepts bare ampersands even in strict mode. Its event spans let us
/// check the original text and attributes without treating CDATA, comments or instructions as XML text.
fn validate_references(text: &str, options: &Options<'_>) -> Result<(), String> {
    let mut events = options.events(text.as_bytes());
    while let Some(event) = events.next().map_err(xml_error)? {
        if !matches!(event, Event::Text(_) | Event::Start { .. }) {
            continue;
        }
        let raw = text.get(events.span()).ok_or_else(|| invalid(ROOT))?;
        if raw.split('&').skip(1).any(|tail| tail.split_once(';').and_then(|(name, _)| craft_xml::entity(name)).is_none()) {
            return Err(tl!("Image Processor settings contain an unescaped ampersand.").into());
        }
    }
    Ok(())
}

fn parse_value(kind: Kind, text: Cow<'_, str>, current: bool) -> Option<FieldValue> {
    let trimmed = text.trim();
    Some(match kind {
        OpenDocuments => {
            let open = trimmed.parse::<bool>().ok()?;
            if open {
                "open"
            } else if current {
                "current"
            } else {
                "folder"
            }
            .into()
        }
        Bool(_) => trimmed.parse::<bool>().ok()?.into(),
        Text => text.into_owned().into(),
        Int(max) => {
            let value = trimmed.parse::<i64>().ok()?;
            (1..=max).contains(&value).then_some(value)?.into()
        }
        Quality => {
            let value = trimmed.parse::<f64>().ok()?;
            (1.0..=12.0).contains(&value).then_some(value)?.into()
        }
    })
}
