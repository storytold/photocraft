//! Bounded encoded sources awaiting an explicit historical XCF sample-order choice.
use super::{OpenSource, Started};
use crate::{EngineError, Result, Session};
use photocraft_doc::{DocId, LayerId};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_HELD: usize = 16;
const MAX_HELD_BYTES: usize = 1024 * 1024 * 1024;

pub(super) struct Held {
    id: u64,
    name: String,
    bytes: Arc<Vec<u8>>,
    options: photocraft_io::ImportOptions,
    destination: Destination,
}

enum Destination {
    Open,
    SmartObject,
    Place { doc: DocId, revision: u64, layer: Option<LayerId>, linked: Option<String>, params: Value },
}

pub(super) fn hold(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
) -> Result<Value> {
    hold_with_budget(s, name, bytes, question, options, Destination::Open, MAX_HELD_BYTES)
}

pub(crate) fn hold_smart_object(
    s: &mut Session,
    path: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
) -> Result<Value> {
    hold_with_budget(s, path, bytes, question, options, Destination::SmartObject, MAX_HELD_BYTES)
}

pub(crate) fn hold_place(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
    linked: Option<String>,
    params: Value,
) -> Result<Value> {
    let target = s.active().ok_or(EngineError::NoDocument)?;
    let destination = Destination::Place { doc: target.doc.id, revision: target.revision, layer: target.active_layer, linked, params };
    hold_with_budget(s, name, bytes, question, options, destination, MAX_HELD_BYTES)
}

fn hold_with_budget(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
    destination: Destination,
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
    let field = if matches!(destination, Destination::Place { .. }) { "heldHistoricalXcfPlace" } else { "heldHistoricalXcfOpen" };
    s.jobs.historical_xcf.push(Held { id, name, bytes, options, destination });
    Ok(json!({field:id,"historicalXcfSampleOrderRequired":{"version":question.version,"bitsPerSample":question.bits_per_sample}}))
}

impl Session {
    /// Open using explicit per-import options. An unset historical order is never inferred.
    pub fn start_open_with_options(&mut self, name: &str, source: OpenSource, options: &photocraft_io::ImportOptions) -> Result<Started> {
        let options = crate::file_cmds::fresh_import_options(*options);
        self.start_open_inner(name, source, false, None, true, Some(options))
    }

    /// Interactive counterpart: typed historical, memory and profile questions remain pending.
    pub fn start_open_for_user_with_options(
        &mut self,
        name: &str,
        source: OpenSource,
        options: &photocraft_io::ImportOptions,
        background: bool,
    ) -> Result<Started> {
        let options = crate::file_cmds::fresh_import_options(*options);
        self.start_open_inner(name, source, true, None, background, Some(options))
    }

    /// Resume the exact retained bytes once. A refused startup keeps the capability
    /// available for retry; an accepted job owns its source through completion or cancellation.
    pub fn resume_historical_xcf_open(&mut self, id: u64, order: photocraft_io::HistoricalXcfSampleOrder, background: bool) -> Result<Started> {
        let held = self
            .jobs
            .historical_xcf
            .iter()
            .find(|held| held.id == id)
            .ok_or_else(|| EngineError::Other("historical XCF source choice is no longer available".into()))?;
        // Share the encoded allocation with startup; removing it first would lose
        // the only retry capability if worker registration or inline startup fails.
        let name = held.name.clone();
        let bytes = Arc::clone(&held.bytes);
        let mut options = held.options;
        options.historical_xcf_sample_order = Some(order);
        let started = match &held.destination {
            Destination::Open => self.start_open_for_user_with_options(&name, OpenSource::Bytes(bytes), &options, background)?,
            Destination::SmartObject => crate::file_cmds::resume_smart_object(self, name, bytes, options, background)?,
            Destination::Place { doc, linked, params, .. } => {
                self.validate_historical_xcf_place(id)?;
                if self.active().is_none_or(|state| state.doc.id != *doc) {
                    return Err(EngineError::Other("select the placement document before answering".into()));
                }
                let mut params = params.clone();
                params["historicalXcfSampleOrder"] = json!(order);
                if linked.is_some() {
                    params["verifyHistoricalXcfLink"] = json!(true);
                }
                let mut input = crate::file_cmds::PlaceInput::from_shared_bytes(self, &name, bytes, linked.clone(), &params)?;
                input.set_import_options(options);
                let command = input.command();
                if let Some(conflict) = self.job_conflict(command, true) {
                    return Err(EngineError::Other(conflict));
                }
                self.start_registered(command, params, background || self.cooperative_jobs(), |s| crate::file_cmds::place_input(s, input))?
            }
        };
        self.jobs.historical_xcf.retain(|held| held.id != id);
        Ok(started)
    }

    /// Validate the held drop against its original document and selection.
    pub fn validate_historical_xcf_place(&self, id: u64) -> Result<()> {
        let held = self.jobs.historical_xcf.iter().find(|held| held.id == id);
        let Some(Destination::Place { doc, revision, layer, .. }) = held.map(|held| &held.destination) else {
            return Err(EngineError::Other("the historical XCF placement is no longer available".into()));
        };
        let target =
            self.documents().iter().find(|state| state.doc.id == *doc).ok_or_else(|| EngineError::Other("the placement document was closed".into()))?;
        if target.revision != *revision || target.active_layer != *layer {
            return Err(EngineError::Other("the placement target changed; drop the file again".into()));
        }
        Ok(())
    }

    /// Whether the session still owns the exact encoded source awaiting its choice.
    pub fn holds_historical_xcf_open(&self, id: u64) -> bool {
        self.jobs.historical_xcf.iter().any(|held| held.id == id)
    }

    /// Cancel a still-pending encoded source without admitting a document.
    pub fn cancel_historical_xcf_open(&mut self, id: u64) -> bool {
        let before = self.jobs.historical_xcf.len();
        self.jobs.historical_xcf.retain(|held| held.id != id);
        before != self.jobs.historical_xcf.len()
    }
}

#[cfg(test)]
#[path = "historical_xcf_open/tests.rs"]
mod tests;
