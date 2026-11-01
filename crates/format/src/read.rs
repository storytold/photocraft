//! Reading a whole file to open it: the read side of [`crate::atomic`].
//!
//! What it adds over `std::fs::read` for multi-gigabyte PSB files:
//!
//! - **Bounded reads.** Each read asks for at most [`READ_CHUNK`] bytes. `std` asks Windows for up
//!   to 4 GiB in one `ReadFile`, and some file systems (exFAT, network redirectors) reject a read
//!   that large with `ERROR_INVALID_PARAMETER` ("The parameter is incorrect", #375).
//! - **A clear out-of-memory error.** The buffer is reserved up front with `try_reserve_exact`,
//!   and a file larger than the memory available says how much it needed. A file that grows past
//!   its stat length while being read grows the buffer with `try_reserve` too.
//! - **A total limit.** No read returns more than [`MAX_OPEN_BYTES`], so a file that keeps growing
//!   while it is read cannot keep an open or a save from finishing.
//!
//! - **Reparse points that refuse traversal.** Cloud-synced folders (iCloud Drive on Windows,
//!   OneDrive and friends) put junctions and symlinks on the way to a file, and a process may be
//!   refused when it follows one: `ERROR_UNTRUSTED_MOUNT_POINT` ("The path cannot be traversed
//!   because it contains an untrusted mount point", os error 448, #10586). [`open_for_read`] then
//!   follows the links by hand (reading a link's target does not traverse it), falls back to the
//!   OS's canonical path, and reports the original error only when none of those opens the file.
//!
//! On `wasm32` there is no file system and every call returns the `std` "unsupported" error.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

/// Largest single read: big enough that a 5 GB file takes under a hundred reads.
pub const READ_CHUNK: usize = 64 << 20;

/// Most bytes [`read_file`] and [`read_all`] return. Far above any document memory can hold (a
/// PSB can run to tens of gigabytes), it only bounds a file that never stops growing; a file over it
/// is a `FileTooLarge` error, never a truncated read.
pub const MAX_OPEN_BYTES: u64 = 1 << 38;

/// Read all of `path`, in reads of at most [`READ_CHUNK`] bytes and at most [`MAX_OPEN_BYTES`] in
/// all. Anything but a regular file is an `InvalidInput` error.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    read_file_with(path, READ_CHUNK)
}

/// [`read_file`] with a chosen read size (tests use small ones to exercise the loop).
pub fn read_file_with(path: &Path, chunk: usize) -> io::Result<Vec<u8>> {
    read_path(path, chunk, MAX_OPEN_BYTES)
}

/// [`read_file_with`] with a chosen total limit (tests use small ones).
fn read_path(path: &Path, chunk: usize, max: u64) -> io::Result<Vec<u8>> {
    #[cfg(unix)]
    let (mut f, metadata) = {
        use rustix::fs::{Mode, OFlags, open};
        let fd = open(path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).map_err(io::Error::from)?;
        let f = std::fs::File::from(fd);
        let metadata = f.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file"));
        }
        rustix::fs::fcntl_setfl(&f, OFlags::empty()).map_err(io::Error::from)?;
        (f, metadata)
    };
    #[cfg(not(unix))]
    let (mut f, metadata) = {
        let f = open_for_read(path)?;
        let metadata = f.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file"));
        }
        (f, metadata)
    };
    let len = metadata.len();
    read_all_with(&mut f, len, chunk, max)
}

/// Most links [`open_for_read`] follows by hand on one path, like the OS limit.
const MAX_LINK_DEPTH: usize = 40;

/// Open `path` for reading, working around a path the OS refuses to traverse.
///
/// This is `File::open` unless that fails with an error that names a link on the path rather than
/// the file ([`refuses_traversal`]: on Windows `ERROR_UNTRUSTED_MOUNT_POINT`, 448, which iCloud
/// Drive and other cloud folders can raise, and its relatives). Then the path is tried again with
/// its symlinks and junctions followed by hand ([`follow_links`]), and then in the OS's canonical
/// form. If none opens, the first error is returned unchanged, so the message still names the
/// real cause.
pub fn open_for_read(path: &Path) -> io::Result<File> {
    open_with(path, &|p| File::open(p), &refuses_traversal, &fallback_paths)
}

/// [`open_for_read`] with each step injectable: `open` opens a path, `retry` says whether an error
/// is worth another attempt by a different route, and `fallbacks` lists those routes.
fn open_with(
    path: &Path,
    open: &dyn Fn(&Path) -> io::Result<File>,
    retry: &dyn Fn(&io::Error) -> bool,
    fallbacks: &dyn Fn(&Path) -> Vec<PathBuf>,
) -> io::Result<File> {
    let first = match open(path) {
        Ok(f) => return Ok(f),
        Err(e) if retry(&e) => e,
        Err(e) => return Err(e),
    };
    for candidate in fallbacks(path) {
        if candidate != path
            && let Ok(f) = open(&candidate)
        {
            return Ok(f);
        }
    }
    Err(first)
}

/// Whether `e` says a link on the path could not be followed, as opposed to the file being
/// missing or unreadable. Only Windows reports these.
fn refuses_traversal(e: &io::Error) -> bool {
    // 448 ERROR_UNTRUSTED_MOUNT_POINT, 1920 ERROR_CANT_ACCESS_FILE, 1921 ERROR_CANT_RESOLVE_FILENAME.
    cfg!(windows) && matches!(e.raw_os_error(), Some(448 | 1920 | 1921))
}

/// The other routes to `path`: its links followed by hand, then the OS's canonical form.
fn fallback_paths(path: &Path) -> Vec<PathBuf> {
    let by_hand = follow_links(path, &link_target);
    let canonical = std::fs::canonicalize(path).ok();
    by_hand.into_iter().chain(canonical).collect()
}

/// Where the symlink or junction at `path` points, `None` for anything else (or when it can't be
/// read). Reading a link opens the link itself, so it works where following it is refused.
fn link_target(path: &Path) -> Option<PathBuf> {
    let is_link = std::fs::symlink_metadata(path).ok()?.file_type().is_symlink();
    if is_link { std::fs::read_link(path).ok() } else { None }
}

/// One step of a path being walked by [`follow_links`].
enum Piece {
    /// A drive prefix or the root: starts the path over.
    Anchor(OsString),
    CurDir,
    ParentDir,
    Name(OsString),
}

/// The pieces of `path`, last first, so that `pop` takes them in path order.
fn pieces(path: &Path) -> Vec<Piece> {
    let mut v: Vec<Piece> = path
        .components()
        .map(|c| match c {
            Component::Prefix(_) | Component::RootDir => Piece::Anchor(c.as_os_str().to_owned()),
            Component::CurDir => Piece::CurDir,
            Component::ParentDir => Piece::ParentDir,
            Component::Normal(n) => Piece::Name(n.to_owned()),
        })
        .collect();
    v.reverse();
    v
}

/// `path` with every symlink and junction on it replaced by its target, found with `link_target`
/// (the target of the link at a path, `None` when it isn't one). `None` when there is no link to
/// replace, or when links nest deeper than [`MAX_LINK_DEPTH`] (a loop).
fn follow_links(path: &Path, link_target: &dyn Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    let mut todo = pieces(path);
    let mut resolved = PathBuf::new();
    let mut followed = 0usize;
    while let Some(piece) = todo.pop() {
        match piece {
            Piece::Anchor(a) => resolved.push(a),
            Piece::CurDir => {}
            Piece::ParentDir => {
                if !resolved.pop() {
                    resolved.push("..");
                }
            }
            Piece::Name(name) => {
                resolved.push(name);
                let Some(target) = link_target(&resolved) else { continue };
                followed += 1;
                if followed > MAX_LINK_DEPTH {
                    return None;
                }
                // A relative target starts where the link sat; an absolute one starts over.
                resolved.pop();
                if target.has_root() || target.is_absolute() {
                    resolved.clear();
                }
                todo.extend(pieces(&target));
            }
        }
    }
    (followed > 0).then_some(resolved)
}

/// Read `reader` to its end in reads of at most [`READ_CHUNK`] bytes, for an already open file
/// (a capability-scoped handle, say). `size_hint` (the file's length) sizes the buffer up front.
/// More than [`MAX_OPEN_BYTES`] in all is a `FileTooLarge` error.
pub fn read_all(reader: &mut impl Read, size_hint: u64) -> io::Result<Vec<u8>> {
    read_all_with(reader, size_hint, READ_CHUNK, MAX_OPEN_BYTES)
}

/// Read `reader` to its end in reads of at most `chunk` bytes, failing once it has given more
/// than `max` bytes. The hint only sizes the buffer: a file that grows or shrinks while being
/// read is still read to its end, within `max`. Past the hint the buffer grows only after a probe
/// read has found more bytes, and fallibly, so a file read exactly to its stat length is never
/// over-allocated and one that outgrows memory is an `OutOfMemory` error, not an abort.
fn read_all_with(reader: &mut impl Read, size_hint: u64, chunk: usize, max: u64) -> io::Result<Vec<u8>> {
    if size_hint > max {
        return Err(too_large(max));
    }
    let mut buf = buffer_for(size_hint)?;
    let chunk = chunk.max(1);
    // One byte over `max` is enough to tell a file over the limit from one exactly at it.
    let mut reader = reader.take(max.saturating_add(1));
    loop {
        if buf.len() == buf.capacity() {
            let mut probe = [0; 4096];
            let n = loop {
                match reader.read(&mut probe) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    r => break r?,
                }
            };
            if n == 0 {
                break;
            }
            let needed = buf.len() as u64 + n as u64;
            buf.try_reserve(n.max(chunk)).map_err(|_| out_of_memory(needed))?;
            buf.extend_from_slice(&probe[..n]);
        }
        let room = u64::try_from((buf.capacity() - buf.len()).min(chunk)).unwrap_or(u64::MAX);
        if (&mut reader).take(room).read_to_end(&mut buf)? == 0 && buf.len() < buf.capacity() {
            break;
        }
    }
    if buf.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(buf)
}

/// The error for a file over the `max` bytes a read may return.
fn too_large(max: u64) -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, format!("the file is larger than {} and was not read", size_text(max)))
}

/// The `OutOfMemory` error for a read that needed `len` bytes.
fn out_of_memory(len: u64) -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, format!("not enough memory to read the whole file ({})", size_text(len)))
}

/// An empty buffer with room for `len` bytes, or an `OutOfMemory` error saying how much was needed.
pub fn buffer_for(len: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let reserved = usize::try_from(len).ok().and_then(|n| buf.try_reserve_exact(n).ok());
    match reserved {
        Some(()) => Ok(buf),
        None => Err(out_of_memory(len)),
    }
}

/// `len` bytes for people: "4.75 GB", "759 MB", "12 KB".
fn size_text(len: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = len as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    match UNITS.get(unit) {
        Some(name) if unit > 0 => format!("{v:.2} {name}"),
        _ => format!("{len} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft-read-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_whole_file_across_many_chunks() {
        let data: Vec<u8> = (0..100_003u32).map(|i| (i * 7 % 251) as u8).collect();
        let p = temp("chunks", &data);
        for chunk in [1, 7, 4096, 100_003, 1 << 20] {
            assert_eq!(read_file_with(&p, chunk).unwrap(), data, "chunk {chunk}");
        }
        assert_eq!(read_file(&p).unwrap(), data);
        assert_eq!(read_file_with(&p, 0).unwrap(), data, "a zero chunk still makes progress");
        // From an open reader, whatever the hint says.
        for hint in [0, 10, data.len() as u64, 1 << 20] {
            assert_eq!(read_all(&mut std::io::Cursor::new(&data), hint).unwrap(), data, "hint {hint}");
        }
    }

    /// Nothing over the total limit is returned (#8601): a file larger than it, a reader that
    /// gives more than it whatever its hint says, and one that never ends are `FileTooLarge`
    /// errors, while a file exactly at the limit is read whole.
    #[test]
    fn reads_stop_at_the_total_limit() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let p = temp("limit", &data);
        assert_eq!(read_path(&p, 7, data.len() as u64).unwrap(), data, "exactly at the limit");
        assert_eq!(read_path(&p, 7, data.len() as u64 - 1).unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        for hint in [0, 100, data.len() as u64] {
            let e = read_all_with(&mut std::io::Cursor::new(&data), hint, 64, 5_000).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "hint {hint}");
        }
        let e = read_all_with(&mut std::io::Cursor::new(&data), 1 << 40, 64, 5_000).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "a hint over the limit fails before reading");
        let e = read_all_with(&mut std::io::repeat(1), 0, 1 << 20, 3 << 20).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge, "a reader that never ends");
        assert!(e.to_string().contains("3.15 MB"), "{e}");
    }

    /// A file read exactly to its stat length keeps the buffer it reserved: the end is found by a
    /// probe read, without growing the buffer first.
    #[test]
    fn an_exact_hint_is_not_over_allocated() {
        let data = vec![7u8; 100_000];
        let buf = read_all_with(&mut std::io::Cursor::new(&data), data.len() as u64, 4096, MAX_OPEN_BYTES).unwrap();
        assert_eq!(buf, data);
        assert_eq!(buf.capacity(), data.len());
    }

    #[test]
    fn empty_and_missing_files() {
        assert!(read_file(&temp("empty", b"")).unwrap().is_empty());
        let missing = std::env::temp_dir().join("photocraft-read-does-not-exist.psb");
        assert_eq!(read_file(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_file_too_large_for_memory_is_a_clear_error() {
        // More than any address space can hold.
        let e = buffer_for(u64::MAX).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert!(e.to_string().contains("18446744.07 TB"), "{e}");
        let e = buffer_for(1 << 62).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(buffer_for(1024).unwrap().capacity(), 1024);
    }

    /// The OS's "untrusted mount point" error (448, #10586), as `io::Error`.
    fn untrusted_mount_point() -> io::Error {
        io::Error::from_raw_os_error(448)
    }

    /// A stand-in for [`refuses_traversal`] that is the same on every platform.
    fn is_448(e: &io::Error) -> bool {
        e.raw_os_error() == Some(448)
    }

    /// A file inside an iCloud-like folder opens through its target when the OS refuses the
    /// path as written (#10586): the first attempt fails with os error 448, the retry succeeds.
    #[test]
    fn a_path_the_os_refuses_to_traverse_opens_through_a_fallback() {
        let real = temp("untrusted-448", b"psd bytes");
        let virtual_path = Path::new("iCloudDrive-link").join("art.psd");
        let tried = std::cell::RefCell::new(Vec::new());
        let open = |p: &Path| {
            tried.borrow_mut().push(p.to_path_buf());
            if p == virtual_path { Err(untrusted_mount_point()) } else { File::open(p) }
        };
        let fallbacks = |_: &Path| vec![virtual_path.clone(), real.clone()];
        let mut f = open_with(&virtual_path, &open, &is_448, &fallbacks).expect("the fallback path opens");
        let mut text = String::new();
        f.read_to_string(&mut text).unwrap();
        assert_eq!(text, "psd bytes");
        assert_eq!(*tried.borrow(), vec![virtual_path.clone(), real.clone()], "the unchanged path is not retried");
    }

    /// When no route opens the file, the caller sees the original error (448), not a later one.
    #[test]
    fn when_every_route_fails_the_first_error_is_reported() {
        let missing = std::env::temp_dir().join("photocraft-read-448-missing.psd");
        let open = |p: &Path| if p == Path::new("blocked") { Err(untrusted_mount_point()) } else { File::open(p) };
        let e = open_with(Path::new("blocked"), &open, &is_448, &|_| vec![missing.clone()]).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(448));
    }

    /// An ordinary failure (a missing file) is final: no fallback is looked for.
    #[test]
    fn other_errors_do_not_trigger_fallbacks() {
        let missing = std::env::temp_dir().join("photocraft-read-not-there.psd");
        let looked = std::cell::Cell::new(false);
        let e = open_with(&missing, &|p| File::open(p), &is_448, &|_| {
            looked.set(true);
            Vec::new()
        })
        .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(!looked.get());
        // An open that works never looks either.
        let ok = temp("fallbacks-unused", b"x");
        assert!(open_with(&ok, &|p| File::open(p), &is_448, &|_| panic!("no fallback needed")).is_ok());
    }

    #[test]
    fn only_traversal_errors_are_retried() {
        assert!(!refuses_traversal(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!refuses_traversal(&io::Error::from(io::ErrorKind::PermissionDenied)));
        // 448 means "untrusted mount point" on Windows and nothing on other systems.
        assert_eq!(refuses_traversal(&untrusted_mount_point()), cfg!(windows));
        assert_eq!(refuses_traversal(&io::Error::from_raw_os_error(1920)), cfg!(windows));
    }

    /// A table of links: `follow_links` looks each path up in it.
    fn links<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&Path) -> Option<PathBuf> + 'a {
        move |p| table.iter().find(|(from, _)| Path::new(from) == p).map(|(_, to)| PathBuf::from(to))
    }

    #[test]
    fn links_on_the_way_are_followed_by_hand() {
        let table = [("/users/me/iCloudDrive", "/store/icloud"), ("/store/icloud/Art", "projects"), ("/store/icloud/projects", "/data/p")];
        let follow = links(&table);
        // A link to a folder, a relative link below it, then an absolute one, in one path.
        assert_eq!(follow_links(Path::new("/users/me/iCloudDrive/Art/a.psd"), &follow), Some(PathBuf::from("/data/p/a.psd")));
        // No links: nothing to try.
        assert_eq!(follow_links(Path::new("/users/me/Documents/a.psd"), &follow), None);
        // `.` and `..` are folded in the usual way.
        assert_eq!(follow_links(Path::new("/users/me/./x/../iCloudDrive/a.psd"), &follow), Some(PathBuf::from("/store/icloud/a.psd")));
    }

    #[test]
    fn a_loop_of_links_gives_up() {
        let table = [("/a", "/b"), ("/b", "/a")];
        assert_eq!(follow_links(Path::new("/a/x.psd"), &links(&table)), None);
        // A link to itself.
        assert_eq!(follow_links(Path::new("/s/x"), &|p: &Path| (p == Path::new("/s")).then(|| PathBuf::from("/s"))), None);
    }

    /// Real links: a plain open still works through them, and following them by hand reaches the
    /// same file. (A junction needs no privilege on Windows; if the system refuses to make one,
    /// the test has nothing to check.)
    #[test]
    fn real_links_resolve_to_the_same_file() {
        let real = temp("real-link", b"through the link");
        let real_dir = real.parent().unwrap().to_path_buf();
        let link = real_dir.with_file_name(format!("{}-link", real_dir.file_name().unwrap().to_string_lossy()));
        let _ = std::fs::remove_dir(&link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&real_dir, &link).is_ok();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&real_dir).output().is_ok_and(|o| o.status.success());
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if !made {
            return;
        }
        let through = link.join("f.bin");
        assert_eq!(read_file(&through).unwrap(), b"through the link");
        let by_hand = follow_links(&through, &link_target).expect("the link is followed");
        assert_eq!(std::fs::read(&by_hand).unwrap(), b"through the link");
        assert_ne!(by_hand, through);
        // On Windows a junction is removed like a folder; on Unix the link is a file.
        let _ = std::fs::remove_dir(&link).or_else(|_| std::fs::remove_file(&link));
    }

    #[test]
    fn sizes_read_like_the_os_shows_them() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(4_750_000_000), "4.75 GB");
        assert_eq!(size_text(759_000_000), "759.00 MB");
    }
}
