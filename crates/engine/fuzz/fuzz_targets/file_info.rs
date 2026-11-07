#![no_main]
//! Read arbitrary text as a document's XMP packet the way File › File Info and the camera data
//! panel do (`read_file_info`, `camera_data`), then write File Info fields into it
//! (`write_file_info`): none may panic, and when the packet is plain well-formed XML the written
//! one must read back the fields we put in. These readers match tags as text, so a broken packet,
//! or tag-like text inside a comment, CDATA section or processing instruction, can fool them.

use libfuzzer_sys::fuzz_target;
use photocraft_engine::file_cmds::{camera_data, read_file_info, write_file_info};
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let Ok(xmp) = std::str::from_utf8(data) else { return };
    let _ = read_file_info(Some(xmp));
    let _ = camera_data(None, Some(xmp));
    let info = json!({ "title": "Seed <&> \"title\"", "author": "PhotoCraft", "keywords": ["a", "b"] });
    let written = write_file_info(Some(xmp), &info);
    let back = read_file_info(Some(&written));
    let _ = camera_data(None, Some(&written));
    if plain_xml(xmp) {
        assert_eq!(back.get("title"), info.get("title"), "File Info lost the title it wrote: {written}");
    }
});

/// Well-formed XML made of elements, attributes and text only, apart from `<?xpacket?>` markers.
fn plain_xml(xmp: &str) -> bool {
    !xmp.contains("<![CDATA[")
        && roxmltree::Document::parse(xmp).is_ok_and(|d| d.descendants().all(|n| !n.is_comment() && n.pi().is_none_or(|pi| pi.target == "xpacket")))
}
