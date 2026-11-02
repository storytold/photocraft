//! Bounded, local-only discovery. Never executes candidates to inspect them or copies a runtime.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::Error;

const LIMIT: usize = 256;

struct Locations {
    explicit: Option<OsString>,
    desktop_bins: Vec<PathBuf>,
    desktop_executables: Vec<PathBuf>,
    search_path: Vec<PathBuf>,
}

pub(super) fn resolve(requested: &Path) -> Result<PathBuf, Error> {
    let mut locations = Locations {
        explicit: std::env::var_os("PHOTOCRAFT_CODEX_PATH"),
        desktop_bins: Vec::new(),
        desktop_executables: Vec::new(),
        search_path: std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).take(LIMIT).collect()).unwrap_or_default(),
    };
    if !requested.as_os_str().is_empty() {
        locations.explicit = Some(requested.as_os_str().to_owned());
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            locations.desktop_bins.push(PathBuf::from(local).join("OpenAI/Codex/bin"));
        }
        // Sandboxed launchers can redirect LOCALAPPDATA. The user's installation stays here.
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            locations.desktop_bins.push(PathBuf::from(profile).join("AppData/Local/OpenAI/Codex/bin"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        locations.desktop_executables.push("/Applications/Codex.app/Contents/Resources/codex".into());
        if let Some(home) = std::env::var_os("HOME") {
            locations.desktop_executables.push(PathBuf::from(home).join("Applications/Codex.app/Contents/Resources/codex"));
        }
    }
    locations.find()
}

fn executable_file(path: &Path) -> Option<PathBuf> {
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    path.canonicalize().ok()
}

fn executable_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

impl Locations {
    fn in_path(&self, name: &OsStr) -> Option<PathBuf> {
        // Never search empty/relative PATH entries: the open document's directory should not
        // be able to supply a program just because a terminal included '.' in PATH.
        self.search_path.iter().take(LIMIT).filter(|dir| dir.is_absolute()).find_map(|dir| executable_file(&dir.join(name)))
    }

    fn find(&self) -> Result<PathBuf, Error> {
        if let Some(explicit) = &self.explicit {
            let path = Path::new(explicit);
            let selected = if path.is_absolute() || path.components().count() > 1 {
                executable_file(path)
            } else if path.as_os_str().is_empty() {
                None
            } else {
                self.in_path(explicit)
            };
            return selected.ok_or_else(|| {
                Error::RuntimeUnavailable("The configured Codex executable is unavailable. Correct PHOTOCRAFT_CODEX_PATH or remove it to use automatic detection. Keep Codex in its complete installation directory.".into())
            });
        }
        let mut latest: Option<(SystemTime, PathBuf)> = None;
        for root in self.desktop_bins.iter().take(4).filter(|root| root.is_absolute()) {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            for entry in entries.take(LIMIT).filter_map(Result::ok) {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let candidate = entry.path().join(executable_name());
                let Some(path) = executable_file(&candidate) else { continue };
                let modified = path.metadata().and_then(|metadata| metadata.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                let next = (modified, path);
                if latest.as_ref().is_none_or(|previous| &next > previous) {
                    latest = Some(next);
                }
            }
        }
        if let Some((_, path)) = latest {
            return Ok(path);
        }
        if let Some(path) = self.desktop_executables.iter().take(4).filter(|path| path.is_absolute()).find_map(|path| executable_file(path)) {
            return Ok(path);
        }
        self.in_path(OsStr::new(executable_name())).ok_or_else(|| {
            Error::RuntimeUnavailable("Codex was not found. Install and open the complete Codex desktop app, then check the connection again. A standalone codex executable may lack image-editing components.".into())
        })
    }
}

#[cfg(test)]
mod tests;
