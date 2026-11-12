#![no_main]
//! Complete bounded XCF import, including its supported compressed transports.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if matches!(photocraft_io::import("fuzz.xcf", data), Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(_))) {
        for order in [photocraft_io::HistoricalXcfSampleOrder::LittleEndian, photocraft_io::HistoricalXcfSampleOrder::BigEndian] {
            let options = photocraft_io::ImportOptions { historical_xcf_sample_order: Some(order), ..Default::default() };
            let _ = photocraft_io::import_with_options("fuzz.xcf", data, &options);
        }
    }
});
