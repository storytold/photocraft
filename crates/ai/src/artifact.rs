use std::collections::BTreeMap;

use crate::{AiError, AiResult, Result};

/// Session-owned bounded artifacts. No file writes and no automatic document apply.
pub struct ArtifactStore {
    results: BTreeMap<u64, (AiResult, usize)>,
    byte_limit: usize,
    entry_limit: usize,
    bytes: usize,
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new(128 * 1024 * 1024, 16)
    }
}

impl ArtifactStore {
    pub fn new(byte_limit: usize, entry_limit: usize) -> Self {
        Self { results: BTreeMap::new(), byte_limit, entry_limit, bytes: 0 }
    }

    pub fn insert(&mut self, id: u64, result: AiResult) -> Result<()> {
        if result.artifacts.is_empty() {
            return Err(AiError::InvalidImage("empty result".into()));
        }
        if self.results.contains_key(&id) {
            return Err(AiError::InvalidRequest("duplicate artifact id".into()));
        }
        let bytes = result.bytes()?;
        let total = self.bytes.checked_add(bytes).ok_or(AiError::StorageFull)?;
        if total > self.byte_limit || self.results.len() >= self.entry_limit {
            return Err(AiError::StorageFull);
        }
        self.results.insert(id, (result, bytes));
        self.bytes = total;
        Ok(())
    }

    pub fn get(&self, id: u64) -> Option<&AiResult> {
        self.results.get(&id).map(|(result, _)| result)
    }
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.results.keys().copied()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn discard(&mut self, id: u64) -> bool {
        let Some((_, bytes)) = self.results.remove(&id) else { return false };
        self.bytes = self.bytes.saturating_sub(bytes);
        true
    }
}
