#![no_main]
//! Parse arbitrary bytes as a Color Lookup file (.cube, .3dl, .look; the first byte picks the
//! extension), then write the result back out as .cube and parse that again.

use libfuzzer_sys::fuzz_target;
use photocraft_cms::lutfile;

fuzz_target!(|data: &[u8]| {
    let Some((&pick, bytes)) = data.split_first() else { return };
    let name = ["fuzz.cube", "fuzz.3dl", "fuzz.look"][usize::from(pick % 3)];
    let Ok(lut) = lutfile::parse(name, bytes) else { return };
    let _ = lutfile::parse_cube(&lutfile::write_cube(&lut));
});
