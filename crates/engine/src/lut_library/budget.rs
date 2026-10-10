//! The library size budget, shared by folder and `.zip` installs so both refuse with the same
//! message and a `.zip` is checked before it is unpacked, not after.

use std::fs;
use std::path::Path;

use super::{InstallOptions, LutLibrary, clean_component};

/// The pack name an install will use: the requested one, else `fallback` (the folder or archive
/// name), always rebuilt from the restricted character set.
pub(super) fn pack_name(opts: &InstallOptions, fallback: &str) -> String {
    clean_component(opts.pack.as_deref().map(str::trim).filter(|p| !p.is_empty()).unwrap_or(fallback))
}

/// The refusal for an install that would take the library past `cap` bytes.
pub(super) fn grow_error(cap: u64) -> String {
    format!("the LUT library would grow past {} GiB; remove a pack first", cap >> 30)
}

/// Bytes in all the files below `dir`.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(k) if k.is_dir() => dir_bytes(&e.path()),
            Ok(k) if k.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

impl LutLibrary {
    /// Bytes an install into `dest` may still add: `cap` minus the library's current size, with the
    /// pack at `dest` added back when it exists because installing replaces it.
    pub(super) fn budget_for(&self, dest: &Path, cap: u64) -> u64 {
        let replaced = if dest.exists() { dir_bytes(dest) } else { 0 };
        cap.saturating_sub(dir_bytes(&self.root).saturating_sub(replaced))
    }
}
