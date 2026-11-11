//! Bounded encoded sources awaiting an explicit historical XCF sample-order choice.
use super::{OpenSource, Started};
use crate::{EngineError, Result, Session};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_HELD: usize = 16;
const MAX_HELD_BYTES: usize = 1024 * 1024 * 1024;

pub(super) struct Held {
    id: u64,
    name: String,
    bytes: Arc<Vec<u8>>,
    options: photocraft_io::ImportOptions,
}

pub(super) fn hold(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
) -> Result<Value> {
    hold_with_budget(s, name, bytes, question, options, MAX_HELD_BYTES)
}

fn hold_with_budget(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
    byte_budget: usize,
) -> Result<Value> {
    // A truncated Vec still owns its allocation. Charging its length would let retained
    // source capabilities exceed the pending-choice memory budget without changing the file.
    let retained = s.jobs.historical_xcf.iter().try_fold(bytes.capacity(), |total, held| total.checked_add(held.bytes.capacity()));
    if s.jobs.historical_xcf.len() >= MAX_HELD || retained.is_none_or(|total| total > byte_budget) {
        return Err(EngineError::Other("too many historical XCF sources await a sample-order choice; finish or cancel an earlier choice".into()));
    }
    let id = s.jobs.next_historical_xcf.checked_add(1).ok_or_else(|| EngineError::Other("historical source identity limit reached".into()))?;
    s.jobs.next_historical_xcf = id;
    s.jobs.historical_xcf.push(Held { id, name, bytes, options });
    Ok(json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrderRequired":{"version":question.version,"bitsPerSample":question.bits_per_sample}}))
}

/// Retain exactly the file that produced the typed requirement. Reopening is safe only after
/// checking its handle identity against that import, and again after the bounded snapshot.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn snapshot(path: &str, expected: &photocraft_io::FileIdentity, ctl: &photocraft_raster::Interrupt) -> Result<Arc<Vec<u8>>> {
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    let failed = |error: std::io::Error| EngineError::Other(format!("{path}: {error}"));
    let mut file = std::fs::File::open(path).map_err(failed)?;
    if &photocraft_io::FileIdentity::of_handle(&file).map_err(failed)? != expected {
        return Err(EngineError::SourceChanged { name: path.into() });
    }
    let size = file.metadata().map_err(failed)?.len();
    if size > MAX_HELD_BYTES as u64 {
        return Err(EngineError::Other("historical XCF source exceeds the pending-choice byte budget".into()));
    }
    let reader = SnapshotReader { file: &mut file, ctl, read: 0, size };
    let bytes = photocraft_format::read::read_all(&mut std::io::Read::take(reader, MAX_HELD_BYTES as u64 + 1), size)
        .map_err(|error| if ctl.check().is_err() { EngineError::Cancelled } else { failed(error) })?;
    if bytes.len() as u64 != size || bytes.len() > MAX_HELD_BYTES || &photocraft_io::FileIdentity::of_handle(&file).map_err(failed)? != expected {
        return Err(EngineError::SourceChanged { name: path.into() });
    }
    ctl.check().map_err(|_| EngineError::Cancelled)?;
    Ok(Arc::new(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
struct SnapshotReader<'a, 'b> {
    file: &'a mut std::fs::File,
    ctl: &'a photocraft_raster::Interrupt<'b>,
    read: u64,
    size: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::io::Read for SnapshotReader<'_, '_> {
    fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
        self.ctl.check().map_err(|_| std::io::Error::other("cancelled historical source snapshot"))?;
        // Keep cancellation latency bounded even when read_all requests a large remainder.
        let length = destination.len().min(256 * 1024);
        let destination = destination.get_mut(..length).ok_or_else(|| std::io::Error::other("invalid historical snapshot buffer"))?;
        let count = std::io::Read::read(self.file, destination)?;
        self.read = self.read.saturating_add(count as u64);
        self.ctl.progress((self.read as f64 / self.size.max(1) as f64).min(1.0) as f32);
        Ok(count)
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn snapshot(_: &str, _: &photocraft_io::FileIdentity, _: &photocraft_raster::Interrupt) -> Result<Arc<Vec<u8>>> {
    Err(EngineError::Other("cannot open filesystem paths on the web".into()))
}

impl Session {
    /// Open using explicit per-import options. An unset historical order is never inferred.
    pub fn start_open_with_options(&mut self, name: &str, source: OpenSource, options: &photocraft_io::ImportOptions) -> Result<Started> {
        self.start_open_inner(name, source, false, options.psd_high_memory_approved, options.psd_approved_estimate, None, true, Some(*options))
    }

    /// Interactive counterpart: typed historical, memory and profile questions remain pending.
    pub fn start_open_for_user_with_options(
        &mut self,
        name: &str,
        source: OpenSource,
        options: &photocraft_io::ImportOptions,
        background: bool,
    ) -> Result<Started> {
        self.start_open_inner(name, source, true, options.psd_high_memory_approved, options.psd_approved_estimate, None, background, Some(*options))
    }

    /// Resume the exact retained bytes once. Invalid or missing choices must be rejected by
    /// the host before this typed API; cancellation or failure releases the job's source.
    pub fn resume_historical_xcf_open(&mut self, id: u64, order: photocraft_io::HistoricalXcfSampleOrder, background: bool) -> Result<Started> {
        let index = self
            .jobs
            .historical_xcf
            .iter()
            .position(|held| held.id == id)
            .ok_or_else(|| EngineError::Other("historical XCF source choice is no longer available".into()))?;
        let held = self.jobs.historical_xcf.remove(index);
        let mut options = held.options;
        options.historical_xcf_sample_order = Some(order);
        self.start_open_for_user_with_options(&held.name, OpenSource::Bytes(held.bytes), &options, background)
    }

    /// Cancel a still-pending encoded source without admitting a document.
    pub fn cancel_historical_xcf_open(&mut self, id: u64) -> bool {
        let before = self.jobs.historical_xcf.len();
        self.jobs.historical_xcf.retain(|held| held.id != id);
        before != self.jobs.historical_xcf.len()
    }
}

#[cfg(test)]
mod tests;
