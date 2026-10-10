#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_codecs::{decode_as_with, DecodeOptions, Format, Limits};

// jxl-oxide parses untrusted codestreams and containers: run with `-timeout=10` so a hang is
// reported, not just a panic. The codec turns a jxl-oxide panic into an error, so jxl-oxide is
// also called directly, where its panics still reach the fuzzer. Both orientation paths are
// covered.
fuzz_target!(|data: &[u8]| {
    let keep_orientation = data.first().is_some_and(|b| b & 1 == 1);
    let _ = photocraft_jxl::decode(data, &photocraft_jxl::Options { alloc_limit: 256 << 20 });
    let limits = Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 };
    let _ = decode_as_with(Format::Jxl, data, &DecodeOptions { limits, keep_orientation });
});
