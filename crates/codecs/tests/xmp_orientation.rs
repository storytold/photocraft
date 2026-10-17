//! Rewriting orientation must not corrupt unrelated XMP fields or literal text.

use std::borrow::Cow;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, decode, encode, upright_xmp};

const TIFF_NS: &str = "http://ns.adobe.com/tiff/1.0/";

#[test]
fn xmp_orientation_preserves_similarly_named_properties_and_literal_text() {
    let xmp = format!(
        r#"<?xpacket begin=""?><x xmlns:tiff="{TIFF_NS}" xmlns:notiff="urn:custom"
 notiff:Orientation="6" tiff:OrientationNote='7' Orientation="8"
 note="tiff:Orientation='6'" tiff:Orientation = '8'>
<!-- <tiff:Orientation>3</tiff:Orientation> tiff:Orientation="4" -->
<![CDATA[<tiff:Orientation>5</tiff:Orientation> tiff:Orientation="6"]]>
<description>tiff:Orientation="7" and &lt;tiff:Orientation&gt;8&lt;/tiff:Orientation&gt;</description>
<notiff:Orientation>6</notiff:Orientation><tiff:OrientationNote>7</tiff:OrientationNote>
<tiff:Orientation>6</tiff:Orientation>
</x><?xpacket end="w"?>"#
    );
    let expected = xmp
        .replace("tiff:Orientation = '8'", "tiff:Orientation = '1'")
        .replace("<tiff:Orientation>6</tiff:Orientation>", "<tiff:Orientation>1</tiff:Orientation>");
    assert_eq!(upright_xmp(&xmp), expected);
}

#[test]
fn xmp_orientation_resolves_namespace_aliases_and_scopes() {
    let xmp = format!(
        r#"<x xmlns:photo="{TIFF_NS}" xmlns:tiff="urn:custom" photo:Orientation="6" tiff:Orientation="7">
<photo:Orientation>8</photo:Orientation>
<x xmlns:photo="urn:other" photo:Orientation="6"><photo:Orientation>7</photo:Orientation></x>
<x xmlns:tiff="{TIFF_NS}" tiff:Orientation='3'/>
<Orientation xmlns="{TIFF_NS}" Orientation="8">4</Orientation>
<Orientation>5</Orientation>
</x>"#
    );
    let expected = xmp
        .replacen("photo:Orientation=\"6\"", "photo:Orientation=\"1\"", 1)
        .replace("<photo:Orientation>8</photo:Orientation>", "<photo:Orientation>1</photo:Orientation>")
        .replace("tiff:Orientation='3'", "tiff:Orientation='1'")
        .replace("Orientation=\"8\">4</Orientation>", "Orientation=\"8\">1</Orientation>");
    assert_eq!(upright_xmp(&xmp), expected);
}

#[test]
fn xmp_orientation_keeps_quotes_whitespace_and_other_values() {
    for quote in ['\'', '"'] {
        let xmp = format!("<x xmlns:tiff='{TIFF_NS}' tiff:Orientation \n=\t{quote} 06 \r\n{quote}><tiff:Orientation>\n 8 \t</tiff:Orientation></x>");
        let expected = xmp.replace(" 06 ", " 1 ").replace(" 8 ", " 1 ");
        assert_eq!(upright_xmp(&xmp), expected);
        assert!(matches!(upright_xmp(&expected), Cow::Borrowed(_)));
    }
    // XML character references in an attribute are still an orientation value.
    let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="&#54;"/>"#);
    assert_eq!(upright_xmp(&xmp), xmp.replace("&#54;", "1"));
    for value in ["", "1", " 1 ", "6 degrees", "8suffix"] {
        let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="{value}"><tiff:Orientation>{value}</tiff:Orientation></x>"#);
        assert!(matches!(upright_xmp(&xmp), Cow::Borrowed(_)), "{xmp}");
    }
}

#[test]
fn xmp_orientation_preserves_malformed_packets_without_partial_edits() {
    let missing_close = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6"><unclosed>"#);
    let dtd = format!(r#"<!DOCTYPE x [<!ENTITY o "6">]><x xmlns:tiff="{TIFF_NS}" tiff:Orientation="&o;"/>"#);
    let empty_dtd = format!(r#"<!DOCTYPE x><x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6"/>"#);
    for xmp in [
        "tiff:Orientation='12'",
        r#"<rdf:Description tiff:Orientation="6"/>"#,
        "<tiff:Orientation>8</tiff:Orientation><tiff:Orientation>3</tiff:Orientation>",
        missing_close.as_str(),
        dtd.as_str(),
        empty_dtd.as_str(),
    ] {
        assert!(matches!(upright_xmp(xmp), Cow::Borrowed(_)), "{xmp}");
    }
}

#[test]
fn xmp_orientation_does_not_modify_non_scalar_elements() {
    for content in ["6<x/>7", "6<!-- separator -->7", "6<![CDATA[7]]>"] {
        let xmp = format!(r#"<tiff:Orientation xmlns:tiff="{TIFF_NS}">{content}</tiff:Orientation>"#);
        assert!(matches!(upright_xmp(&xmp), Cow::Borrowed(_)), "{xmp}");
    }
}

#[test]
fn xmp_orientation_export_preserves_unrelated_metadata() {
    let xmp = format!(
        r#"<x xmlns:tiff="{TIFF_NS}" xmlns:notiff="urn:custom" notiff:Orientation="6" tiff:Orientation='8'>
<description>tiff:Orientation="7"</description><!-- tiff:Orientation="6" -->
<![CDATA[tiff:Orientation='5']]></x>"#
    );
    let expected = xmp.replace("tiff:Orientation='8'", "tiff:Orientation='1'");
    let mut img = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![40, 80, 120]).unwrap();
    img.meta.xmp = Some(xmp);
    for format in [Format::Jpeg, Format::Png, Format::WebP, Format::Tiff] {
        let bytes = encode(&img, format, &EncodeOptions::default()).unwrap();
        assert_eq!(decode(&bytes).unwrap().meta.xmp.as_deref(), Some(expected.as_str()), "{format:?}");
    }
}

#[test]
fn xmp_orientation_handles_numeric_cdata_and_character_references() {
    for content in ["<![CDATA[ 6 ]]>", "&#54;", " &#x38; "] {
        let xmp = format!(r#"<tiff:Orientation xmlns:tiff="{TIFF_NS}">{content}</tiff:Orientation>"#);
        let expected = xmp.replace(" 6 ", " 1 ").replace("&#54;", "1").replace("&#x38;", "1");
        assert_eq!(upright_xmp(&xmp), expected);
    }
}

#[test]
fn xmp_orientation_keeps_large_attribute_spacing() {
    let gap = " ".repeat(300);
    let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation{gap}={gap}"6"/>"#);
    assert_eq!(upright_xmp(&xmp), xmp.replace("\"6\"", "\"1\""));
}

#[test]
fn xmp_orientation_structure_limit_keeps_packet_unchanged() {
    for markers in ["<".repeat(257), "=".repeat(4097), "xmlns".repeat(257)] {
        let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6"><!-- {markers} --></x>"#);
        assert!(matches!(upright_xmp(&xmp), Cow::Borrowed(_)));
    }
}

#[test]
fn xmp_orientation_bounds_cumulative_cdata_copying_before_parsing() {
    // Both packets are below the node/attribute/namespace budgets, but repeated
    // coalescing would copy a growing string. Bound total work, not just sections.
    for (payload_bytes, sections) in [(512, 128), (4096, 64)] {
        let content = format!("<![CDATA[{}]]>y", "x".repeat(payload_bytes)).repeat(sections);
        let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6"><description>{content}</description></x>"#);
        assert!(xmp.len() < 300_000);
        assert!(matches!(upright_xmp(&xmp), Cow::Borrowed(_)), "{payload_bytes} bytes per section, {sections} sections");
    }
    // Ordinary adjacent sections remain supported and are preserved byte-for-byte.
    let content = "<![CDATA[text]]>tail".repeat(8);
    let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6"><description>{content}</description></x>"#);
    assert_eq!(upright_xmp(&xmp), xmp.replace("Orientation=\"6\"", "Orientation=\"1\""));
}

#[test]
fn xmp_orientation_bounds_recursive_xml_nesting_before_parsing() {
    let nested = format!("{}text{}", "<a>".repeat(256), "</a>".repeat(256));
    let xmp = format!(r#"<x xmlns:tiff="{TIFF_NS}" tiff:Orientation="6">{nested}</x>"#);
    assert!(xmp.len() < 2000);
    assert!(matches!(upright_xmp(&xmp), Cow::Borrowed(_)));
}
