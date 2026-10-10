//! A small cache of parsed LUT files, keyed by path, size and modification time. Previewing a LUT
//! builds the Color Lookup adjustment again on every frame; this keeps that from reading and
//! parsing the same file each time. The cache holds the parser's own result, so whatever the
//! parser learns to report (input domain, size limits) reaches Color Lookup unchanged.

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_cms::lutfile::{self, LutFile};

/// Most tables kept.
const ENTRIES: usize = 6;
/// Tables larger than this many values are not kept.
const MAX_KEPT_VALUES: usize = 4_000_000;

type Key = (String, u64, u64);

static CACHE: Mutex<Vec<(Key, Arc<LutFile>)>> = Mutex::new(Vec::new());

fn key_of(path: &str) -> Option<Key> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
    Some((path.to_string(), meta.len(), mtime))
}

/// Read and parse the LUT file `path` with `read`, or return the cached result when the file is
/// unchanged. Errors read like Color Lookup's own (`can't read <path>`, `<path>: <parse error>`).
pub fn load(path: &str, read: fn(&str) -> Option<Vec<u8>>) -> Result<Arc<LutFile>, String> {
    let key = key_of(path);
    if let Some(k) = &key {
        let cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, hit)) = cache.iter().find(|(c, _)| c == k) {
            return Ok(hit.clone());
        }
    }
    let bytes = read(path).ok_or_else(|| format!("can't read {path}"))?;
    let parsed = Arc::new(lutfile::parse(path, &bytes).map_err(|e| format!("{path}: {}", e.0))?);
    if let Some(k) = key
        && parsed.data.len() <= MAX_KEPT_VALUES
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|(c, _)| *c != k);
        cache.insert(0, (k, parsed.clone()));
        cache.truncate(ENTRIES);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    #[test]
    fn an_unchanged_file_is_parsed_once_and_a_changed_file_is_read_again() {
        let p = std::env::temp_dir().join(format!("photocraft-lutcache-{}.cube", std::process::id()));
        std::fs::write(&p, lutfile::write_cube(&LutFile::identity(3))).unwrap();
        let path = p.to_string_lossy().into_owned();
        let (a, b) = (load(&path, read).unwrap(), load(&path, read).unwrap());
        assert!(Arc::ptr_eq(&a, &b), "the second load comes from the cache");
        std::fs::write(&p, lutfile::write_cube(&LutFile::identity(5))).unwrap();
        assert_eq!(load(&path, read).unwrap().size, 5, "a changed file is parsed again");
        assert!(load("/nonexistent/x.cube", read).unwrap_err().starts_with("can't read"));
        std::fs::write(&p, "not a LUT").unwrap();
        assert!(load(&path, read).unwrap_err().contains(&path));
    }
}
