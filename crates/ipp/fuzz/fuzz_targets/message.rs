#![no_main]

use libfuzzer_sys::fuzz_target;

// The IPP message decoder on its own (a reply, and a request as the test fakes read it): never
// panic, never take more than the attribute cap.
fuzz_target!(|data: &[u8]| {
    let _ = photocraft_ipp::Reply::parse(data);
    let _ = photocraft_ipp::parse_request(data);
});
