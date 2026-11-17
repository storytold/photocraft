//! Host-side blobs: image bytes a panel never has to touch.
//!
//! The interpreter costs 10–100× native speed, so encoding a 24 MP PNG inside a plug-in would take
//! minutes. Instead the host keeps the bytes and hands the plug-in an opaque id (`"b:17"`): a
//! capture returns one, `doc.commit` takes one, an `image` widget shows one, and `blob.free`
//! releases one. Ids are host-generated, so a plug-in can only name a blob it was actually given.

use std::collections::BTreeMap;

use crate::{Error, Result};

/// Largest single blob an instance holds, by default (`Limits::max_blob_bytes`).
pub const MAX_BLOB_BYTES: usize = 256 << 20;
/// Total blob bytes one instance holds, by default (`Limits::max_blob_total_bytes`).
pub const MAX_TOTAL_BYTES: usize = 1 << 30;

/// A blob name: `b:` and a counter. Plug-ins never choose one.
pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("b:").is_some_and(|n| !n.is_empty() && n.len() <= 20 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The blobs one plug-in instance owns.
#[derive(Debug, Clone)]
pub struct Blobs {
    next: u64,
    max_one: usize,
    max_total: usize,
    held: BTreeMap<String, Vec<u8>>,
    total: usize,
}

impl Default for Blobs {
    fn default() -> Self {
        Self::with_caps(MAX_BLOB_BYTES, MAX_TOTAL_BYTES)
    }
}

impl Blobs {
    /// With explicit caps (the host's [`crate::Limits`], so tests can shrink them).
    pub fn with_caps(max_one: usize, max_total: usize) -> Self {
        Self { next: 0, max_one, max_total, held: BTreeMap::new(), total: 0 }
    }

    /// Stores `bytes` under a fresh id. Crossing a cap is an error rather than an eviction, so the
    /// operation that would cross it is the one that reports it.
    pub fn put(&mut self, bytes: Vec<u8>) -> Result<String> {
        if bytes.len() > self.max_one {
            return Err(Error::Limit(format!("blob is {} bytes (limit {})", bytes.len(), self.max_one)));
        }
        let after = self.total.saturating_add(bytes.len());
        if after > self.max_total {
            return Err(Error::Limit(format!("instance holds {after} blob bytes (limit {})", self.max_total)));
        }
        self.next += 1;
        let id = format!("b:{}", self.next);
        self.total = after;
        self.held.insert(id.clone(), bytes);
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Option<&[u8]> {
        self.held.get(id).map(Vec::as_slice)
    }

    /// Releases a blob. `false` when the id is unknown: `blob.free` is idempotent, not an error.
    pub fn free(&mut self, id: &str) -> bool {
        match self.held.remove(id) {
            Some(bytes) => {
                self.total = self.total.saturating_sub(bytes.len());
                true
            }
            None => false,
        }
    }

    /// How many blobs are held.
    pub fn len(&self) -> usize {
        self.held.len()
    }
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
    /// Total bytes held.
    pub fn total(&self) -> usize {
        self.total
    }
    /// Every id, in creation order.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<(u64, String)> = self.held.keys().filter_map(|k| k.strip_prefix("b:").and_then(|n| n.parse().ok()).map(|n| (n, k.clone()))).collect();
        ids.sort();
        ids.into_iter().map(|(_, id)| id).collect()
    }

    /// Drops every blob, for when the instance stops.
    pub fn clear(&mut self) {
        self.held.clear();
        self.total = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_opaque_and_sequential() {
        let mut b = Blobs::default();
        let first = b.put(vec![1, 2, 3]).unwrap();
        let second = b.put(Vec::new()).unwrap();
        assert_eq!((first.as_str(), second.as_str()), ("b:1", "b:2"));
        assert!(valid_id(&first) && valid_id(&second));
        assert_eq!(b.get("b:1"), Some([1, 2, 3].as_slice()));
        assert_eq!(b.len(), 2);
        assert_eq!(b.total(), 3);
        assert_eq!(b.ids(), ["b:1", "b:2"]);
    }

    #[test]
    fn freeing_is_idempotent_and_releases_the_budget() {
        let mut b = Blobs::with_caps(100, 150);
        let id = b.put(vec![0; 100]).unwrap();
        assert_eq!(b.total(), 100);
        // The second blob crosses the total, and says so instead of evicting the first.
        assert!(matches!(b.put(vec![0; 100]), Err(Error::Limit(_))));
        assert!(b.free(&id));
        assert!(!b.free(&id));
        assert!(!b.free("b:99"));
        assert_eq!(b.total(), 0);
        assert!(b.put(vec![0; 100]).is_ok());
    }

    #[test]
    fn one_blob_over_the_cap_is_refused_and_clearing_empties() {
        let mut b = Blobs::with_caps(8, 64);
        assert!(matches!(b.put(vec![0; 9]), Err(Error::Limit(_))));
        b.put(vec![0; 8]).unwrap();
        b.clear();
        assert!(b.is_empty() && b.total() == 0);
        assert!(b.get("b:1").is_none());
    }

    #[test]
    fn only_host_shaped_ids_are_valid() {
        for bad in ["", "b:", "b:x", "17", "b:1 ", "c:1", "b:123456789012345678901234"] {
            assert!(!valid_id(bad), "{bad:?}");
        }
        assert!(valid_id("b:0"));
        assert!(valid_id("b:18446744073709551615"));
    }
}
