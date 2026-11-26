//! Scoped historical questions retain only already-read bytes in engine storage.
use std::path::Path;
use std::sync::Arc;

use photocraft_io::{HistoricalXcfOrder, ImportOptions};
use serde_json::{Value, json};

use super::{AutomationError, Filesystem, Headless, files};

pub(super) struct Owned {
    requested: String,
    save_path: Option<String>,
}

fn bad(message: &str) -> AutomationError {
    AutomationError::BadRequest(message.into())
}
fn owned_string(value: &str) -> Result<String, AutomationError> {
    if value.len() > crate::security::MAX_REQUEST_BYTES {
        return Err(bad("historical source path exceeds the request allowance"));
    }
    let mut copy = String::new();
    copy.try_reserve_exact(value.len()).map_err(|_| bad("cannot reserve historical source provenance"))?;
    copy.push_str(value);
    Ok(copy)
}
fn name(requested: &str) -> &str {
    Path::new(requested).file_name().and_then(|name| name.to_str()).unwrap_or(requested)
}

impl Headless {
    /// Explicit opt-in for an authenticated, rooted open. Default opens retain
    /// their typed question error. No pathname or encoded buffer is re-read on answer.
    pub fn open_with_historical_hold(&mut self, path: &Path, options: &ImportOptions) -> Result<Value, AutomationError> {
        let requested = path.to_str().ok_or_else(|| bad("automation paths must be valid UTF-8"))?;
        if requested.len() > crate::security::MAX_REQUEST_BYTES {
            return Err(bad("historical source path exceeds the request allowance"));
        }
        let Filesystem::Workspace(workspace) = &self.filesystem else {
            return Err(bad("historical source holds require an automation read-root workspace"));
        };
        let (bytes, save_path) = workspace.read_with_save_path(requested)?;
        let mut options = *options;
        if options.historical_xcf_order == HistoricalXcfOrder::Default {
            options.historical_xcf_order = HistoricalXcfOrder::RequireExplicit;
        }
        match self.session.with_native_xcf_import_authority(|| files::open_bytes_with_options(name(requested), &bytes, &options)) {
            Ok(opened) => self.admit_opened(requested, opened, save_path),
            Err(AutomationError::Import(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(question))) => {
                // The engine owns the sole source queue and shares its byte/count
                // limits with GUI questions and PSD approvals. This sidecar is
                // bounded metadata; stale external revocations do not accumulate.
                self.historical_opens.retain(|id, _| self.session.holds_historical_xcf_open(*id));
                if self.historical_opens.len() >= photocraft_engine::CoreSession::MAX_PENDING_PREFLIGHTS {
                    return Err(bad("too many historical source decisions are waiting"));
                }
                self.historical_opens.try_reserve(1).map_err(|_| bad("cannot reserve historical request ownership"))?;
                let owned = Owned { requested: owned_string(requested)?, save_path };
                let id = self.session.retain_authorized_historical_xcf_bytes(name(requested), Arc::new(bytes), question, &options)?;
                self.historical_opens.insert(id, owned);
                Ok(
                    json!({"heldHistoricalXcfOpen":id,"name":name(requested),"historicalXcfSampleOrderRequired":{"version":question.version,"bitsPerSample":question.bits_per_sample},"warnings":[]}),
                )
            }
            Err(error) => Err(error),
        }
    }

    /// Resolve only a question created by this scoped backend. The source and
    /// options remain captured; request parameters cannot replace either one.
    pub fn resolve_historical_xcf_open(&mut self, params: &Value) -> Result<Value, AutomationError> {
        let object = params.as_object().ok_or_else(|| bad("historical answer must be an object"))?;
        if object.keys().any(|key| !matches!(key.as_str(), "id" | "historicalXcfSampleOrder" | "cancel")) {
            return Err(bad("historical answer accepts only id, historicalXcfSampleOrder and cancel"));
        }
        let id = params.get("id").and_then(Value::as_u64).filter(|id| *id != 0).ok_or_else(|| bad("id must be a positive request identity"))?;
        let cancel = match params.get("cancel") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            _ => return Err(bad("cancel must be a boolean")),
        };
        let order = photocraft_engine::file_cmds::historical_xcf_sample_order(params, "doc.resolveHistoricalXcfOpen")?;
        if cancel && order.is_some() {
            return Err(bad("a cancelled request cannot also select a sample order"));
        }
        if !cancel && order.is_none() {
            return Err(bad("choose little-endian or big-endian, or cancel"));
        }
        let owned = self.historical_opens.get(&id).ok_or_else(|| bad("historical source request does not belong to this automation backend"))?;
        if !self.session.holds_historical_xcf_open(id) {
            self.historical_opens.remove(&id);
            return Err(bad("historical source request is no longer available"));
        }
        if cancel {
            self.session.cancel_historical_xcf_open(id);
            self.historical_opens.remove(&id);
            return Ok(json!({"id":id,"cancelled":true}));
        }
        let requested = owned_string(&owned.requested)?;
        let save_path = owned.save_path.as_deref().map(owned_string).transpose()?;
        let (bytes, mut options) = self.session.historical_xcf_open_bytes(id).ok_or_else(|| bad("historical source request is no longer available"))?;
        let order = order.ok_or_else(|| bad("choose little-endian or big-endian, or cancel"))?;
        options.historical_xcf_order = HistoricalXcfOrder::Explicit(order);
        let opened = self.session.with_native_xcf_import_authority(|| files::open_bytes_with_options(name(&requested), &bytes, &options))?;
        let result = self.admit_opened(&requested, opened, save_path)?;
        // Failed decode/admission remains retryable. A successful admission or
        // an explicit owned cancellation consumes this one request only.
        self.session.cancel_historical_xcf_open(id);
        self.historical_opens.remove(&id);
        Ok(json!({"id":id,"result":result}))
    }
}
