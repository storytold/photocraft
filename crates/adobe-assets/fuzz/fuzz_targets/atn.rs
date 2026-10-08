//! Fuzz the `.atn` action-file parser: any input must return `Ok` or `Err`, never panic, and
//! anything that parses must be byte stable after one normalization pass (flag bytes other than
//! 0/1).
#![no_main]

use libfuzzer_sys::fuzz_target;
use photocraft_adobe_assets::atn;

fuzz_target!(|data: &[u8]| {
    if let Ok(set) = atn::parse(data) {
        let b = atn::write(&set);
        assert_eq!(atn::write(&atn::parse(&b).expect("re-parse")), b);
    }
});
