#![no_main]
//! Decode arbitrary bytes as DDS (DirectDraw Surface: uncompressed, BC1-BC7, mip chains), so libFuzzer gets coverage
//! feedback for this decoder on its own instead of only through `decode_any`.

use libfuzzer_sys::fuzz_target;
use photocraft_codecs::{DecodeOptions, Format, Limits, decode_as_with};

fuzz_target!(|data: &[u8]| {
    let opts = DecodeOptions { limits: Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 }, ..Default::default() };
    let _ = decode_as_with(Format::Dds, data, &opts);
});
