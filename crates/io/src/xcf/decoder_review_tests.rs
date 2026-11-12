use super::*;

fn header(version: u32, width: u32, height: u32, base_type: u32, precision: u32) -> Vec<u8> {
    let mut bytes = format!("gimp xcf v{version:03}\0").into_bytes();
    for value in [width, height, base_type] {
        bytes.extend(value.to_be_bytes());
    }
    if version >= 4 {
        bytes.extend(precision.to_be_bytes());
    }
    bytes
}

#[test]
fn header_probe_and_import_share_validation_and_error_precedence() {
    for bytes in [header(9, 0, 1, 0, 250), header(9, 1, 1, 3, u32::MAX), header(9, 1, 1, 0, u32::MAX), header(9, 1, 1, 2, 250)] {
        let probe = historical_sample_order_request(&bytes).unwrap_err().to_string();
        let imported = import("invalid.xcf", &bytes, &Interrupt::NONE).unwrap_err().to_string();
        assert_eq!(probe, imported);
    }
}

#[test]
fn shared_header_stops_before_payload_and_sets_versioned_pointer_width() {
    for (version, base_type) in [(7, 0), (9, 1), (11, 0)] {
        let bytes = header(version, 1, 1, base_type, 250);
        let (cursor, parsed) = read_header(&bytes).unwrap();
        assert_eq!(cursor.pos, bytes.len());
        assert_eq!(cursor.wide, version >= 11);
        assert_eq!((parsed.width, parsed.height, parsed.base_type), (1, 1, base_type));
        let request = historical_sample_order_request(&bytes).unwrap().unwrap();
        let Err(IoError::HistoricalXcfSampleOrderRequired(required)) = import("historical.xcf", &bytes, &Interrupt::NONE) else {
            panic!("historical header must produce the same request before payload decoding")
        };
        assert_eq!(request, required);
    }
}
