use super::*;

fn locations() -> Locations {
    Locations { explicit: None, desktop_bins: Vec::new(), desktop_executables: Vec::new(), search_path: Vec::new() }
}

fn executable(path: &Path, seconds: u64) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    path.canonicalize().unwrap()
}

#[test]
fn newest_desktop_runtime_wins_over_path_and_is_rediscovered_after_update() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("desktop");
    let old = executable(&root.join("a-old").join(executable_name()), 100);
    executable(&dir.path().join("cli").join(executable_name()), 400);
    let mut search = locations();
    search.desktop_bins = vec![root.clone()];
    search.search_path = vec![dir.path().join("cli")];
    assert_eq!(search.find().unwrap(), old);
    let updated = executable(&root.join("b-new").join(executable_name()), 200);
    assert_eq!(search.find().unwrap(), updated);
    std::fs::remove_file(&updated).unwrap();
    assert_eq!(search.find().unwrap(), old);
}

#[test]
fn explicit_override_is_authoritative_and_never_silently_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let selected = executable(&dir.path().join("selected runtime").join(executable_name()), 100);
    let mut search = locations();
    search.explicit = Some(selected.clone().into_os_string());
    search.search_path = vec![dir.path().join("selected runtime")];
    assert_eq!(search.find().unwrap(), selected);
    search.explicit = Some(dir.path().join("missing").into_os_string());
    assert!(matches!(search.find(), Err(Error::RuntimeUnavailable(message)) if message.contains("PHOTOCRAFT_CODEX_PATH")));
    search.explicit = Some(OsString::new());
    assert!(search.find().is_err());
}

#[test]
fn known_bundle_and_absolute_path_are_supported_without_shell_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let selected = executable(&dir.path().join("app/Contents/Resources").join(executable_name()), 100);
    let mut search = locations();
    search.desktop_executables = vec![selected.clone()];
    assert_eq!(search.find().unwrap(), selected);
    search.desktop_executables.clear();
    search.search_path = vec![dir.path().join("app/Contents/Resources")];
    assert_eq!(search.find().unwrap(), selected);
}

#[test]
fn missing_runtime_skips_directories_and_nested_unrelated_copies() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("bin").join(executable_name())).unwrap();
    executable(&dir.path().join("desktop/too/deep").join(executable_name()), 100);
    let mut search = locations();
    search.search_path = vec![PathBuf::new(), PathBuf::from("."), dir.path().join("bin")];
    search.desktop_bins = vec![dir.path().join("desktop")];
    assert!(matches!(search.find(), Err(Error::RuntimeUnavailable(_))));
}

#[test]
fn path_scan_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    executable(&bin.join(executable_name()), 100);
    let mut search = locations();
    search.search_path = vec![dir.path().join("missing"); LIMIT];
    search.search_path.push(bin);
    assert!(search.find().is_err());
}

#[cfg(unix)]
#[test]
fn unix_non_executable_files_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(executable_name()), b"not executable").unwrap();
    let mut search = locations();
    search.search_path = vec![dir.path().to_owned()];
    assert!(search.find().is_err());
}
