use super::*;

#[test]
fn ordinary_replacement_retains_the_source_and_rejects_existing_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let destination = directory.path().join("copy.png");
    let alias = directory.path().join("alias.png");
    std::fs::write(&source, b"original import source").unwrap();
    std::fs::write(&destination, b"old output").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    std::fs::hard_link(&source, &alias).unwrap();
    for path in [&source, &alias] {
        assert!(write(path, b"export", &source).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"original import source");
    }
    write(&destination, b"export", &source).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"export");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&destination).unwrap().permissions().mode() & 0o777, 0o640);
    }
    assert_eq!(std::fs::read(&source).unwrap(), b"original import source");
}

#[test]
fn a_removed_source_still_allows_a_fresh_export() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("removed.xcf");
    let destination = directory.path().join("copy.png");
    write(&destination, b"export", &source).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read(destination).unwrap(), b"export");
}

/// Reproduce the original preflight/encode/publish race without timing or threads.
#[test]
fn a_late_destination_alias_cannot_replace_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let destination = directory.path().join("copy.png");
    std::fs::write(&source, b"original import source").unwrap();
    std::fs::write(&destination, b"old output").unwrap();
    // Check whether this Windows account can create symbolic links, before taking a lease.
    let probe = directory.path().join("probe");
    let hard_link_race = match symlink(&source, &probe) {
        Ok(()) => {
            std::fs::remove_file(&probe).unwrap();
            false
        }
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(1314) => {
            // Exercise an identity alias even without Developer Mode; the Unix run
            // exercises the symlink-redirection case independently.
            eprintln!("using a late hard-link alias on Windows without symlink privilege");
            true
        }
        Err(error) => panic!("cannot prepare symlink race: {error}"),
    };
    let swapped = std::cell::Cell::new(false);
    let result = write_with(&destination, b"export", &source, &|| {
        std::fs::remove_file(&destination)?;
        swapped.set(true);
        if hard_link_race { std::fs::hard_link(&source, &destination) } else { symlink(&source, &destination) }
    });
    assert!(swapped.get());
    assert_eq!(std::fs::read(&source).unwrap(), b"original import source");
    #[cfg(windows)]
    if hard_link_race {
        if result.is_ok() {
            // Replacing another hard-link entry is safe: the source still owns its old bytes.
            assert_eq!(std::fs::read(&destination).unwrap(), b"export");
        }
    } else {
        assert!(result.is_err(), "Windows must refuse replacing the leased source through a symlink");
    }
    #[cfg(unix)]
    {
        result.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"export");
        assert!(!std::fs::symlink_metadata(destination).unwrap().file_type().is_symlink());
    }
}

#[cfg(windows)]
#[test]
fn the_source_lease_blocks_replacement_by_identity_during_publication() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.xcf");
    let destination = directory.path().join("copy.png");
    std::fs::write(&source, b"original import source").unwrap();
    write_with(&destination, b"export", &source, &|| {
        // No symlink privilege needed: exercise the actual replacement boundary directly.
        assert!(photocraft_format::atomic_write(&source, b"corruption").is_err());
        assert_eq!(std::fs::read(&source)?, b"original import source");
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"export");
    assert_eq!(std::fs::read(source).unwrap(), b"original import source");
}

#[test]
fn an_unavailable_source_never_allows_a_replacement_without_identity() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("removed.xcf");
    let destination = directory.path().join("copy.png");
    std::fs::write(&destination, b"old output").unwrap();
    assert!(write(&destination, b"export", &source).is_err());
    assert_eq!(std::fs::read(destination).unwrap(), b"old output");
}

#[cfg(unix)]
#[test]
fn replacing_the_parent_path_cannot_redirect_a_held_directory() {
    let directory = tempfile::tempdir().unwrap();
    let sources = directory.path().join("sources");
    let outputs = directory.path().join("outputs");
    let moved = directory.path().join("moved");
    std::fs::create_dir(&sources).unwrap();
    std::fs::create_dir(&outputs).unwrap();
    let source = sources.join("image.png");
    let destination = outputs.join("image.png");
    std::fs::write(&source, b"original import source").unwrap();
    std::fs::write(&destination, b"old output").unwrap();
    write_with(&destination, b"export", &source, &|| {
        std::fs::rename(&outputs, &moved)?;
        symlink(&sources, &outputs)
    })
    .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"original import source");
    assert_eq!(std::fs::read(moved.join("image.png")).unwrap(), b"export");
}

#[cfg(unix)]
fn symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}
#[cfg(windows)]
fn symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
}

/// Unknown identity must not turn into an overwrite when a file appears after preflight.
#[test]
fn an_unavailable_source_requires_no_clobber_at_publication() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("removed.xcf");
    let destination = directory.path().join("copy.png");
    let result = write_with(&destination, b"export", &source, &|| std::fs::write(&destination, b"another writer"));
    assert!(result.is_err());
    assert_eq!(std::fs::read(destination).unwrap(), b"another writer");
    assert!(!source.exists());
}
