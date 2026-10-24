//! Bounded metadata data translation. Source tag names select a finite public
//! schema, never XML instructions, a network namespace lookup or executable code.
use photocraft_doc::Document;
use std::collections::BTreeMap;

const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/";
const PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
const MAX_PACKET: usize = super::MAX_STRING;

#[derive(Clone, Copy)]
enum Shape {
    Text,
    Bag,
    Seq,
    Alt,
}

fn schema(key: &str) -> Option<(&str, &str, Shape)> {
    let tail = key.strip_prefix("Xmp.")?;
    let (prefix, name) = tail.split_once('.')?;
    let (uri, shape) = match (prefix, name) {
        ("dc", "contributor" | "language" | "publisher" | "relation" | "subject" | "type") => (DC, Shape::Bag),
        ("dc", "creator" | "date") => (DC, Shape::Seq),
        ("dc", "title" | "description" | "rights") => (DC, Shape::Alt),
        ("dc", "coverage" | "format" | "identifier" | "source") => (DC, Shape::Text),
        ("xmp", "CreatorTool" | "CreateDate" | "ModifyDate" | "MetadataDate" | "Label" | "Rating") => (XMP, Shape::Text),
        (
            "photoshop",
            "AuthorsPosition"
            | "CaptionWriter"
            | "Category"
            | "City"
            | "Country"
            | "Credit"
            | "DateCreated"
            | "Headline"
            | "Instructions"
            | "Source"
            | "State"
            | "TransmissionReference"
            | "Urgency",
        ) => (PHOTOSHOP, Shape::Text),
        ("photoshop", "SupplementalCategories" | "DocumentAncestors") => (PHOTOSHOP, Shape::Bag),
        _ => return None,
    };
    Some((uri, tail, shape))
}

fn append(out: &mut String, text: &str) -> Option<()> {
    if out.len().checked_add(text.len())? > MAX_PACKET {
        return None;
    }
    out.push_str(text);
    Some(())
}
fn escape(out: &mut String, text: &str) -> Option<()> {
    for ch in text.chars() {
        match ch {
            '&' => append(out, "&amp;")?,
            '<' => append(out, "&lt;")?,
            '>' => append(out, "&gt;")?,
            '"' => append(out, "&quot;")?,
            '\'' => append(out, "&apos;")?,
            _ => {
                let mut bytes = [0; 4];
                append(out, ch.encode_utf8(&mut bytes))?;
            }
        }
    }
    Some(())
}
fn language(value: &str) -> Option<(&str, &str)> {
    if let Some(value) = value.strip_prefix("lang=\"") {
        let (lang, text) = value.split_once("\" ")?;
        if lang.is_empty() || lang.len() > 35 || !lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        Some((lang, text))
    } else if value.starts_with("lang=") {
        None
    } else {
        Some(("x-default", value))
    }
}

/// Plain source text is already retained separately. Unknown types, structures,
/// invalid language alternatives and source binary values do not become RDF.
fn xmp(tree: &roxmltree::Document<'_>) -> Option<String> {
    let root = tree.root_element();
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for tag in root.children().filter(|n| n.has_tag_name("tag") && n.attribute("encoding").is_none() && !n.children().any(|child| child.is_element())) {
        let (Some(key), Some(value)) = (tag.attribute("name"), tag.text()) else {
            continue;
        };
        if schema(key).is_some() {
            groups.entry(key).or_default().push(value);
        }
    }
    let mut out = String::new();
    append(
        &mut out,
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\">",
    )?;
    let mut count = 0;
    for (key, values) in groups {
        let Some((uri, tail, shape)) = schema(key) else {
            continue;
        };
        let (prefix, name) = tail.split_once('.')?;
        if root.children().filter(|n| n.has_tag_name("namespace") && n.attribute("prefix") == Some(prefix)).any(|n| n.attribute("url") != Some(uri)) {
            continue;
        }
        if matches!(shape, Shape::Text) && values.len() != 1 {
            continue;
        }
        let alternatives = if matches!(shape, Shape::Alt) {
            let Some(mut parsed) = values.iter().map(|value| language(value)).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let unique: std::collections::BTreeSet<_> = parsed.iter().map(|(lang, _)| *lang).collect();
            if unique.len() != parsed.len() {
                continue;
            }
            parsed.sort_by_key(|(lang, _)| *lang != "x-default");
            parsed
        } else {
            Vec::new()
        };
        append(&mut out, "<")?;
        append(&mut out, prefix)?;
        append(&mut out, ":")?;
        append(&mut out, name)?;
        append(&mut out, ">")?;
        match shape {
            Shape::Text => escape(&mut out, values.first()?)?,
            Shape::Bag | Shape::Seq => {
                let kind = if matches!(shape, Shape::Bag) { "Bag" } else { "Seq" };
                append(&mut out, "<rdf:")?;
                append(&mut out, kind)?;
                append(&mut out, ">")?;
                for value in values {
                    append(&mut out, "<rdf:li>")?;
                    escape(&mut out, value)?;
                    append(&mut out, "</rdf:li>")?;
                }
                append(&mut out, "</rdf:")?;
                append(&mut out, kind)?;
                append(&mut out, ">")?;
            }
            Shape::Alt => {
                append(&mut out, "<rdf:Alt>")?;
                for (lang, text) in alternatives {
                    append(&mut out, "<rdf:li xml:lang=\"")?;
                    escape(&mut out, lang)?;
                    append(&mut out, "\">")?;
                    escape(&mut out, text)?;
                    append(&mut out, "</rdf:li>")?;
                }
                append(&mut out, "</rdf:Alt>")?;
            }
        }
        append(&mut out, "</")?;
        append(&mut out, prefix)?;
        append(&mut out, ":")?;
        append(&mut out, name)?;
        append(&mut out, ">")?;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    append(&mut out, "</rdf:Description></rdf:RDF></x:xmpmeta>")?;
    Some(out)
}

/// Classic TIFF IFD0 ASCII fields already supported by native TIFF metadata.
/// No dimensions, orientation, profiles, maker notes or unknown binary types
/// are guessed from source display strings.
fn exif(tree: &roxmltree::Document<'_>) -> Option<Vec<u8>> {
    let mut fields: BTreeMap<u16, &str> = BTreeMap::new();
    let mut duplicate = std::collections::BTreeSet::new();
    for tag in
        tree.root_element().children().filter(|n| n.has_tag_name("tag") && n.attribute("encoding").is_none() && !n.children().any(|child| child.is_element()))
    {
        let (Some(key), Some(value)) = (tag.attribute("name"), tag.text()) else {
            continue;
        };
        let id = match key {
            "Exif.Image.ImageDescription" => 270,
            "Exif.Image.Make" => 271,
            "Exif.Image.Model" => 272,
            "Exif.Image.Software" => 305,
            "Exif.Image.DateTime" => 306,
            "Exif.Image.Artist" => 315,
            "Exif.Image.Copyright" => 33432,
            _ => continue,
        };
        if !value.is_ascii() || value.as_bytes().contains(&0) || value.len() > MAX_PACKET {
            continue;
        }
        if fields.insert(id, value).is_some() {
            duplicate.insert(id);
        }
    }
    fields.retain(|id, _| !duplicate.contains(id));
    if fields.is_empty() {
        return None;
    }
    let count = u16::try_from(fields.len()).ok()?;
    let data_start = 8usize.checked_add(2)?.checked_add(fields.len().checked_mul(12)?)?.checked_add(4)?;
    let total = fields.values().try_fold(data_start, |size, value| size.checked_add(value.len().checked_add(1)?.checked_add((value.len() + 1) % 2)?))?;
    if total > MAX_PACKET {
        return None;
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"II\x2a\0\x08\0\0\0");
    out.extend_from_slice(&count.to_le_bytes());
    let mut data = Vec::new();
    for (id, value) in fields {
        let len = value.len().checked_add(1)?;
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
        if len <= 4 {
            let mut inline = [0u8; 4];
            inline.get_mut(..value.len())?.copy_from_slice(value.as_bytes());
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&u32::try_from(data_start.checked_add(data.len())?).ok()?.to_le_bytes());
            data.extend_from_slice(value.as_bytes());
            data.push(0);
            if data.len() % 2 != 0 {
                data.push(0);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
    Some(out)
}

/// Source packets lack a type registry. Only unambiguous known schema fields
/// become native packets; existing native raw packets take priority.
pub(super) fn translate(doc: &mut Document, tree: &roxmltree::Document<'_>, budget: &mut usize) -> (bool, bool) {
    let mut imported = (false, false);
    if doc.metadata.xmp.is_none()
        && let Some(packet) = xmp(tree)
        && let Some(left) = budget.checked_sub(packet.len())
    {
        *budget = left;
        doc.metadata.xmp = Some(packet.into());
        imported.0 = true;
    }
    if doc.metadata.exif.is_none()
        && let Some(packet) = exif(tree)
        && let Some(left) = budget.checked_sub(packet.len())
    {
        *budget = left;
        doc.metadata.exif = Some(std::sync::Arc::new(packet));
        imported.1 = true;
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::Size;
    #[test]
    fn native_packets_obey_shared_budget_and_existing_packet_priority() {
        let source = r#"<metadata><tag name="Xmp.xmp.Label">Label</tag><tag name="Exif.Image.Artist">AB</tag></metadata>"#;
        let tree = roxmltree::Document::parse(source).unwrap();
        let mut doc = Document::new("Metadata", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut empty = 0;
        assert_eq!(translate(&mut doc, &tree, &mut empty), (false, false));
        assert!(doc.metadata.xmp.is_none() && doc.metadata.exif.is_none());
        let mut budget = super::super::MAX_METADATA_BYTES;
        assert_eq!(translate(&mut doc, &tree, &mut budget), (true, true));
        let packet = doc.metadata.exif.as_ref().unwrap();
        assert_eq!(packet.get(18..22), Some(b"AB\0\0".as_slice()));
        let before = (doc.metadata.xmp.clone(), doc.metadata.exif.clone(), budget);
        assert_eq!(translate(&mut doc, &tree, &mut budget), (false, false));
        assert_eq!((doc.metadata.xmp, doc.metadata.exif, budget), before);
    }
    #[test]
    fn oversized_escaped_xmp_and_invalid_language_never_create_partial_packets() {
        let value = "&lt;".repeat(MAX_PACKET / 2);
        let xml = format!("<metadata><tag name=\"Xmp.dc.title\">{value}</tag></metadata>");
        let tree = roxmltree::Document::parse(&xml).unwrap();
        assert!(xmp(&tree).is_none());
        let invalid = roxmltree::Document::parse(r#"<metadata><tag name="Xmp.dc.title">lang=&quot;../../bad&quot; Unsafe</tag></metadata>"#).unwrap();
        assert!(xmp(&invalid).is_none());
    }
}
