#![no_main]
//! Register arbitrary bytes as a font (TTF/OTF, or a TTC/OTC collection: the bytes a document
//! or a font folder can bring), then lay out and rasterise type in every family it adds. The
//! font parser, shaper and rasteriser must reject or draw it, never panic.

use libfuzzer_sys::fuzz_target;
use photocraft_color::PixelFormat;
use photocraft_doc::TextLayer;
use photocraft_text::TextEngine;

fuzz_target!(|data: &[u8]| {
    let _ = photocraft_text::fonts::face_count(data);
    let _ = photocraft_text::cjk::face_index_for_family(data, "Seed");
    let mut engine = TextEngine::new();
    let families = engine.fonts.register_font_data(data.to_vec());
    // Mixed scripts, a ligature, combining marks and a missing glyph's fallback.
    for (i, family) in families.iter().take(4).enumerate() {
        let layer = TextLayer {
            text: "Ag Wy 1/2 ffi \u{e9}e\u{301} \u{41f}\u{440}\u{438} \u{5d0}\u{5d1} \u{4e2d}\n\u{fffd}".into(),
            font_family: family.clone(),
            size_pt: [6.0, 12.0, 48.0, 0.5][i],
            ..Default::default()
        };
        let _ = engine.render(&layer, 72.0, PixelFormat::RGBA8);
    }
});
