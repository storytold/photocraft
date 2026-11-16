//! #13729: Export As's Metadata is None (default) or Copyright and Contact Info. Copyright writes
//! only the title, author, copyright notice and copyright URL (with an empty usage-terms entry and
//! the document and instance ids) as XMP; None writes no XMP and no EXIF from the document.

use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, SampleType, Size};
use std::sync::Arc;

const COPYRIGHT: &str = "Copyright and Contact Info";

/// Invented File Info, with the ids in both of XMP's forms (attribute and element) and fields
/// Copyright must leave out.
const PACKET: &str = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" xmlns:mm="http://ns.adobe.com/xap/1.0/mm/" mm:DocumentID="xmp.did:test-doc-0001">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Harbor Study</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>Test Author</rdf:li></rdf:Seq></dc:creator>
   <dc:rights><rdf:Alt><rdf:li xml:lang="x-default">Example Studio 2026</rdf:li></rdf:Alt></dc:rights>
   <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Private caption</rdf:li></rdf:Alt></dc:description>
   <dc:subject><rdf:Bag><rdf:li>boats</rdf:li></rdf:Bag></dc:subject>
   <xmpRights:WebStatement>https://example.com/rights</xmpRights:WebStatement>
   <xmpRights:UsageTerms><rdf:Alt><rdf:li xml:lang="x-default">Kept private</rdf:li></rdf:Alt></xmpRights:UsageTerms>
   <mm:InstanceID>xmp.iid:test-inst-0002</mm:InstanceID>
   <photoshop:AuthorsPosition>Head of nothing</photoshop:AuthorsPosition>
   <photoshop:DocumentAncestors><rdf:Bag><rdf:li>xmp.did:ancestor-0003</rdf:li></rdf:Bag></photoshop:DocumentAncestors>
   <photoshop:TextLayers><rdf:Bag><rdf:li>Layer text nobody should see</rdf:li></rdf:Bag></photoshop:TextLayers>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

const KEPT: [&str; 6] = ["Harbor Study", "Test Author", "Example Studio 2026", "https://example.com/rights", "xmp.did:test-doc-0001", "xmp.iid:test-inst-0002"];
const LEFT_OUT: [&str; 7] = ["Private caption", "boats", "Kept private", "Head of nothing", "ancestor-0003", "Layer text nobody should see", "Free-form note"];

/// EXIF holding only an Orientation tag (1).
fn exif() -> Vec<u8> {
    b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0".to_vec()
}

fn doc() -> Document {
    let mut doc = Document::with_background("Meta", Size::new(16, 12), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.metadata.xmp = Some(PACKET.into());
    doc.metadata.exif = Some(Arc::new(exif()));
    doc.metadata.text = vec![("Comment".into(), "Free-form note".into())];
    doc
}

fn export_with(metadata: &str, fmt: &str) -> Vec<u8> {
    export_packet(PACKET, metadata, fmt)
}

fn export_packet(packet: &str, metadata: &str, fmt: &str) -> Vec<u8> {
    let f = json!({"format": fmt, "scale": 100, "metadata": metadata, "profile": "document"}).as_object().unwrap().clone();
    let mut doc = doc();
    doc.metadata.xmp = Some(packet.into());
    let out = export_document(&doc, &f, None, &photocraft_engine::color_cmds::document_profile(&doc)).unwrap();
    photocraft_io::export(&out, &format!("out.{fmt}"), &settings(&f).io_options()).unwrap().bytes
}

fn contains(bytes: &[u8], s: &str) -> bool {
    bytes.windows(s.len()).any(|w| w == s.as_bytes())
}

/// The XMP packet a reader finds in the written file.
fn read_xmp(bytes: &[u8], fmt: &str) -> Option<String> {
    photocraft_io::import(&format!("out.{fmt}"), bytes).unwrap().document.metadata.xmp.map(|x| x.to_string())
}

#[test]
fn copyright_writes_only_the_copyright_and_contact_fields_on_jpeg_and_png() {
    for fmt in ["jpg", "png"] {
        let bytes = export_with("copyright", fmt);
        let xmp = read_xmp(&bytes, fmt).unwrap_or_else(|| panic!("{fmt}: an XMP packet"));
        for kept in KEPT {
            assert!(xmp.contains(kept), "{fmt}: {kept} kept in {xmp}");
        }
        assert!(xmp.contains("xmpRights:UsageTerms"), "{fmt}: the empty usage terms");
        for left in LEFT_OUT {
            assert!(!contains(&bytes, left), "{fmt}: {left} left out of the file");
        }
    }
}

#[test]
fn none_writes_no_xmp_and_no_exif() {
    for fmt in ["jpg", "png"] {
        let bytes = export_with("none", fmt);
        assert_eq!(read_xmp(&bytes, fmt), None, "{fmt}: no XMP");
        for marker in ["Exif\0\0", "eXIf", "http://ns.adobe.com/xap/1.0/", "XML:com.adobe.xmp"] {
            assert!(!contains(&bytes, marker), "{fmt}: no {marker:?}");
        }
        for s in KEPT.iter().chain(&LEFT_OUT) {
            assert!(!contains(&bytes, s), "{fmt}: {s} not written");
        }
    }
}

/// The earlier choices load as Copyright and Contact Info: remembered, from a preset and from a
/// recorded action. They never fail and never write the whole packet.
#[test]
fn earlier_exif_and_all_load_as_copyright() {
    for old in ["exif", "all"] {
        assert!(valid_setting("metadata", &json!(old)));
        let f = json!({"metadata": old}).as_object().unwrap().clone();
        assert_eq!(metadata_choice(&f), "copyright");
        let s = settings(&f);
        assert!(!s.xmp_all && s.xmp_contact && s.embed_metadata, "{old}");
        assert_eq!(preset_settings(&f)["metadata"], "all", "{old}: saved as a value every preset list accepts");
        let bytes = export_with(old, "jpg");
        assert!(KEPT.iter().all(|k| contains(&bytes, k)) && LEFT_OUT.iter().all(|l| !contains(&bytes, l)), "{old}");
    }
    let none = json!({"metadata": "none"}).as_object().unwrap().clone();
    assert_eq!(preset_settings(&none)["metadata"], "none");
    assert!(!valid_setting("metadata", &json!("everything")));
}

fn harness() -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc(), None);
        app
    })
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn selected(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::RadioButton, label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn metadata_is_two_radio_buttons_none_first_and_selected() {
    let mut h = harness();
    open_dialog(&mut h);
    let none = h.get_by_role_and_label(Role::RadioButton, "None").rect();
    let copyright = h.get_by_role_and_label(Role::RadioButton, COPYRIGHT).rect();
    assert!(none.min.y < copyright.min.y, "None comes first");
    assert!(selected(&h, "None") && !selected(&h, COPYRIGHT), "None by default");
    assert!(h.query_by_label("EXIF").is_none() && h.query_by_label("All").is_none(), "no earlier entries");
    h.get_by_role_and_label(Role::RadioButton, COPYRIGHT).click();
    h.run_steps(4);
    assert!(selected(&h, COPYRIGHT) && !selected(&h, "None"));
}

#[test]
fn a_remembered_exif_opens_on_copyright() {
    let mut h = harness();
    h.state_mut().session.prefs.edit(|p| {
        p.dialogs.insert(COMMAND.into(), json!({"metadata": "exif"}));
    });
    open_dialog(&mut h);
    assert!(selected(&h, COPYRIGHT));
}

const HEAD: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:mm="http://ns.adobe.com/xap/1.0/mm/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/">"#;
const TAIL: &str = "</rdf:RDF></x:xmpmeta>";

/// CodeRabbit on #15023: only the document's own descriptions count. A title or id inside an excluded
/// structure (an ancestor's description) is not written, even when the document has nothing allowed.
#[test]
fn a_description_nested_in_an_excluded_structure_is_not_the_documents() {
    let nested = r#"<rdf:Description rdf:about=""><photoshop:DocumentAncestors><rdf:Bag><rdf:li><rdf:Description><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Ancestor title</rdf:li></rdf:Alt></dc:title><mm:DocumentID>xmp.did:nested-0009</mm:DocumentID></rdf:Description></rdf:li></rdf:Bag></photoshop:DocumentAncestors>"#;
    for fmt in ["jpg", "png"] {
        // Nothing allowed at the document level: no packet at all.
        let only_nested = format!("{HEAD}{nested}</rdf:Description>{TAIL}");
        let bytes = export_packet(&only_nested, "copyright", fmt);
        for left in ["Ancestor title", "nested-0009"] {
            assert!(!contains(&bytes, left), "{fmt}: {left} left out");
        }
        // Something allowed at the top: only that is written.
        let both = format!(
            "{HEAD}{nested}<dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">Example Studio 2026</rdf:li></rdf:Alt></dc:rights></rdf:Description>{TAIL}"
        );
        let bytes = export_packet(&both, "copyright", fmt);
        assert!(contains(&bytes, "Example Studio 2026"), "{fmt}: the document's own rights");
        for left in ["Ancestor title", "nested-0009"] {
            assert!(!contains(&bytes, left), "{fmt}: {left} left out beside it");
        }
    }
}

/// A standalone `rdf:Description` (no `rdf:RDF` around it) is still read.
#[test]
fn a_standalone_description_is_still_read() {
    let packet = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/" rdf:about=""><dc:rights><rdf:Alt><rdf:li xml:lang="x-default">Example Studio 2026</rdf:li></rdf:Alt></dc:rights></rdf:Description>"#;
    assert!(contains(&export_packet(packet, "copyright", "png"), "Example Studio 2026"));
}

/// CodeRabbit on #15023: an allowed property keeps its value, not the qualifiers beside it.
#[test]
fn an_allowed_property_keeps_its_value_and_not_its_private_qualifiers() {
    let qualified = r#"<rdf:Description rdf:about=""><mm:DocumentID rdf:parseType="Resource"><rdf:value>xmp.did:test-doc-0001</rdf:value><dc:description>Private qualifier note</dc:description></mm:DocumentID><dc:title><rdf:Alt><rdf:li xml:lang="x-default" rdf:parseType="Resource"><rdf:value>Harbor Study</rdf:value><dc:description>Second private note</dc:description></rdf:li></rdf:Alt></dc:title></rdf:Description>"#;
    for fmt in ["jpg", "png"] {
        let bytes = export_packet(&format!("{HEAD}{qualified}{TAIL}"), "copyright", fmt);
        for kept in ["xmp.did:test-doc-0001", "Harbor Study"] {
            assert!(contains(&bytes, kept), "{fmt}: {kept} kept");
        }
        for left in ["Private qualifier note", "Second private note"] {
            assert!(!contains(&bytes, left), "{fmt}: {left} left out");
        }
    }
}

/// CodeRabbit on #15023: the qualified value written as a nested `rdf:Description` (not only as
/// `rdf:parseType="Resource"`) keeps its value and drops its qualifiers, on a property and on an item.
#[test]
fn a_qualified_value_in_a_nested_description_keeps_its_value() {
    let nested = r#"<rdf:Description rdf:about=""><mm:DocumentID><rdf:Description><rdf:value>xmp.did:test-doc-0001</rdf:value><dc:description>Private note one</dc:description></rdf:Description></mm:DocumentID><dc:title><rdf:Alt><rdf:li xml:lang="x-default"><rdf:Description><rdf:value>Harbor Study</rdf:value><dc:description>Private note two</dc:description></rdf:Description></rdf:li></rdf:Alt></dc:title></rdf:Description>"#;
    for fmt in ["jpg", "png"] {
        let bytes = export_packet(&format!("{HEAD}{nested}{TAIL}"), "copyright", fmt);
        for kept in ["xmp.did:test-doc-0001", "Harbor Study"] {
            assert!(contains(&bytes, kept), "{fmt}: {kept} kept");
        }
        for left in ["Private note one", "Private note two"] {
            assert!(!contains(&bytes, left), "{fmt}: {left} left out");
        }
    }
}
