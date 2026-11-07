#![no_main]
//! Decode arbitrary bytes as DPX and as Cineon (the two film-scan formats share one decoder), so
//! libFuzzer gets coverage feedback for it on its own instead of only through `decode_any`.

use libfuzzer_sys::fuzz_target;
use photocraft_codecs::{DecodeOptions, Format, Limits, decode_as_with};

fuzz_target!(|data: &[u8]| {
    let opts = DecodeOptions { limits: Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 }, ..Default::default() };
    let _ = decode_as_with(Format::Dpx, data, &opts);
    let _ = decode_as_with(Format::Cineon, data, &opts);
});
