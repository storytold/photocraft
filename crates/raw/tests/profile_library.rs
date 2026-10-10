//! Opt-in validation of external, locally installed DCPs. Assets are never copied into tests.
use photocraft_raw::CameraProfile;

#[test]
#[ignore = "set PHOTOCRAFT_DCP_TEST_ROOT to an installed camera-profile directory"]
fn installed_profile_library_is_parsed_without_copying_assets() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_DCP_TEST_ROOT").expect("PHOTOCRAFT_DCP_TEST_ROOT"));
    let mut paths = vec![root];
    let mut models = std::collections::BTreeSet::new();
    let mut rejected = std::collections::BTreeMap::<String, usize>::new();
    let (mut accepted, mut total) = (0, 0);
    while let Some(path) = paths.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap().flatten() {
                if !entry.file_type().unwrap().is_symlink() {
                    paths.push(entry.path());
                }
            }
            continue;
        }
        if path.extension().and_then(|s| s.to_str()).is_none_or(|s| !s.eq_ignore_ascii_case("dcp")) {
            continue;
        }
        total += 1;
        assert!(total <= 40000);
        let bytes = std::fs::read(&path).unwrap();
        match CameraProfile::from_dcp(&bytes) {
            Ok(profile) => {
                models.insert(profile.camera_model);
                accepted += 1;
            }
            Err(e) => {
                *rejected.entry(e.to_string()).or_default() += 1;
            }
        }
    }
    eprintln!("DCP library: {accepted}/{total} profiles parsed, {} calibrated camera models; unsupported: {rejected:?}", models.len());
    assert!(accepted > 0, "no usable camera profiles");
}
