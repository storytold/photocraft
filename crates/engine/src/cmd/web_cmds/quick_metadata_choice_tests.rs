//! #13788: Quick Export › Metadata is None (the default) or Copyright and Contact Info. The
//! latter writes the author, copyright notice, title, copyrighted flag and copyright URL (and an
//! empty usage-terms entry) as XMP, never keywords or the description; None writes no XMP and no
//! EXIF. An earlier `all` loads as Copyright and Contact Info; Save for Web is unchanged.

use super::super::optimizer::web_xmp;
use super::*;
use crate::prefs::ExportMetadata;

/// Invented File Info, with fields Copyright and Contact Info must leave out.
const PACKET: &str = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Harbor Study</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>Test Author</rdf:li></rdf:Seq></dc:creator>
   <dc:rights><rdf:Alt><rdf:li xml:lang="x-default">Example Studio 2026</rdf:li></rdf:Alt></dc:rights>
   <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Private caption</rdf:li></rdf:Alt></dc:description>
   <dc:subject><rdf:Bag><rdf:li>boats</rdf:li></rdf:Bag></dc:subject>
   <xmpRights:Marked>True</xmpRights:Marked>
   <xmpRights:WebStatement>https://example.com/rights</xmpRights:WebStatement>
   <photoshop:AuthorsPosition>Head of nothing</photoshop:AuthorsPosition>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

const KEPT: [&str; 4] = ["Harbor Study", "Test Author", "Example Studio 2026", "https://example.com/rights"];
const LEFT_OUT: [&str; 3] = ["boats", "Private caption", "Head of nothing"];

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s.edit("metadata", |d, _| {
        d.metadata.xmp = Some(PACKET.into());
        d.metadata.exif = Some(std::sync::Arc::new(b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0".to_vec()));
        Ok(())
    })
    .unwrap();
    s
}

fn quick_export(s: &mut Session, ext: &str) -> Vec<u8> {
    // Its own folder per call, removed with the guard, so tests running together never meet.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("out.{ext}"));
    s.execute("file.export.quickExport", json!({"path": path.to_string_lossy()})).unwrap();
    std::fs::read(&path).unwrap()
}

fn contains(bytes: &[u8], s: &str) -> bool {
    bytes.windows(s.len()).any(|w| w == s.as_bytes())
}

fn read_xmp(bytes: &[u8], ext: &str) -> Option<String> {
    photocraft_io::import(&format!("x.{ext}"), bytes).unwrap().document.metadata.xmp.map(|x| x.to_string())
}

#[test]
fn a_fresh_preferences_file_has_none_and_the_choices_are_two() {
    assert_eq!(crate::prefs::Export::default().metadata, ExportMetadata::None);
    assert_eq!(ExportMetadata::NAMES, ["none", "copyright"]);
}

#[test]
fn none_writes_no_xmp_and_no_exif_on_png_and_jpg() {
    let mut s = session();
    for ext in ["png", "jpg"] {
        let bytes = quick_export(&mut s, ext);
        assert_eq!(read_xmp(&bytes, ext), None, "{ext}");
        assert!(!contains(&bytes, "Exif\0\0") && !contains(&bytes, "eXIf"), "{ext}: no EXIF");
        assert!(KEPT.iter().chain(&LEFT_OUT).all(|t| !contains(&bytes, t)), "{ext}");
    }
}

#[test]
fn copyright_and_contact_info_writes_its_fields_and_never_keywords() {
    let mut s = session();
    s.execute("file.export.exportPreferences", json!({"metadata": "copyright"})).unwrap();
    for ext in ["png", "jpg"] {
        let bytes = quick_export(&mut s, ext);
        let xmp = read_xmp(&bytes, ext).unwrap_or_else(|| panic!("{ext}: a packet"));
        for kept in KEPT {
            assert!(xmp.contains(kept), "{ext}: {kept} in {xmp}");
        }
        assert!(xmp.contains("<xmpRights:Marked>True</xmpRights:Marked>") && xmp.contains("xmpRights:UsageTerms"), "{ext}: {xmp}");
        assert!(LEFT_OUT.iter().all(|t| !contains(&bytes, t)), "{ext}: no keywords, description or position");
        assert!(!contains(&bytes, "Exif\0\0") && !contains(&bytes, "eXIf"), "{ext}: no EXIF, as before");
    }
}

#[test]
fn an_earlier_all_loads_and_is_accepted_as_copyright_and_contact_info() {
    let export: crate::prefs::Export = serde_json::from_value(json!({"metadata": "all"})).unwrap();
    assert_eq!(export.metadata, ExportMetadata::Copyright);
    let stored: crate::prefs::Export = serde_json::from_value(json!({"metadata": "copyright"})).unwrap();
    assert_eq!(stored.metadata, ExportMetadata::Copyright);
    let mut s = session();
    let r = s.execute("file.export.exportPreferences", json!({"metadata": "all"})).unwrap();
    assert_eq!(r["values"]["metadata"], "copyright");
    assert!(s.execute("file.export.exportPreferences", json!({"metadata": "everything"})).is_err());
}

#[test]
fn save_for_web_keeps_its_own_metadata_packets() {
    let doc = session().active().unwrap().doc.clone();
    let copyright = web_xmp(&doc, super::super::settings::WebMetadata::Copyright).unwrap();
    assert!(copyright.contains("Example Studio 2026") && !copyright.contains("Harbor Study") && !copyright.contains("Test Author"));
    let contact = web_xmp(&doc, super::super::settings::WebMetadata::CopyrightAndContact).unwrap();
    assert!(contact.contains("Test Author") && contact.contains("Head of nothing") && !contact.contains("Harbor Study"));
    assert_eq!(web_xmp(&doc, super::super::settings::WebMetadata::All).as_deref(), Some(PACKET));
}

/// CodeRabbit on #15080: the packet reaches the PNG-8 file too (Smaller File on), with Copyright and
/// Contact Info; None still writes none.
#[test]
fn png8_carries_the_packet_when_copyright_is_chosen_and_none_otherwise() {
    let mut s = session();
    s.execute("file.export.exportPreferences", json!({"pngSmallerFile": true, "metadata": "copyright"})).unwrap();
    let bytes = quick_export(&mut s, "png");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(contains(&bytes, "PLTE"), "a palette PNG");
    let xmp = read_xmp(&bytes, "png").expect("a packet in the PNG-8");
    for kept in KEPT {
        assert!(xmp.contains(kept), "{kept} in {xmp}");
    }
    assert!(LEFT_OUT.iter().all(|t| !contains(&bytes, t)), "no keywords, description or position");
    s.execute("file.export.exportPreferences", json!({"metadata": "none"})).unwrap();
    let bytes = quick_export(&mut s, "png");
    assert_eq!(read_xmp(&bytes, "png"), None);
    assert!(!contains(&bytes, "XML:com.adobe.xmp"));
}

#[test]
fn a_png_that_does_not_parse_is_returned_as_it_was() {
    use super::super::quick::png_with_xmp;
    assert_eq!(png_with_xmp(b"not a png".to_vec(), "<x/>"), b"not a png");
    let bare = b"\x89PNG\r\n\x1a\n".to_vec();
    assert_eq!(png_with_xmp(bare.clone(), "<x/>"), bare, "no IDAT to put it before");
}

/// CodeRabbit on #15080: the word `unknown` is dropped only as the copyrighted flag's unset value; an
/// author, title or notice that says `unknown` is kept.
#[test]
fn a_field_that_literally_says_unknown_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    let packet = PACKET.replace("Test Author", "unknown").replace("Harbor Study", "unknown").replace("Example Studio 2026", "unknown");
    s.edit("metadata", |d, _| {
        d.metadata.xmp = Some(packet.clone().into());
        Ok(())
    })
    .unwrap();
    s.execute("file.export.exportPreferences", json!({"metadata": "copyright"})).unwrap();
    let xmp = read_xmp(&quick_export(&mut s, "png"), "png").expect("a packet");
    assert_eq!(xmp.matches(">unknown<").count(), 3, "author, title and notice: {xmp}");
}
