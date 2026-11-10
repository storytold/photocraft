#![no_main]
//! Complete bounded XCF import, including its supported compressed transports.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = photocraft_io::import("fuzz.xcf", data);
});
