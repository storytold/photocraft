//! Compatibility surface for callers that inspect ZIP archives.
//!
//! ZIP parsing, decompression, CRC checks, and writing are owned by `craft-zip`. This module only
//! adapts its bounded archive API to the older PhotoCraft call shape while downstream callers
//! migrate.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::FormatError;

/// An entry name from a ZIP archive.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The decoded entry path.
    pub name: String,
}

/// Read-only view of an archive in memory. The shared reader validates and decodes entries.
pub struct ZipReader<'a> {
    archive: Mutex<craft_zip::Archive<'a>>,
    /// Archive entries in central-directory order.
    pub entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

pub(crate) fn map_read_error(error: craft_zip::Error) -> FormatError {
    match error {
        craft_zip::Error::TooManyEntries { .. }
        | craft_zip::Error::TooManyEndRecords(_)
        | craft_zip::Error::EntryTooLarge { .. }
        | craft_zip::Error::BudgetSpent { .. } => FormatError::LimitExceeded(format!("zip: {error}")),
        _ => FormatError::corrupt(format!("zip: {error}")),
    }
}

impl<'a> ZipReader<'a> {
    /// Indexes an archive through craft-zip, with a bounded classic-format entry count.
    pub fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let archive = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: u16::MAX.into(), max_total_bytes: u64::MAX }).map_err(map_read_error)?;
        let entries: Vec<Entry> = archive.names().map(|name| Entry { name: name.into_owned() }).collect();
        let mut by_name = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            by_name.entry(entry.name.clone()).or_insert(index);
        }
        Ok(Self { archive: Mutex::new(archive), entries, by_name })
    }

    /// Finds the first entry with this name.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.by_name.get(name).and_then(|&index| self.entries.get(index))
    }

    /// The entry's claimed decoded ZIP size, not a checksum or integrity result.
    pub fn declared_size(&self, name: &str) -> Result<Option<u64>, FormatError> {
        let mut archive = self.archive.lock().map_err(|_| FormatError::corrupt("zip reader lock was poisoned"))?;
        Ok(archive.declared_size(name))
    }

    /// The entry's claimed encoded ZIP size, without decompressing it.
    pub fn compressed_size(&self, name: &str) -> Result<Option<u64>, FormatError> {
        let mut archive = self.archive.lock().map_err(|_| FormatError::corrupt("zip reader lock was poisoned"))?;
        Ok(archive.compressed_size(name))
    }

    /// Read only a bounded entry prefix. This deliberately does not verify the
    /// checksum/trailer; callers must still use `read` for complete validation.
    pub fn read_head(&self, name: &str, count: usize) -> Result<Option<Vec<u8>>, FormatError> {
        // Metadata inspection must not become an unbounded full-entry decoder.
        if count > 64 * 1024 {
            return Err(FormatError::LimitExceeded("ZIP header probe exceeds 64 KiB".into()));
        }
        let mut archive = self.archive.lock().map_err(|_| FormatError::corrupt("zip reader lock was poisoned"))?;
        archive.read_head(name, count).map_err(map_read_error)
    }

    /// Reads one entry through craft-zip, bounded by `max` decoded bytes.
    pub fn read(&self, entry: &Entry, max: usize) -> Result<Vec<u8>, FormatError> {
        let mut archive = self.archive.lock().map_err(|_| FormatError::corrupt("zip reader lock was poisoned"))?;
        archive.read(&entry.name, max as u64).map_err(map_read_error)?.ok_or_else(|| FormatError::corrupt(format!("missing `{}`", entry.name)))
    }

    /// Reads the named entry through craft-zip, bounded by `max` decoded bytes.
    pub fn read_by_name(&self, name: &str, max: usize) -> Result<Vec<u8>, FormatError> {
        let entry = self.find(name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        self.read(entry, max)
    }
}

/// Compatibility writer that adds stored entries through craft-zip.
#[derive(Default)]
pub struct ZipWriter {
    archive: craft_zip::Writer,
}

impl ZipWriter {
    /// Creates an empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a stored entry.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), FormatError> {
        self.archive.add(name, data, craft_zip::Compression::Stored).map_err(|error| FormatError::Unsupported(format!("zip: {error}")))
    }

    /// Finishes the archive.
    pub fn finish(self) -> Result<Vec<u8>, FormatError> {
        self.archive.finish().map_err(|error| FormatError::Unsupported(format!("zip: {error}")))
    }
}

#[cfg(test)]
mod metadata_forwarding_tests {
    use super::*;

    #[test]
    fn declared_sizes_and_bounded_prefixes_leave_full_reads_authoritative() {
        let mut writer = ZipWriter::new();
        writer.add("entry", &[17, 31, 57, 89]).unwrap();
        let bytes = writer.finish().unwrap();
        let zip = ZipReader::new(&bytes).unwrap();
        assert_eq!(zip.declared_size("entry").unwrap(), Some(4));
        assert_eq!(zip.compressed_size("entry").unwrap(), Some(4));
        assert_eq!(zip.declared_size("missing").unwrap(), None);
        assert_eq!(zip.compressed_size("missing").unwrap(), None);
        assert_eq!(zip.read_head("missing", 2).unwrap(), None);
        assert!(matches!(zip.read_head("entry", usize::MAX), Err(FormatError::LimitExceeded(_))));
        assert_eq!(zip.read_head("entry", 0).unwrap(), Some(vec![]));
        assert_eq!(zip.read_head("entry", 2).unwrap(), Some(vec![17, 31]));
        assert_eq!(zip.read_head("entry", 20).unwrap(), Some(vec![17, 31, 57, 89]));
        assert!(zip.read_by_name("entry", 3).is_err());
        assert_eq!(zip.read_by_name("entry", 4).unwrap(), vec![17, 31, 57, 89]);
    }

    #[test]
    fn prefix_metadata_does_not_certify_a_damaged_entry_checksum() {
        let payload = [17, 31, 57, 89, 123, 231, 77, 19];
        let mut writer = ZipWriter::new();
        writer.add("entry", &payload).unwrap();
        let mut bytes = writer.finish().unwrap();
        let start = bytes.windows(payload.len()).position(|window| window == payload).unwrap();
        bytes[start] ^= 1;
        let zip = ZipReader::new(&bytes).unwrap();
        assert_eq!(zip.declared_size("entry").unwrap(), Some(8));
        assert_eq!(zip.read_head("entry", 1).unwrap(), Some(vec![16]));
        assert!(zip.read_by_name("entry", 8).is_err(), "full read retains CRC validation after a prefix probe");
    }
}
