//! Starting XMP belongs to newly created documents, never to an opened file.
use super::*;
use chrono::{DateTime, Local};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_geom::Size;

const PROPERTIES: [&str; 7] =
    ["xmp:CreatorTool", "xmp:CreateDate", "xmp:ModifyDate", "xmp:MetadataDate", "dc:format", "photoshop:ColorMode", "photoshop:ICCProfile"];

fn packet(s: &Session) -> &str {
    s.active().unwrap().doc.metadata.xmp.as_deref().expect("a newly created document has starting XMP")
}

fn property<'a>(xmp: &'a str, name: &str) -> Option<&'a str> {
    super::info::find_element(xmp, name).map(|(_, _, value)| value)
}

fn assert_starting_packet(s: &Session, before: i64, after: i64, rgb: bool, profile: Option<&str>) {
    let xmp = packet(s);
    let namespaces = [
        ("x", "adobe:ns:meta/"),
        ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
        ("xmp", "http://ns.adobe.com/xap/1.0/"),
        ("dc", "http://purl.org/dc/elements/1.1/"),
        ("photoshop", "http://ns.adobe.com/photoshop/1.0/"),
    ];
    let canonical = craft_xml::Canonical::new(&namespaces);
    let xml = craft_xml::Options::canonical(&canonical).strict().parse(xmp.as_bytes()).expect("starting XMP is well-formed namespace-aware XML");
    assert_eq!(property(xmp, "xmp:CreatorTool"), Some(format!("PhotoCraft {}", env!("CARGO_PKG_VERSION")).as_str()));
    assert_eq!(property(xmp, "dc:format"), Some("image/vnd.adobe.photoshop"));
    assert_eq!(property(xmp, "photoshop:ColorMode"), rgb.then_some("3"));
    assert_eq!(property(xmp, "photoshop:ICCProfile"), profile);
    let created = property(xmp, "xmp:CreateDate").unwrap();
    assert_eq!(property(xmp, "xmp:ModifyDate"), Some(created));
    assert_eq!(property(xmp, "xmp:MetadataDate"), Some(created));
    let date = DateTime::parse_from_rfc3339(created).expect("the starting date is ISO 8601");
    assert!((before..=after).contains(&date.timestamp()), "the date is the creation time: {created}, {before}..={after}");
    assert_eq!(date.offset().local_minus_utc(), date.with_timezone(&Local).offset().local_minus_utc(), "local offset, not a forced UTC date");
    assert!(created.ends_with(&date.format("%:z").to_string()), "the ISO date includes its numeric local offset: {created}");
    let mut count = 0;
    for name in PROPERTIES {
        let occurrences = xml.descendants().filter(|element| element.name == name).count();
        assert_eq!(occurrences, usize::from(property(xmp, name).is_some()), "each property occurs at most once: {name}");
        count += occurrences;
    }
    assert_eq!(count, 5 + usize::from(rgb) + usize::from(profile.is_some()));
}

#[test]
fn new_rgb_documents_start_with_basic_xmp_without_tagging_untagged_content() {
    for depth in [8, 16, 32] {
        for params in [
            json!({"width": 3, "height": 2, "depth": depth, "background": "white"}),
            json!({"width": 3, "height": 2, "depth": depth, "background": "transparent"}),
            json!({"width": 3, "height": 2, "depth": depth, "artboard": true}),
        ] {
            let mut s = crate::session();
            let before = Local::now().timestamp();
            s.execute("file.new", params).unwrap();
            let after = Local::now().timestamp();
            assert_eq!(s.active().unwrap().doc.mode, ColorMode::Rgb);
            assert!(s.active().unwrap().doc.icc_profile.is_none(), "starting metadata must not tag an untagged document");
            assert_starting_packet(&s, before, after, true, None);
        }
    }
}

#[test]
fn tagged_new_and_preset_documents_write_all_seven_properties_from_their_profile() {
    for (profile, description) in [("srgb", "sRGB IEC61966-2.1"), ("display-p3", "Display P3")] {
        for preset in [false, true] {
            let mut s = crate::session();
            let settings = json!({"width": 3, "height": 2, "mode": "rgb", "depth": 16, "background": "white", "profile": profile});
            let params = if preset {
                s.execute("document.presets.save", json!({"name": "Metadata profile", "settings": settings})).unwrap();
                s.execute("document.presets.get", json!({"name": "Metadata profile"})).unwrap()["params"].clone()
            } else {
                settings
            };
            let before = Local::now().timestamp();
            s.execute("file.new", params).unwrap();
            let after = Local::now().timestamp();
            let doc = &s.active().unwrap().doc;
            assert!(doc.icc_profile.is_some());
            assert_eq!(crate::color_cmds::document_profile(doc).description, description);
            assert_starting_packet(&s, before, after, true, Some(description));
        }
    }
}

#[test]
fn clipboard_and_paste_without_a_document_get_fresh_xmp_without_inheriting_metadata_or_tagging() {
    for depth in [8, 16, 32] {
        for paste_without_document in [false, true] {
            let mut s = crate::session();
            s.execute("file.new", json!({"width": 3, "height": 2, "depth": depth, "profile": "display-p3"})).unwrap();
            Arc::make_mut(s.active_mut().unwrap().doc_mut()).metadata.xmp = Some("<source-only-metadata/>".into());
            s.execute("select.all", json!({})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            if paste_without_document {
                s.execute("file.close", json!({})).unwrap();
                assert!(s.documents().is_empty());
            }
            let before = Local::now().timestamp();
            s.execute(if paste_without_document { "edit.paste" } else { "file.newFromClipboard" }, json!({})).unwrap();
            let after = Local::now().timestamp();
            let doc = &s.active().unwrap().doc;
            assert_eq!((doc.size.width, doc.size.height), (3, 2));
            assert_eq!(doc.mode, ColorMode::Rgb);
            assert!(doc.icc_profile.is_none(), "the existing clipboard profile behavior stays untagged");
            assert!(!packet(&s).contains("source-only-metadata"));
            assert_starting_packet(&s, before, after, true, None);
        }
    }
}

#[test]
fn unmeasured_modes_omit_color_mode_and_untagged_profile_properties() {
    for (mode, depth, expected) in
        [("gray", 8, ColorMode::Grayscale), ("cmyk", 8, ColorMode::Cmyk), ("lab", 8, ColorMode::Lab), ("bitmap", 1, ColorMode::Bitmap)]
    {
        let mut s = crate::session();
        let before = Local::now().timestamp();
        s.execute("file.new", json!({"width": 3, "height": 2, "mode": mode, "depth": depth})).unwrap();
        let after = Local::now().timestamp();
        assert_eq!(s.active().unwrap().doc.mode, expected);
        assert!(s.active().unwrap().doc.icc_profile.is_none());
        assert_starting_packet(&s, before, after, false, None);
    }
}

#[test]
fn file_info_ok_and_editing_descriptive_fields_preserve_the_starting_dates() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 3, "height": 2, "profile": "display-p3"})).unwrap();
    let original = packet(&s).to_string();
    let submitted = s.execute("file.fileInfo", json!({})).unwrap();
    s.execute("file.fileInfo", submitted).unwrap();
    assert_eq!(packet(&s), original, "OK without changes keeps the complete starting packet");
    s.execute("file.fileInfo", json!({"title": "Before dawn"})).unwrap();
    for name in PROPERTIES {
        assert_eq!(property(packet(&s), name), property(&original, name), "File Info does not rewrite {name}");
    }
    assert_eq!(s.execute("file.fileInfo", json!({})).unwrap()["title"], "Before dawn");
}

#[test]
fn opening_files_keeps_their_own_packet_or_absence_of_a_packet() {
    const IMPORTED: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"><xmp:CreatorTool>Imported Tool 1.2</xmp:CreatorTool><xmp:CreateDate>2001-02-03T04:05:06+05:45</xmp:CreateDate><xmp:ModifyDate>2002-03-04T05:06:07-04:00</xmp:ModifyDate><xmp:MetadataDate>2003-04-05T06:07:08+00:00</xmp:MetadataDate><dc:format>image/custom</dc:format><photoshop:ColorMode>3</photoshop:ColorMode><photoshop:ICCProfile>Imported Profile</photoshop:ICCProfile></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    for xmp in [None, Some(IMPORTED)] {
        let mut doc = Document::with_background("Imported", Size::new(3, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.metadata.xmp = xmp.map(Into::into);
        let bytes = encode(&doc, "imported.psd", None).unwrap().0;
        // Export legitimately synchronizes resolution metadata. Open must retain the packet
        // actually in the file, including those fields, rather than regenerate starting XMP.
        let encoded = import("imported.psd", &bytes).unwrap();
        if let Some(original) = xmp {
            for name in PROPERTIES {
                assert_eq!(property(encoded.metadata.xmp.as_deref().unwrap(), name), property(original, name));
            }
        } else {
            assert!(encoded.metadata.xmp.is_none());
        }
        let mut s = crate::session();
        open_bytes_as(&mut s, "imported.psd", &bytes, None, Some("imported.psd".into())).unwrap();
        assert_eq!(s.documents().len(), 1);
        assert_eq!(s.active().unwrap().doc.metadata.xmp, encoded.metadata.xmp, "opening must not seed or rewrite metadata");
    }
}

#[test]
fn starting_xmp_remains_a_real_openraster_metadata_loss() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 3, "height": 2})).unwrap();
    assert!(s.active().unwrap().doc.metadata.xmp.is_some());
    assert!(!holds_at(&s.active().unwrap().doc, "new.ora"));
    assert!(losses_at(&s.active().unwrap().doc, "new.ora").iter().any(|loss| loss.to_lowercase().contains("metadata")));
    // This is why the explicit metadata-free OpenRaster fixtures clear the starting packet.
    Arc::make_mut(s.active_mut().unwrap().doc_mut()).metadata.xmp = None;
    assert!(holds_at(&s.active().unwrap().doc, "new.ora"));
    assert!(losses_at(&s.active().unwrap().doc, "new.ora").is_empty());
}
