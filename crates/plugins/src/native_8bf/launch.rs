//! Platform paths, kept separate from process creation so discovery never executes a worker.
use super::pe::Architecture;
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

pub(super) fn worker_path(architecture: Architecture, executable: &Path, macos: bool, env: impl Fn(&str) -> Option<OsString>) -> io::Result<PathBuf> {
    let (key, file) = match architecture {
        Architecture::X86 => ("PHOTOCRAFT_8BF_WORKER32", "photocraft-8bf-worker32.exe"),
        Architecture::X64 => ("PHOTOCRAFT_8BF_WORKER64", "photocraft-8bf-worker64.exe"),
    };
    if let Some(path) = env(key).filter(|v| !v.is_empty()) {
        return Ok(path.into());
    }
    let folder = executable.parent().ok_or_else(|| io::Error::other("cannot find PhotoCraft executable folder"))?;
    if macos
        && folder.file_name().is_some_and(|n| n == "MacOS")
        && let Some(contents) = folder.parent().filter(|p| p.file_name().is_some_and(|n| n == "Contents"))
    {
        return Ok(contents.join("Resources/8bf").join(file));
    }
    Ok(folder.join(file))
}

#[cfg(not(windows))]
pub(super) fn wine_prefix(macos: bool, env: impl Fn(&str) -> Option<OsString>) -> io::Result<PathBuf> {
    let prefix = if let Some(prefix) = env("PHOTOCRAFT_8BF_WINEPREFIX").filter(|v| !v.is_empty()) {
        PathBuf::from(prefix)
    } else if macos {
        PathBuf::from(
            env("HOME").filter(|v| !v.is_empty()).ok_or_else(|| io::Error::other("configure PHOTOCRAFT_8BF_WINEPREFIX for an isolated 8BF Wine prefix"))?,
        )
        .join("Library/Application Support/Photocraft/8bf-wine")
    } else {
        let config = env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".config")))
            .ok_or_else(|| io::Error::other("configure PHOTOCRAFT_8BF_WINEPREFIX or XDG_CONFIG_HOME for an isolated 8BF Wine prefix"))?;
        config.join("photocraft/8bf-wine")
    };
    if !prefix.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "8BF Wine prefix must be an absolute path"));
    }
    Ok(prefix)
}

#[cfg(any(not(windows), test))]
pub(super) fn wine_program(macos: bool, env: impl Fn(&str) -> Option<OsString>, is_file: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(program) = env("PHOTOCRAFT_8BF_WINE").filter(|v| !v.is_empty()) {
        return program.into();
    }
    // Finder does not inherit a terminal's Homebrew PATH. Respect PATH first,
    // then try standard package-manager locations; a custom runtime can be explicit.
    if macos {
        if let Some(paths) = env("PATH")
            && let Some(program) = std::env::split_paths(&paths).map(|p| p.join("wine")).find(|p| is_file(p))
        {
            return program;
        }
        for program in ["/opt/homebrew/bin/wine", "/usr/local/bin/wine", "/opt/local/bin/wine"] {
            let program = PathBuf::from(program);
            if is_file(&program) {
                return program;
            }
        }
    }
    "wine".into()
}

#[cfg(not(windows))]
pub(super) fn is_executable(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            metadata.is_file()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn env(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        move |key| values.iter().find_map(|(k, v)| (*k == key).then(|| OsString::from(v)))
    }
    #[test]
    fn bundled_workers_are_resources_and_cli_workers_are_adjacent() {
        let app = Path::new("/Applications/PhotoCraft.app/Contents/MacOS/PhotoCraft");
        for (arch, bits) in [(Architecture::X86, 32), (Architecture::X64, 64)] {
            assert_eq!(
                worker_path(arch, app, true, env(&[])).unwrap(),
                PathBuf::from(format!("/Applications/PhotoCraft.app/Contents/Resources/8bf/photocraft-8bf-worker{bits}.exe"))
            );
            assert_eq!(
                worker_path(arch, Path::new("/opt/photocraft/photocraft"), true, env(&[])).unwrap(),
                PathBuf::from(format!("/opt/photocraft/photocraft-8bf-worker{bits}.exe"))
            );
        }
    }
    #[test]
    fn override_wins_without_requiring_worker_to_exist() {
        assert_eq!(
            worker_path(Architecture::X86, Path::new("/app/photocraft"), true, env(&[("PHOTOCRAFT_8BF_WORKER32", "/custom path/worker.exe")])).unwrap(),
            PathBuf::from("/custom path/worker.exe")
        );
        assert_eq!(
            worker_path(Architecture::X64, Path::new("/app/photocraft.exe"), false, env(&[])).unwrap(),
            PathBuf::from("/app/photocraft-8bf-worker64.exe")
        );
    }
    // Prefix paths are Unix paths; these behavioral checks run on Linux and macOS.
    #[cfg(unix)]
    #[test]
    fn prefix_defaults_follow_platform_and_override_needs_no_home() {
        assert_eq!(wine_prefix(true, env(&[("HOME", "/Users/test")])).unwrap(), PathBuf::from("/Users/test/Library/Application Support/Photocraft/8bf-wine"));
        assert_eq!(wine_prefix(false, env(&[("HOME", "/home/test")])).unwrap(), PathBuf::from("/home/test/.config/photocraft/8bf-wine"));
        assert_eq!(wine_prefix(false, env(&[("XDG_CONFIG_HOME", "/custom/config")])).unwrap(), PathBuf::from("/custom/config/photocraft/8bf-wine"));
        assert_eq!(wine_prefix(true, env(&[("PHOTOCRAFT_8BF_WINEPREFIX", "/custom prefix")])).unwrap(), PathBuf::from("/custom prefix"));
        assert!(wine_prefix(true, env(&[("PHOTOCRAFT_8BF_WINEPREFIX", "relative")])).is_err());
        assert!(wine_prefix(false, env(&[])).is_err());
        assert_eq!(
            wine_prefix(false, env(&[("HOME", "/home/test"), ("XDG_CONFIG_HOME", "relative")])).unwrap(),
            PathBuf::from("/home/test/.config/photocraft/8bf-wine")
        );
    }
    #[test]
    fn runtime_override_and_finder_fallback_preserve_precedence() {
        assert_eq!(wine_program(true, env(&[("PHOTOCRAFT_8BF_WINE", "/custom runtime/wine")]), |_| true), PathBuf::from("/custom runtime/wine"));
        assert_eq!(wine_program(true, env(&[]), |p| p == Path::new("/opt/homebrew/bin/wine")), PathBuf::from("/opt/homebrew/bin/wine"));
        assert_eq!(wine_program(true, env(&[("PATH", "/custom/bin")]), |p| p == Path::new("/custom/bin/wine")), PathBuf::from("/custom/bin/wine"));
        assert_eq!(wine_program(false, env(&[]), |_| true), PathBuf::from("wine"));
        assert_eq!(wine_program(true, env(&[]), |_| false), PathBuf::from("wine"));
    }
}
