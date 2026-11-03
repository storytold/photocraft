//! Shared document Quick Export naming for native paths and browser download names.

use std::path::{Path, PathBuf};

/// Append local `YYYYMMDD-HHMMSS-SSS` before the extension. Browser callers pass the returned
/// name to their existing download service; only native callers inspect destination availability.
/// Paths ending in a directory separator return an error.
pub fn timestamped_path(path: &Path) -> crate::Result<PathBuf> {
    timestamped_path_at(path, &chrono::Local::now())
}

fn timestamped_path_at<T: chrono::TimeZone>(path: &Path, time: &chrono::DateTime<T>) -> crate::Result<PathBuf>
where
    T::Offset: std::fmt::Display,
{
    if path.as_os_str().as_encoded_bytes().last().is_some_and(|&b| std::path::is_separator(char::from(b))) {
        return Err(crate::EngineError::Other("Quick Export needs a filename".into()));
    }
    let stem = path.file_stem().filter(|s| !s.is_empty()).ok_or_else(|| crate::EngineError::Other("Quick Export needs a filename".into()))?;
    let mut name = stem.to_os_string();
    name.push(time.format("_%Y%m%d-%H%M%S-%3f").to_string());
    if let Some(ext) = path.extension() {
        name.push(".");
        name.push(ext);
    }
    Ok(path.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn timestamp_preserves_components_and_pads_milliseconds() {
        let time = chrono::FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 11, 3, 10, 52, 27).unwrap() + chrono::Duration::milliseconds(7);
        for (input, expected) in [
            ("photo.png", "photo_20261103-105227-007.png"),
            ("dir/写真.edit_2.jpeg", "dir/写真.edit_2_20261103-105227-007.jpeg"),
            ("image", "image_20261103-105227-007"),
        ] {
            assert_eq!(timestamped_path_at(Path::new(input), &time).unwrap(), Path::new(expected));
        }
        assert!(timestamped_path_at(Path::new("/"), &time).is_err());
    }

    #[test]
    fn timestamp_rejects_directory_style_paths() {
        let time = chrono::Utc.with_ymd_and_hms(2026, 11, 3, 0, 0, 0).unwrap();
        for input in ["exports/photos/", "/exports/photos/", "/"] {
            assert!(timestamped_path_at(Path::new(input), &time).unwrap_err().to_string().contains("Quick Export needs a filename"));
        }
        #[cfg(windows)]
        for input in ["exports\\photos\\", "C:\\exports\\photos\\", "C:\\"] {
            assert!(timestamped_path_at(Path::new(input), &time).is_err());
        }
        #[cfg(not(windows))]
        assert_eq!(timestamped_path_at(Path::new("photo\\"), &time).unwrap(), Path::new("photo\\_20261103-000000-000"));
    }

    #[cfg(unix)]
    #[test]
    fn timestamp_preserves_non_unicode_names() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"photo_\xff.png"));
        let time = chrono::Utc.with_ymd_and_hms(2026, 11, 3, 0, 0, 0).unwrap();
        let named = timestamped_path_at(path, &time).unwrap();
        assert_eq!(named.as_os_str().as_bytes(), b"photo_\xff_20261103-000000-000.png");
        assert!(timestamped_path_at(Path::new(std::ffi::OsStr::from_bytes(b"photos_\xff/")), &time).is_err());
    }
}
