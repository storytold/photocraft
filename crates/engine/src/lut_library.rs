//! The LUT library: creative 3D LUTs installed once from a folder or a `.zip` (a "LUT pack") and
//! kept in a directory next to the preferences (desktop: `<config dir>/Presets/LUTs/<pack>/…`).
//!
//! Color Lookup loads a LUT from a path and embeds the parsed table in the document, so the
//! library only has to hold the files and list them:
//!
//! - [`LutLibrary::install`] validates every `.cube`, `.3dl` and `.look` file below a folder (or
//!   inside a `.zip`) with the same parser Color Lookup uses, skips LUTs that are already
//!   installed in another pack (same content), copies the rest into a staging directory and swaps
//!   it in as one pack, so a failed or cancelled install leaves the library as it was. Invalid
//!   files are skipped and reported, never a reason to fail the whole pack.
//! - [`LutLibrary::list`] walks the library (no parsing) and returns the packs with their LUTs.
//! - [`LutLibrary::remove_pack`] deletes one pack.
//! - Favourites and recently used LUTs are kept in `.state.json` ([`state`]), file headers can be
//!   read cheaply ([`meta`]), and parsed tables are cached so
//!   previewing many LUTs does not parse the same file twice ([`cache`]).
//!
//! Names that reach the file system are always rebuilt from a restricted character set
//! ([`clean_component`]), symbolic links are never followed, and depth, file count, file size and
//! library size are all bounded, so a hostile folder or archive cannot escape the library or
//! exhaust the disk. A [`Session`](crate::Session) has no library by default (headless, tests and
//! the web build never touch the disk); the desktop app attaches one. Commands: `lut_library_cmds`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

mod budget;
pub mod cache;
mod hashes;
pub mod meta;
mod read;
pub mod state;
mod zip;

/// File extensions that count as a LUT (compared case-insensitively).
pub const LUT_EXTS: [&str; 3] = ["cube", "3dl", "look"];
/// Largest LUT file read (a 129³ `.cube` is about 40 MB).
pub const MAX_LUT_BYTES: u64 = 64 << 20;
/// Most LUT files looked at in one install, and listed from one pack.
pub const MAX_FILES: usize = 5000;
/// Deepest folder level searched below the folder being installed.
pub const MAX_DEPTH: usize = 6;
/// The library never grows beyond this many bytes; an install that would is refused.
pub const MAX_LIBRARY_BYTES: u64 = 4 << 30;
/// Longest pack, folder or file name kept (in characters).
pub const MAX_NAME_CHARS: usize = 96;
/// Staging directories left by a crash are removed once they are this old.
const STALE_SECS: u64 = 3600;

/// One installed LUT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LutInfo {
    /// File name without the extension.
    pub name: String,
    /// Path inside the pack, with `/` separators.
    pub file: String,
    pub bytes: u64,
    /// Modification time, seconds since the Unix epoch (0 when unknown).
    pub mtime: u64,
}

/// One installed pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    pub name: String,
    pub luts: Vec<LutInfo>,
}

impl PackInfo {
    pub fn bytes(&self) -> u64 {
        self.luts.iter().map(|l| l.bytes).sum()
    }
}

/// How an install behaves.
#[derive(Clone, Debug)]
pub struct InstallOptions {
    /// Pack name; default the folder's (or archive's) name.
    pub pack: Option<String>,
    /// Replace an installed pack of the same name instead of failing.
    pub replace: bool,
    /// Install LUTs even when their content is already in another pack.
    pub allow_duplicates: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        InstallOptions { pack: None, replace: true, allow_duplicates: false }
    }
}

/// What an install did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
    pub pack: String,
    /// Paths (inside the pack) of the LUTs installed.
    pub installed: Vec<String>,
    /// `(path in the source, reason)` for every file left out.
    pub skipped: Vec<(String, String)>,
    /// `(path in the source, "Pack/path" of the identical installed LUT)`.
    pub duplicates: Vec<(String, String)>,
    /// A pack of the same name was replaced.
    pub replaced: bool,
}

/// A directory of installed LUT packs.
#[derive(Clone, Debug)]
pub struct LutLibrary {
    root: PathBuf,
    /// Bumped whenever the packs, favourites or recent list change, so views can cache them.
    pub rev: u64,
    state: state::State,
}

pub(crate) fn is_lut(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| LUT_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn reserved(stem: &str) -> bool {
    let up = stem.split('.').next().unwrap_or_default().to_ascii_uppercase();
    matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((up.starts_with("COM") || up.starts_with("LPT")) && up.len() == 4 && up.chars().last().is_some_and(|c| c.is_ascii_digit()))
}

/// A file or folder name that is safe on Windows, macOS and Linux: letters, digits and
/// ` _-.()+,'` only, no leading or trailing dot or space, no reserved device name, at most
/// [`MAX_NAME_CHARS`] characters, never empty.
pub fn clean_component(name: &str) -> String {
    let mapped: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '(' | ')' | '+' | ',' | '\'') { c } else { '_' }).collect();
    let mut out: String = mapped.trim_matches([' ', '.']).chars().take(MAX_NAME_CHARS).collect();
    out = out.trim_end_matches([' ', '.']).to_string();
    if out.is_empty() {
        out = "_".into();
    }
    if reserved(&out) {
        out.insert(0, '_');
    }
    out
}

/// A LUT file found below the folder being installed: its path and its name components.
pub(crate) struct Found {
    pub path: PathBuf,
    pub rel: Vec<String>,
}

/// Every LUT file below `src` (depth- and count-bounded, symbolic links and hidden entries
/// skipped), in a stable order.
pub(crate) fn collect(src: &Path) -> Result<Vec<Found>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(src.to_path_buf(), Vec::new())];
    while let Some((dir, rel)) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "__MACOSX" {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            let mut next = rel.clone();
            next.push(name);
            if kind.is_dir() {
                if next.len() <= MAX_DEPTH {
                    stack.push((entry.path(), next));
                }
            } else if kind.is_file() && is_lut(&entry.path()) {
                if out.len() >= MAX_FILES {
                    return Err(format!("more than {MAX_FILES} LUT files in {}; install a smaller folder", src.display()));
                }
                out.push(Found { path: entry.path(), rel: next });
            }
        }
    }
    out.sort_by_key(|f| f.rel.join("/").to_lowercase());
    Ok(out)
}

/// The cleaned in-pack path for `rel` (`dir/dir/name.ext`), made unique among `taken`.
fn pack_path(rel: &[String], taken: &mut HashSet<String>) -> String {
    let (dirs, file) = match rel.split_last() {
        Some((file, dirs)) => (dirs, file.as_str()),
        None => (&[][..], "_"),
    };
    let ext = Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or("cube").to_ascii_lowercase();
    let stem = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("_");
    let dir_part: Vec<String> = dirs.iter().map(|d| clean_component(d)).collect();
    let stem = clean_component(stem);
    let build = |s: &str| {
        let mut parts = dir_part.clone();
        parts.push(format!("{s}.{ext}"));
        parts.join("/")
    };
    let mut candidate = build(&stem);
    let mut n = 2;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = build(&format!("{stem} {n}"));
        n += 1;
    }
    candidate
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn unique_suffix() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}-{t}", std::process::id())
}

/// The web build has no library (nothing calls this), and `process::id` is unsupported there.
#[cfg(target_arch = "wasm32")]
pub(crate) fn unique_suffix() -> String {
    "0".into()
}

/// Write a file so a crash leaves the old or the new content, never half of one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", unique_suffix()));
    fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// What staging a pack produced.
#[derive(Default)]
struct Staged {
    installed: Vec<String>,
    skipped: Vec<(String, String)>,
    duplicates: Vec<(String, String)>,
    /// `(content hash, path in the pack)` of every installed file.
    hashes: Vec<(String, String)>,
}

impl LutLibrary {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let state = state::State::load(&root);
        LutLibrary { root, rev: 0, state }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path of an installed LUT.
    pub fn path_of(&self, pack: &str, file: &str) -> PathBuf {
        file.split('/').fold(self.root.join(pack), |p, part| p.join(part))
    }

    /// Remove staging directories a crashed install left behind.
    fn remove_stale(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let age = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
            if (name.starts_with(".installing-") || name.starts_with(".replaced-")) && age > STALE_SECS {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }

    /// Every installed pack, sorted by name, with its LUTs sorted by path. Nothing is parsed. A
    /// missing library is empty, not an error.
    pub fn list(&self) -> Result<Vec<PackInfo>, String> {
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", self.root.display())),
        };
        let mut packs = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let mut luts = Vec::new();
            if let Ok(found) = collect(&entry.path()) {
                for f in found {
                    let meta = fs::metadata(&f.path).ok();
                    let mtime =
                        meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                    let file = f.rel.join("/");
                    let stem = Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                    luts.push(LutInfo { name: stem, file, bytes: meta.map_or(0, |m| m.len()), mtime });
                }
            }
            packs.push(PackInfo { name, luts });
        }
        packs.sort_by_key(|p| p.name.to_lowercase());
        Ok(packs)
    }

    /// Delete the pack `name` (exactly as [`list`](Self::list) returned it).
    pub fn remove_pack(&mut self, name: &str) -> Result<(), String> {
        if name.is_empty() || clean_component(name) != name || name.starts_with('.') {
            return Err(format!("`{name}` is not a pack name"));
        }
        let dir = self.root.join(name);
        let meta = fs::symlink_metadata(&dir).map_err(|_| format!("no LUT pack named `{name}`"))?;
        if !meta.is_dir() {
            return Err(format!("no LUT pack named `{name}`"));
        }
        fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        self.state.forget_pack(name);
        self.state.save(&self.root);
        self.rev += 1;
        Ok(())
    }

    /// Install the LUTs below the folder (or inside the `.zip`) `src` as the pack `pack` (default:
    /// its name), replacing an installed pack of that name when `replace` is set. See
    /// [`install_with`](Self::install_with).
    pub fn install(&self, src: &Path, pack: Option<&str>, replace: bool, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<InstallReport, String> {
        self.install_with(src, &InstallOptions { pack: pack.map(str::to_string), replace, allow_duplicates: false }, progress)
    }

    /// Install LUTs from a folder or a `.zip`. `progress(fraction, message)` is called before each
    /// file and returns `false` to cancel. The library only changes when the whole pack installed.
    pub fn install_with(&self, src: &Path, opts: &InstallOptions, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<InstallReport, String> {
        self.install_capped(src, opts, MAX_LIBRARY_BYTES, progress)
    }

    /// [`install_with`](Self::install_with) with the library size limit as a parameter, so tests can
    /// use a small one.
    pub(crate) fn install_capped(
        &self,
        src: &Path,
        opts: &InstallOptions,
        cap: u64,
        progress: &mut dyn FnMut(f32, &str) -> bool,
    ) -> Result<InstallReport, String> {
        let meta = fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if meta.is_file() && src.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            // Outside the library, so the extracted folder is never mistaken for part of it.
            let unzipped = std::env::temp_dir().join(format!("photocraft-luts-unzip-{}", unique_suffix()));
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("LUTs");
            let named = InstallOptions { pack: opts.pack.clone().or_else(|| Some(stem.to_string())), ..opts.clone() };
            // Cap the unpacking at what the library has room for, so an archive that cannot fit is
            // refused before it fills the temp directory. LUTs that install_dir later skips as
            // duplicates still count toward this early cap, so it is slightly conservative.
            self.remove_stale();
            let budget = self.budget_for(&self.root.join(budget::pack_name(&named, stem)), cap);
            let result = zip::extract(src, &unzipped, budget, cap, progress).and_then(|()| self.install_dir(&unzipped, &named, cap, progress));
            let _ = fs::remove_dir_all(&unzipped);
            return result;
        }
        self.install_dir(src, opts, cap, progress)
    }

    fn install_dir(&self, src: &Path, opts: &InstallOptions, cap: u64, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<InstallReport, String> {
        let meta = fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if !meta.is_dir() {
            return Err(format!("{}: not a folder or .zip", src.display()));
        }
        let fallback = src.file_name().and_then(|n| n.to_str()).unwrap_or("LUTs");
        let pack = budget::pack_name(opts, fallback);
        if pack.starts_with('.') {
            return Err(format!("`{pack}` is not a valid pack name"));
        }
        // Installing the library into itself would copy it forever.
        if let (Ok(a), Ok(b)) = (fs::canonicalize(src), fs::canonicalize(&self.root))
            && a.starts_with(&b)
        {
            return Err("that folder is already inside the LUT library".into());
        }
        let found = collect(src)?;
        if found.is_empty() {
            return Err(format!("no .cube, .3dl or .look files found in {}", src.display()));
        }
        let dest = self.root.join(&pack);
        let replaced = dest.exists();
        if replaced && !opts.replace {
            return Err(format!("a LUT pack named `{pack}` is already installed"));
        }
        fs::create_dir_all(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        self.remove_stale();
        let budget = self.budget_for(&dest, cap);
        let known = if opts.allow_duplicates { HashMap::new() } else { self.known_hashes(Some(&pack)) };
        let stage = self.root.join(format!(".installing-{}", unique_suffix()));
        fs::create_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        let staged = match self.fill_stage(&found, &stage, budget, cap, &known, progress) {
            Ok(s) if !s.installed.is_empty() => s,
            Ok(s) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(if let Some((file, other)) = s.duplicates.first() {
                    let packs: HashSet<&str> = s.duplicates.iter().filter_map(|(_, o)| o.split('/').next()).collect();
                    let mut packs: Vec<&str> = packs.into_iter().collect();
                    packs.sort_unstable();
                    format!(
                        "every LUT in {} is already installed (for example {file} is {other} in {}); install it anyway with allowDuplicates",
                        src.display(),
                        packs.join(", ")
                    )
                } else {
                    let why = s.skipped.first().map(|(f, r)| format!(" (first problem: {f}: {r})")).unwrap_or_default();
                    format!("none of the {} files in {} is a usable LUT{why}", found.len(), src.display())
                });
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(e);
            }
        };
        hashes::write_sidecar(&stage, &staged.hashes);
        let old = self.root.join(format!(".replaced-{}", unique_suffix()));
        if replaced {
            fs::rename(&dest, &old).map_err(|e| {
                let _ = fs::remove_dir_all(&stage);
                format!("{}: {e}", dest.display())
            })?;
        }
        if let Err(e) = fs::rename(&stage, &dest) {
            if replaced {
                let _ = fs::rename(&old, &dest);
            }
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("{}: {e}", dest.display()));
        }
        if replaced {
            let _ = fs::remove_dir_all(&old);
        }
        Ok(InstallReport { pack, installed: staged.installed, skipped: staged.skipped, duplicates: staged.duplicates, replaced })
    }

    /// Validate and write the files into `stage`.
    fn fill_stage(
        &self,
        found: &[Found],
        stage: &Path,
        budget: u64,
        cap: u64,
        known: &HashMap<String, (String, String)>,
        progress: &mut dyn FnMut(f32, &str) -> bool,
    ) -> Result<Staged, String> {
        let mut out = Staged::default();
        let mut taken = HashSet::new();
        let mut used = 0u64;
        let total = found.len().max(1) as f32;
        for (i, f) in found.iter().enumerate() {
            let shown = f.rel.join("/");
            if !progress(i as f32 / total, &shown) {
                return Err("cancelled".into());
            }
            let bytes = match hashes::read_capped(&f.path) {
                Ok(b) => b,
                Err(e) => {
                    out.skipped.push((shown, e));
                    continue;
                }
            };
            let name = f.rel.last().map_or("lut.cube", String::as_str);
            if let Err(e) = photocraft_cms::lutfile::parse(name, &bytes) {
                out.skipped.push((shown, e.0));
                continue;
            }
            let hash = hashes::hash_bytes(&bytes);
            if let Some((pack, file)) = known.get(&hash) {
                out.duplicates.push((shown, format!("{pack}/{file}")));
                continue;
            }
            used = used.saturating_add(bytes.len() as u64);
            if used > budget {
                return Err(budget::grow_error(cap));
            }
            let rel = pack_path(&f.rel, &mut taken);
            let target = rel.split('/').fold(stage.to_path_buf(), |p, part| p.join(part));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::write(&target, &bytes).map_err(|e| format!("{}: {e}", target.display()))?;
            out.hashes.push((hash, rel.clone()));
            out.installed.push(rel);
        }
        progress(1.0, "Done");
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "lut_library/tests_more.rs"]
mod tests_more;
