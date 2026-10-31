//! Bounded reads of user-selected copies; never hand a temporary path to the editor.
use std::io::Read;

pub const MAX_IMPORT_BYTES: u64 = 128 * 1024 * 1024;

pub fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("Could not read the selected image: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err("The selected file is too large (maximum 128 MiB).".into());
    }
    if bytes.is_empty() {
        return Err("The selected file is empty.".into());
    }
    Ok(bytes)
}

pub fn read_path(path: &str) -> Result<(String, Vec<u8>), String> {
    let path = std::path::Path::new(path);
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("Image").to_owned();
    let file = std::fs::File::open(path).map_err(|e| format!("Could not open the selected image: {e}"))?;
    read_bounded(file, MAX_IMPORT_BYTES).map(|bytes| (name, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_boundary_and_rejects_truncated_or_empty_imports() {
        assert_eq!(read_bounded(&b"image"[..], 5), Ok(b"image".to_vec()));
        assert!(read_bounded(&b"image!"[..], 5).is_err());
        assert!(read_bounded(&b""[..], 5).is_err());
    }
    #[test]
    fn reports_io_failure_without_partial_success() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("unavailable"))
            }
        }
        assert!(read_bounded(Broken, 8).is_err());
        assert!(read_path("/nonexistent-photocraft-test/image.png").is_err());
    }
}
