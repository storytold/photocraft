//! The free-form data a tool stores with a document (`document.userData`): JSON values under keys,
//! kept as one JSON object in the document's XMP packet, the `UserData` property of
//! [`NAMESPACE`]. Files that keep the layers (PSD, PSB, layered TIFF, `.pcraft`) carry it; flat
//! exports leave it out ([`strip`]), since it describes the layered document and can outgrow
//! the room a format has for XMP.
//!
//! PhotoCraft writes the property as an element under the `photocraft` prefix. An XMP toolkit
//! that saves the packet again may pick another prefix or write it as an attribute; reading and
//! stripping follow the prefixes the packet binds to the namespace, and decode character
//! references in both forms.

use std::borrow::Cow;

use serde_json::{Map, Value};

/// The XMP namespace of PhotoCraft's own properties. It is written into users' files: never
/// change it.
pub const NAMESPACE: &str = "https://github.com/storytold/photocraft/ns/1.0/";

/// The prefix PhotoCraft binds to [`NAMESPACE`].
const PREFIX: &str = "photocraft";

/// The property's local name.
const LOCAL: &str = "UserData";

const EMPTY_PACKET: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";

/// Why user data can't be read or stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserDataError {
    /// The property is there but can't be read: the reason.
    Unreadable(String),
    /// The data holds a character XML can't carry.
    Character(char),
    /// The packet is too damaged to hold the property.
    Damaged,
}

impl std::fmt::Display for UserDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the document's user data can't be read: {why}"),
            Self::Character(c) => write!(f, "U+{:04X} is a character XML can't carry", *c as u32),
            Self::Damaged => f.write_str("the document's XMP metadata is damaged, so user data can't be stored in it"),
        }
    }
}

impl std::error::Error for UserDataError {}

/// Whether XML can carry `c` as text: XML 1.0's characters, less the Unicode non-characters
/// (U+FDD0 to U+FDEF and every U+xxFFFE / U+xxFFFF), which strict readers refuse.
fn allowed(c: char) -> bool {
    let u = c as u32;
    let xml = matches!(u, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF);
    xml && !(0xFDD0..=0xFDEF).contains(&u) && u & 0xFFFE != 0xFFFE
}

/// The prefixes `xmp` binds to [`NAMESPACE`], and `photocraft` in any case.
fn prefixes(xmp: &str) -> Vec<&str> {
    let mut out = vec![PREFIX];
    let mut rest = xmp;
    while let Some(i) = rest.find("xmlns:") {
        rest = rest.get(i + "xmlns:".len()..).unwrap_or_default();
        let Some(eq) = rest.find('=') else { break };
        let name = rest.get(..eq).unwrap_or_default().trim();
        let value = rest.get(eq + 1..).unwrap_or_default().trim_start();
        let Some(quote) = value.chars().next().filter(|q| *q == '"' || *q == '\'') else { continue };
        let uri = value.get(1..).and_then(|v| v.split(quote).next()).unwrap_or_default();
        if uri == NAMESPACE && !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Where the property sits in `xmp` under `prefix`.
enum Found {
    /// `<prefix:UserData …>inner</prefix:UserData>` (or self-closing) at `start..end`.
    Element { start: usize, end: usize, inner: (usize, usize) },
    /// ` prefix:UserData="value"` at `start..end` (the leading space included).
    Attribute { start: usize, end: usize, value: (usize, usize) },
}

/// The first element or attribute named `prefix:UserData` in `xmp` at or after `from`.
fn find(xmp: &str, prefix: &str, from: usize) -> Option<Found> {
    let name = format!("{prefix}:{LOCAL}");
    let mut at = from;
    while let Some(i) = xmp.get(at..).and_then(|r| r.find(&name)).map(|i| i + at) {
        let before = xmp.get(..i).and_then(|b| b.chars().next_back());
        let after = xmp.get(i + name.len()..).unwrap_or_default();
        at = i + name.len();
        if before == Some('<') && after.starts_with(['>', '/', ' ', '\t', '\r', '\n']) {
            let start = i - 1;
            let close = after.find('>')?;
            let open_end = i + name.len() + close + 1;
            if after.get(..close).is_some_and(|t| t.trim_end().ends_with('/')) {
                return Some(Found::Element { start, end: open_end, inner: (open_end, open_end) });
            }
            let end_tag = format!("</{name}>");
            let inner_end = xmp.get(open_end..)?.find(&end_tag)? + open_end;
            return Some(Found::Element { start, end: inner_end + end_tag.len(), inner: (open_end, inner_end) });
        }
        if let Some(space) = before.filter(|c| c.is_whitespace())
            && after.trim_start().starts_with('=')
        {
            let value = after.trim_start().get(1..)?.trim_start();
            let quote = value.chars().next().filter(|q| *q == '"' || *q == '\'')?;
            let v0 = xmp.len() - value.len() + 1;
            let v1 = xmp.get(v0..)?.find(quote)? + v0;
            return Some(Found::Attribute { start: i - space.len_utf8(), end: v1 + 1, value: (v0, v1) });
        }
    }
    None
}

/// Every occurrence of the property, under any of the packet's prefixes, as `(start, end)`
/// spans with the prefix of each element, in packet order and never overlapping.
fn occurrences(xmp: &str) -> Vec<(usize, usize, Option<String>)> {
    let mut out = Vec::new();
    for prefix in prefixes(xmp) {
        let mut from = 0;
        while let Some(found) = find(xmp, prefix, from) {
            let (start, end, element) = match found {
                Found::Element { start, end, .. } => (start, end, Some(prefix.to_string())),
                Found::Attribute { start, end, .. } => (start, end, None),
            };
            out.push((start, end, element));
            from = end;
        }
    }
    out.sort_by_key(|(start, _, _)| *start);
    let mut end = 0;
    out.retain(|(a, b, _)| {
        let keep = *a >= end;
        if keep {
            end = *b;
        }
        keep
    });
    out
}

/// The text of `raw` with its entity and character references (`&amp;`, `&#34;`, `&#x22;`)
/// decoded. An unknown entity, or a reference to a character XML can't carry, is an error.
fn decode(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i + 1..).unwrap_or_default();
        let end = after.find(';').filter(|e| *e <= 10).ok_or("it has an unterminated character reference")?;
        let name = after.get(..end).unwrap_or_default();
        let c = match name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => name.strip_prefix('#').and_then(|dec| dec.parse::<u32>().ok()),
                };
                code.and_then(char::from_u32).filter(|c| allowed(*c)).ok_or_else(|| format!("it has an invalid reference `&{name};`"))?
            }
        };
        out.push(c);
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// `text` escaped for element content.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The user data in `xmp`: empty when the packet has none. A property that is there but can't be
/// read (not a JSON object, a bad character reference) is an error saying why, so a caller never
/// overwrites data it couldn't read.
pub fn read(xmp: Option<&str>) -> Result<Map<String, Value>, UserDataError> {
    let Some(x) = xmp else { return Ok(Map::new()) };
    let raw = prefixes(x).into_iter().filter_map(|p| find(x, p, 0)).min_by_key(|f| match f {
        Found::Element { start, .. } | Found::Attribute { start, .. } => *start,
    });
    let (a, b) = match raw {
        None => return Ok(Map::new()),
        Some(Found::Element { inner, .. }) => inner,
        Some(Found::Attribute { value, .. }) => value,
    };
    let text = decode(x.get(a..b).unwrap_or_default().trim()).map_err(UserDataError::Unreadable)?;
    match serde_json::from_str(&text) {
        Ok(Value::Object(data)) => Ok(data),
        Ok(_) => Err(UserDataError::Unreadable("it is not a JSON object".into())),
        Err(e) => Err(UserDataError::Unreadable(format!("it is not valid JSON ({e})"))),
    }
}

/// Bytes `value` takes in the packet: its JSON text, escaped as it is stored.
pub fn stored_len(value: &Value) -> usize {
    escape(&value.to_string()).len()
}

/// `xmp` with its user data replaced by `data` and every other property kept verbatim (a packet
/// is created when there is none); empty `data` removes the property. It is an error when `data`
/// holds a character XML can't carry (U+FFFE, U+FFFF and the other non-characters), which would
/// make strict readers drop the whole packet, or when the packet is too damaged to hold it.
pub fn write(xmp: Option<&str>, data: &Map<String, Value>) -> Result<String, UserDataError> {
    let json = Value::Object(data.clone()).to_string();
    if let Some(c) = json.chars().find(|c| !allowed(*c)) {
        return Err(UserDataError::Character(c));
    }
    // A new packet only where there is none: a damaged one is refused, not replaced, which
    // would lose what is left of it (the copyright, say).
    let mut x = match xmp.filter(|s| !s.trim().is_empty()) {
        None => EMPTY_PACKET.to_string(),
        Some(s) if s.contains("</rdf:RDF>") => s.to_string(),
        Some(_) => return Err(UserDataError::Damaged),
    };
    let spans = occurrences(&x);
    // Rewritten where the first element was (under its own prefix, which is bound there), so
    // repeated edits don't pile up `rdf:Description`s: its start once the spans before it are gone.
    let slot = spans.iter().position(|(_, _, element)| element.is_some()).and_then(|k| {
        let removed: usize = spans.iter().take(k).map(|(a, b, _)| b - a).sum();
        let (start, _, prefix) = spans.get(k)?;
        Some((start - removed, prefix.clone()?))
    });
    for (start, end, _) in spans.into_iter().rev() {
        x.replace_range(start..end, "");
    }
    if !data.is_empty() {
        match slot {
            Some((at, prefix)) => x.insert_str(at, &format!("<{prefix}:{LOCAL}>{}</{prefix}:{LOCAL}>", escape(&json))),
            None => {
                let at = x.rfind("</rdf:RDF>").unwrap_or(x.len());
                let element = format!("<{PREFIX}:{LOCAL}>{}</{PREFIX}:{LOCAL}>", escape(&json));
                x.insert_str(at, &format!("  <rdf:Description rdf:about=\"\" xmlns:{PREFIX}=\"{NAMESPACE}\">\n   {element}\n  </rdf:Description>\n "));
            }
        }
    }
    match read(Some(&x)) {
        Ok(back) if back == *data => Ok(x),
        _ => Err(UserDataError::Damaged),
    }
}

/// `xmp` without its user data, for a file that doesn't keep the layers.
pub fn strip(xmp: &str) -> Cow<'_, str> {
    let spans = occurrences(xmp);
    if spans.is_empty() {
        return Cow::Borrowed(xmp);
    }
    let mut x = xmp.to_string();
    for (start, end, _) in spans.into_iter().rev() {
        x.replace_range(start..end, "");
    }
    Cow::Owned(x)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn data(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    const COPYRIGHT: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:rights><rdf:Alt><rdf:li xml:lang="x-default">Studio</rdf:li></rdf:Alt></dc:rights></rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn write_and_read_back_keeping_other_properties() {
        let d = data(json!({"pipeline": {"loaded": [{"layer": 3, "note": "a < b & \"c\""}]}, "n": 1}));
        let x = write(Some(COPYRIGHT), &d).unwrap();
        assert_eq!(read(Some(&x)).unwrap(), d);
        assert!(x.contains("<rdf:li xml:lang=\"x-default\">Studio</rdf:li>"), "{x}");
        assert!(x.contains(&format!("xmlns:photocraft=\"{NAMESPACE}\"")));
        // Rewritten in place, not piled up; empty removes it.
        let x2 = write(Some(&x), &data(json!({"n": 2}))).unwrap();
        assert_eq!(x2.matches("UserData>").count(), 2, "one element: {x2}");
        assert_eq!(x2.matches("<rdf:Description").count(), 2);
        let cleared = write(Some(&x2), &Map::new()).unwrap();
        assert!(read(Some(&cleared)).unwrap().is_empty() && !cleared.contains("UserData"));
        assert!(cleared.contains("Studio"));
        // No packet: one is created.
        assert_eq!(read(Some(&write(None, &d).unwrap())).unwrap(), d);
        assert!(read(None).unwrap().is_empty() && read(Some(COPYRIGHT)).unwrap().is_empty());
    }

    /// Another toolkit's spelling: its own prefix, the attribute form and numeric references.
    #[test]
    fn reads_other_prefixes_attributes_and_character_references() {
        let attr = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:ns1="{NAMESPACE}" ns1:UserData="{{&#34;a&#x22;:[1,&quot;&#xe9;&#233;&quot;]}}"/></rdf:RDF></x:xmpmeta>"#
        );
        assert_eq!(read(Some(&attr)).unwrap(), data(json!({"a": [1, "éé"]})));
        // A set rewrites it as an element; the old attribute is gone and the packet stays valid.
        let x = write(Some(&attr), &data(json!({"b": true}))).unwrap();
        assert!(!x.contains("ns1:UserData="), "{x}");
        assert_eq!(read(Some(&x)).unwrap(), data(json!({"b": true})));
        let elem = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pc='{NAMESPACE}'><pc:UserData>{{"k":"&#60;tag&#62;"}}</pc:UserData></rdf:Description></rdf:RDF></x:xmpmeta>"#
        );
        assert_eq!(read(Some(&elem)).unwrap(), data(json!({"k": "<tag>"})));
        // Rewritten under the prefix bound where it was.
        let x = write(Some(&elem), &data(json!({"k": 2}))).unwrap();
        assert!(x.contains("<pc:UserData>{\"k\":2}</pc:UserData>"), "{x}");
        // An unrelated namespace's `UserData` is not ours.
        let other = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:acme="urn:acme"><acme:UserData>keep</acme:UserData></rdf:Description></rdf:RDF>"#;
        assert!(read(Some(other)).unwrap().is_empty());
        assert_eq!(strip(other), other);
    }

    /// A property that is there but unreadable is an error, never an empty map a set would
    /// silently replace.
    #[test]
    fn unreadable_data_is_an_error() {
        for inner in ["not json", "[1, 2]", "{\"a\": &bogus;}", "{\"a\": \"&#xFFFE;\"}", "{\"a\": \"&#55296;\"}", "{\"a\": 1 &amp"] {
            let x = format!(
                r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:photocraft="{NAMESPACE}"><photocraft:UserData>{inner}</photocraft:UserData></rdf:Description></rdf:RDF>"#
            );
            assert!(read(Some(&x)).is_err(), "{inner}");
        }
        // A damaged packet can't hold it, and is not replaced by a new one either.
        let unclosed = format!(r#"<rdf:RDF><rdf:Description xmlns:photocraft="{NAMESPACE}"><photocraft:UserData>{{}}"#);
        let open_element = format!(r#"<rdf:RDF><rdf:Description xmlns:photocraft="{NAMESPACE}"><photocraft:UserData>{{}}</rdf:Description></rdf:RDF>"#);
        for damaged in [unclosed.as_str(), open_element.as_str(), "<x:xmpmeta>garbage"] {
            assert_eq!(write(Some(damaged), &data(json!({"a": 1}))), Err(UserDataError::Damaged), "{damaged}");
        }
        assert!(write(Some("  "), &data(json!({"a": 1}))).is_ok(), "blank is no packet");
    }

    /// Characters XML can't carry are refused before they reach the packet.
    #[test]
    fn non_characters_are_refused() {
        for bad in ["\u{FFFE}", "\u{FFFF}", "\u{FDD0}", "\u{FDEF}", "\u{1FFFE}", "\u{10FFFF}"] {
            let e = write(Some(COPYRIGHT), &data(json!({ "k": format!("a{bad}b") }))).unwrap_err();
            assert_eq!(e, UserDataError::Character(bad.chars().next().unwrap()), "{bad:?}");
            assert!(e.to_string().contains("XML can't carry"), "{e}");
            assert!(write(None, &data(json!({ bad: 1 }))).is_err(), "in a key: {bad:?}");
        }
        // Control characters are escaped by JSON, and ordinary non-ASCII is fine.
        let ok = data(json!({"k": "tab\t nul\u{0} é 漢 😀 \u{FFFD}"}));
        assert_eq!(read(Some(&write(None, &ok).unwrap())).unwrap(), ok);
    }

    /// Regressions: an attribute form before the element (the rewrite slot must account for it),
    /// and a multi-byte space before an attribute (a byte offset inside a character would panic).
    #[test]
    fn several_forms_and_unusual_spacing_rewrite_cleanly() {
        let x = format!(
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:a=\"{NAMESPACE}\"\u{a0}a:UserData=\"{{&quot;old&quot;:1}}\"/><rdf:Description xmlns:photocraft=\"{NAMESPACE}\"><photocraft:UserData>{{\"old\":2}}</photocraft:UserData></rdf:Description></rdf:RDF>"
        );
        assert_eq!(read(Some(&x)).unwrap(), data(json!({"old": 1})), "the first one in the packet");
        let w = write(Some(&x), &data(json!({"new": true}))).unwrap();
        assert_eq!(w.matches("UserData").count(), 2, "one element left: {w}");
        assert!(w.contains("<rdf:Description xmlns:photocraft=\"https://github.com/storytold/photocraft/ns/1.0/\"><photocraft:UserData>{\"new\":true}</photocraft:UserData></rdf:Description>"), "{w}");
        assert!(!strip(&x).contains("UserData"));
    }

    #[test]
    fn stored_len_counts_the_escaped_text() {
        let v = json!({"k": "<<<&&&>>>"});
        assert_eq!(stored_len(&v), v.to_string().len() + 3 * 3 + 3 * 4 + 3 * 3);
    }

    #[test]
    fn strip_removes_every_form_and_keeps_the_rest() {
        let x = write(Some(COPYRIGHT), &data(json!({"big": "x".repeat(100)}))).unwrap();
        let s = strip(&x);
        assert!(!s.contains("UserData") && s.contains("Studio"), "{s}");
        assert!(read(Some(&s)).unwrap().is_empty());
        let attr = format!(r#"<rdf:Description xmlns:q="{NAMESPACE}" q:UserData='{{}}' xmlns:dc="http://purl.org/dc/elements/1.1/"/>"#);
        assert_eq!(strip(&attr), format!(r#"<rdf:Description xmlns:q="{NAMESPACE}" xmlns:dc="http://purl.org/dc/elements/1.1/"/>"#));
        assert!(matches!(strip(COPYRIGHT), Cow::Borrowed(_)));
    }
}
