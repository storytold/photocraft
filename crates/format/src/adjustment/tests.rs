use super::*;

struct Blob(Vec<u8>);
impl Fetch for Blob {
    fn tile(&mut self, _: &str, _: usize) -> Result<Vec<u8>> {
        Err(FormatError::corrupt("unexpected blob fetch"))
    }
    fn blob(&mut self, _: &str) -> Result<Arc<Vec<u8>>> {
        Ok(Arc::new(self.0.clone()))
    }
    fn blob_bounded(&mut self, _: &str, _: usize) -> Result<Arc<Vec<u8>>> {
        Err(FormatError::corrupt("unexpected blob fetch"))
    }
}

#[test]
fn malformed_binary_luts_fail_cleanly() {
    for bytes in [vec![0], vec![0; 3], f32::NAN.to_le_bytes().to_vec(), f32::INFINITY.to_le_bytes().to_vec()] {
        assert!(matches!(load_lut("hash", &mut Blob(bytes)), Err(FormatError::Corrupt(_))));
    }
}

#[test]
fn samples_are_little_endian_and_keep_finite_bit_patterns() {
    let samples = [0.0f32, -0.0, f32::MIN_POSITIVE, f32::MAX, -1.0];
    let bytes = samples.iter().flat_map(|v| v.to_le_bytes()).collect();
    let decoded = load_lut("hash", &mut Blob(bytes)).unwrap();
    assert_eq!(decoded.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), samples.map(f32::to_bits));
}
