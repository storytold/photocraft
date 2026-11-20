//! XML presets for the Image Processor dialog. The element names and order follow the measured settings file.

use std::fmt::Write;

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use roxmltree::{Document, ParsingOptions};

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
    if fields.text("source") == Some("current") {
        let _ = writeln!(xml, "<?PhotoCraft {CURRENT_SOURCE}?>");
    }
    let _ = writeln!(xml, "<{ROOT}>");
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
                push_text(&mut xml, text).ok_or_else(|| invalid(tag))?;
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

fn push_text(xml: &mut String, text: &str) -> Option<()> {
    for c in text.chars() {
        match c {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            // Keep each element on one line. XML also normalizes literal carriage returns.
            '\r' => xml.push_str("&#13;"),
            '\n' => xml.push_str("&#10;"),
            '\t' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => xml.push(c),
            _ => return None,
        }
    }
    Some(())
}

pub(super) fn decode(bytes: &[u8]) -> Result<DialogFields, String> {
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| tl!("Image Processor settings must be UTF-8.").to_string())?;
    let document = Document::parse_with_options(text, ParsingOptions { nodes_limit: 4096, ..Default::default() })
        .map_err(|error| crate::i18n::fmt(tl!("Could not read Image Processor settings: {error}"), &[("error", &error.to_string())]))?;
    let root = document.root_element();
    if !root.has_tag_name(ROOT) || root.tag_name().namespace().is_some() {
        return Err(tl!("This file does not contain Image Processor settings.").into());
    }
    let current = document.root().children().filter_map(|node| node.pi()).any(|pi| pi.target == "PhotoCraft" && pi.value == Some(CURRENT_SOURCE));
    let mut seen = std::collections::HashSet::new();
    let mut updates = DialogFields::new();
    for child in root.children() {
        if child.is_text() && child.text().is_some_and(|text| !text.trim().is_empty()) {
            return Err(invalid(ROOT));
        }
        if !child.is_element() || child.tag_name().namespace().is_some() {
            continue;
        }
        let tag = child.tag_name().name();
        let Some(&(_, field, kind)) = ELEMENTS.iter().find(|(name, _, _)| *name == tag) else { continue };
        if !seen.insert(tag) || child.children().any(|node| node.is_element()) {
            return Err(invalid(tag));
        }
        let text: String = child.children().filter(|node| node.is_text()).filter_map(|node| node.text()).collect();
        let value = parse_value(kind, text, current).ok_or_else(|| invalid(tag))?;
        if let Some(field) = field {
            updates.set(field, value);
        }
    }
    Ok(updates)
}

fn parse_value(kind: Kind, text: String, current: bool) -> Option<FieldValue> {
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
        Text => text.into(),
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
