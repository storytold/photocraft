//! The document tab's profile mark asks `differs_from_working` every frame and the answer is cached. A working space
//! named by an ICC file must not keep a stale answer when that file is replaced (CodeRabbit on #16082).

use super::*;

#[test]
fn replacing_the_working_profile_file_drops_the_cached_answer() {
    // Removed with its guard, also when an assertion fails.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("working.icc");
    let srgb = Builtin::Srgb.profile().to_bytes();
    let p3 = Builtin::DisplayP3.profile().to_bytes();
    std::fs::write(&file, srgb.as_slice()).unwrap();

    let mut s = Session::new();
    s.color.settings.working_rgb = file.to_string_lossy().into_owned();
    let tag = p3.clone();
    assert!(s.color.differs_from_working(ColorMode::Rgb, &tag), "Display P3 differs from the sRGB file");

    // The file now holds Display P3 (a different length, so the stamp moves even within one timestamp tick).
    std::fs::write(&file, p3.as_slice()).unwrap();
    assert!(!s.color.differs_from_working(ColorMode::Rgb, &tag), "the same tag matches the replaced file");

    // And back: a file that fails to read falls back to the built-in sRGB, and recovering it is noticed too.
    std::fs::remove_file(&file).unwrap();
    assert!(s.color.differs_from_working(ColorMode::Rgb, &tag), "an unreadable file falls back to sRGB");
    std::fs::write(&file, p3.as_slice()).unwrap();
    assert!(!s.color.differs_from_working(ColorMode::Rgb, &tag), "recovering the file is noticed");
}
