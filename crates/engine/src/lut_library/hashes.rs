//! Content hashes of installed LUTs, so installing the same LUTs twice (a pack and its parent
//! folder, or a pack downloaded again) can be noticed. Each pack keeps its hashes in a hidden
//! `.hashes` file written at install time; packs installed without one are hashed once, on demand.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use super::{LutLibrary, MAX_LUT_BYTES, collect};

const HASH_FILE: &str = ".hashes";

pub(super) fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(super) fn write_sidecar(dir: &Path, hashes: &[(String, String)]) {
    let text: String = hashes.iter().map(|(h, f)| format!("{h}\t{f}\n")).collect();
    let _ = super::write_atomic(&dir.join(HASH_FILE), text.as_bytes());
}

fn read_sidecar(dir: &Path) -> Option<Vec<(String, String)>> {
    let text = fs::read_to_string(dir.join(HASH_FILE)).ok()?;
    Some(text.lines().filter_map(|l| l.split_once('\t')).map(|(h, f)| (h.to_string(), f.to_string())).collect())
}

/// The content of a LUT file, or why it cannot be used: unreadable, or larger than
/// [`MAX_LUT_BYTES`] (install refuses those too). The length is checked before the file is opened
/// and again while reading, so a file that grows in between is still capped.
pub(super) fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_LUT_BYTES {
        return Err(format!("file too large ({len} bytes)"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path).and_then(|f| f.take(MAX_LUT_BYTES + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_LUT_BYTES {
        return Err(format!("file too large (over {MAX_LUT_BYTES} bytes)"));
    }
    Ok(bytes)
}

/// [`read_capped`] without the reason, for callers that only skip what they cannot read.
pub(super) fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    read_capped(path).ok()
}

/// Pack-relative paths of the LUTs in `dir` that can have a hash (not over [`MAX_LUT_BYTES`]), so
/// an oversize file does not make the sidecar look stale and force a rebuild on every scan.
fn hashable(dir: &Path) -> HashSet<String> {
    let Ok(found) = collect(dir) else { return HashSet::new() };
    found.iter().filter(|f| fs::metadata(&f.path).is_ok_and(|m| m.len() <= MAX_LUT_BYTES)).map(|f| f.rel.join("/")).collect()
}

/// Hash every LUT in `dir` (pack-relative paths) and keep the result next to them. Files over
/// [`MAX_LUT_BYTES`] are left out: they cannot be loaded, so they never need a hash.
fn rebuild(dir: &Path) -> Vec<(String, String)> {
    let Ok(found) = collect(dir) else { return Vec::new() };
    let hashes: Vec<(String, String)> = found.iter().filter_map(|f| read_bounded(&f.path).map(|b| (hash_bytes(&b), f.rel.join("/")))).collect();
    write_sidecar(dir, &hashes);
    hashes
}

/// `(content hashes, deepest folder level)` of the pack in `dir`.
fn pack_hashes(dir: &Path) -> (HashSet<String>, usize) {
    let present = hashable(dir);
    let mut hashes = read_sidecar(dir).unwrap_or_default();
    if hashes.len() != present.len() || hashes.iter().any(|(_, f)| !present.contains(f)) {
        hashes = rebuild(dir);
    }
    let depth = hashes.iter().map(|(_, f)| f.matches('/').count()).max().unwrap_or(0);
    (hashes.into_iter().map(|(h, _)| h).collect(), depth)
}

impl LutLibrary {
    /// Packs whose every LUT is also in one other pack, as `pack → the pack that covers it`. Of two
    /// packs with identical content the one with deeper folders (then the later name) is flagged, so
    /// a parent folder installed over its own sub-folder is the duplicate. Nothing is deleted.
    pub fn redundant_packs(&self) -> HashMap<String, String> {
        let Ok(entries) = fs::read_dir(&self.root) else { return HashMap::new() };
        let mut packs: Vec<(String, HashSet<String>, usize)> = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !e.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let (hashes, depth) = pack_hashes(&e.path());
            if !hashes.is_empty() {
                packs.push((name, hashes, depth));
            }
        }
        let key = |p: &(String, HashSet<String>, usize)| (p.2, p.0.to_lowercase());
        let mut out = HashMap::new();
        for a in &packs {
            let cover = packs.iter().filter(|b| b.0 != a.0 && a.1.is_subset(&b.1) && (a.1 != b.1 || key(a) > key(b))).min_by_key(|b| key(b));
            if let Some(b) = cover {
                out.insert(a.0.clone(), b.0.clone());
            }
        }
        out
    }
    /// `content hash → (pack, path in pack)` of every installed LUT outside the pack `exclude`.
    pub(super) fn known_hashes(&self, exclude: Option<&str>) -> HashMap<String, (String, String)> {
        let mut out = HashMap::new();
        let Ok(entries) = fs::read_dir(&self.root) else { return out };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || exclude == Some(name.as_str()) || !entry.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let dir = entry.path();
            let present = hashable(&dir);
            let mut hashes = read_sidecar(&dir).unwrap_or_default();
            // A pack edited by hand (or installed by an older build) no longer matches its sidecar.
            if hashes.len() != present.len() || hashes.iter().any(|(_, f)| !present.contains(f)) {
                hashes = rebuild(&dir);
            }
            for (hash, file) in hashes {
                out.entry(hash).or_insert_with(|| (name.clone(), file));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-lut-hashes-{name}-{}", super::super::unique_suffix()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_capped_says_why_it_refused() {
        let dir = temp("capped");
        fs::write(dir.join("ok.cube"), b"TITLE \"x\"\n").unwrap();
        assert_eq!(read_capped(&dir.join("ok.cube")).map(|b| b.len()), Ok(10));
        let big = fs::File::create(dir.join("huge.cube")).unwrap();
        big.set_len(MAX_LUT_BYTES + 1).unwrap();
        drop(big);
        assert!(read_capped(&dir.join("huge.cube")).unwrap_err().contains("too large"));
        assert!(read_capped(&dir.join("missing.cube")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebuild_skips_a_file_over_the_size_cap_without_reading_it() {
        let dir = temp("oversize");
        fs::write(dir.join("small.cube"), b"TITLE \"x\"\n").unwrap();
        // A sparse file: the length is set, nothing is written, so a read would allocate 64 MiB.
        let big = fs::File::create(dir.join("huge.cube")).unwrap();
        big.set_len(MAX_LUT_BYTES + 1).unwrap();
        drop(big);
        assert!(read_bounded(&dir.join("huge.cube")).is_none());
        let hashes = rebuild(&dir);
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].1, "small.cube");
        // The sidecar is written without the oversize file too.
        assert_eq!(read_sidecar(&dir).map(|h| h.len()), Some(1));
        // And that sidecar is not stale, so it is not rebuilt on the next scan.
        assert_eq!(pack_hashes(&dir).0.len(), 1);
        fs::write(dir.join(HASH_FILE), "abc\tsmall.cube\n").unwrap();
        assert!(pack_hashes(&dir).0.contains("abc"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_under_the_cap_is_read() {
        let dir = temp("atcap");
        let f = fs::File::create(dir.join("edge.cube")).unwrap();
        f.set_len(1024).unwrap();
        drop(f);
        assert_eq!(read_bounded(&dir.join("edge.cube")).map(|b| b.len()), Some(1024));
        let _ = fs::remove_dir_all(&dir);
    }
}
