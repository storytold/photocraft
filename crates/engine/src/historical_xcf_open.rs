//! Bounded encoded sources awaiting an explicit historical XCF sample-order choice.
use super::{JobCtx, OPEN_JOB, OpenSource, Started};
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
    path: Option<String>,
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

/// Retain the source identity alongside the exact options and bounded encoded allocation.
pub(super) fn hold_with_path(
    s: &mut Session,
    name: String,
    bytes: Arc<Vec<u8>>,
    question: photocraft_io::HistoricalXcfSampleOrderRequest,
    options: photocraft_io::ImportOptions,
    path: Option<String>,
) -> Result<Value> {
    let result = hold(s, name, bytes, question, options)?;
    if let Some(held) = s.jobs.historical_xcf.last_mut() {
        held.path = path;
    }
    Ok(result)
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
    s.jobs.historical_xcf.push(Held { id, name, bytes, options, path: None });
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
        self.start_open_transaction(
            name,
            source,
            true,
            options.psd_high_memory_approved,
            options.psd_approved_estimate,
            None,
            background,
            Some(*options),
            None,
            false,
        )
    }

    /// Resume the exact retained bytes once. Invalid or missing choices must be rejected by
    /// the host before this typed API; cancellation or failure releases the job's source.
    pub fn resume_historical_xcf_open(&mut self, id: u64, order: photocraft_io::HistoricalXcfSampleOrder, background: bool) -> Result<Started> {
        self.resume_historical_xcf_open_inner(id, order, background, false)
    }

    fn resume_historical_xcf_open_inner(
        &mut self,
        id: u64,
        order: photocraft_io::HistoricalXcfSampleOrder,
        background: bool,
        in_command: bool,
    ) -> Result<Started> {
        let index = self
            .jobs
            .historical_xcf
            .iter()
            .position(|held| held.id == id)
            .ok_or_else(|| EngineError::Other("historical XCF source choice is no longer available".into()))?;
        let held = self.jobs.historical_xcf.remove(index);
        let mut options = held.options;
        options.historical_xcf_sample_order = Some(order);
        self.start_open_transaction(
            &held.name,
            OpenSource::Bytes(held.bytes),
            true,
            options.psd_high_memory_approved,
            options.psd_approved_estimate,
            None,
            background,
            Some(options),
            held.path,
            in_command,
        )
    }

    /// Cancel a still-pending encoded source without admitting a document.
    pub fn cancel_historical_xcf_open(&mut self, id: u64) -> bool {
        let before = self.jobs.historical_xcf.len();
        self.jobs.historical_xcf.retain(|held| held.id != id);
        before != self.jobs.historical_xcf.len()
    }
}

/// Prepared worker data stays private until the transaction admits it atomically.
pub(super) struct PreparedOpen {
    imported: photocraft_io::ImportResult,
    decision: Decision,
    ctx: JobCtx,
}

enum Decision {
    Held { question: Option<Value>, damaged: bool, color_settings: Box<crate::color_cmds::ColorSettings>, ignore_exif: bool },
    Prepared { color: Value },
}

/// Immutable request-time policy inputs; worker preparation never borrows the live session.
pub(super) struct WorkerPolicy {
    color: crate::color_cmds::ColorSettings,
    ignore_exif: bool,
    exr_aces_cg: bool,
    raw_resolution: f64,
    refuse_preview: bool,
    authorize: Option<fn(&str, &Value) -> Result<()>>,
}

impl WorkerPolicy {
    /// Snapshot only settings used by native open policy and preparation authority.
    pub(super) fn capture(s: &Session) -> Self {
        Self {
            color: s.color.settings.clone(),
            ignore_exif: s.prefs().file_handling.ignore_exif_profile_tag,
            exr_aces_cg: s.prefs().file_handling.exr_aces_cg,
            raw_resolution: crate::file_cmds::raw_open_resolution(s),
            refuse_preview: s.batch_open.is_some() || s.batch_open_notices.is_some(),
            authorize: s.authorize,
        }
    }

    /// Finish color policy and native caches on the execution thread, or retain an unprepared color/damage hold.
    pub(super) fn prepare(self, name: &str, mut imported: photocraft_io::ImportResult, damaged: bool, interactive: bool, ctx: &JobCtx) -> Result<PreparedOpen> {
        ctx.check()?;
        if self.refuse_preview && imported.preview_only {
            return Err(EngineError::Other(format!("{name}: only an embedded preview could be read; Batch cannot process it as the source")));
        }
        crate::file_cmds::tag_exr_colorspace_with_policy(self.exr_aces_cg, name, &mut imported.document);
        crate::file_cmds::raw_resolution(self.raw_resolution, &imported.warnings, &mut imported.document);
        if let Some(view) = &imported.initial_view {
            view.validate(&imported.document).map_err(|error| EngineError::Other(error.to_string()))?;
        }
        let mut color = crate::color_cmds::ColorState::default();
        color.settings = self.color.clone();
        // A held source capability grants no new access to a user-configured ICC
        // path. Check the actual captured working profile before resolving it.
        let working = match crate::color_cmds::mode_space(imported.document.mode) {
            photocraft_cms::ColorSpace::Rgb => Some(&self.color.working_rgb),
            photocraft_cms::ColorSpace::Cmyk => Some(&self.color.working_cmyk),
            photocraft_cms::ColorSpace::Gray => Some(&self.color.working_gray),
            _ => None,
        };
        if let Some(profile) = working {
            let _authority = self.authorize.map(|gate| crate::jobs::render_authorization_scope(Some(gate)));
            crate::jobs::authorize_render_command("edit.profileInfo", &json!({"profile":profile}))?;
        }
        let raw = imported.warnings.iter().any(|warning| warning.contains(photocraft_io::raw::DEVELOPED_NOTE));
        let question = color.open_question(&imported.document).filter(|_| !raw);
        let damaged = interactive && damaged;
        let decision = if damaged || question.is_some() {
            // The eventual answer must use the working spaces described by the
            // captured question, even if live settings change before UI polling.
            Decision::Held { question, damaged, color_settings: Box::new(self.color), ignore_exif: self.ignore_exif }
        } else {
            ctx.progress(0.95, "Preparing layers");
            let report = color.open_policy(&mut imported.document, self.ignore_exif);
            ctx.check()?;
            if crate::type_extra_cmds::has_type_layers(&imported.document) {
                ctx.progress(0.95, "Loading fonts");
                let _ = photocraft_text::shared();
                ctx.check()?;
            }
            // The job runner already carries inherited authority. Do not replace
            // that scope with None when a nested caller has no session-local gate.
            let _authority = self.authorize.map(|gate| crate::jobs::render_authorization_scope(Some(gate)));
            crate::import_prepare::prepare_imported_document(&mut imported.document, ctx)?;
            super::warm_layer_bounds(&imported.document, || ctx.check())?;
            if let Some(view) = &imported.initial_view {
                view.validate(&imported.document).map_err(|error| EngineError::Other(error.to_string()))?;
            }
            Decision::Prepared { color: report }
        };
        ctx.check()?;
        Ok(PreparedOpen { imported, decision, ctx: ctx.clone() })
    }
}

/// Install already-prepared data with source protection before any fallible view handoff; rollback keeps the old tab active.
fn install_prepared(
    s: &mut Session,
    document: photocraft_doc::Document,
    path: Option<String>,
    source_read_only: bool,
    view: Option<photocraft_io::NativeInitialView>,
    ctx: &JobCtx,
) -> Result<(usize, photocraft_doc::DocId)> {
    ctx.check()?;
    if let Some(view) = &view {
        view.validate(&document).map_err(|error| EngineError::Other(error.to_string()))?;
    }
    let previous = s.active().map(|state| state.doc.id);
    let index = s.add_document(document, path.clone().filter(|_| !source_read_only));
    if let Some(state) = s.active_mut() {
        state.source_read_only = source_read_only;
        state.protected_import_source = path.filter(|_| source_read_only);
    }
    let result = (|| {
        ctx.check()?;
        s.apply_import_initial_view(view)?;
        ctx.check()?;
        let id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
        Ok((index, id))
    })();
    if result.is_err() {
        let _ = s.close(index);
        if let Some(previous) = previous.and_then(|id| s.documents().iter().position(|state| state.doc.id == id)) {
            s.set_active(previous);
        }
        s.sync_tools_mask_target();
    }
    result
}

/// Install only prepared data, retaining captured prompt policy and one-time view state.
pub(super) fn install(s: &mut Session, name: &str, path: Option<String>, prepared: PreparedOpen) -> Result<Value> {
    let PreparedOpen { mut imported, decision, ctx } = prepared;
    ctx.check()?;
    let path = match crate::file_cmds::template_name(s, name) {
        Some(untitled) => {
            imported.document.name = untitled;
            None
        }
        None => {
            imported.document.name = crate::file_cmds::file_name(name);
            path
        }
    };
    let has_initial_view = imported.initial_view.is_some();
    let initial_path_selection = imported.initial_view.as_ref().and_then(|view| view.selected_path);
    let value = if let Decision::Held { question, damaged, color_settings, ignore_exif } = decision {
        let conversion = crate::import_report_cmds::summary(imported.document.metadata.import_conversion.as_ref());
        let held = s.hold_open_with_policy(imported.document, imported.source_read_only, imported.initial_view, Some(*color_settings), Some(ignore_exif));
        if let Err(error) = ctx.check() {
            let _ = s.take_held_open_with_view(held);
            return Err(error);
        }
        json!({"held":held,"name":name,"warnings":imported.warnings,"color":question,"damaged":damaged,"importConversion":conversion,"hasInitialView":has_initial_view,"initialPathSelection":initial_path_selection})
    } else {
        let Decision::Prepared { color } = decision else { return Err(EngineError::Other("invalid prepared open decision".into())) };
        let (index, id) = install_prepared(s, imported.document, path, imported.source_read_only, imported.initial_view, &ctx)?;
        crate::file_cmds::batch_open_notice(s, name, &imported.warnings, &color);
        crate::file_cmds::remember_batch_open(s);
        let conversion = s.active().map(|state| crate::import_report_cmds::summary(state.doc.metadata.import_conversion.as_ref())).unwrap_or(Value::Null);
        json!({"document":index,"docId":id.0,"name":name,"warnings":imported.warnings,"color":color,"importConversion":conversion,"hasInitialView":has_initial_view,"initialPathSelection":initial_path_selection})
    };
    Ok(value)
}

/// The shared transaction owns decoding, approval and historical source admission.
pub(super) fn interactive(s: &mut Session, name: &str, source: OpenSource, path: Option<String>, background: bool) -> Result<Started> {
    s.start_open_transaction(name, source, true, false, None, None, background, None, path, false)
}

/// Validate every control before consuming a retained source. A valid resume is one attempt.
pub(crate) fn command(s: &mut Session, params: &Value) -> Result<Value> {
    let order = crate::file_cmds::historical_xcf_sample_order(params, OPEN_JOB)?;
    let boolean = |name: &str| -> Result<bool> {
        match params.get(name) {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            _ => Err(EngineError::Other(format!("file.open: {name} must be boolean"))),
        }
    };
    let cancel = boolean("cancel")?;
    let interactive = boolean("interactive")?;
    let approve = boolean("approveHighMemory")?;
    let background = s.jobs.spawn;
    let started = if let Some(id) = params.get("heldHistoricalXcfOpen") {
        let id = id.as_u64().ok_or_else(|| EngineError::Other("file.open: heldHistoricalXcfOpen must be an unsigned identity".into()))?;
        if !s.holds_historical_xcf_open(id) {
            return Err(EngineError::Other("historical XCF source choice is no longer available".into()));
        }
        if cancel {
            s.cancel_historical_xcf_open(id);
            return Ok(json!({"cancelled":true,"heldHistoricalXcfOpen":id}));
        }
        let order = order.ok_or_else(|| EngineError::Other("file.open: choose historicalXcfSampleOrder before resuming this file".into()))?;
        // Dispatch must retain ownership of its pending worker metadata on resume too.
        s.resume_historical_xcf_open_inner(id, order, background, true)?
    } else {
        if cancel {
            return Err(EngineError::Other("file.open: cancel requires a held historical source".into()));
        }
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| EngineError::Other("file.open: path is required".into()))?
            .to_string();
        let mut options = super::import_options(s.prefs());
        options.historical_xcf_sample_order = order;
        s.start_open_transaction(&path, OpenSource::Path(path.clone()), interactive, approve, None, None, background, Some(options), Some(path.clone()), true)?
    };
    match started {
        Started::Done(value) => Ok(value),
        Started::Job(id) => Ok(json!({"job":id.0})),
    }
}

impl Session {
    /// Whether this session still owns the inert encoded source capability.
    pub fn holds_historical_xcf_open(&self, id: u64) -> bool {
        self.jobs.historical_xcf.iter().any(|held| held.id == id)
    }

    /// Validate the entire set before releasing any retained source; this never opens files.
    pub fn cancel_held_historical_xcf_opens(&mut self, ids: &[u64]) -> Result<()> {
        if ids.len() > MAX_HELD || ids.iter().any(|id| !self.holds_historical_xcf_open(*id)) {
            return Err(EngineError::Other("historical source choice is no longer available".into()));
        }
        self.jobs.historical_xcf.retain(|held| !ids.contains(&held.id));
        Ok(())
    }
}

#[cfg(test)]
#[path = "historical_xcf_open/tests.rs"]
mod tests;

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn historical_command_validation_and_bulk_cancel_do_not_partially_consume_sources() {
        let mut session = Session::new();
        let mut ids = Vec::new();
        for name in ["first.xcf", "second.xcf"] {
            let bytes = source();
            let question = photocraft_io::historical_xcf_sample_order_request(name, &bytes).unwrap().unwrap();
            let result = super::hold(&mut session, name.into(), bytes, question, Default::default()).unwrap();
            ids.push(result["heldHistoricalXcfOpen"].as_u64().unwrap());
        }
        for params in [
            json!({"heldHistoricalXcfOpen":ids[0]}),
            json!({"heldHistoricalXcfOpen":ids[0],"historicalXcfSampleOrder":"native"}),
            json!({"heldHistoricalXcfOpen":ids[0],"historicalXcfSampleOrder":"big-endian","interactive":17}),
            json!({"heldHistoricalXcfOpen":ids[0],"cancel":true,"approveHighMemory":"yes"}),
        ] {
            assert!(command(&mut session, &params).is_err());
            assert!(ids.iter().all(|id| session.holds_historical_xcf_open(*id)));
        }
        assert!(session.cancel_held_historical_xcf_opens(&[ids[0], u64::MAX]).is_err());
        assert!(ids.iter().all(|id| session.holds_historical_xcf_open(*id)));
        session.cancel_held_historical_xcf_opens(&ids).unwrap();
        assert!(session.jobs.historical_xcf.is_empty());
        assert!(session.documents().is_empty());
    }

    #[test]
    fn approved_native_open_rejects_a_different_byte_snapshot_before_preparation() {
        let bytes = native_source();
        let expected = super::super::SourceFingerprint::of(&OpenSource::Bytes(Arc::new(bytes.as_ref().clone()))).unwrap();
        let mut session = Session::new();
        assert!(matches!(
            session.start_open_transaction("native.pcraft", OpenSource::Bytes(bytes), false, true, None, Some(expected), false, None, None, false),
            Err(EngineError::SourceChanged { .. })
        ));
        assert!(session.documents().is_empty());
        assert!(session.pending_preflights.is_empty());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn registered_open_obeys_dispatch_worker_policy_and_installs_only_when_polled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker-native.pcraft");
        std::fs::write(&path, native_source().as_slice()).unwrap();
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        let params = json!({"path":path.to_str().unwrap()});
        let Started::Job(id) = session.start("file.open", params.clone()).unwrap() else { panic!("registered open must inherit background dispatch") };
        assert!(session.documents().is_empty(), "the worker cannot publish a document before polling");
        let running = session.jobs.running.iter().find(|job| job.id == id).unwrap();
        assert_eq!(running.command, "file.open");
        assert_eq!(running.params, params, "outer dispatch owns the original command metadata");
        let opened = session.wait_job(id).unwrap();
        assert!(opened.get("document").is_some(), "{opened}");
        assert_eq!(session.documents().len(), 1);
        let photocraft_doc::LayerContent::Smart(smart) = &session.active().unwrap().doc.layers[0].content else {
            panic!("native Smart Filter must remain editable")
        };
        assert!(smart.cache.is_some(), "worker preparation must finish before installation");
    }

    /// Author an embedded native Smart Filter whose absent cache requires real preparation.
    fn native_source() -> Arc<Vec<u8>> {
        use photocraft_color::{BlendMode, ColorMode, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent, SmartFilter, SmartObject, SmartSource};
        use photocraft_geom::{Affine, Size};
        let mut inner = Document::new("Independent native input", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
        let mut pixels = photocraft_raster::Surface::new(inner.pixel_format());
        pixels.fill_rect(inner.bounds(), &[1., 0., 0., 1.]);
        inner.layers.push(Layer::new("Original pixels", LayerContent::Raster(pixels)));
        let bytes = photocraft_format::save_to_bytes(&inner, &Default::default()).unwrap();
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "input.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, None);
        smart.smart_filters.push(SmartFilter {
            command: "image.adjustments.invert".into(),
            params: json!({}),
            blend: BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
        let mut outer = Document::new("Native import", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
        outer.layers.push(Layer::new("Editable invert", LayerContent::Smart(smart)));
        Arc::new(photocraft_format::save_to_bytes(&outer, &Default::default()).unwrap())
    }

    #[cfg(not(target_arch = "wasm32"))]
    struct PreparationGate {
        entered: std::sync::mpsc::SyncSender<()>,
        resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    struct PreparationProbe {
        threads: Vec<std::thread::ThreadId>,
        gate: Option<Arc<PreparationGate>>,
        deny: bool,
    }

    #[cfg(not(target_arch = "wasm32"))]
    static PREPARATION_PROBE: std::sync::Mutex<PreparationProbe> = std::sync::Mutex::new(PreparationProbe { threads: Vec::new(), gate: None, deny: false });
    #[cfg(not(target_arch = "wasm32"))]
    static PREPARATION_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(not(target_arch = "wasm32"))]
    /// Observe actual stored-filter authorization and optionally pause or deny native preparation.
    fn observe_preparation(id: &str, _: &Value) -> Result<()> {
        if id != "image.adjustments.invert" {
            return Ok(());
        }
        let (gate, deny) = {
            let mut probe = PREPARATION_PROBE.lock().unwrap();
            probe.threads.push(std::thread::current().id());
            (probe.gate.take(), probe.deny)
        };
        if let Some(gate) = gate {
            gate.entered.send(()).map_err(|error| EngineError::Other(format!("preparation probe signal: {error}")))?;
            gate.resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|error| EngineError::Other(format!("preparation probe resume: {error}")))?;
        }
        if deny { Err(EngineError::Other("test denied native filter preparation".into())) } else { Ok(()) }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Require real native filter preparation on the worker, with captured color settings and no repeated UI-thread render.
    fn background_open_prepares_on_worker_and_installs_without_replaying_policy_or_filters() {
        let _serial = PREPARATION_TEST.lock().unwrap();
        *PREPARATION_PROBE.lock().unwrap() = PreparationProbe { threads: Vec::new(), gate: None, deny: false };
        let main = std::thread::current().id();
        let mut s = Session::new();
        s.color.settings.ask_on_missing = false;
        s.authorize = Some(observe_preparation);
        let Started::Job(id) = interactive(&mut s, "native.pcraft", OpenSource::Bytes(native_source()), Some("native.pcraft".into()), true).unwrap() else {
            panic!("native worker expected")
        };
        // A later UI settings change must not turn the already-started request into a hold.
        s.color.settings.ask_on_missing = true;
        let value = s.wait_job(id).unwrap();
        assert!(value.get("document").is_some());
        assert!(value.get("held").is_none());
        let probe = PREPARATION_PROBE.lock().unwrap();
        assert!(!probe.threads.is_empty(), "fixture must execute real stored native filter preparation");
        assert!(probe.threads.iter().all(|thread| *thread != main), "polling installation must not replay native filters");
        let state = s.active().unwrap();
        let photocraft_doc::LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("editable native Smart layer") };
        let cache = smart.cache.as_ref().expect("worker-prepared cache");
        assert_eq!(cache.pixel(0, 0), vec![0., 1., 1., 1.]);
        assert_eq!(state.path.as_deref(), Some("native.pcraft"));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Cancel while the worker is inside native authorization and prove no partially prepared document is installed.
    fn cancellation_during_worker_preparation_preserves_existing_document() {
        let _serial = PREPARATION_TEST.lock().unwrap();
        let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (resume, resume_rx) = std::sync::mpsc::sync_channel(1);
        let gate = Arc::new(PreparationGate { entered, resume: std::sync::Mutex::new(resume_rx) });
        *PREPARATION_PROBE.lock().unwrap() = PreparationProbe { threads: Vec::new(), gate: Some(gate.clone()), deny: false };
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        s.color.settings.ask_on_missing = false;
        s.authorize = Some(observe_preparation);
        let Started::Job(id) = interactive(&mut s, "cancel.pcraft", OpenSource::Bytes(native_source()), None, true).unwrap() else {
            panic!("native worker expected")
        };
        entered_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("worker must reach real native filter preparation");
        assert!(s.cancel_job(id));
        resume.send(()).unwrap();
        s.join_cancelled_jobs();
        let _ = s.poll_jobs();
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert!(s.jobs.historical_xcf.is_empty());
        assert!(s.held_opens.is_empty());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Preserve inherited render authorization on the worker and reject a denied filter before installation.
    fn worker_preparation_keeps_inherited_authority_and_failure_is_not_installed() {
        let _serial = PREPARATION_TEST.lock().unwrap();
        *PREPARATION_PROBE.lock().unwrap() = PreparationProbe { threads: Vec::new(), gate: None, deny: true };
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        s.color.settings.ask_on_missing = false;
        let _authority = crate::jobs::render_authorization_scope(Some(observe_preparation));
        let Started::Job(id) = interactive(&mut s, "denied.pcraft", OpenSource::Bytes(native_source()), None, true).unwrap() else {
            panic!("native worker expected")
        };
        assert!(s.wait_job(id).is_err());
        assert!(!PREPARATION_PROBE.lock().unwrap().threads.is_empty(), "inherited authorization cannot be erased by a missing session-local gate");
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert!(s.held_opens.is_empty());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Bind a worker's P3 prompt to its captured policy when live settings change before the held import is installed.
    fn background_color_hold_retains_request_policy_before_prompt_installation() {
        let mut s = Session::new();
        s.color.settings.working_rgb = photocraft_cms::Builtin::DisplayP3.id().into();
        s.color.settings.ask_on_missing = true;
        s.edit_prefs(|prefs| prefs.file_handling.ignore_exif_profile_tag = false);
        let captured = s.color.settings.clone();
        let Started::Job(id) = interactive(&mut s, "profile.pcraft", OpenSource::Bytes(native_source()), None, true).unwrap() else {
            panic!("native worker expected")
        };
        s.color.settings.working_rgb = photocraft_cms::Builtin::AdobeRgbCompat.id().into();
        s.edit_prefs(|prefs| prefs.file_handling.ignore_exif_profile_tag = true);
        let live = s.color.settings.clone();
        let value = s.wait_job(id).unwrap();
        let held = value["held"].as_u64().expect("captured missing-profile question");
        assert_eq!(value["color"]["working"].as_str(), Some(photocraft_cms::Builtin::DisplayP3.profile().description.as_str()));
        assert!(s.documents().is_empty());
        let pending = s.take_held_open_with_policy(held).unwrap();
        assert_eq!(pending.color_settings.as_ref(), Some(&captured));
        assert_eq!(pending.ignore_exif_profile_tag, Some(false));
        assert_eq!(s.color.settings, live);
        assert!(s.prefs().file_handling.ignore_exif_profile_tag);
        let photocraft_doc::LayerContent::Smart(smart) = &pending.document.layers[0].content else { panic!("editable pending native Smart layer") };
        assert!(smart.cache.is_none(), "hold remains unprepared until its captured color question is answered");
        assert!(s.take_held_open_with_policy(held).is_none());
    }

    #[test]
    /// Keep pending color/damage choices unprepared and preserve captured raw and EXR policy inputs.
    fn worker_policy_preserves_holds_defaults_and_native_color_policy_parity() {
        let make = || photocraft_io::import("native.pcraft", &native_source()).unwrap();
        let mut s = Session::new();
        s.color.settings.ask_on_missing = true;
        let PreparedOpen { imported, decision: Decision::Held { question: Some(_), damaged: false, .. }, .. } =
            WorkerPolicy::capture(&s).prepare("native.pcraft", make(), false, true, &JobCtx::new()).unwrap()
        else {
            panic!("color hold expected")
        };
        let photocraft_doc::LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("native Smart") };
        assert!(smart.cache.is_none(), "pending color choice must precede native preparation");
        s.color.settings.ask_on_missing = false;
        let PreparedOpen { decision: Decision::Held { question: None, damaged: true, .. }, .. } =
            WorkerPolicy::capture(&s).prepare("native.pcraft", make(), true, true, &JobCtx::new()).unwrap()
        else {
            panic!("damage hold expected")
        };
        let mut raw = make();
        raw.warnings.push(photocraft_io::raw::DEVELOPED_NOTE.into());
        let mut policy = WorkerPolicy::capture(&s);
        policy.raw_resolution = 123.;
        let PreparedOpen { imported, decision: Decision::Prepared { color }, .. } = policy.prepare("native.pcraft", raw, false, false, &JobCtx::new()).unwrap()
        else {
            panic!("prepared raw expected")
        };
        assert_eq!(imported.document.resolution_dpi, 123.);
        let mut expected = make().document;
        let expected_color = s.color.open_policy(&mut expected, s.prefs().file_handling.ignore_exif_profile_tag);
        assert_eq!(color, expected_color);
        assert_eq!(imported.document.icc_profile, expected.icc_profile);
        let mut exr = make();
        exr.document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
        s.color.settings.ask_on_mismatch = false;
        let mut policy = WorkerPolicy::capture(&s);
        policy.exr_aces_cg = true;
        let PreparedOpen { imported, decision: Decision::Prepared { .. }, .. } = policy.prepare("source.exr", exr, false, false, &JobCtx::new()).unwrap()
        else {
            panic!("prepared EXR expected")
        };
        assert_eq!(imported.document.icc_profile, Some(photocraft_cms::Builtin::AcesCg.profile().to_bytes()));
        for policy in [crate::color_cmds::Policy::Preserve, crate::color_cmds::Policy::Convert, crate::color_cmds::Policy::Off] {
            s.color.settings.policy_rgb = policy;
            let mut input = make();
            input.document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
            let mut expected = input.document.clone();
            let expected_color = s.color.open_policy(&mut expected, s.prefs().file_handling.ignore_exif_profile_tag);
            let PreparedOpen { imported, decision: Decision::Prepared { color }, .. } =
                WorkerPolicy::capture(&s).prepare("native.pcraft", input, false, false, &JobCtx::new()).unwrap()
            else {
                panic!("prepared policy expected")
            };
            assert_eq!(color, expected_color);
            assert_eq!(imported.document.icc_profile, expected.icc_profile);
        }
    }

    #[test]
    /// Reject invalid initial view state or cancellation without replacing the active document or losing protection state.
    fn prepared_install_rejects_invalid_view_and_cancel_before_publication() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        let imported = photocraft_io::import("native.pcraft", &native_source()).unwrap();
        let view = photocraft_io::NativeInitialView { target: photocraft_io::NativeImportTarget::AlphaChannel(99), ..Default::default() };
        assert!(install_prepared(&mut s, imported.document.clone(), Some("protected.xcf".into()), true, Some(view), &JobCtx::new()).is_err());
        let ctx = JobCtx::new();
        ctx.cancel();
        assert!(matches!(install_prepared(&mut s, imported.document, Some("protected.xcf".into()), true, None, &ctx), Err(EngineError::Cancelled)));
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert!(!s.active().unwrap().source_read_only);
        assert!(s.active().unwrap().protected_import_source.is_none());
    }

    /// Author a minimal v7 high-depth header whose sample order requires a choice without relying on a private corpus.
    fn source() -> Arc<Vec<u8>> {
        // Public XCF framing: v7, 16-bit nonlinear RGB, empty drawable lists.
        let mut bytes = b"gimp xcf v007\0".to_vec();
        for value in [1u32, 1, 0, 250, 0, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        Arc::new(bytes)
    }

    #[test]
    /// Pin native path preservation, untitled template identity and protected import-only source identity for
    /// registered open commands.
    fn registered_open_preserves_writable_path_template_identity_and_read_only_source() {
        let directory = tempfile::tempdir().unwrap();
        let mut seed = Session::new();
        seed.execute("file.new", json!({"width":2,"height":3})).unwrap();
        let bytes = photocraft_format::save_to_bytes(&seed.active().unwrap().doc, &Default::default()).unwrap();
        for extension in ["pcraft", "psdt"] {
            let path = directory.path().join(format!("original.{extension}"));
            std::fs::write(&path, &bytes).unwrap();
            let path = path.to_string_lossy().into_owned();
            let mut s = Session::new();
            s.color.settings.ask_on_missing = false;
            s.execute("file.open", json!({"path":path})).unwrap();
            let state = s.active().unwrap();
            if extension == "psdt" {
                assert!(state.path.is_none());
                assert!(state.doc.name.starts_with("Untitled-"));
            } else {
                assert_eq!(state.path.as_deref(), Some(path.as_str()));
                assert_eq!(state.doc.name, "original.pcraft");
            }
            assert!(!state.source_read_only);
        }
        let path = directory.path().join("original.bin");
        std::fs::write(&path, source().as_slice()).unwrap();
        let path = path.to_string_lossy().into_owned();
        let mut s = Session::new();
        s.color.settings.ask_on_missing = false;
        s.execute("file.open", json!({"path":path,"historicalXcfSampleOrder":"big-endian"})).unwrap();
        let state = s.active().unwrap();
        assert!(state.path.is_none());
        assert_eq!(state.doc.name, "original.bin");
        assert!(state.source_read_only);
        assert_eq!(state.protected_import_source.as_deref(), Some(path.as_str()));
    }
    #[test]
    #[cfg(any(unix, windows))]
    /// A held XCF decision uses the original acquired bytes even if the named file changes before resuming.
    fn historical_path_choice_reuses_original_bytes_after_file_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.bin");
        let original = source();
        std::fs::write(&path, original.as_slice()).unwrap();
        let path = path.to_str().unwrap();
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        let Started::Done(value) = interactive(&mut session, path, OpenSource::Path(path.into()), Some(path.into()), false).unwrap() else {
            panic!("inline interactive open started a job")
        };
        let id = value["heldHistoricalXcfOpen"].as_u64().unwrap();
        assert!(session.documents().is_empty());
        assert_eq!(session.jobs.historical_xcf[0].bytes.as_slice(), original.as_slice());
        std::fs::write(path, b"replacement is not an image").unwrap();
        session.execute("file.open", json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":"big-endian"})).unwrap();
        assert!(session.jobs.historical_xcf.is_empty());
        let state = session.active().unwrap();
        assert_eq!(state.doc.size, photocraft_geom::Size::new(1, 1));
        assert!(state.source_read_only);
        assert_eq!(state.protected_import_source.as_deref(), Some(path));
    }
}
