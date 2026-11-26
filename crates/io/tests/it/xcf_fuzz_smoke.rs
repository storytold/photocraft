#![cfg_attr(not(test), no_main)]
//! One driver for the integration smoke and the standalone XCF fuzz target.

#[cfg(not(test))]
use libfuzzer_sys::fuzz_target;
use photocraft_io::{HistoricalXcfOrder, HistoricalXcfSampleOrder, ImportOptions, IoError};

#[cfg(not(test))]
fuzz_target!(|data: &[u8]| exercise_import(data, drop));

fn exercise_import(data: &[u8], mut observe: impl FnMut(Result<(), IoError>)) {
    // Keep the compatible import path in the sweep as well as the interactive
    // caller's required-choice policy; Default deliberately does not ask.
    observe(photocraft_io::import("fuzz.xcf", data).map(drop));
    let options = ImportOptions { historical_xcf_order: HistoricalXcfOrder::RequireExplicit, ..Default::default() };
    let result = photocraft_io::import_with_options("fuzz.xcf", data, &options).map(drop);
    let needs_order = matches!(result, Err(IoError::HistoricalXcfSampleOrderRequired(_)));
    observe(result);
    if needs_order {
        for order in [HistoricalXcfSampleOrder::LittleEndian, HistoricalXcfSampleOrder::BigEndian] {
            let options = ImportOptions { historical_xcf_order: HistoricalXcfOrder::Explicit(order), ..Default::default() };
            observe(photocraft_io::import_with_options("fuzz.xcf", data, &options).map(drop));
        }
    }
}

// Original minimal XCF header/properties and null lists. The smoke needs no
// duplicate pixel writer; exact drawable/order oracles live in the importer tests.
#[cfg(test)]
fn empty_xcf(version: u32, precision: u32) -> Vec<u8> {
    let mut bytes = format!("gimp xcf v{version:03}\0").into_bytes();
    for value in [1_u32, 1, 0, precision] {
        bytes.extend(value.to_be_bytes());
    }
    bytes.extend([0; 8]); // End properties.
    bytes.extend([0; 16]); // Null 64-bit layer and channel pointers (version >=11).
    if version >= 18 {
        bytes.extend([0; 8]); // Null path pointer.
    }
    bytes
}

#[test]
fn modern_import_and_required_policy_need_no_retry() {
    let data = empty_xcf(26, 150);
    let mut results = Vec::new();
    exercise_import(&data, |result| results.push(result));
    assert!(matches!(results.as_slice(), [Ok(()), Ok(())]));
}

#[test]
fn historical_required_import_retries_both_orders_while_normal_import_keeps_default() {
    let data = empty_xcf(11, 250);
    let mut results = Vec::new();
    exercise_import(&data, |result| results.push(result));
    assert!(matches!(results.as_slice(), [Ok(()), Err(IoError::HistoricalXcfSampleOrderRequired(_)), Ok(()), Ok(())]));
}
