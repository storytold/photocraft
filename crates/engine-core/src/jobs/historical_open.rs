//! Explicit source-specific continuations for Open only. Color/damage holds stay separate.
use super::{OpenSource, SourceFingerprint, Started};
use crate::{EngineError, Result, Session};
use photocraft_io::{HistoricalXcfOrder, HistoricalXcfSampleOrder, HistoricalXcfSampleOrderRequest, ImportOptions};
use serde_json::{Value, json};

#[derive(Clone)]
pub(super) struct Settings {
    pub options: ImportOptions,
    pub as_ext: Option<String>,
    pub origin: Option<String>,
}

pub(super) struct Held {
    id: u64,
    name: String,
    pub source: OpenSource,
    question: HistoricalXcfSampleOrderRequest,
    settings: Settings,
}

/// A one-shot answer is neither an action recording nor a permission to read a path.
pub(crate) fn answer_command(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "file.resolveHistoricalXcfOpen";
    let bad = |msg: &str| EngineError::BadParams { cmd: COMMAND.into(), msg: msg.into() };
    let id = params.get("id").and_then(Value::as_u64).filter(|id| *id != 0).ok_or_else(|| bad("id must be a positive request identity"))?;
    let boolean = |field: &str, default: bool| match params.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(bad("cancel and background must be booleans")),
    };
    let cancel = boolean("cancel", false)?;
    let background = boolean("background", true)?;
    let order = crate::file_cmds::historical_xcf_sample_order(params, COMMAND)?;
    if cancel {
        if order.is_some() {
            return Err(bad("a cancelled request cannot also select a sample order"));
        }
        let cancelled = session.cancel_historical_xcf_open(id);
        return Ok(json!({"id": id, "cancelled": cancelled, "alreadyReleased": !cancelled}));
    }
    let order = order.ok_or_else(|| bad("choose little-endian or big-endian, or cancel"))?;
    match session.resume_historical_xcf_open(id, order, background)? {
        Started::Job(job) => Ok(json!({"id": id, "job": job.0})),
        Started::Done(result) => Ok(json!({"id": id, "result": result})),
    }
}

#[cfg(any(unix, windows))]
#[allow(clippy::too_many_arguments)]
pub(super) fn import_path(
    name: &str,
    path: &str,
    as_ext: Option<&str>,
    options: &ImportOptions,
    expected: Option<&SourceFingerprint>,
    ctl: &photocraft_raster::Interrupt,
    opened: &mut Option<photocraft_io::FileIdentity>,
    capture: &mut photocraft_io::HistoricalXcfSourceCapture,
) -> std::result::Result<(photocraft_io::ImportResult, bool), photocraft_io::IoError> {
    let identity = match expected {
        None => None,
        Some(SourceFingerprint::Path { identity, .. }) => Some(identity),
        Some(SourceFingerprint::Bytes { .. }) => return Err(photocraft_io::IoError::SourceChanged),
    };
    match as_ext {
        Some(ext) => photocraft_io::import_file_for_open_verified_as_with_historical_source(
            name,
            std::path::Path::new(path),
            ext,
            identity,
            options,
            ctl,
            opened,
            capture,
        ),
        None => photocraft_io::import_file_for_open_verified_with_historical_source(name, std::path::Path::new(path), identity, options, ctl, opened, capture),
    }
}

impl Session {
    /// Opt-in interactive Open: require a choice for unmarked historical samples.
    /// Existing `start_open*` compatibility entrypoints keep their current policy.
    pub fn start_open_for_user_with_options(&mut self, name: &str, source: OpenSource, options: &ImportOptions, background: bool) -> Result<Started> {
        self.start_historical_open(name, source, None, options, background)
    }

    /// Opt-in forced XCF Open As, using the same worker and held-source lifecycle.
    /// This narrow continuation does not change other forced decoders.
    pub fn start_open_as_for_user_with_options(
        &mut self,
        name: &str,
        source: OpenSource,
        as_ext: &str,
        options: &ImportOptions,
        background: bool,
    ) -> Result<Started> {
        let ext = as_ext.trim_start_matches('.').to_ascii_lowercase();
        if !photocraft_io::is_xcf_input_name(&format!("source.{ext}")) {
            return Err(EngineError::BadParams { cmd: "file.openAs".into(), msg: "historical continuation requires an XCF decoder".into() });
        }
        self.start_historical_open(name, source, Some(ext), options, background)
    }

    fn start_historical_open(&mut self, name: &str, source: OpenSource, as_ext: Option<String>, options: &ImportOptions, background: bool) -> Result<Started> {
        let mut options = *options;
        // Estimates are capabilities for a previous source, never fresh-source defaults.
        options.psd_approved_estimate = None;
        if options.historical_xcf_order == HistoricalXcfOrder::Default {
            options.historical_xcf_order = HistoricalXcfOrder::RequireExplicit;
        }
        let origin = match &source {
            OpenSource::Path(path) => Some(path.clone()),
            OpenSource::Bytes(_) => None,
        };
        let settings = Settings { options, as_ext, origin };
        self.start_open_inner(name, source, true, options.psd_high_memory_approved, None, None, background, Some(settings))
    }

    pub(super) fn hold_historical_open(
        &mut self,
        name: String,
        source: OpenSource,
        question: HistoricalXcfSampleOrderRequest,
        settings: Settings,
        byte_limit: u64,
    ) -> Result<Value> {
        let id = self.store_historical_open(name, source, question, settings, byte_limit)?;
        Ok(json!({"heldHistoricalXcfOpen": id, "historicalXcfSampleOrderRequired": {"version": question.version, "bitsPerSample": question.bits_per_sample}}))
    }

    fn store_historical_open(
        &mut self,
        name: String,
        source: OpenSource,
        question: HistoricalXcfSampleOrderRequest,
        settings: Settings,
        byte_limit: u64,
    ) -> Result<u64> {
        // A path is not an admitted continuation: confirmation must never reopen it.
        if !matches!(source, OpenSource::Bytes(_)) {
            return Err(EngineError::Other("historical Open requires its verified encoded bytes".into()));
        }
        if self.pending_preflights.len().saturating_add(self.jobs.historical_opens.len()) >= Self::MAX_PENDING_PREFLIGHTS {
            return Err(EngineError::Other("too many source decisions are waiting for confirmation".into()));
        }
        if self.pending_preflight_bytes_with(&source)? > byte_limit {
            return Err(EngineError::Other("too much encoded source data is waiting for confirmation".into()));
        }
        let id = self.jobs.next_historical_open.checked_add(1).ok_or_else(|| EngineError::Other("historical Open request identities exhausted".into()))?;
        self.jobs.historical_opens.try_reserve(1).map_err(|_| EngineError::Other("cannot reserve a historical Open decision".into()))?;
        self.jobs.next_historical_open = id;
        self.jobs.historical_opens.push(Held { id, name, source, question, settings });
        Ok(id)
    }

    /// Retain already-authorized encoded bytes after a host receives the typed
    /// historical question. This grants no path authority or document admission;
    /// the host remains responsible for matching its own request identities.
    pub fn retain_authorized_historical_xcf_bytes(
        &mut self,
        name: &str,
        bytes: std::sync::Arc<Vec<u8>>,
        question: HistoricalXcfSampleOrderRequest,
        options: &ImportOptions,
    ) -> Result<u64> {
        let mut options = *options;
        options.historical_xcf_order = HistoricalXcfOrder::RequireExplicit;
        let settings = Settings { options, as_ext: None, origin: None };
        self.store_historical_open(name.into(), OpenSource::Bytes(bytes), question, settings, Self::MAX_PENDING_PREFLIGHT_BYTES)
    }

    /// Borrow the exact retained interpretation without consuming it or opening
    /// a path. A host must check that it owns this identity before exposing data.
    /// Failed host decoding/admission can retry; success or cancellation releases
    /// the request through `cancel_historical_xcf_open`.
    pub fn historical_xcf_open_bytes(&self, id: u64) -> Option<(std::sync::Arc<Vec<u8>>, ImportOptions)> {
        let held = self.jobs.historical_opens.iter().find(|held| held.id == id)?;
        let OpenSource::Bytes(bytes) = &held.source else { return None };
        Some((bytes.clone(), held.settings.options))
    }

    /// Whether this exact request is still available. IDs are never reused.
    pub fn holds_historical_xcf_open(&self, id: u64) -> bool {
        self.jobs.historical_opens.iter().any(|held| held.id == id)
    }

    /// The typed question belonging to this still-live request identity.
    pub fn historical_xcf_open_request(&self, id: u64) -> Option<HistoricalXcfSampleOrderRequest> {
        self.jobs.historical_opens.iter().find(|held| held.id == id).map(|held| held.question)
    }

    /// Release only this request; a stale or already-consumed ID changes nothing.
    pub fn cancel_historical_xcf_open(&mut self, id: u64) -> bool {
        let Some(index) = self.jobs.historical_opens.iter().position(|held| held.id == id) else { return false };
        self.jobs.historical_opens.remove(index);
        true
    }

    /// Resume exactly the retained source/options. Failed startup leaves the request
    /// retryable; an accepted worker owns it through completion, failure or cancellation.
    pub fn resume_historical_xcf_open(&mut self, id: u64, order: HistoricalXcfSampleOrder, background: bool) -> Result<Started> {
        let held = self
            .jobs
            .historical_opens
            .iter()
            .find(|held| held.id == id)
            .ok_or_else(|| EngineError::Other("historical Open request is no longer available".into()))?;
        let name = held.name.clone();
        let source = held.source.clone();
        let mut settings = held.settings.clone();
        settings.options.historical_xcf_order = HistoricalXcfOrder::Explicit(order);
        let expected = SourceFingerprint::of(&source)?;
        let started =
            self.start_open_inner(&name, source, true, settings.options.psd_high_memory_approved, None, Some(expected), background, Some(settings))?;
        self.cancel_historical_xcf_open(id);
        Ok(started)
    }
}

#[cfg(test)]
mod tests;
