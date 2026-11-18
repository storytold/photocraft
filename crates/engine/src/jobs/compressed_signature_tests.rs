#[test]
fn compressed_worker_admission_uses_the_io_transport_signature_contract() {
    for bytes in [&b"\x1f\x8b"[..], &b"BZh"[..], &b"\xfd7zXZ\0"[..]] {
        assert!(super::has_compressed_import_signature(bytes));
        assert!(photocraft_io::import_compression(bytes).is_some());
        for end in 0..bytes.len() {
            assert!(!super::has_compressed_import_signature(&bytes[..end]));
        }
    }
    for bytes in [&b"gimp xcf "[..], &b"8BPS"[..], &b"<svg/>"[..], &b"PK\x03\x04"[..], &b""[..]] {
        assert!(!super::has_compressed_import_signature(bytes));
    }
}
