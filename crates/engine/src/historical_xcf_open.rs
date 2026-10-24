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
    Decoded(Box<photocraft_io::ImportResult>, bool, JobCtx),
    Choice(Arc<Vec<u8>>, photocraft_io::HistoricalXcfSampleOrderRequest),
}

pub(super) fn start(s: &mut Session, name: &str, source: OpenSource, interactive: bool) -> Result<Started> {
    let path = match &source {
        OpenSource::Path(path) => Some(path.clone()),
        OpenSource::Bytes(_) => None,
    };
    run(s, Request { name: name.into(), source, path, interactive, order: None, held: None }, true)
}

pub(super) fn interactive(s: &mut Session, name: &str, source: OpenSource, path: Option<String>, background: bool) -> Result<Started> {
    run(s, Request { name: name.into(), source, path, interactive: true, order: None, held: None }, background)
}

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
    let resumed = request.held;
    let started = run(s, request, false);
    // Once the worker owns the exact bytes, cancellation/failure releases them
    // through the normal job lifetime rather than leaving an orphaned choice.
    if let Some(id) = resumed {
        s.jobs.historical_xcf.retain(|held| held.id != id);
    }
    match started? {
        Started::Done(value) => Ok(value),
        Started::Job(_) => Err(EngineError::Other("file.open: unexpected nested background job".into())),
    }
}

fn run(s: &mut Session, request: Request, background: bool) -> Result<Started> {
    let Request { name, source, path, interactive, order, held } = request;
    let label = format!("Opening {name}");
    let name_w = name.clone();
    let mut options = super::import_options(s.prefs());
    options.historical_xcf_sample_order = order;
    let work = move |ctx: &JobCtx| {
        ctx.progress(0., "Reading");
        let bytes = match source {
            OpenSource::Bytes(bytes) => bytes,
            OpenSource::Path(path) => Arc::new(super::read_path(&path)?),
        };
        ctx.check()?;
        ctx.progress(0.02, "Decoding");
        let imported = match ctx.stage(0.02, 0.95, "Decoding", |ctl| photocraft_io::import_for_open_with(&name_w, &bytes, &options, ctl)) {
            Ok(result) => result,
            Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(question)) if interactive => return Ok(Read::Choice(bytes, question)),
            Err(photocraft_io::IoError::Cancelled) => return Err(EngineError::Cancelled),
            Err(error) => return Err(EngineError::Other(error.to_string())),
        };
        ctx.progress(0.95, "Preparing layers");
        super::warm_layer_bounds(&imported.0.document, || ctx.check())?;
        if crate::type_extra_cmds::has_type_layers(&imported.0.document) {
            ctx.progress(0.97, "Loading fonts");
            let _ = photocraft_text::shared();
            ctx.check()?;
        }
        Ok(Read::Decoded(Box::new(imported.0), imported.1, ctx.clone()))
    };
    let apply = move |s: &mut Session, result: Read| {
        let Read::Decoded(imported, damaged, ctx) = result else {
            let Read::Choice(bytes, question) = result else { return Err(EngineError::Other("invalid historical open result".into())) };
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
            return Ok(
                json!({"heldHistoricalXcfOpen":id,"name":name,"historicalXcfSampleOrderRequired":{"version":question.version,"bitsPerSample":question.bits_per_sample}}),
            );
        };
        let mut imported = *imported;
        crate::file_cmds::tag_exr_colorspace(s, &name, &mut imported.document);
        crate::file_cmds::raw_defaults(s, &imported.warnings, &mut imported.document);
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
        if let Some(view) = &imported.initial_view {
            view.validate(&imported.document).map_err(|error| EngineError::Other(error.to_string()))?;
        }
        let raw = imported.warnings.iter().any(|warning| warning.contains(photocraft_io::raw::DEVELOPED_NOTE));
        let damaged = interactive && damaged;
        let question = s.color.open_question(&imported.document).filter(|_| !raw);
        let value = if damaged || question.is_some() {
            let conversion = crate::import_report_cmds::summary(imported.document.metadata.import_conversion.as_ref());
            let held = s.hold_open(imported.document, imported.source_read_only, imported.initial_view);
            json!({"held":held,"name":name,"warnings":imported.warnings,"color":question,"damaged":damaged,"importConversion":conversion})
        } else {
            let (index, color) = s.open_imported_document(imported.document, path.clone().filter(|_| !imported.source_read_only), &ctx)?;
            s.apply_import_initial_view(imported.initial_view)?;
            let id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
            if let Some(state) = s.active_mut() {
                state.source_read_only = imported.source_read_only;
                state.protected_import_source = path.filter(|_| imported.source_read_only);
            }
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

    fn source() -> Arc<Vec<u8>> {
        // Public XCF framing: v7, 16-bit nonlinear RGB, empty drawable lists.
        let mut bytes = b"gimp xcf v007\0".to_vec();
        for value in [1u32, 1, 0, 250, 0, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        Arc::new(bytes)
    }

    fn hold(s: &mut Session) -> u64 {
        let Started::Done(value) = interactive(s, "source.bin", OpenSource::Bytes(source()), Some("source.bin".into()), false).unwrap() else {
            panic!("inline open started a job")
        };
        assert_eq!(value["historicalXcfSampleOrderRequired"]["bitsPerSample"], 16);
        value["heldHistoricalXcfOpen"].as_u64().unwrap()
    }

    #[test]
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

    #[test]
    fn historical_failed_and_cancelled_resume_release_encoded_source() {
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
            assert!(s.jobs.historical_xcf.is_empty());
            assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
            assert_eq!(s.documents().len(), 1);
        }
    }

    #[test]
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
