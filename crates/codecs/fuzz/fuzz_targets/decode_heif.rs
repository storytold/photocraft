#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_codecs::{decode_as_with, DecodeOptions, Format, Limits};

// heic-decoder parses untrusted HEIF/HEVC: run with `-timeout=10` so a hang is reported, not
// just a panic. The codec turns a decoder panic into an error, so the decoder is also called
// directly, where its panics still reach the fuzzer. Both orientation paths are covered.
fuzz_target!(|data: &[u8]| {
    let keep_orientation = data.first().is_some_and(|b| b & 1 == 1);
    let options = heic_decoder::DecodeOptions { apply_transforms: !keep_orientation };
    if heic_decoder::probe(data).is_ok_and(|i| i.width.saturating_mul(i.height) <= 1 << 22 && i.coded_width.saturating_mul(i.coded_height) <= 1 << 22) {
        if let Ok(image) = heic_decoder::decode_with(data, &options) {
            let _ = image.to_rgba16();
        }
    }
    let limits = Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 };
    let _ = decode_as_with(Format::Heif, data, &DecodeOptions { limits, keep_orientation });
});
