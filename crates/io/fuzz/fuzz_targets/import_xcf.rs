#![no_main]
//! Import arbitrary bytes as GIMP .xcf through the same entry point File › Open uses. Gzip input
//! is named `.xcf.gz` so it reaches the bounded inflate; anything else is named `.xcf`, which sends
//! it to the XCF parser even without the `gimp xcf` magic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // A raster or PSD signature wins over the name and goes to a decoder with targets of its own
    // (crates/codecs/fuzz, crates/psd/fuzz); spending time there only finds big legal canvases.
    if photocraft_codecs::detect(data).is_some() || data.starts_with(b"8BPS") {
        return;
    }
    let name = if data.starts_with(&[0x1f, 0x8b]) { "fuzz.xcf.gz" } else { "fuzz.xcf" };
    let _ = photocraft_io::import(name, data);
});
