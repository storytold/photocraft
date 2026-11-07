//! Per-load quotas for decoded Color Lookup samples, separate from compressed blob storage.
use std::collections::HashMap;
use std::sync::Arc;

use crate::manifest::Hash;
use crate::{FormatError, LoadOptions, Result};

pub(crate) struct LutCache {
    tables: HashMap<Hash, Arc<Vec<f32>>>,
    bytes: u64,
    max_bytes: u64,
    max_blob_bytes: usize,
}

impl LutCache {
    /// Bound both decoded bytes and metadata; shared references consume no extra quota.
    pub(crate) fn new(opts: &LoadOptions) -> Self {
        Self { tables: HashMap::new(), bytes: 0, max_bytes: opts.max_total_bytes, max_blob_bytes: opts.max_blob_bytes }
    }

    pub(crate) fn get(&self, hash: &str) -> Option<Arc<Vec<f32>>> {
        self.tables.get(hash).cloned()
    }

    /// Check before decoding, including empty tables that consume only cache metadata.
    pub(crate) fn admit(&self, bytes: usize) -> Result<()> {
        if self.tables.len() >= 65_536 || bytes > self.max_blob_bytes || bytes as u64 > self.max_bytes.saturating_sub(self.bytes) {
            return Err(FormatError::LimitExceeded("decoded Color Lookup cache quota exceeded".into()));
        }
        Ok(())
    }

    /// Insert only after successful admission and decoding.
    pub(crate) fn insert(&mut self, hash: &str, lut: Arc<Vec<f32>>) {
        self.bytes = self.bytes.saturating_add((lut.len() as u64).saturating_mul(4));
        self.tables.insert(hash.to_owned(), lut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct tables exhaust the byte quota; references to an existing table do not.
    #[test]
    fn decoded_bytes_are_bounded_and_shared() {
        let mut cache = LutCache::new(&LoadOptions { max_total_bytes: 8, max_blob_bytes: 8, ..Default::default() });
        let lut = Arc::new(vec![0.0, 1.0]);
        cache.admit(8).unwrap();
        cache.insert("first", lut.clone());
        assert!(Arc::ptr_eq(&cache.get("first").unwrap(), &lut));
        assert!(matches!(cache.admit(4), Err(FormatError::LimitExceeded(_))));
        assert_eq!(cache.bytes, 8);
    }

    /// A single decoded table cannot exceed the configured per-blob budget.
    #[test]
    fn per_table_limit_is_checked_before_allocation() {
        let cache = LutCache::new(&LoadOptions { max_total_bytes: 100, max_blob_bytes: 4, ..Default::default() });
        cache.admit(4).unwrap();
        assert!(matches!(cache.admit(8), Err(FormatError::LimitExceeded(_))));
    }

    /// Even zero-byte tables have a finite metadata quota.
    #[test]
    fn empty_tables_cannot_grow_metadata_without_bound() {
        let mut cache = LutCache::new(&LoadOptions::default());
        let empty = Arc::new(Vec::new());
        for index in 0..65_536 {
            cache.admit(0).unwrap();
            cache.insert(&index.to_string(), empty.clone());
        }
        assert!(matches!(cache.admit(0), Err(FormatError::LimitExceeded(_))));
        assert_eq!(cache.bytes, 0);
        assert!(cache.get("0").is_some());
    }
}
