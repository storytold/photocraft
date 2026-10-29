#![no_main]
//! Import arbitrary bytes as PSD or .pcraft, then write the document as PSD, flat TIFF, layered
//! TIFF or .pcraft (the input's length picks one, so each run costs one writer) and open the
//! result again. A hostile document must not make a writer panic, and whatever a writer produces
//! must open: we hand these files on to other people and other applications.

use libfuzzer_sys::fuzz_target;
use photocraft_io::{ExportOptions, export, import, is_psd};

const WRITERS: [(&str, bool); 4] = [("psd", false), ("tif", false), ("tif", true), ("pcraft", false)];

fuzz_target!(|data: &[u8]| {
    let name = if is_psd(data) {
        "fuzz.psd"
    } else if photocraft_format::is_pcraft(data) {
        "fuzz.pcraft"
    } else {
        return;
    };
    let Ok(imported) = import(name, data) else { return };
    let doc = imported.document;
    // Big but legal canvases only make each run slow (the import above has already read them).
    if u64::from(doc.size.width) * u64::from(doc.size.height) > 1 << 16 || doc.layer_count() > 64 {
        return;
    }
    let (ext, tiff_layers) = WRITERS[data.len() % WRITERS.len()];
    // Never read the files that linked smart objects name: the input chooses those paths.
    let opts = ExportOptions { tiff_layers, read_linked_files: false, ..ExportOptions::default() };
    let Ok(out) = export(&doc, ext, &opts) else { return };
    if let Err(e) = import(&format!("fuzz.{ext}"), &out.bytes) {
        panic!("a {ext} file we wrote (layers: {tiff_layers}) does not open: {e}");
    }
});
