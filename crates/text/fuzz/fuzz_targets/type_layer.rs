#![no_main]
//! Parse arbitrary bytes as a PSD type layer's `TySh` block and as Photoshop's EngineData text
//! (the type a PSD carries): neither may panic. EngineData that parses is written again, and
//! what we write must parse and write the same bytes; a type layer that reads is built back into
//! `TySh`, which must read again.

use libfuzzer_sys::fuzz_target;
use photocraft_text::{engine_data, psd, psd_styles};

fuzz_target!(|data: &[u8]| {
    if let Ok(v) = engine_data::parse(data) {
        let out = engine_data::write(&v);
        let back = engine_data::parse(&out).expect("EngineData we wrote parses");
        assert_eq!(engine_data::write(&back), out, "EngineData is not byte-stable across a round trip");
    }
    let _ = psd::parse_tysh(data);
    let _ = psd::engine_text_is_blank(data);
    let _ = psd_styles::read_style_sheets(data, 72.0);
    if let Some(layer) = psd::text_layer_from_tysh(data, 72.0) {
        let built = psd::build_tysh(&layer, 72.0, None);
        assert!(psd::text_layer_from_tysh(&built, 72.0).is_some(), "a TySh block we built does not read");
    }
});
