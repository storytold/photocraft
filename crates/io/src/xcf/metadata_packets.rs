//! Bounded metadata data translation. Source tag names select a finite public
//! schema, never XML instructions, a network namespace lookup or executable code.
use super::{IoError, Result as IoResult, Warnings};
use photocraft_doc::Document;
use photocraft_raster::Interrupt;
use std::borrow::Cow;
use std::collections::BTreeMap;

const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/";
const PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
const MAX_PACKET: usize = craft_xcf::MAX_STRING;

// Selected packets are bounded by the shared record reader. Resolve duplicates
// before parsing: repeated aliases must not multiply XML decoding work.
const MAX_XML_BYTES: usize = 4 << 20;
const MAX_XML_NODES: usize = 4096;
const MAX_XML_ATTRIBUTES: usize = 4096;

pub(super) fn apply(
    document: &mut Document,
    source: &craft_xcf::records::metadata::ImageMetadata<'_>,
    warnings: &mut Warnings,
    ctl: &Interrupt<'_>,
) -> IoResult<()> {
    use craft_xcf::records::metadata::PacketKind;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !source.complete() {
        return Ok(());
    }
    let mut selected = [(PacketKind::ImageMetadata, usize::MAX, None), (PacketKind::Metadata, usize::MAX, None)];
    for (ordinal, packet) in source.packets.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        for (kind, first, bytes) in &mut selected {
            if packet.value.kind == *kind {
                if bytes.is_none() {
                    *first = ordinal;
                }
                *bytes = Some(packet.value.bytes);
            }
        }
    }
    selected.sort_by_key(|(_, ordinal, _)| *ordinal);
    let used = document
        .metadata
        .text
        .iter()
        .fold(document.metadata.text.capacity().saturating_mul(std::mem::size_of::<(String, String)>()), |used, (key, value)| {
            used.saturating_add(key.capacity()).saturating_add(value.capacity())
        })
        .saturating_add(document.metadata.exif.as_ref().map_or(0, |packet| packet.capacity()))
        .saturating_add(document.metadata.xmp.as_ref().map_or(0, |packet| packet.len()));
    let mut budget = craft_xcf::MAX_METADATA_BYTES.saturating_sub(used);
    let mut storage = TextStorage { charged_capacity: document.metadata.text.capacity() };
    for (_, _, packet) in selected {
        let Some(packet) = packet else { continue };
        // This historical non-XML envelope has no schema understood here.
        if packet.starts_with(b"GIMP_XMP_1\0") {
            warnings.note("the legacy XCF XMP envelope has no native metadata mapping; saved pixels were preserved");
            continue;
        }
        let packet = packet.strip_suffix(&[0]).unwrap_or(packet);
        let tree = match parse_xml(packet, budget, ctl) {
            Ok(tree) => tree,
            Err(IoError::Cancelled) => return Err(IoError::Cancelled),
            Err(_) => {
                warnings.note("optional XCF XML metadata is invalid or exceeds native metadata limits; saved pixels were preserved");
                continue;
            }
        };
        if tree.name != "metadata" {
            warnings.note("the XCF XML metadata schema has no native mapping; saved pixels were preserved");
            continue;
        }
        let mut incomplete = false;
        for node in tree.children(craft_xml::Local("tag")) {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            if let (Some(key), Some(value)) = (node.attr("name"), plain_tag_text(node)) {
                if matches!(key.split('.').next(), Some("Exif" | "Xmp" | "Iptc")) {
                    incomplete |= !push_text(document, key, &value, &mut storage, &mut budget);
                }
            } else {
                incomplete = true;
            }
        }
        let statuses = translate(document, &tree, &mut budget, ctl);
        if [statuses.0, statuses.1].contains(&PacketStatus::Cancelled) {
            return Err(IoError::Cancelled);
        }
        incomplete |= [statuses.0, statuses.1].contains(&PacketStatus::OverBudget);
        if incomplete {
            warnings.note("some XCF metadata tags could not be mapped within native metadata limits; usable values and saved pixels were preserved");
        }
    }
    Ok(())
}

/// Use the shared pull reader as an interruptible, allocation-bounded preflight
/// before building the optional tree; no XCF framing is reimplemented here.
fn parse_xml(bytes: &[u8], budget: usize, ctl: &Interrupt<'_>) -> IoResult<craft_xml::El> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    // Admit a conservative full text/string copy allowance before tree allocation.
    // Nodes and attributes have separate hard counts; the tree is transient.
    if bytes.len() > MAX_XML_BYTES || bytes.len().saturating_mul(4) > budget {
        return Err(super::invalid("optional metadata exceeds the bounded XML allowance"));
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| super::invalid("optional metadata is not UTF-8"))?;
    if craft_svg::guard::too_wide(xml, 64, 65536) {
        return Err(super::invalid("optional metadata exceeds the bounded XML attribute work"));
    }
    let namespaces = craft_xml::Canonical::new(&[]);
    let options = craft_xml::Options {
        names: craft_xml::Names::Canonical(&namespaces),
        ns_decls: false,
        strict: true,
        strict_end_tags: true,
        doctype: craft_xml::Doctype::Reject,
        unknown_entity: craft_xml::UnknownEntity::Error,
        cdata: true,
        comments: true,
        pis: true,
        limits: craft_xml::Limits {
            max_input: MAX_XML_BYTES,
            max_depth: 64,
            max_nodes: MAX_XML_NODES,
            max_elements: MAX_XML_NODES,
            max_attrs: 64,
            max_attrs_total: MAX_XML_ATTRIBUTES,
        },
        ..Default::default()
    };
    let mut events = options.events(bytes);
    let mut nodes = 0usize;
    let mut attributes = 0usize;
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some(event) = events.next().map_err(|_| super::invalid("invalid optional metadata XML"))? else { break };
        if !matches!(event, craft_xml::Event::End { .. }) {
            nodes = nodes.saturating_add(1);
        }
        if let craft_xml::Event::Start { attrs, .. } = event {
            attributes = attributes.saturating_add(attrs.len());
        }
        if nodes > MAX_XML_NODES || attributes > MAX_XML_ATTRIBUTES {
            return Err(super::invalid("optional metadata exceeds the bounded XML tree allowance"));
        }
    }
    // The common XML preflight also validates qualified names, declarations and
    // XML character rules before the application-specific tree is retained.
    let limits = craft_svg::preflight::Limits {
        bytes: MAX_XML_BYTES,
        depth: 64,
        allow_dtd: false,
        entity_bytes: MAX_PACKET,
        refuse_entity_markup: true,
        attributes: 64,
        attribute_work: 65536,
        xml_nodes: MAX_XML_NODES as u32,
    };
    craft_svg::preflight::parse_text(xml, &limits).map_err(|_| super::invalid("invalid optional metadata XML"))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let tree = options.parse(bytes).map_err(|_| super::invalid("invalid optional metadata XML"))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(tree)
}

/// Borrow a single segment; admit the complete size before joining split text.
fn plain_tag_text(tag: &craft_xml::El) -> Option<Cow<'_, str>> {
    if tag.attr("encoding").is_some() || tag.elements().next().is_some() {
        return None;
    }
    let mut text = tag.children.iter().filter_map(|child| match child {
        craft_xml::Node::Text(text) | craft_xml::Node::CData(text) => Some(text.as_str()),
        _ => None,
    });
    let first = text.next()?;
    let Some(second) = text.next() else { return Some(Cow::Borrowed(first)) };
    let total = text.clone().try_fold(first.len().checked_add(second.len())?, |total, next| total.checked_add(next.len()))?;
    if total > MAX_PACKET {
        return None;
    }
    let mut joined = String::new();
    joined.try_reserve_exact(total).ok()?;
    joined.push_str(first);
    joined.push_str(second);
    for segment in text {
        joined.push_str(segment);
    }
    Some(Cow::Owned(joined))
}

#[derive(Clone, Copy)]
enum Shape {
    Text,
    Bag,
    Seq,
    Alt,
}

/// Resolve only the finite admitted XMP tag schemas and their native RDF container shapes; unknown namespaces remain unmapped.
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
        ("photoshop", "SupplementalCategories") => (PHOTOSHOP, Shape::Bag),
        _ => return None,
    };
    Some((uri, tail, shape))
}

/// Append packet text only when the checked resulting length stays inside the packet bound.
fn append(out: &mut String, text: &str) -> Result<(), PacketStatus> {
    if out.len().saturating_add(text.len()) > MAX_PACKET {
        return Err(PacketStatus::OverBudget);
    }
    out.try_reserve(text.len()).map_err(|_| PacketStatus::OverBudget)?;
    out.push_str(text);
    Ok(())
}
/// Encode XML-sensitive characters through the same bounded packet writer, failing before oversized escaped output is committed.
fn escape(out: &mut String, text: &str, ctl: &Interrupt<'_>) -> Result<(), PacketStatus> {
    for (index, ch) in text.chars().enumerate() {
        if index % 4096 == 0 && ctl.cancelled() {
            return Err(PacketStatus::Cancelled);
        }
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
    Ok(())
}
/// Validate a stored language-alternative prefix or choose the native default-language entry for unqualified text.
fn language(value: &str) -> Option<(&str, &str)> {
    if let Some(value) = value.strip_prefix("lang=\"") {
        let (lang, text) = value.split_once("\" ")?;
        let mut subtags = lang.split('-');
        let primary = subtags.next()?;
        if lang.len() > 35
            || !(1..=8).contains(&primary.len())
            || !primary.bytes().all(|b| b.is_ascii_alphabetic())
            || !subtags.clone().all(|part| (1..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return None;
        }
        let mut private = primary.eq_ignore_ascii_case("x");
        let mut required = usize::from(private);
        for part in subtags {
            if part.len() < required {
                return None;
            }
            required = 0;
            if !private && part.len() == 1 {
                private = part.eq_ignore_ascii_case("x");
                required = if private { 1 } else { 2 };
            }
        }
        (required == 0).then_some((lang, text))
    } else if value.starts_with("lang=") {
        None
    } else {
        Some(("x-default", value))
    }
}

/// Validate an RDF language alternative as a whole, rejecting duplicate languages and placing `x-default` first.
fn language_alternatives<'a>(values: &'a [Cow<'_, str>]) -> Option<Vec<(&'a str, &'a str)>> {
    let mut parsed = values.iter().map(|value| language(value)).collect::<Option<Vec<_>>>()?;
    let unique: std::collections::BTreeSet<_> = parsed.iter().map(|(lang, _)| lang.to_ascii_lowercase()).collect();
    if unique.len() != parsed.len() {
        return None;
    }
    // Preserve the source spelling of ordinary language tags.
    for (lang, _) in &mut parsed {
        if lang.eq_ignore_ascii_case("x-default") {
            *lang = "x-default";
        }
    }
    parsed.sort_by_key(|(lang, _)| *lang != "x-default");
    Some(parsed)
}

/// Plain source text is already retained separately. Unknown types, structures,
/// invalid language alternatives and source binary values do not become RDF.
fn xmp(root: &craft_xml::El, ctl: &Interrupt<'_>) -> Result<Option<String>, PacketStatus> {
    let mut groups: BTreeMap<&str, Vec<Option<Cow<'_, str>>>> = BTreeMap::new();
    for tag in root.children(craft_xml::Local("tag")) {
        if ctl.cancelled() {
            return Err(PacketStatus::Cancelled);
        }
        if let Some(key) = tag.attr("name")
            && schema(key).is_some()
        {
            groups.entry(key).or_default().push(plain_tag_text(tag));
        }
    }
    let mut out = String::new();
    append(
        &mut out,
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\">",
    )?;
    let mut count = 0;
    for (key, values) in groups {
        if ctl.cancelled() {
            return Err(PacketStatus::Cancelled);
        }
        let Some((uri, tail, shape)) = schema(key) else {
            continue;
        };
        let Some((prefix, name)) = tail.split_once('.') else { continue };
        if root.children(craft_xml::Local("namespace")).filter(|n| n.attr("prefix") == Some(prefix)).any(|n| n.attr("url") != Some(uri)) {
            continue;
        }
        if matches!(shape, Shape::Text) && values.len() != 1 {
            continue;
        }
        let Some(values) = values.into_iter().collect::<Option<Vec<_>>>() else { continue };
        if key == "Xmp.xmp.Rating" && !values.first().is_some_and(|rating| rating.trim().parse::<f64>().is_ok_and(|n| n == -1. || (0. ..=5.).contains(&n))) {
            continue;
        }
        let alternatives = if matches!(shape, Shape::Alt) {
            let Some(parsed) = language_alternatives(&values) else {
                continue;
            };
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
            Shape::Text => {
                if let Some(value) = values.first() {
                    escape(&mut out, value, ctl)?;
                }
            }
            Shape::Bag | Shape::Seq => {
                let kind = if matches!(shape, Shape::Bag) { "Bag" } else { "Seq" };
                append(&mut out, "<rdf:")?;
                append(&mut out, kind)?;
                append(&mut out, ">")?;
                for value in values {
                    append(&mut out, "<rdf:li>")?;
                    escape(&mut out, &value, ctl)?;
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
                    escape(&mut out, lang, ctl)?;
                    append(&mut out, "\">")?;
                    escape(&mut out, text, ctl)?;
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
        return Ok(None);
    }
    append(&mut out, "</rdf:Description></rdf:RDF></x:xmpmeta>")?;
    Ok(Some(out))
}

fn exif(tree: &craft_xml::El, ctl: &Interrupt<'_>) -> Result<Option<Vec<u8>>, PacketStatus> {
    let mut fields: BTreeMap<u16, Cow<'_, str>> = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for tag in tree.children(craft_xml::Local("tag")) {
        if ctl.cancelled() {
            return Err(PacketStatus::Cancelled);
        }
        let Some(key) = tag.attr("name") else { continue };
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
        if !seen.insert(id) {
            fields.remove(&id);
            continue;
        }
        let Some(value) = plain_tag_text(tag) else { continue };
        if !value.is_ascii() || value.as_bytes().contains(&0) {
            continue;
        }
        fields.insert(id, value);
    }
    if fields.is_empty() {
        return Ok(None);
    }
    // The fixed field registry above caps the IFD to seven records.
    let count = fields.len() as u16;
    let data_start = 8 + 2 + fields.len() * 12 + 4;
    let total = fields.values().fold(data_start, |size, value| {
        let len = value.len().saturating_add(1);
        if len <= 4 { size } else { size.saturating_add(len).saturating_add(len % 2) }
    });
    if total > MAX_PACKET {
        return Err(PacketStatus::OverBudget);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| PacketStatus::OverBudget)?;
    out.extend_from_slice(b"II\x2a\0\x08\0\0\0");
    out.extend_from_slice(&count.to_le_bytes());
    let mut data = Vec::new();
    data.try_reserve_exact(total - data_start).map_err(|_| PacketStatus::OverBudget)?;
    for (id, value) in fields {
        if ctl.cancelled() {
            return Err(PacketStatus::Cancelled);
        }
        let len = value.len() + 1; // Bounded by packet preflight.
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        if len <= 4 {
            let mut inline = [0u8; 4];
            inline[..value.len()].copy_from_slice(value.as_bytes());
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&((data_start + data.len()) as u32).to_le_bytes());
            data.extend_from_slice(value.as_bytes());
            data.push(0);
            if data.len() % 2 != 0 {
                data.push(0);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);
    Ok(Some(out))
}

/// Source packets lack a type registry. Only unambiguous known schema fields
/// become native packets; existing native raw packets take priority.
pub(super) fn translate(doc: &mut Document, tree: &craft_xml::El, budget: &mut usize, ctl: &Interrupt<'_>) -> (PacketStatus, PacketStatus) {
    let xmp = if doc.metadata.xmp.is_none() {
        retain_packet(xmp(tree, ctl), budget, String::capacity, |packet| doc.metadata.xmp = Some(packet.into()))
    } else {
        PacketStatus::Unavailable
    };
    let exif = if doc.metadata.exif.is_none() {
        retain_packet(exif(tree, ctl), budget, Vec::capacity, |packet| doc.metadata.exif = Some(std::sync::Arc::new(packet)))
    } else {
        PacketStatus::Unavailable
    };
    (xmp, exif)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PacketStatus {
    Retained,
    Unavailable,
    OverBudget,
    Cancelled,
}

fn retain_packet<T>(packet: Result<Option<T>, PacketStatus>, budget: &mut usize, len: impl FnOnce(&T) -> usize, save: impl FnOnce(T)) -> PacketStatus {
    let packet = match packet {
        Ok(Some(packet)) => packet,
        Ok(None) => return PacketStatus::Unavailable,
        Err(status) => return status,
    };
    let Some(left) = budget.checked_sub(len(&packet)) else { return PacketStatus::OverBudget };
    *budget = left;
    save(packet);
    PacketStatus::Retained
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::Size;
    /// Verify synthesized EXIF/XMP packets respect the shared allowance and never overwrite already present native packets.
    #[test]
    fn native_packets_obey_shared_budget_and_existing_packet_priority() {
        let source = r#"<metadata><tag name="Xmp.xmp.Label">Label</tag><tag name="Exif.Image.Artist">AB</tag></metadata>"#;
        let tree = craft_xml::parse(source.as_bytes()).unwrap();
        let mut doc = Document::new("Metadata", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        let mut empty = 0;
        assert_eq!(translate(&mut doc, &tree, &mut empty, &Interrupt::NONE), (PacketStatus::OverBudget, PacketStatus::OverBudget));
        assert!(doc.metadata.xmp.is_none() && doc.metadata.exif.is_none());
        let mut budget = craft_xcf::MAX_METADATA_BYTES;
        assert_eq!(translate(&mut doc, &tree, &mut budget, &Interrupt::NONE), (PacketStatus::Retained, PacketStatus::Retained));
        let packet = doc.metadata.exif.as_ref().unwrap();
        assert_eq!(packet.get(18..22), Some(b"AB\0\0".as_slice()));
        let before = (doc.metadata.xmp.clone(), doc.metadata.exif.clone(), budget);
        assert_eq!(translate(&mut doc, &tree, &mut budget, &Interrupt::NONE), (PacketStatus::Unavailable, PacketStatus::Unavailable));
        assert_eq!((doc.metadata.xmp, doc.metadata.exif, budget), before);
    }
    /// Reject oversized escaped XMP and malformed language labels instead of publishing partial RDF packets.
    #[test]
    fn oversized_escaped_xmp_and_invalid_language_never_create_partial_packets() {
        let value = "&lt;".repeat(MAX_PACKET / 2);
        let xml = format!("<metadata><tag name=\"Xmp.dc.title\">{value}</tag></metadata>");
        let tree = craft_xml::parse(xml.as_bytes()).unwrap();
        assert_eq!(xmp(&tree, &Interrupt::NONE), Err(PacketStatus::OverBudget));
        let invalid = craft_xml::parse(br#"<metadata><tag name="Xmp.dc.title">lang=&quot;../../bad&quot; Unsafe</tag></metadata>"#).unwrap();
        assert_eq!(xmp(&invalid, &Interrupt::NONE).unwrap(), None);
    }

    #[test]
    fn inline_exif_fields_fit_a_packet_exactly_at_the_cap() {
        let value = "A".repeat(MAX_PACKET - 39);
        let xml = format!("<metadata><tag name=\"Exif.Image.Artist\">B</tag><tag name=\"Exif.Image.ImageDescription\">{value}</tag></metadata>");
        let tree = craft_xml::parse(xml.as_bytes()).unwrap();
        let packet = exif(&tree, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(packet.len(), MAX_PACKET);
        assert!(super::super::metadata::has_exif_directory(&packet));
    }

    #[test]
    fn structure_and_malformed_language_stay_inert_while_valid_bags_are_written() {
        let tree = craft_xml::parse(
            br#"<metadata>
            <tag name="Xmp.photoshop.DocumentAncestors">uuid:example</tag>
            <tag name="Xmp.photoshop.SupplementalCategories">Portrait</tag>
            <tag name="Xmp.dc.title">lang=&quot;en--US&quot; Bad</tag>
            <tag name="Xmp.dc.description">lang=&quot;en-US&quot; Good</tag>
        </metadata>"#,
        )
        .unwrap();
        let packet = xmp(&tree, &Interrupt::NONE).unwrap().unwrap();
        assert!(packet.contains("SupplementalCategories") && packet.contains("Portrait"));
        assert!(packet.contains(r#"<rdf:li xml:lang="en-US">Good</rdf:li>"#));
        assert!(!packet.contains("DocumentAncestors") && !packet.contains("dc:title"));
    }

    #[test]
    fn typed_and_repeated_metadata_must_be_translatable_as_a_whole() {
        let tree = craft_xml::parse(
            br#"<metadata>
            <tag name="Xmp.xmp.Rating">9</tag>
            <tag name="Xmp.xmp.Label">Label</tag><tag name="Xmp.xmp.Label" encoding="base64">TGFiZWw=</tag>
            <tag name="Xmp.dc.creator">Author</tag><tag name="Xmp.dc.creator" encoding="base64">QQ==</tag>
            <tag name="Xmp.dc.title">lang=&quot;en-a&quot; Bad</tag>
            <tag name="Xmp.dc.description">lang=&quot;en-a-valid&quot; Good</tag>
            <tag name="Exif.Image.Artist">Ren&#233;</tag><tag name="Exif.Image.Artist">Alice</tag>
            <tag name="Exif.Image.Make">Maker</tag>
        </metadata>"#,
        )
        .unwrap();
        let packet = xmp(&tree, &Interrupt::NONE).unwrap().unwrap();
        assert!(packet.contains(r#"xml:lang="en-a-valid">Good"#));
        assert!(!["Rating", "Label", "dc:creator", "dc:title"].iter().any(|field| packet.contains(field)));
        let packet = exif(&tree, &Interrupt::NONE).unwrap().unwrap();
        assert!(packet.windows(5).any(|bytes| bytes == b"Maker"));
        assert!(!packet.windows(5).any(|bytes| bytes == b"Alice"));
    }

    #[test]
    fn xmp_ratings_preserve_valid_reals_and_refuse_out_of_range_values() {
        for value in ["-1", "0", "4", "4.0", "4.5", "5"] {
            let xml = format!(r#"<metadata><tag name="Xmp.xmp.Rating">{value}</tag></metadata>"#);
            let tree = craft_xml::parse(xml.as_bytes()).unwrap();
            assert!(xmp(&tree, &Interrupt::NONE).unwrap().unwrap().contains(&format!("<xmp:Rating>{value}</xmp:Rating>")));
        }
        for value in ["-0.5", "-1.5", "9", "NaN", "inf", "-inf"] {
            let xml = format!(r#"<metadata><tag name="Xmp.xmp.Rating">{value}</tag></metadata>"#);
            let tree = craft_xml::parse(xml.as_bytes()).unwrap();
            assert!(xmp(&tree, &Interrupt::NONE).unwrap().is_none(), "{value}");
        }
    }

    #[test]
    fn raw_exif_entry_data_must_be_inside_the_packet() {
        let tree = craft_xml::parse(br#"<metadata><tag name="Exif.Image.Artist">Photographer</tag></metadata>"#).unwrap();
        let mut packet = exif(&tree, &Interrupt::NONE).unwrap().unwrap();
        assert!(super::super::metadata::has_exif_directory(&packet));
        packet[18..22].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!super::super::metadata::has_exif_directory(&packet));
    }

    fn source_packet(name: &str, packet: &[u8]) -> (u32, Vec<u8>) {
        let mut writer = super::super::tests::W { out: Vec::new(), wide: true };
        writer.string(name);
        writer.u32(0);
        writer.u32(packet.len() as u32);
        writer.out.extend_from_slice(packet);
        (21, writer.out)
    }

    fn saved_image() -> super::super::tests::TestImage {
        let mut source = super::super::tests::TestImage::rgb8(1, 1);
        source.layers.push(super::super::tests::TestLayer::rgba("Saved", 1, 1, vec![17, 31, 57, 255]));
        source
    }

    #[test]
    fn public_import_maps_fractional_rating_and_keeps_raw_exif_ahead_of_xml() {
        let xml = br#"<metadata><tag name="Xmp.xmp.Rating">4.5</tag><tag name="Exif.Image.Artist">Synthesized</tag><tag name="Iptc.Application2.Caption">Caption</tag></metadata>"#;
        let raw = b"II\x2a\0\x08\0\0\0\0\0\0\0\0\0";
        let mut source = saved_image();
        source.image_props.extend([source_packet("gimp-image-metadata", xml), source_packet("exif-data", raw)]);
        let imported = super::super::import("packets", &source.write(), &Interrupt::NONE).unwrap();
        assert!(imported.document.metadata.xmp.as_ref().unwrap().contains("<xmp:Rating>4.5</xmp:Rating>"));
        assert_eq!(imported.document.metadata.exif.as_deref().unwrap().as_slice(), raw);
        assert!(imported.document.metadata.text.iter().any(|(key, value)| key == "Iptc.Application2.Caption" && value == "Caption"));
        let native = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        assert_eq!(reopened.metadata.xmp, imported.document.metadata.xmp);
        assert_eq!(reopened.metadata.exif, imported.document.metadata.exif);
        assert_eq!(reopened.metadata.text, imported.document.metadata.text);
        assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
    }

    #[test]
    fn packet_duplicates_are_selected_before_optional_xml_is_parsed() {
        let mut source = saved_image();
        // A duplicate alias must not cause even an invalid earlier XML packet to
        // be parsed, reported, or selected ahead of its final authoritative value.
        for _ in 0..64 {
            source.image_props.push(source_packet("gimp-image-metadata", b"<invalid"));
        }
        source.image_props.push(source_packet("gimp-image-metadata", br#"<metadata><tag name="Xmp.xmp.Label">Final</tag></metadata>"#));
        let imported = super::super::import("duplicates", &source.write(), &Interrupt::NONE).unwrap();
        assert!(imported.document.metadata.xmp.as_ref().unwrap().contains("<xmp:Label>Final</xmp:Label>"));
        assert!(!imported.warnings.iter().any(|warning| warning.contains("XML metadata is invalid")));
        // The final invalid value also cannot resurrect an earlier valid packet.
        source.image_props.push(source_packet("gimp-image-metadata", b"<invalid"));
        let imported = super::super::import("final invalid", &source.write(), &Interrupt::NONE).unwrap();
        assert!(imported.document.metadata.xmp.is_none());
        assert!(imported.warnings.iter().any(|warning| warning.contains("XML metadata is invalid")));
    }

    #[test]
    fn different_packet_kinds_keep_first_seen_preference_with_final_duplicate_values() {
        let first = br#"<metadata><tag name="Xmp.xmp.Label">First</tag></metadata>"#;
        let last = br#"<metadata><tag name="Xmp.xmp.Label">Final</tag></metadata>"#;
        let fallback = br#"<metadata><tag name="Xmp.xmp.Label">Fallback</tag></metadata>"#;
        for (reverse, expected) in [(false, "Final"), (true, "Fallback")] {
            let mut source = saved_image();
            let mut packets = vec![source_packet("gimp-image-metadata", first), source_packet("gimp-metadata", fallback)];
            if reverse {
                packets.reverse();
            }
            packets.push(source_packet("gimp-image-metadata", last));
            source.image_props.extend(packets);
            let imported = super::super::import("kind preference", &source.write(), &Interrupt::NONE).unwrap();
            assert!(imported.document.metadata.xmp.as_ref().unwrap().contains(&format!("<xmp:Label>{expected}</xmp:Label>")));
        }
    }

    #[test]
    fn invalid_or_bounded_optional_xml_preserves_saved_pixels() {
        let deep = format!("{}{}", "<metadata>".repeat(65), "</metadata>".repeat(65));
        for packet in [b"<metadata><tag>".as_slice(), b"<!DOCTYPE metadata [<!ENTITY x 'entity'>]><metadata>&x;</metadata>", deep.as_bytes()] {
            let mut source = saved_image();
            source.image_props.push(source_packet("gimp-image-metadata", packet));
            let imported = super::super::import("invalid metadata", &source.write(), &Interrupt::NONE).unwrap();
            assert!(imported.document.metadata.xmp.is_none());
            assert!(imported.document.metadata.exif.is_none());
            assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [17. / 255., 31. / 255., 57. / 255., 1.]);
            assert!(imported.warnings.iter().any(|warning| warning.contains("XML metadata is invalid")));
        }
    }

    #[test]
    fn metadata_xml_preflight_polls_cancellation_before_tree_construction() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 8;
        let xml = format!("<metadata>{}</metadata>", "<tag name=\"Xmp.xmp.Label\">value</tag>".repeat(30));
        assert!(matches!(parse_xml(xml.as_bytes(), craft_xcf::MAX_METADATA_BYTES, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert!(parse_xml(xml.as_bytes(), craft_xcf::MAX_METADATA_BYTES, &Interrupt::NONE).is_ok());
        assert!(parse_xml(xml.as_bytes(), 0, &Interrupt::NONE).is_err());
    }
}

/// Slots already admitted during this image's metadata traversal. Capacity is charged,
/// not just length, so short duplicate entries and spare vector slots cannot evade the cap.
#[derive(Default)]
struct TextStorage {
    charged_capacity: usize,
}

/// Copy into fallibly allocated staging storage without mutating the admitted document.
fn staged_text(value: &str) -> Option<String> {
    let mut text = String::new();
    text.try_reserve_exact(value.len()).ok()?;
    text.push_str(value);
    Some(text)
}

/// Admit all newly retained string and pair-vector storage before committing an entry.
/// Duplicates remain separate: XMP bags, sequences and language alternatives need them.
fn push_text(doc: &mut Document, key: &str, value: &str, storage: &mut TextStorage, budget: &mut usize) -> bool {
    if key.len() > MAX_PACKET || value.len() > MAX_PACKET {
        return false;
    }
    let entries = &mut doc.metadata.text;
    let Some(length) = entries.len().checked_add(1) else { return false };
    // Geometric growth keeps repeated short entries linear in total copying work.
    // If the next complete buffer cannot fit, omit the optional entry rather
    // than retrying exact +1 growth and copying the whole prefix on every tag.
    let capacity = if length > entries.capacity() {
        if entries.capacity() == 0 {
            1
        } else {
            let Some(grown) = entries.capacity().checked_mul(2) else { return false };
            grown.max(length)
        }
    } else {
        entries.capacity()
    };
    let slot_bytes = std::mem::size_of::<(String, String)>();
    let added_slots = capacity.saturating_sub(storage.charged_capacity);
    let Some(minimum) = added_slots.checked_mul(slot_bytes).and_then(|bytes| bytes.checked_add(key.len())).and_then(|bytes| bytes.checked_add(value.len()))
    else {
        return false;
    };
    // Refuse before allocating even temporary copies. A previously allocated but
    // uncharged vector still spends its complete retained capacity on first use.
    if minimum > *budget {
        return false;
    }
    let (Some(key), Some(value)) = (staged_text(key), staged_text(value)) else { return false };
    let mut replacement = if length > entries.capacity() {
        let mut next = Vec::new();
        if next.try_reserve_exact(capacity).is_err() {
            return false;
        }
        Some(next)
    } else {
        None
    };
    let capacity = replacement.as_ref().map_or(entries.capacity(), Vec::capacity);
    // An allocator may grant extra capacity. Account for its actual retained
    // buffers before swapping anything; refusal drops all staging and needs no refund.
    let Some(cost) = capacity
        .saturating_sub(storage.charged_capacity)
        .checked_mul(slot_bytes)
        .and_then(|bytes| bytes.checked_add(key.capacity()))
        .and_then(|bytes| bytes.checked_add(value.capacity()))
    else {
        return false;
    };
    let Some(remaining) = budget.checked_sub(cost) else { return false };
    if let Some(next) = &mut replacement {
        next.append(entries);
        next.push((key, value));
    } else {
        entries.push((key, value));
    }
    if let Some(next) = replacement {
        *entries = next;
    }
    storage.charged_capacity = capacity;
    *budget = remaining;
    true
}
