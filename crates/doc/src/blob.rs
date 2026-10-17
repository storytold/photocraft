//! [`Blob`]: raw bytes a document keeps verbatim, held in memory or left in the file it was opened
//! from.
//!
//! A PSD's embedded smart objects (`lnk2`) can be most of the file: hundreds of megabytes that
//! are only needed to edit a smart object's contents or to save. Opening such a file can leave
//! them where they are ([`Blob::deferred`]) and read them when asked. Reading can fail (the file
//! changed or became unreadable), so every read is a `Result`: nothing falls back to empty bytes.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

/// Where deferred bytes are read from.
pub trait DeferredBytes: Send + Sync {
    /// Number of bytes.
    fn len(&self) -> u64;

    /// `true` when there are no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `len` bytes starting `at` bytes in.
    fn read(&self, at: u64, len: u64) -> Result<Vec<u8>, BlobError>;

    /// What the bytes are read from, for messages (a file path).
    fn origin(&self) -> String;
}

/// A deferred read that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobError(pub String);

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BlobError {}

/// Bytes kept verbatim: in memory, or deferred to the file they came from. Cloning shares them.
#[derive(Clone)]
pub enum Blob {
    /// In memory.
    Memory(Arc<Vec<u8>>),
    /// Read from the source on demand.
    Deferred(Arc<dyn DeferredBytes>),
}

impl Blob {
    /// Bytes in memory.
    pub fn new(bytes: Vec<u8>) -> Self {
        Blob::Memory(Arc::new(bytes))
    }

    /// Bytes left in `source`.
    pub fn deferred(source: Arc<dyn DeferredBytes>) -> Self {
        Blob::Deferred(source)
    }

    /// Number of bytes.
    pub fn len(&self) -> u64 {
        match self {
            Blob::Memory(b) => b.len() as u64,
            Blob::Deferred(d) => d.len(),
        }
    }

    /// `true` when there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` when the bytes are in memory.
    pub fn is_in_memory(&self) -> bool {
        matches!(self, Blob::Memory(_))
    }

    /// The bytes when they are in memory.
    pub fn in_memory(&self) -> Option<&Arc<Vec<u8>>> {
        match self {
            Blob::Memory(b) => Some(b),
            Blob::Deferred(_) => None,
        }
    }

    /// The bytes when they are in memory, empty when deferred. Only for data that is never
    /// deferred (everything but the embedded smart-object blocks); use [`Self::bytes`] otherwise.
    pub fn resident(&self) -> &[u8] {
        match self {
            Blob::Memory(b) => b,
            Blob::Deferred(_) => &[],
        }
    }

    /// All the bytes: shared when in memory, read from the source otherwise.
    pub fn bytes(&self) -> Result<Arc<Vec<u8>>, BlobError> {
        match self {
            Blob::Memory(b) => Ok(b.clone()),
            Blob::Deferred(d) => d.read(0, d.len()).map(Arc::new),
        }
    }

    /// `len` bytes starting `at` bytes in (fails past the end).
    pub fn read_part(&self, at: u64, len: u64) -> Result<Cow<'_, [u8]>, BlobError> {
        if at.checked_add(len).is_none_or(|end| end > self.len()) {
            return Err(BlobError(format!("read of {len} bytes at {at} is past the end of {} bytes", self.len())));
        }
        match self {
            Blob::Memory(b) => {
                let (a, n) = (usize::try_from(at).map_err(|_| BlobError("offset too large".into()))?, usize::try_from(len).map_err(|_| BlobError("length too large".into()))?);
                b.get(a..a + n).map(Cow::Borrowed).ok_or_else(|| BlobError("read past the end".into()))
            }
            Blob::Deferred(d) => d.read(at, len).map(Cow::Owned),
        }
    }

    /// The same bytes in memory (reading them when deferred).
    pub fn loaded(&self) -> Result<Blob, BlobError> {
        self.bytes().map(Blob::Memory)
    }

    /// `true` when both are the same allocation or the same deferred source.
    pub fn ptr_eq(&self, other: &Blob) -> bool {
        match (self, other) {
            (Blob::Memory(a), Blob::Memory(b)) => Arc::ptr_eq(a, b),
            (Blob::Deferred(a), Blob::Deferred(b)) => std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)),
            _ => false,
        }
    }
}

impl Default for Blob {
    fn default() -> Self {
        Blob::new(Vec::new())
    }
}

impl From<Vec<u8>> for Blob {
    fn from(b: Vec<u8>) -> Self {
        Blob::new(b)
    }
}

impl From<Arc<Vec<u8>>> for Blob {
    fn from(b: Arc<Vec<u8>>) -> Self {
        Blob::Memory(b)
    }
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blob::Memory(b) => f.debug_tuple("Blob").field(&b.len()).finish(),
            Blob::Deferred(d) => write!(f, "Blob({} deferred to {})", d.len(), d.origin()),
        }
    }
}

/// Equal bytes are equal blobs, wherever each one keeps them (an unreadable one is equal only to
/// itself).
impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        if self.ptr_eq(other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        match (self.bytes(), other.bytes()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fake(Vec<u8>, bool);

    impl DeferredBytes for Fake {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read(&self, at: u64, len: u64) -> Result<Vec<u8>, BlobError> {
            if self.1 {
                return Err(BlobError("gone".into()));
            }
            Ok(self.0[at as usize..(at + len) as usize].to_vec())
        }
        fn origin(&self) -> String {
            "fake".into()
        }
    }

    #[test]
    fn deferred_and_memory_blobs_read_the_same() {
        let m = Blob::new(vec![1, 2, 3, 4]);
        let d = Blob::deferred(Arc::new(Fake(vec![1, 2, 3, 4], false)));
        assert_eq!(m, d);
        assert_eq!(d.read_part(1, 2).unwrap().as_ref(), &[2, 3]);
        assert_eq!(m.read_part(1, 2).unwrap().as_ref(), &[2, 3]);
        assert!(d.read_part(3, 2).is_err() && m.read_part(3, 2).is_err());
        assert!(d.loaded().unwrap().is_in_memory());
        let gone = Blob::deferred(Arc::new(Fake(vec![1, 2, 3, 4], true)));
        assert!(gone.bytes().is_err());
        assert_ne!(gone, m);
        assert_eq!(gone, gone.clone());
        assert_eq!(format!("{m:?}"), "Blob(4)");
    }
}
