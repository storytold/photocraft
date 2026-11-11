use super::*;

#[test]
fn gray_lab_pcs_is_not_rewritten_as_a_luminance_curve() {
    let mut assigned = Builtin::SGray.profile().clone();
    assigned.pcs = photocraft_cms::profile::Pcs::Lab;
    let bytes = assigned.with_encoded_bytes().to_bytes();
    assert_eq!(profile(ColorMode::Grayscale, Precision::from_header(26, 650).unwrap(), Some(bytes.as_ref().clone())).unwrap(), bytes);
    for code in [600, 675] {
        let error = profile(ColorMode::Grayscale, Precision::from_header(26, code).unwrap(), Some(bytes.as_ref().clone()));
        assert!(error.is_err(), "precision {code} must not reinterpret Lab L* as luminance");
    }
}
