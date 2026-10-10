//! A minimal, bounded `.zip` reader for LUT packs: stored and deflated entries only, no zip64,
//! no encryption. Only LUT files are extracted, into a directory of the caller's choosing, under
//! names rebuilt from a restricted character set (a hostile archive cannot write elsewhere).

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::{MAX_DEPTH, MAX_FILES, MAX_LUT_BYTES, clean_component, is_lut};

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
/// Largest central directory read (50 000 entries with long names).
const MAX_CENTRAL_BYTES: u64 = 16 << 20;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

struct Entry {
    name: String,
    method: u16,
    flags: u16,
    compressed: u64,
    offset: u64,
}

fn central_directory(f: &mut fs::File) -> Result<Vec<Entry>, String> {
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let tail = len.min(66_000);
    f.seek(SeekFrom::Start(len - tail)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; tail as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    let at = (0..buf.len().saturating_sub(21)).rev().find(|&i| u32_at(&buf, i) == Some(EOCD_SIG)).ok_or("not a zip file")?;
    let (count, size, offset) = (u16_at(&buf, at + 10), u32_at(&buf, at + 12), u32_at(&buf, at + 16));
    let (Some(count), Some(size), Some(offset)) = (count, size, offset) else { return Err("damaged zip file".into()) };
    if count == u16::MAX || size == u32::MAX || offset == u32::MAX {
        return Err("zip64 archives are not supported".into());
    }
    if u64::from(size) > MAX_CENTRAL_BYTES || u64::from(offset) + u64::from(size) > len {
        return Err("damaged zip file".into());
    }
    f.seek(SeekFrom::Start(u64::from(offset))).map_err(|e| e.to_string())?;
    let mut cd = vec![0u8; size as usize];
    f.read_exact(&mut cd).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut p = 0usize;
    while u32_at(&cd, p) == Some(CENTRAL_SIG) && entries.len() < 50_000 {
        let (flags, method) = (u16_at(&cd, p + 8).unwrap_or(0), u16_at(&cd, p + 10).unwrap_or(0));
        let compressed = u64::from(u32_at(&cd, p + 20).unwrap_or(0));
        let (n, x, c) = (u16_at(&cd, p + 28).unwrap_or(0) as usize, u16_at(&cd, p + 30).unwrap_or(0) as usize, u16_at(&cd, p + 32).unwrap_or(0) as usize);
        let offset = u64::from(u32_at(&cd, p + 42).unwrap_or(0));
        let name = cd.get(p + 46..p + 46 + n).map(|b| String::from_utf8_lossy(b).into_owned()).ok_or("damaged zip file")?;
        entries.push(Entry { name, method, flags, compressed, offset });
        p += 46 + n + x + c;
    }
    Ok(entries)
}

/// The archive path as clean components, or `None` for anything that is not a plain relative LUT path.
fn safe_parts(name: &str) -> Option<Vec<String>> {
    let norm = name.replace('\\', "/");
    if norm.ends_with('/') || norm.starts_with('/') || norm.contains(':') {
        return None;
    }
    let parts: Vec<&str> = norm.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.starts_with('.') || *p == "__MACOSX") || parts.len() > MAX_DEPTH + 1 {
        return None;
    }
    is_lut(Path::new(&norm)).then(|| parts.into_iter().map(clean_component).collect())
}

/// Extract the LUT files of the archive `zip` into `dest` (created). `budget` bounds the total
/// extracted size; going over it refuses with the library-size message for `cap`. `progress`
/// returns `false` to cancel.
pub(super) fn extract(zip: &Path, dest: &Path, budget: u64, cap: u64, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<(), String> {
    let mut f = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let entries = central_directory(&mut f).map_err(|e| format!("{}: {e}", zip.display()))?;
    let wanted: Vec<(&Entry, Vec<String>)> = entries.iter().filter_map(|e| safe_parts(&e.name).map(|p| (e, p))).collect();
    if wanted.is_empty() {
        return Err(format!("no .cube, .3dl or .look files found in {}", zip.display()));
    }
    if wanted.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} LUT files in {}", zip.display()));
    }
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut used = 0u64;
    for (i, (e, parts)) in wanted.iter().enumerate() {
        if !progress(i as f32 / wanted.len() as f32 * 0.5, &e.name) {
            return Err("cancelled".into());
        }
        if e.flags & 1 != 0 || e.compressed > MAX_LUT_BYTES {
            continue;
        }
        f.seek(SeekFrom::Start(e.offset)).map_err(|er| er.to_string())?;
        let mut head = [0u8; 30];
        if f.read_exact(&mut head).is_err() || u32_at(&head, 0) != Some(LOCAL_SIG) {
            continue;
        }
        let skip = u64::from(u16_at(&head, 26).unwrap_or(0)) + u64::from(u16_at(&head, 28).unwrap_or(0));
        f.seek(SeekFrom::Current(skip as i64)).map_err(|er| er.to_string())?;
        let raw = f.by_ref().take(e.compressed);
        let mut data = Vec::new();
        let ok = match e.method {
            0 => raw.take(MAX_LUT_BYTES + 1).read_to_end(&mut data).is_ok(),
            8 => flate2::read::DeflateDecoder::new(raw).take(MAX_LUT_BYTES + 1).read_to_end(&mut data).is_ok(),
            _ => false,
        };
        if !ok || data.len() as u64 > MAX_LUT_BYTES {
            continue;
        }
        used = used.saturating_add(data.len() as u64);
        if used > budget {
            return Err(super::budget::grow_error(cap));
        }
        let mut path = parts.iter().fold(dest.to_path_buf(), |p, part| p.join(part));
        let mut n = 2;
        while path.exists() {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("lut").to_string();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("cube").to_string();
            path.set_file_name(format!("{stem} {n}.{ext}"));
            n += 1;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|er| format!("{}: {er}", parent.display()))?;
        }
        fs::write(&path, &data).map_err(|er| format!("{}: {er}", path.display()))?;
    }
    Ok(())
}
