//! Repair data-URL failures that usvg drops before invoking its image resolver.
//! This changes only attribute values. Resolver callbacks still decide whether an
//! image is used, so hidden/unused resources do not acquire omission warnings.

use std::borrow::Cow;
use std::ops::Range;

use crate::IoError;
use craft_svg::tree::Limits;

#[cfg(test)]
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const INVALID_IMAGE: &str = "data:image/png,%00";

fn invalid(message: &str) -> IoError {
    IoError::Svg(message.into())
}

/// Validate the original decoded XML before patching it. The caller must charge
/// its original bytes/work before any shorter result replaces an embedded source.
/// This does not rewrite an XML encoding declaration for a UTF-8 byte transport.
pub(super) fn repair<'a>(text: &'a str, limits: &Limits) -> Result<Cow<'a, str>, IoError> {
    if !text.contains("href") && !text.starts_with("<?xml") {
        return Ok(Cow::Borrowed(text));
    }
    let xml_limits = craft_svg::preflight::Limits {
        bytes: limits.bytes,
        depth: limits.depth,
        entity_bytes: limits.entity_bytes,
        refuse_entity_markup: limits.refuse_entity_markup,
        attributes: limits.attributes,
        attribute_work: limits.attribute_work,
        xml_nodes: limits.xml_nodes,
    };
    let xml = craft_svg::preflight::parse_text(text, &xml_limits).map_err(|error| match error {
        craft_svg::preflight::Error::TooDeep(_) => invalid("layers are nested too deeply"),
        error => IoError::Svg(error.to_string()),
    })?;
    if craft_svg::tree::height(&xml, limits.tree_depth).map_err(super::svg_parse_error)?.is_none() {
        return Err(invalid("layers are nested too deeply"));
    }
    // The expanded attribute values fit source + entity work. Streaming URL
    // validation reads them and emits at most that much decoded data, never a Vec.
    let mut work =
        text.len().checked_add(limits.entity_bytes).and_then(|n| n.checked_mul(2)).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let max_patches = (limits.bytes / std::mem::size_of::<Range<usize>>()).min(usize::try_from(limits.xml_nodes).unwrap_or(usize::MAX));
    let mut patches: Vec<Range<usize>> = Vec::new();
    for node in xml.descendants().filter(|node| node.is_element()) {
        if !matches!(node.tag_name().name(), "image" | "feImage") {
            continue;
        }
        // Shared preflight inspects local image names in every namespace and
        // checks XLink before plain href. Normalize both representations, even
        // when one is inert or unselected. Only actual resolver callbacks count
        // omissions, so this does not turn hidden/unused resources into warnings.
        for href in node.attributes().filter(|a| a.name() == "href" && matches!(a.namespace(), None | Some(XLINK_NS))) {
            if !malformed_data_url(href.value(), &mut work)? {
                continue;
            }
            let range = verified_value_range(text, node.range(), href.range(), href.range_qname(), href.range_value(), href.name())?;
            if patches.len() == patches.capacity() {
                if patches.len() >= max_patches {
                    return Err(invalid("SVG data-URL repair metadata exceeds its limit"));
                }
                let capacity = patches.capacity().max(8).saturating_mul(2).min(max_patches);
                patches.try_reserve_exact(capacity.saturating_sub(patches.len())).map_err(|_| invalid("could not allocate bounded SVG data-URL repairs"))?;
            }
            patches.push(range);
        }
    }
    if patches.is_empty() {
        return Ok(Cow::Borrowed(text));
    }
    patches.sort_unstable_by_key(|range| (range.start, range.end));
    patches.dedup();
    let mut end = 0;
    let mut length = text.len();
    for range in &patches {
        if range.start < end {
            return Err(invalid("SVG data-URL source ranges overlap"));
        }
        end = range.end;
        length =
            length.checked_sub(range.len()).and_then(|n| n.checked_add(INVALID_IMAGE.len())).ok_or_else(|| invalid("SVG data-URL repair size overflows"))?;
    }
    if length > limits.bytes {
        return Err(invalid("repaired SVG exceeds the text byte limit"));
    }
    let mut result = String::new();
    result.try_reserve_exact(length).map_err(|_| invalid("could not allocate bounded repaired SVG text"))?;
    let mut cursor = 0;
    for range in patches {
        result.push_str(text.get(cursor..range.start).ok_or_else(|| invalid("invalid SVG data-URL source range"))?);
        result.push_str(INVALID_IMAGE);
        cursor = range.end;
    }
    result.push_str(text.get(cursor..).ok_or_else(|| invalid("invalid SVG data-URL source range"))?);
    Ok(Cow::Owned(result))
}

fn malformed_data_url(value: &str, work: &mut usize) -> Result<bool, IoError> {
    *work = work.checked_sub(value.len()).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let url = match data_url::DataUrl::process(value) {
        Ok(url) => url,
        Err(data_url::DataUrlError::NoComma) => return Ok(true),
        Err(data_url::DataUrlError::NotADataUrl) => return Ok(false),
    };
    let mut exhausted = false;
    let decoded = url.decode(|bytes| {
        if let Some(left) = work.checked_sub(bytes.len()) {
            *work = left;
            Ok(())
        } else {
            exhausted = true;
            Err(())
        }
    });
    if exhausted {
        return Err(invalid("SVG data-URL validation work exceeds its limit"));
    }
    Ok(decoded.is_err())
}

/// roxmltree bounds QName and equals-spacing bookkeeping internally. Refuse a
/// projection that does not identify the attribute's actual quoted value, rather
/// than editing a truncated position or a DTD entity declaration by mistake.
fn verified_value_range(
    text: &str,
    owner: Range<usize>,
    attribute: Range<usize>,
    qname: Range<usize>,
    value: Range<usize>,
    local_name: &str,
) -> Result<Range<usize>, IoError> {
    let error = || invalid("SVG data-URL attribute source positions cannot be represented safely");
    if attribute.start < owner.start || attribute.end > owner.end || qname.start != attribute.start || qname.end > value.start {
        return Err(error());
    }
    let open = value.start.checked_sub(1).ok_or_else(error)?;
    if value.end.checked_add(1) != Some(attribute.end) || open < qname.end {
        return Err(error());
    }
    let name = text.get(qname.clone()).ok_or_else(error)?;
    if name.rsplit(':').next() != Some(local_name) {
        return Err(error());
    }
    let equals = text.get(qname.end..open).ok_or_else(error)?;
    if equals.trim_matches([' ', '\t', '\r', '\n']) != "=" {
        return Err(error());
    }
    let quote = text.get(open..value.start).ok_or_else(error)?;
    if !matches!(quote, "'" | "\"") || text.get(value.end..attribute.end) != Some(quote) || text.get(value.clone()).is_none() {
        return Err(error());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits { bytes: 1 << 20, depth: 32, tree_depth: 32, entity_bytes: 65_536, ..Default::default() }
    }

    #[test]
    fn malformed_image_urls_are_repaired_in_both_svg_namespaces_and_filter_resources() {
        for tag in ["image", "feImage"] {
            for text in [
                format!("<svg xmlns='{SVG_NS}'><{tag} href='data:image/png;base64,A'/></svg>"),
                format!("<svg><{tag} href='data:image/png;base64,A'/></svg>"),
                format!("<s:svg xmlns:s='{SVG_NS}' xmlns:link='{XLINK_NS}'><s:{tag} link:href='data:image/png;base64,A'/></s:svg>"),
            ] {
                let repaired = repair(&text, &limits()).unwrap();
                assert!(matches!(repaired, Cow::Owned(_)));
                assert_eq!(repaired.matches(INVALID_IMAGE).count(), 1);
            }
        }
    }

    #[test]
    fn unselected_href_and_foreign_local_image_names_are_normalized_for_shared_preflight() {
        for attrs in ["href='data:image/png,%00' x:href='data:image/png;base64,A'", "x:href='data:image/png;base64,A' href='data:image/png,%00'"] {
            let text = format!("<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}'><image {attrs}/></svg>");
            let repaired = repair(&text, &limits()).unwrap();
            assert!(matches!(repaired, Cow::Owned(_)));
            assert_eq!(repaired.matches(INVALID_IMAGE).count(), 2);
            assert!(!repaired.contains("base64,A"));
        }
        let text = format!(
            "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' xmlns:f='urn:foreign'><image x:href='data:image/png,%00' href='data:image/png;base64,A'/><f:image href='data:image/png;base64,A'/><image f:href='data:image/png;base64,A'/></svg>"
        );
        let repaired = repair(&text, &limits()).unwrap();
        assert_eq!(repaired.matches(INVALID_IMAGE).count(), 3);
        assert!(repaired.contains("f:href='data:image/png;base64,A'"), "foreign href attributes remain inert and unchanged");
    }

    #[test]
    fn both_href_forms_on_hidden_unused_and_foreign_resources_do_not_preflight_as_omissions() {
        for tag in ["image", "feImage"] {
            let text = format!(
                "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' xmlns:f='urn:foreign'><defs><{tag} id='unused' href='data:image/png;base64,A' x:href='data:image/png;base64,A'/></defs><{tag} display='none' x:href='data:image/png;base64,A' href='data:image/png;base64,A'/><f:{tag} href='data:image/png;base64,A' x:href='data:image/png;base64,A'/></svg>"
            );
            let original = text.clone();
            let repaired = repair(&text, &limits()).unwrap();
            assert_eq!(text, original, "repair never mutates the original source");
            assert_eq!(repaired.matches(INVALID_IMAGE).count(), 6);
            let resolver = craft_svg::tree::ImageResolver::new(limits());
            resolver.check_document(&repaired).unwrap();
            let usage = resolver.usage();
            assert_eq!(
                (usage.invalid_raster_images, usage.invalid_svg_images),
                (0, 0),
                "shared whole-document scanning cannot invent omissions before resolution"
            );
            assert!(matches!(repair(&repaired, &limits()).unwrap(), Cow::Borrowed(_)), "normalization is idempotent");
        }
    }

    #[test]
    fn entity_expanded_urls_patch_the_owning_attributes_without_rewriting_the_doctype() {
        let text = format!(
            "<!DOCTYPE svg [<!ENTITY bad 'data:image/png;base64,A'>]><svg xmlns='{SVG_NS}'><image href='&bad;'/><image href='data:image/png;base64,&#65;'/></svg>"
        );
        let repaired = repair(&text, &limits()).unwrap();
        assert!(repaired.contains("<!ENTITY bad 'data:image/png;base64,A'>"));
        assert_eq!(repaired.matches(INVALID_IMAGE).count(), 2);
        assert!(!repaired.contains("href='&bad;'"));
        assert!(matches!(repair(&repaired, &limits()).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn valid_data_and_external_urls_are_not_rewritten() {
        for value in ["data:image/png;base64,AA==", "data:image/svg+xml,%3Csvg%2F%3E", "file.png", "#local"] {
            let text = format!("<svg><image href='{value}'/></svg>");
            assert!(matches!(repair(&text, &limits()).unwrap(), Cow::Borrowed(_)));
        }
        let text = "<svg><image href='data:image/png;base64'/></svg>";
        assert!(repair(text, &limits()).unwrap().contains(INVALID_IMAGE));
    }

    #[test]
    fn unrepresentable_attribute_positions_and_output_growth_fail_safely() {
        let text = format!("<svg><image href{}='data:image/png;base64,A'/></svg>", " ".repeat(300));
        assert!(matches!(repair(&text, &limits()), Err(IoError::Svg(message)) if message.contains("source positions")));
        let text = "<svg><image href='data:;base64,A'/></svg>";
        assert!(matches!(repair(text, &Limits { bytes: text.len(), ..limits() }), Err(IoError::Svg(message)) if message.contains("byte limit")));
    }
}
