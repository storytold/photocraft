//! Source-preserving native export publication. No destination link is trusted from preflight.
#[cfg(unix)]
use std::fs::File;
use std::{io, path::Path};

const PROTECTED: &str = "the destination is the import-only source; choose a different native or export path";

/// Publish while retaining the platform's source-identity protection until completion.
pub(super) fn write(path: &Path, bytes: &[u8], source: &Path) -> io::Result<()> {
    write_with(path, bytes, source, &|| Ok(()))
}

/// The callback injects namespace changes after validation, immediately before publication.
fn write_with(path: &Path, bytes: &[u8], source: &Path, before_publish: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
    if photocraft_engine::file_cmds::same_file(source, path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, PROTECTED));
    }
    publish(path, bytes, source, before_publish)
}

/// Retain a read lease on the source while the normal metadata-preserving writer
/// resolves links and publishes. Replacing the source itself must respect this lease;
/// replacing a different hard-link entry leaves the original source intact.
#[cfg(windows)]
fn publish(path: &Path, bytes: &[u8], source: &Path, before_publish: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
    use std::{io::Write, os::windows::fs::OpenOptionsExt};
    const FILE_SHARE_READ: u32 = 0x0001;
    // A data-read handle is required: metadata-only handles do not participate
    // in the sharing check that prevents replacement.
    let source_file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(source)
        .ok()
        .filter(|file| file.metadata().is_ok_and(|metadata| metadata.is_file()));
    if let Some(source_file) = source_file {
        before_publish()?;
        let result = photocraft_format::atomic_write(path, bytes);
        // Keep the source handle alive across every rename/retry, not just the path check.
        drop(source_file);
        return result;
    }
    // An unavailable source has no identity to lock. A new destination can still be published
    // atomically, but do not replace anything while source identity cannot be established.
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    photocraft_format::atomic::sync_file(temporary.as_file())?;
    before_publish()?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(error.error.kind(), "cannot replace an existing destination while the import source is unavailable; choose an unused file name")
        } else {
            error.error
        }
    })?;
    // Same Windows durability requirement as the ordinary atomic writer.
    const FILE_ADD_FILE: u32 = 0x0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let directory = std::fs::OpenOptions::new().access_mode(FILE_ADD_FILE).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(parent)?;
    directory.sync_all().map_err(|error| io::Error::new(error.kind(), format!("the export was written, but its folder could not be flushed: {error}")))
}

/// All publication operations use one held parent-directory descriptor and a single leaf.
/// renameat replaces that leaf itself: a late symlink/hard-link swap cannot redirect the
/// write into its target, and renaming/replacing an ancestor cannot retarget the descriptor.
#[cfg(unix)]
fn publish(path: &Path, bytes: &[u8], source: &Path, before_publish: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
    use rustix::fs::{AtFlags, Mode, OFlags, fsync, open, openat, renameat, unlinkat};
    use std::{io::Write, os::fd::AsFd};
    let source_file = open(source, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
        .ok()
        .map(File::from)
        .filter(|file| file.metadata().is_ok_and(|metadata| metadata.is_file()));
    // Follow an existing user-selected link once. From here on neither validation nor
    // publication resolves the destination name through ambient paths again.
    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let leaf = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export destination needs a file name"))?;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let directory = open(parent, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    let old = match openat(&directory, leaf, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()) {
        Ok(fd) => Some(File::from(fd)),
        Err(rustix::io::Errno::NOENT) => None,
        Err(error) => return Err(error.into()),
    };
    if source_file.is_none() && old.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cannot replace an existing destination while the import source is unavailable; choose an unused file name",
        ));
    }
    if let Some(old) = &old {
        let metadata = old.metadata()?;
        if !metadata.is_file() || metadata.permissions().readonly() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "export destination is not a writable regular file"));
        }
        if let Some(source) = &source_file
            && photocraft_engine::file_cmds::same_open_files(source, old)?
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, PROTECTED));
        }
    }
    let mut temporary = None;
    for _ in 0..8 {
        let name = photocraft_format::atomic::temp_name(&leaf.to_string_lossy());
        match openat(
            &directory,
            name.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH,
        ) {
            Ok(fd) => {
                temporary = Some((name, File::from(fd)));
                break;
            }
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let (temporary_name, mut file) = temporary.ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "could not create an export temporary file"))?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        if let Some(old) = &old {
            let _ = photocraft_format::atomic::meta::copy_xattrs(old.as_fd(), file.as_fd());
            let _ = photocraft_format::atomic::meta::copy_created_file(old, &file);
            // macOS ACL copying uses its existing safe helper, through live fd aliases.
            #[cfg(target_os = "macos")]
            {
                use std::os::fd::AsRawFd;
                let from = std::path::PathBuf::from(format!("/dev/fd/{}", old.as_raw_fd()));
                let to = std::path::PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()));
                let _ = photocraft_format::atomic::meta::copy_metadata(&from, &to);
            }
            let _ = file.set_permissions(old.metadata()?.permissions());
        }
        photocraft_format::atomic::sync_file(&file)?;
        before_publish()?;
        if source_file.is_some() {
            photocraft_format::atomic::retry_rename(photocraft_format::atomic::RenameRetry::platform(), || {
                renameat(&directory, temporary_name.as_str(), &directory, leaf).map_err(io::Error::from)
            })?;
        } else {
            // Atomic no-clobber publication rejects any late file or symlink. Prefer
            // no-replace rename on Linux/Apple, including filesystems without hard links.
            #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
            rustix::fs::renameat_with(&directory, temporary_name.as_str(), &directory, leaf, rustix::fs::RenameFlags::NOREPLACE)?;
            #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
            {
                rustix::fs::linkat(&directory, temporary_name.as_str(), &directory, leaf, AtFlags::empty())?;
                let _ = unlinkat(&directory, temporary_name.as_str(), AtFlags::empty());
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = unlinkat(&directory, temporary_name.as_str(), AtFlags::empty());
        return result;
    }
    match fsync(&directory) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(io::Error::from(error).kind(), io::ErrorKind::Unsupported)
                || (error == rustix::io::Errno::INVAL || (cfg!(target_vendor = "apple") && matches!(error.raw_os_error(), 25 | 45 | 102))) =>
        {
            Ok(())
        }
        Err(error) => Err(io::Error::other(format!("the export was written, but its folder could not be flushed: {error}"))),
    }
}

#[cfg(not(any(unix, windows)))]
fn publish(_path: &Path, _bytes: &[u8], _source: &Path, _before_publish: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "source-protecting native export is unavailable on this platform"))
}

#[cfg(test)]
mod tests;
