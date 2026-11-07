#![no_main]
//! Set arbitrary text as a layer's XMP packet (`layer.xmpMetadata`, as an agent or script can):
//! it must be refused or kept, never panic. A packet that is kept goes through a PSD save and
//! open (Photoshop stores it per layer, as UTF-16) and must come back unchanged.

use libfuzzer_sys::fuzz_target;
use photocraft_engine::Session;
use photocraft_io::{ExportOptions, export, import};
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let mut s = Session::new();
    s.execute("file.new", json!({ "width": 4, "height": 4 })).expect("a new document opens");
    if s.execute("layer.xmpMetadata", json!({ "rawData": text })).is_err() {
        return;
    }
    let Some(st) = s.active() else { panic!("the document closed") };
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).expect("the new document has an active layer");
    assert_eq!(layer.xmp.as_deref(), Some(text), "the layer doesn't hold the packet it accepted");
    let opts = ExportOptions { read_linked_files: false, ..ExportOptions::default() };
    let psd = export(&st.doc, "psd", &opts).expect("a 4x4 document saves as PSD");
    let back = import("back.psd", &psd.bytes).expect("a PSD we wrote opens").document;
    let xmp: Vec<Option<&str>> = back.layers.iter().map(|l| l.xmp.as_deref()).collect();
    assert!(xmp.contains(&Some(text)), "the layer XMP packet is lost or changed in a PSD round trip: {xmp:?}");
});
