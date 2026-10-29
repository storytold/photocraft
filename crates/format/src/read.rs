//! Reading a whole file to open it: the read side of [`crate::atomic`].
//!
//! What it adds over `std::fs::read` for multi-gigabyte PSB files:
//!
//! - **Bounded reads.** Each read asks for at most [`READ_CHUNK`] bytes. `std` asks Windows for up
//!   to 4 GiB in one `ReadFile`, and some file systems (exFAT, network redirectors) reject a read
//!   that large with `ERROR_INVALID_PARAMETER` ("The parameter is incorrect", #375).
//! - **A clear out-of-memory error.** The buffer is reserved up front with `try_reserve_exact`,
//!   and a file larger than the memory available says how much it needed.
//!
//! On `wasm32` there is no file system and every call returns the `std` "unsupported" error.

use std::io::{self, Read};
use std::path::Path;

/// Largest single read: big enough that a 5 GB file takes under a hundred reads.
pub const READ_CHUNK: usize = 64 << 20;

/// Read all of `path`, in reads of at most [`READ_CHUNK`] bytes.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    read_file_with(path, READ_CHUNK)
}

/// [`read_file`] with a chosen read size (tests use small ones to exercise the loop).
pub fn read_file_with(path: &Path, chunk: usize) -> io::Result<Vec<u8>> {
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
        let f = std::fs::File::open(path)?;
        let metadata = f.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is not a regular file"));
        }
        (f, metadata)
    };
    let len = metadata.len();
    read_all_with(&mut f, len, chunk)
}

/// Read `reader` to its end in reads of at most [`READ_CHUNK`] bytes, for an already open file
/// (a capability-scoped handle, say). `size_hint` (the file's length) sizes the buffer up front.
pub fn read_all(reader: &mut impl Read, size_hint: u64) -> io::Result<Vec<u8>> {
    read_all_with(reader, size_hint, READ_CHUNK)
}

fn read_all_with(reader: &mut impl Read, size_hint: u64, chunk: usize) -> io::Result<Vec<u8>> {
    // The hint only sizes the buffer: a file that grows or shrinks while being read is still read
    // to its end.
    let mut buf = buffer_for(size_hint)?;
    let chunk = u64::try_from(chunk.max(1)).unwrap_or(u64::MAX);
    while reader.take(chunk).read_to_end(&mut buf)? > 0 {}
    Ok(buf)
}

/// The most [`read_pipe`] takes from standard input (`photocraft -`, `photocraft-cli convert -
/// out.png`, #9468). A pipe has no length to check first, so a stream that never ends (a mistaken
/// `/dev/zero`) stops here instead of filling memory; a bigger image opens from a file.
pub const PIPE_LIMIT: u64 = 4_000_000_000;

/// Read an image piped in on `reader` (standard input) to its end. More than `limit` bytes is a
/// `FileTooLarge` error and nothing at all an `UnexpectedEof` one, so an empty pipe says so
/// rather than failing to decode. The buffer grows as data arrives, and a growth the memory can't
/// hold is an `OutOfMemory` error, not an abort.
pub fn read_pipe(reader: &mut impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    // One byte past the limit tells an input of exactly `limit` bytes from a longer one.
    reader.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
    if buf.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "nothing was piped in"));
    }
    if u64::try_from(buf.len()).unwrap_or(u64::MAX) > limit {
        let message = format!("more than {} was piped in; save the image to a file and open that", size_text(limit));
        return Err(io::Error::new(io::ErrorKind::FileTooLarge, message));
    }
    Ok(buf)
}

/// An empty buffer with room for `len` bytes, or an `OutOfMemory` error saying how much was needed.
pub fn buffer_for(len: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let reserved = usize::try_from(len).ok().and_then(|n| buf.try_reserve_exact(n).ok());
    match reserved {
        Some(()) => Ok(buf),
        None => Err(io::Error::new(io::ErrorKind::OutOfMemory, format!("not enough memory to read the whole file ({})", size_text(len)))),
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

    /// #9468: a piped image is read whole up to the limit; an empty pipe, one past the limit, one
    /// that never ends and one that fails are errors, not panics or an endless read.
    #[test]
    fn piped_input_is_read_to_its_end_within_the_limit() {
        let png = b"\x89PNG\r\n\x1a\n rest of the image";
        assert_eq!(read_pipe(&mut &png[..], 1000).unwrap(), png);
        assert_eq!(read_pipe(&mut &png[..], png.len() as u64).unwrap(), png, "exactly the limit");

        let e = read_pipe(&mut io::empty(), 1000).unwrap_err();
        assert_eq!((e.kind(), e.to_string()), (io::ErrorKind::UnexpectedEof, "nothing was piped in".to_string()));

        let e = read_pipe(&mut &png[..], png.len() as u64 - 1).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        assert!(e.to_string().starts_with("more than 25 bytes was piped in;"), "{e}");
        // An endless stream stops one byte past the limit.
        let e = read_pipe(&mut io::repeat(0), 1 << 20).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        assert!(e.to_string().contains("more than 1.05 MB"), "{e}");
        assert!(read_pipe(&mut io::repeat(0), 0).is_err(), "a zero limit takes nothing");
        assert_eq!(size_text(PIPE_LIMIT), "4.00 GB");

        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"))
            }
        }
        assert_eq!(read_pipe(&mut Broken, 1000).unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        // Bytes that aren't an image are read like any others: the importer turns them down.
        assert_eq!(read_pipe(&mut &b"not an image"[..], 1000).unwrap(), b"not an image");
    }

    #[test]
    fn sizes_read_like_the_os_shows_them() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(4_750_000_000), "4.75 GB");
        assert_eq!(size_text(759_000_000), "759.00 MB");
    }
}
