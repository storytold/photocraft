use photocraft_engine::file_cmds::read_file_info;

fn description(text: &str) -> String {
    let xmp = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF><rdf:Description>\
        <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{text}</rdf:li></rdf:Alt></dc:description>\
        </rdf:Description></rdf:RDF></x:xmpmeta>"
    );
    read_file_info(Some(&xmp))["description"].as_str().unwrap().to_string()
}

#[test]
fn xmp_numeric_references_decode_once_with_xml_character_rules() {
    assert_eq!(description("A&#10;B &#169; &#x1F642; &amp;#38; &#38;lt;"), "A\nB \u{00A9} \u{1F642} &#38; &lt;");
    assert_eq!(description("A&#9;B&#13;C &lt;&gt;&quot;&apos;&amp;"), "A\tB\rC <>\"'&");
    for invalid in
        ["&#0;", "&#1;", "&#xD800;", "&#xFFFE;", "&#xFFFF;", "&#x110000;", "&#-1;", "&#+65;", "&#x+41;", "&#X41;", "&#;", "&#x;", "&#12z;", "&unknown;"]
    {
        assert_eq!(description(invalid), invalid, "invalid references stay literal");
    }
}

#[test]
fn xmp_long_zero_padding_and_malformed_ampersands_remain_bounded() {
    let zeros = "0".repeat(1024);
    assert_eq!(description(&format!("&#{zeros}65; &#x{zeros}41;")), "A A");
    let input = format!("{}&#65; &broken &lt;", "&".repeat(4096));
    assert_eq!(description(&input), format!("{}A &broken <", "&".repeat(4096)));
    assert_eq!(description("&#999999999999999999999999;"), "&#999999999999999999999999;");
}
