//! Per-open historical sample decisions. Encoded sources remain inert until answered.
use super::{JobCtx, OPEN_JOB, OpenSource, Started};
use crate::{EngineError, Result, Session};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_HELD: usize = 16;
const MAX_HELD_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Held {
    id: u64,
    name: String,
    path: Option<String>,
    bytes: Arc<Vec<u8>>,
}

struct Request {
    name: String,
    source: OpenSource,
    path: Option<String>,
    interactive: bool,
    order: Option<photocraft_io::HistoricalXcfSampleOrder>,
    held: Option<u64>,
}

enum Read {
    Decoded(Box<photocraft_io::ImportResult>, Decision, JobCtx),
    Choice(Arc<Vec<u8>>, photocraft_io::HistoricalXcfSampleOrderRequest, JobCtx),
}

enum Decision {
    Held { question: Option<Value>, damaged: bool, color_settings: Box<crate::color_cmds::ColorSettings>, ignore_exif: bool },
    Prepared { color: Value },
}

/// Immutable request-time policy inputs; worker preparation never borrows the live session.
struct WorkerPolicy {
    color: crate::color_cmds::ColorSettings,
    ignore_exif: bool,
    exr_aces_cg: bool,
    raw_resolution: f64,
    authorize: Option<fn(&str, &Value) -> Result<()>>,
}

impl WorkerPolicy {
    /// Snapshot only settings used by native open policy and preparation authority.
    fn capture(s: &Session) -> Self {
        Self {
            color: s.color.settings.clone(),
            ignore_exif: s.prefs().file_handling.ignore_exif_profile_tag,
            exr_aces_cg: s.prefs().file_handling.exr_aces_cg,
            raw_resolution: s.prefs().raw_defaults.resolution,
            authorize: s.authorize,
        }
    }

    /// Finish color policy and native caches on the execution thread, or retain an unprepared color/damage hold.
    fn prepare(self, name: &str, mut imported: photocraft_io::ImportResult, damaged: bool, interactive: bool, ctx: &JobCtx) -> Result<Read> {
        ctx.check()?;
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
        Ok(Read::Decoded(Box::new(imported), decision, ctx.clone()))
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
    let previous = s.active_index();
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
        if let Some(previous) = previous {
            s.set_active(previous);
        }
        s.sync_tools_mask_target();
    }
    result
}

/// Start an open request with its source path identity and optional interactive historical sample-order handoff.
pub(super) fn start(s: &mut Session, name: &str, source: OpenSource, interactive: bool) -> Result<Started> {
    let path = match &source {
        OpenSource::Path(path) => Some(path.clone()),
        OpenSource::Bytes(_) => None,
    };
    run(s, Request { name: name.into(), source, path, interactive, order: None, held: None }, true)
}

/// Open on the selected execution path while allowing an explicit user choice for ambiguous historical sample bytes.
pub(super) fn interactive(s: &mut Session, name: &str, source: OpenSource, path: Option<String>, background: bool) -> Result<Started> {
    run(s, Request { name: name.into(), source, path, interactive: true, order: None, held: None }, background)
}

/// Validate typed open/resume/cancel controls, consume held bytes on valid attempts and preserve invalid-choice
/// retries.
pub(crate) fn command(s: &mut Session, params: &Value) -> Result<Value> {
    let order = crate::file_cmds::historical_xcf_sample_order(params, OPEN_JOB)?;
    let cancel = match params.get("cancel") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::Other("file.open: cancel must be boolean".into())),
    };
    let interactive = match params.get("interactive") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::Other("file.open: interactive must be boolean".into())),
    };
    let request = if let Some(id) = params.get("heldHistoricalXcfOpen") {
        let id = id.as_u64().ok_or_else(|| EngineError::Other("file.open: heldHistoricalXcfOpen must be an unsigned identity".into()))?;
        let index = s
            .jobs
            .historical_xcf
            .iter()
            .position(|held| held.id == id)
            .ok_or_else(|| EngineError::Other("file.open: historical source choice is no longer available".into()))?;
        if cancel {
            s.jobs.historical_xcf.remove(index);
            return Ok(json!({"cancelled": true, "heldHistoricalXcfOpen": id}));
        }
        if order.is_none() {
            return Err(EngineError::Other("file.open: choose historicalXcfSampleOrder before resuming this file".into()));
        }
        let held = s.jobs.historical_xcf.get(index).cloned().ok_or_else(|| EngineError::Other("historical source choice is gone".into()))?;
        Request { name: held.name, source: OpenSource::Bytes(held.bytes), path: held.path, interactive, order, held: Some(id) }
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
        Request { name: path.clone(), source: OpenSource::Path(path.clone()), path: Some(path), interactive, order, held: None }
    };
    let started = run(s, request, false);
    // Successful installation consumes the held identity in `run`'s apply step.
    // Failed authorization, decoding or cancellation keeps the capability retryable.
    match started? {
        Started::Done(value) => Ok(value),
        Started::Job(_) => Err(EngineError::Other("file.open: unexpected nested background job".into())),
    }
}

impl Session {
    /// Whether this session still owns the encoded source for an unanswered historical XCF choice.
    pub fn holds_historical_xcf_open(&self, id: u64) -> bool {
        self.jobs.historical_xcf.iter().any(|held| held.id == id)
    }

    /// Revoke only existing inert historical sources, validating the entire set before releasing any of them.
    /// This cleanup capability accepts no path or decoding policy and performs no file access.
    pub fn cancel_held_historical_xcf_opens(&mut self, ids: &[u64]) -> Result<()> {
        if ids.len() > MAX_HELD || ids.iter().any(|id| !self.jobs.historical_xcf.iter().any(|held| held.id == *id)) {
            return Err(EngineError::Other("historical source choice is no longer available".into()));
        }
        self.jobs.historical_xcf.retain(|held| !ids.contains(&held.id));
        Ok(())
    }
}

/// Decode before installation or hold bounded encoded bytes for a required choice; cancellation and failure publish no
/// partial document.
fn run(s: &mut Session, request: Request, background: bool) -> Result<Started> {
    let Request { name, source, path, interactive, order, held } = request;
    let label = format!("Opening {name}");
    let name_w = name.clone();
    let mut options = super::import_options(s.prefs());
    options.historical_xcf_sample_order = order;
    let policy = WorkerPolicy::capture(s);
    let work = move |ctx: &JobCtx| {
        ctx.progress(0., "Reading");
        let acquired = match source {
            OpenSource::Bytes(bytes) => super::AcquiredOpenSource::Bytes(bytes),
            OpenSource::Path(path) => ctx.stage(0.0, 0.02, "Reading", |ctl| super::acquire_open_path(&name_w, &path, ctl)).map_err(|error| match error {
                photocraft_io::IoError::Cancelled => EngineError::Cancelled,
                error => EngineError::Other(error.to_string()),
            })?,
        };
        ctx.check()?;
        ctx.progress(0.02, "Decoding");
        let imported = match acquired {
            super::AcquiredOpenSource::Bytes(bytes) => {
                match ctx.stage(0.02, 0.95, "Decoding", |ctl| photocraft_io::import_for_open_with(&name_w, &bytes, &options, ctl)) {
                    Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(question)) if interactive => {
                        ctx.check()?;
                        return Ok(Read::Choice(bytes, question, ctx.clone()));
                    }
                    result => result,
                }
            }
            #[cfg(any(unix, windows))]
            super::AcquiredOpenSource::Psd(source) => ctx.stage(0.02, 0.95, "Decoding", |ctl| photocraft_io::import_source_for_open_with(&name_w, source, ctl)),
        }
        .map_err(|error| match error {
            photocraft_io::IoError::Cancelled => EngineError::Cancelled,
            error => EngineError::Other(error.to_string()),
        })?;
        policy.prepare(&name_w, imported.0, imported.1, interactive, ctx)
    };
    let apply = move |s: &mut Session, result: Read| {
        let Read::Decoded(imported, decision, ctx) = result else {
            let Read::Choice(bytes, question, ctx) = result else { return Err(EngineError::Other("invalid historical open result".into())) };
            ctx.check()?;
            let retained = s
                .jobs
                .historical_xcf
                .iter()
                .try_fold(bytes.len(), |total, held| total.checked_add(held.bytes.len()))
                .ok_or_else(|| EngineError::Other("held historical source bytes overflow".into()))?;
            if s.jobs.historical_xcf.len() >= MAX_HELD || retained > MAX_HELD_BYTES {
                return Err(EngineError::Other("too many historical XCF sources await a sample-order choice; finish or cancel an earlier choice".into()));
            }
            let id = s.jobs.next_historical_xcf.checked_add(1).ok_or_else(|| EngineError::Other("historical source identity limit reached".into()))?;
            s.jobs.next_historical_xcf = id;
            s.jobs.historical_xcf.push(Held { id, name: name.clone(), path, bytes });
            if let Err(error) = ctx.check() {
                s.jobs.historical_xcf.retain(|pending| pending.id != id);
                return Err(error);
            }
            return Ok(
                json!({"heldHistoricalXcfOpen":id,"name":name,"historicalXcfSampleOrderRequired":{"version":question.version,"bitsPerSample":question.bits_per_sample}}),
            );
        };
        ctx.check()?;
        let mut imported = *imported;
        let path = match crate::file_cmds::template_name(s, &name) {
            Some(untitled) => {
                imported.document.name = untitled;
                None
            }
            None => {
                imported.document.name = crate::file_cmds::file_name(&name);
                path
            }
        };
        let value = if let Decision::Held { question, damaged, color_settings, ignore_exif } = decision {
            let conversion = crate::import_report_cmds::summary(imported.document.metadata.import_conversion.as_ref());
            let held = s.hold_open_with_policy(imported.document, imported.source_read_only, imported.initial_view, Some(*color_settings), Some(ignore_exif));
            if let Err(error) = ctx.check() {
                let _ = s.take_held_open_with_view(held);
                return Err(error);
            }
            json!({"held":held,"name":name,"warnings":imported.warnings,"color":question,"damaged":damaged,"importConversion":conversion})
        } else {
            let Decision::Prepared { color } = decision else { return Err(EngineError::Other("invalid prepared open decision".into())) };
            let (index, id) = install_prepared(s, imported.document, path, imported.source_read_only, imported.initial_view, &ctx)?;
            let conversion = s.active().map(|state| crate::import_report_cmds::summary(state.doc.metadata.import_conversion.as_ref())).unwrap_or(Value::Null);
            json!({"document":index,"docId":id.0,"name":name,"warnings":imported.warnings,"color":color,"importConversion":conversion})
        };
        if let Some(id) = held {
            s.jobs.historical_xcf.retain(|pending| pending.id != id);
        }
        Ok(value)
    };
    if background {
        s.start_job(OPEN_JOB, json!({"name":label}), &label, false, work, apply)
    } else {
        super::run(s, &label, false, work, apply).map(Started::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let Read::Decoded(imported, Decision::Held { question: Some(_), damaged: false, .. }, _) =
            WorkerPolicy::capture(&s).prepare("native.pcraft", make(), false, true, &JobCtx::new()).unwrap()
        else {
            panic!("color hold expected")
        };
        let photocraft_doc::LayerContent::Smart(smart) = &imported.document.layers[0].content else { panic!("native Smart") };
        assert!(smart.cache.is_none(), "pending color choice must precede native preparation");
        s.color.settings.ask_on_missing = false;
        let Read::Decoded(_, Decision::Held { question: None, damaged: true, .. }, _) =
            WorkerPolicy::capture(&s).prepare("native.pcraft", make(), true, true, &JobCtx::new()).unwrap()
        else {
            panic!("damage hold expected")
        };
        let mut raw = make();
        raw.warnings.push(photocraft_io::raw::DEVELOPED_NOTE.into());
        let mut policy = WorkerPolicy::capture(&s);
        policy.raw_resolution = 123.;
        let Read::Decoded(imported, Decision::Prepared { color }, _) = policy.prepare("native.pcraft", raw, false, false, &JobCtx::new()).unwrap() else {
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
        let Read::Decoded(imported, Decision::Prepared { .. }, _) = policy.prepare("source.exr", exr, false, false, &JobCtx::new()).unwrap() else {
            panic!("prepared EXR expected")
        };
        assert_eq!(imported.document.icc_profile, Some(photocraft_cms::Builtin::AcesCg.profile().to_bytes()));
        for policy in [crate::color_cmds::Policy::Preserve, crate::color_cmds::Policy::Convert, crate::color_cmds::Policy::Off] {
            s.color.settings.policy_rgb = policy;
            let mut input = make();
            input.document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
            let mut expected = input.document.clone();
            let expected_color = s.color.open_policy(&mut expected, s.prefs().file_handling.ignore_exif_profile_tag);
            let Read::Decoded(imported, Decision::Prepared { color }, _) =
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

    /// Start the inline interactive fixture and extract its one-shot held-source identifier after verifying the
    /// requested precision.
    fn hold(s: &mut Session) -> u64 {
        let Started::Done(value) = interactive(s, "source.bin", OpenSource::Bytes(source()), Some("source.bin".into()), false).unwrap() else {
            panic!("inline open started a job")
        };
        assert_eq!(value["historicalXcfSampleOrderRequired"]["bitsPerSample"], 16);
        value["heldHistoricalXcfOpen"].as_u64().unwrap()
    }

    #[test]
    #[cfg(any(unix, windows))]
    /// Keep PSD channels file-backed despite a misleading suffix, while native names retain upstream dispatch priority.
    fn path_acquisition_streams_psd_magic_and_preserves_native_name_priority() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.bin");
        let mut seed = Session::new();
        seed.execute("file.new", json!({"width":2,"height":3})).unwrap();
        let bytes = photocraft_io::export(&seed.active().unwrap().doc, "source.psd", &Default::default()).unwrap().bytes;
        std::fs::write(&path, &bytes).unwrap();
        let path = path.to_str().unwrap();
        let super::super::AcquiredOpenSource::Psd(source) = super::super::acquire_open_path("source.bin", path, &photocraft_raster::Interrupt::NONE).unwrap()
        else {
            panic!("PSD magic must select file-backed parsing")
        };
        assert_eq!(source.read_vec(0, 4).unwrap(), b"8BPS");
        for name in ["folder/source.PcRaFt", "folder\\source.pcraft"] {
            let super::super::AcquiredOpenSource::Bytes(retained) = super::super::acquire_open_path(name, path, &photocraft_raster::Interrupt::NONE).unwrap()
            else {
                panic!("native extension must retain native decoder priority")
            };
            assert_eq!(retained.as_slice(), bytes.as_slice());
        }
        let mut opened = Session::new();
        opened.color.settings.ask_on_missing = false;
        opened.execute("file.open", json!({"path":path})).unwrap();
        assert_eq!(opened.active().unwrap().doc.size, seed.active().unwrap().doc.size);
        assert!(opened.jobs.historical_xcf.is_empty());
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

    #[test]
    /// Require an unanswered historical import to leave the active document intact and cancel to release its encoded
    /// bytes exactly once.
    fn historical_choice_holds_original_without_installing_and_cancel_releases_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        let id = hold(&mut s);
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert!(command(&mut s, &json!({"heldHistoricalXcfOpen":id})).is_err());
        assert!(command(&mut s, &json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":"native"})).is_err());
        assert_eq!(s.jobs.historical_xcf.len(), 1);
        s.execute("file.open", json!({"heldHistoricalXcfOpen":id,"cancel":true})).unwrap();
        assert!(s.jobs.historical_xcf.is_empty());
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        assert!(command(&mut s, &json!({"heldHistoricalXcfOpen":id,"cancel":true})).is_err());
    }

    #[test]
    /// Exercise both explicit byte orders and verify native serialization removes the need for a repeated historical
    /// choice.
    fn historical_explicit_choices_open_read_only_and_native_reopen_needs_no_choice() {
        for order in ["little-endian", "big-endian"] {
            let mut s = Session::new();
            s.color.settings.ask_on_missing = false;
            let id = hold(&mut s);
            s.execute("file.open", json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":order})).unwrap();
            assert!(s.jobs.historical_xcf.is_empty());
            let state = s.active().unwrap();
            assert!(state.source_read_only);
            assert_eq!(state.protected_import_source.as_deref(), Some("source.bin"));
            assert!(state.path.is_none());
            let bytes = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
            let native = photocraft_io::import("reopened.pcraft", &bytes).unwrap();
            assert_eq!(native.document.size, state.doc.size);
        }
    }

    #[test]
    /// Reject noninteractive ambiguity, malformed controls and excess held sources without installing a document.
    fn historical_headless_and_invalid_params_refuse_without_document_mutation() {
        let mut s = Session::new();
        assert!(
            run(
                &mut s,
                Request { name: "source.bin".into(), source: OpenSource::Bytes(source()), path: None, interactive: false, order: None, held: None },
                false
            )
            .is_err()
        );
        for params in [json!({"historicalXcfSampleOrder":17}), json!({"cancel":"yes"}), json!({"heldHistoricalXcfOpen":-1}), json!({})] {
            assert!(s.execute("file.open", params).is_err());
        }
        assert!(s.documents().is_empty());
        assert!(s.jobs.historical_xcf.is_empty());
        for _ in 0..MAX_HELD {
            hold(&mut s);
        }
        assert!(interactive(&mut s, "overflow", OpenSource::Bytes(source()), None, false).is_err());
        assert_eq!(s.jobs.historical_xcf.len(), MAX_HELD);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Observe completed worker threads without receiving or applying their queued payloads; yield only until a bounded deadline.
    fn historical_workers_finished_without_poll(s: &Session) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !s.jobs.running.iter().all(|job| job.handle.as_ref().is_some_and(std::thread::JoinHandle::is_finished)) {
            assert!(std::time::Instant::now() < deadline, "tiny historical-source workers must finish before the test deadline");
            std::thread::yield_now();
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Apply simultaneous ready choices under the held-source count limit and release rejected or cancelled payloads without installing a document.
    fn asynchronous_historical_choices_enforce_admission_when_ready_payloads_are_polled() {
        use crate::jobs::JobOutcome;
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        let mut identities = Vec::new();
        let mut sources = Vec::new();
        let source_len = source().len();
        for index in 0..=MAX_HELD {
            let bytes = source();
            sources.push(Arc::downgrade(&bytes));
            let Started::Job(id) = s.start_open_for_user_with_path(&format!("queued-{index}.xcf"), OpenSource::Bytes(bytes), None).unwrap() else {
                panic!("historical fixture must start a native worker")
            };
            identities.push(id);
        }
        historical_workers_finished_without_poll(&s);
        assert_eq!(s.jobs.running.len(), MAX_HELD + 1);
        assert!(s.jobs.historical_xcf.is_empty(), "worker completion alone does not admit a held choice");
        assert!(sources.iter().all(|bytes| bytes.strong_count() == 1), "each ready payload alone owns its distinct encoded source");
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        let events = s.poll_jobs();
        assert_eq!(events.len(), MAX_HELD + 1);
        assert_eq!(events.iter().filter(|event| matches!(event.outcome, JobOutcome::Done(_))).count(), MAX_HELD);
        let failed: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.outcome {
                JobOutcome::Failed(error) => Some((event.id, error)),
                _ => None,
            })
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, *identities.last().unwrap());
        assert!(failed[0].1.contains("too many historical XCF sources"));
        assert_eq!(s.jobs.historical_xcf.len(), MAX_HELD);
        assert_eq!(s.jobs.historical_xcf.iter().map(|held| held.bytes.len()).sum::<usize>(), MAX_HELD * source_len);
        assert!(sources.iter().take(MAX_HELD).all(|bytes| bytes.strong_count() == 1));
        assert!(sources.last().unwrap().upgrade().is_none(), "capacity rejection releases its ready source payload");
        assert!(s.jobs.running.is_empty());
        assert!(s.held_opens.is_empty());
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
        let held_ids: Vec<_> = s.jobs.historical_xcf.iter().map(|held| held.id).collect();
        for id in held_ids {
            assert_eq!(s.execute("file.open", json!({"heldHistoricalXcfOpen":id,"cancel":true})).unwrap()["cancelled"], true);
        }
        assert!(sources.iter().all(|bytes| bytes.upgrade().is_none()), "cancelling admitted choices releases every retained source");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Cancel a completed but unapplied choice and prove its queued encoded bytes are dropped without creating any hold.
    fn cancelling_ready_historical_choice_releases_payload_before_polling() {
        use crate::jobs::JobOutcome;
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let original = s.active().unwrap().doc.clone();
        let bytes = source();
        let weak = Arc::downgrade(&bytes);
        let Started::Job(id) = s.start_open_for_user_with_path("cancel-ready.xcf", OpenSource::Bytes(bytes), None).unwrap() else {
            panic!("historical fixture must start a native worker")
        };
        historical_workers_finished_without_poll(&s);
        assert_eq!(weak.strong_count(), 1, "completed ready payload retains its source before cancellation");
        assert!(s.jobs.historical_xcf.is_empty());
        assert!(s.cancel_job(id));
        s.join_cancelled_jobs();
        assert!(weak.upgrade().is_none(), "discarding the receiver drops the completed source payload");
        assert!(!s.cancel_job(id), "a cancelled ready job cannot be consumed twice");
        let events = s.poll_jobs();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, id);
        assert_eq!(events[0].outcome, JobOutcome::Cancelled);
        assert!(s.jobs.running.is_empty());
        assert!(s.jobs.historical_xcf.is_empty());
        assert!(s.held_opens.is_empty());
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
    }

    #[test]
    /// Failed or cancelled continuation remains recoverable until an explicit held-only cancellation releases it.
    fn historical_failed_and_cancelled_resume_keep_source_until_explicit_cancel() {
        for cancelled in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":2,"height":2})).unwrap();
            let original = s.active().unwrap().doc.clone();
            let mut malformed = (*source()).clone();
            malformed.truncate(30); // Valid precision header; missing properties/pointers.
            let Started::Done(value) = interactive(&mut s, "broken.bin", OpenSource::Bytes(Arc::new(malformed)), None, false).unwrap() else {
                panic!("inline")
            };
            let id = value["heldHistoricalXcfOpen"].as_u64().unwrap();
            if cancelled {
                let ctx = JobCtx::new();
                ctx.cancel();
                s.set_inline_job_ctx(Some(ctx));
            }
            assert!(s.execute("file.open", json!({"heldHistoricalXcfOpen":id,"historicalXcfSampleOrder":"little-endian"})).is_err());
            assert_eq!(s.jobs.historical_xcf.len(), 1);
            assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
            assert_eq!(s.documents().len(), 1);
            s.set_inline_job_ctx(None);
            s.cancel_held_historical_xcf_opens(&[id]).unwrap();
            assert!(s.jobs.historical_xcf.is_empty());
        }
    }

    #[test]
    /// A stale member cannot partially cancel a held set, and cleanup never admits any of its source documents.
    fn historical_batch_cancel_validates_every_capability_before_releasing_sources() {
        let mut session = Session::new();
        let first = hold(&mut session);
        let second = hold(&mut session);
        assert!(session.cancel_held_historical_xcf_opens(&[first, u64::MAX]).is_err());
        assert_eq!(session.jobs.historical_xcf.len(), 2);
        assert!(session.jobs.historical_xcf.iter().any(|held| held.id == first));
        assert!(session.jobs.historical_xcf.iter().any(|held| held.id == second));
        session.cancel_held_historical_xcf_opens(&[first, second]).unwrap();
        assert!(session.jobs.historical_xcf.is_empty());
        assert!(session.documents().is_empty());
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
}
