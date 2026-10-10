//! Bounded index of external DCP headers. Indexing never loads the proprietary table payloads.
#[derive(Clone)]
pub(super) struct Entry {
    pub path: std::path::PathBuf,
    pub model: String,
    pub name: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn roots() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_CAMERA_PROFILES_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".local/share/photocraft/camera-profiles"));
        #[cfg(target_os = "macos")]
        roots.push(home.join("Library/Application Support/Photocraft/camera-profiles"));
    }
    #[cfg(target_os = "windows")]
    if let Some(dir) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(dir).join("Photocraft/camera-profiles"));
    }
    roots
}

#[cfg(not(target_arch = "wasm32"))]
fn header(path: &std::path::Path) -> Option<Entry> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if !(8..=32 * 1024 * 1024).contains(&size) {
        return None;
    }
    let mut head = [0u8; 8];
    file.read_exact(&mut head).ok()?;
    let le = match head.get(..4)? {
        b"IIRC" => true,
        b"MMCR" => false,
        _ => return None,
    };
    let u16v = |s: &[u8]| {
        let b = <[u8; 2]>::try_from(s).ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32v = |s: &[u8]| {
        let b = <[u8; 4]>::try_from(s).ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let offset = u64::from(u32v(head.get(4..8)?)?);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut count = [0; 2];
    file.read_exact(&mut count).ok()?;
    let count = usize::from(u16v(&count)?);
    if count == 0 || count > 1024 {
        return None;
    }
    let mut directory = vec![0u8; count * 12];
    file.read_exact(&mut directory).ok()?;
    let mut model = None;
    let mut name = None;
    for e in directory.as_chunks::<12>().0 {
        let tag = u16v(e.get(..2)?)?;
        if !matches!(tag, 50708 | 50936) {
            continue;
        }
        let count = u32v(e.get(4..8)?)? as usize;
        if u16v(e.get(2..4)?)? != 2 || !(1..=129).contains(&count) {
            return None;
        }
        let bytes = if count <= 4 {
            e.get(8..8 + count)?.to_vec()
        } else {
            let at = u64::from(u32v(e.get(8..12)?)?);
            if at.checked_add(count as u64)? > size {
                return None;
            }
            file.seek(SeekFrom::Start(at)).ok()?;
            let mut bytes = vec![0; count];
            file.read_exact(&mut bytes).ok()?;
            bytes
        };
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let value = std::str::from_utf8(bytes.get(..end)?).ok()?.trim();
        if value.is_empty() || value.chars().any(char::is_control) {
            return None;
        }
        if tag == 50708 {
            if model.is_some() {
                return None;
            }
            model = Some(value.to_string());
        } else {
            if name.is_some() {
                return None;
            }
            name = Some(value.to_string());
        }
    }
    Some(Entry { path: path.to_path_buf(), model: model?, name: name? })
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn entries(refresh: bool) -> Vec<Entry> {
    use std::{
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    type Cached = Option<(Vec<PathBuf>, Instant, Vec<Entry>)>;
    static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
    let roots = roots();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !refresh
        && let Some((old, time, entries)) = cache.as_ref()
        && *old == roots
        && time.elapsed() < Duration::from_secs(60)
    {
        return entries.clone();
    }
    let mut entries = Vec::new();
    let mut visited = 0usize;
    let mut stack: Vec<_> = roots.iter().map(|p| (p.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(files) = std::fs::read_dir(dir) else { continue };
        for item in files.flatten() {
            visited += 1;
            if visited > 40_000 {
                break;
            }
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir() && depth < 5 {
                stack.push((item.path(), depth + 1));
            } else if kind.is_file()
                && item.path().extension().is_some_and(|s| s.to_str().is_some_and(|s| s.eq_ignore_ascii_case("dcp")))
                && let Some(entry) = header(&item.path())
            {
                entries.push(entry);
            }
        }
        if visited > 40_000 {
            break;
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    *cache = Some((roots, Instant::now(), entries.clone()));
    entries
}

#[cfg(target_arch = "wasm32")]
pub(super) fn entries(_: bool) -> Vec<Entry> {
    Vec::new()
}
