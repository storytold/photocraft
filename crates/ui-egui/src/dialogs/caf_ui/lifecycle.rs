use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use craft::behavior::PixelSize;
use craft::imaging::ui::fill::{FillCommand, FillEvent, FillLimits, FillOpen, FillStrokeEvent};
use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::runtime::env::Label;
use photocraft_engine::edit_menu_cmds::{MAX_CAF_EDITS, MAX_CAF_POINTS};
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::preview::{Base, compute_base, plan_params};
use super::{CMD, CafApp, Hosted, PROXY_SIDE, PhotocraftApp, REMEMBERED};
use crate::dialogs::shared_picture::SharedPicture;

/// Opens the workspace on the active layer and its selection.
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    if !app.session.is_enabled(CMD) {
        return Err("Content-Aware Fill needs a selection on a pixel layer".into());
    }
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    let canvas = st.doc.bounds();
    let (revision, document) = (st.revision, st.doc.id);
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let doc = Arc::clone(&st.doc);
    let k = (canvas.width().max(canvas.height()) as usize).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = ((canvas.width() as usize).div_ceil(k), (canvas.height() as usize).div_ceil(k));

    // The workspace reads its remembered settings itself; the plan is made for what it opens with.
    let open = FillOpen::new([canvas.width(), canvas.height()], name, FillLimits::new(MAX_CAF_EDITS, MAX_CAF_POINTS));
    let mut workspace = craft::imaging::ui::fill::ContentAwareFillWorkspace::new(open).remember(REMEMBERED);
    let base = compute_base(&doc, layer, &workspace.command())?;
    workspace.set_brush(photocraft_algo::content_aware::default_brush_size(bounds(&base)));

    let picture_size = PixelSize::new(canvas.width(), canvas.height());
    let mut caf = CafApp {
        id: dialog_id(CMD),
        document,
        layer,
        canvas,
        revision,
        doc,
        k,
        pw,
        ph,
        base: Base { window: Rect::EMPTY, rect: Rect::EMPTY, hole: Vec::new(), base: Vec::new() },
        base_px: vec![false; pw * ph],
        hole_px: vec![false; pw * ph],
        mask: vec![false; pw * ph],
        known: workspace.session().clone(),
        sample_all: workspace.settings().sample_all_layers,
        last: None,
        error: None,
        dirty: false,
        image: SharedPicture::new(picture_size),
        picture: SharedPicture::new(picture_size),
        preview_error: Arc::new(Mutex::new(None)),
        preview_runs: Arc::new(AtomicUsize::new(0)),
    };
    if let Some(image) = composite(app, pw, ph) {
        caf.image.replace(image, k as u32);
    }
    caf.set_base(base, &caf.known.strokes.clone());
    caf.dirty = false;
    let source = caf.source(2048);
    let host = craft::ui::dialog::HostedWorkspace::new(workspace, source).held_ok();
    let id = app.framework.dialogs.open(host);
    caf.id = id;
    app.caf = Some(caf);
    Ok(id)
}

/// The visible layers composited at the proxy size, as the canvas shows them (so an Apply to a new layer shows).
fn composite(app: &PhotocraftApp, pw: usize, ph: usize) -> Option<egui::ColorImage> {
    let st = app.session.active()?;
    let buf = photocraft_compose::render_reduced(&st.doc, pw as u32, ph as u32);
    let (display, _) = crate::canvas::canvas_display(app, &st.doc, None);
    Some(crate::canvas::display_image(display.as_deref(), &buf))
}

/// Bounds of the fill area.
pub(super) fn bounds(b: &Base) -> Rect {
    let w = b.rect.width() as usize;
    b.hole.iter().enumerate().filter(|(_, h)| **h).fold(Rect::EMPTY, |r, (i, _)| {
        let (x, y) = (b.rect.x0 + (i % w) as i32, b.rect.y0 + (i / w) as i32);
        r.union(&Rect::new(x, y, x + 1, y + 1))
    })
}

/// The workspace's facts for the automation surface: the app's (the plan's window and areas, the preview, the last
/// error) with the workspace's own fields.
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let c = app.caf.as_ref()?;
    let host = app.framework.dialogs.get_by_id::<Hosted>(c.id)?;
    let workspace = host.workspace();
    let sampled = c.mask.iter().zip(&c.hole_px).filter(|(m, h)| **m && !**h).count();
    let w = c.base.window;
    let mut d = json!({
        "layer": c.layer.0,
        "strokes": workspace.session().strokes.len(),
        "fillEdits": workspace.session().fill_edits.len(),
        "window": [w.x0, w.y0, w.width(), w.height()],
        "fillArea": c.base.hole.iter().filter(|h| **h).count(),
        "sampledProxyPixels": sampled,
        "proxyScale": c.k,
        "settings": serde_json::to_value(workspace.settings()).unwrap_or(Value::Null),
        "previewReady": c.picture.is_ready(),
        "previewRevision": craft::ui::canvas::RasterSource::revision(&*c.picture).get(),
        "previewError": c.preview_error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone(),
        "previewRuns": c.preview_runs.load(std::sync::atomic::Ordering::Relaxed),
        "error": c.error,
    });
    if let (Some(Value::Object(fields)), Some(map)) = (crate::distort_ui::workspace_fields(app, CMD), d.as_object_mut()) {
        for (key, value) in fields {
            map.entry(key).or_insert(value);
        }
    }
    Some(d)
}

/// An Apply or OK that failed: the error in the panel (the caller answers the workspace) and the status bar.
fn report(app: &mut PhotocraftApp, e: &str) {
    if let Some(c) = app.caf.as_mut() {
        c.error = Some(e.to_string());
    }
    app.ui.status = e.to_string();
    app.ui.status_error = true;
}

/// The settings an empty recorded Content-Aware Fill step plays with (#14254).
const RECORDED_SETTINGS: [&str; 7] = ["sampling", "sampleAllLayers", "colorAdaptation", "rotationAdaptation", "scale", "mirror", "output"];

/// The workspace's own memory names Rotation Adaptation `rotation`; an empty recorded step reads `rotationAdaptation`.
const WORKSPACE_ROTATION: &str = "rotation";

/// The settings an empty recorded step plays with, from what the workspace remembers (#14254).
pub(crate) fn recorded_from_workspace(memory: Value) -> Value {
    let Value::Object(mut settings) = memory else { return json!({}) };
    if let Some(rotation) = settings.remove(WORKSPACE_ROTATION) {
        settings.insert("rotationAdaptation".into(), rotation);
    }
    settings.retain(|key, _| RECORDED_SETTINGS.contains(&key.as_str()));
    Value::Object(settings)
}

/// The settings an empty recorded step plays with, in the form the workspace remembers them.
pub(crate) fn recorded_to_workspace(settings: &mut Value) {
    if let Some(settings) = settings.as_object_mut()
        && let Some(rotation) = settings.remove("rotationAdaptation")
    {
        settings.insert(WORKSPACE_ROTATION.into(), rotation);
    }
}

/// `command`'s settings that an empty recorded step plays with.
fn recorded(command: &FillCommand) -> Option<Value> {
    let Ok(Value::Object(mut settings)) = serde_json::to_value(command) else { return None };
    settings.retain(|key, _| RECORDED_SETTINGS.contains(&key.as_str()));
    Some(Value::Object(settings))
}

/// Keeps `settings` in `prefs.dialogs` too, where an empty recorded step reads them: the workspace remembers its
/// own copy in the framework's store, which the engine can't see.
fn remember(app: &mut PhotocraftApp, settings: Value) {
    app.session.prefs.edit(|p| p.dialogs.insert(photocraft_engine::atn_cmds::CAF_WORKSPACE_PREFS.into(), settings));
}

/// The background job a command's reply says is still running.
fn pending_job(reply: &Value) -> Option<JobId> {
    (reply.get("pending") == Some(&Value::Bool(true))).then(|| reply.get("job").and_then(Value::as_u64).map(JobId)).flatten()
}

/// A background fill ended: its settings are remembered only when it was applied, not when it failed or was
/// cancelled (#14254).
pub(crate) fn on_job_event(app: &mut PhotocraftApp, e: &JobEvent) {
    let Some(i) = app.jobs.caf_settings.iter().position(|(job, _)| *job == e.id) else { return };
    let (_, settings) = app.jobs.caf_settings.remove(i);
    if matches!(e.outcome, JobOutcome::Done(_)) {
        remember(app, settings);
    }
}

/// Fills with `command` (one history step); the workspace stays as it is. A refusal is reported and keeps the
/// settings of the last fill that ran.
pub(super) fn apply(app: &mut PhotocraftApp, command: &FillCommand) -> Result<(), String> {
    let layer = app.caf.as_ref().ok_or("Content-Aware Fill is not open")?.layer;
    match app.run(CMD, plan_params(command, layer)) {
        Ok(reply) => {
            if let Some(settings) = recorded(command) {
                match pending_job(&reply) {
                    // A background fill can still fail or be cancelled: its settings wait for the job's end.
                    Some(job) => app.jobs.caf_settings.push((job, settings)),
                    None => remember(app, settings),
                }
            }
            Ok(())
        }
        Err(e) => {
            report(app, &e);
            Err(e)
        }
    }
}

fn host(app: &mut PhotocraftApp, id: DialogId) -> Option<&mut Hosted> {
    app.framework.dialogs.get_mut_by_id::<Hosted>(id)
}

/// The "Don't show again" preference of the alert that choosing Custom shows (#14249).
pub(crate) const CUSTOM_SAMPLING_PREF: &str = "contentAwareFill.customSampling";

/// The alert choosing Custom sampling shows, as in Photoshop (#14249).
pub(crate) const CUSTOM_SAMPLING_ALERT: &str = "Use the Sampling Brush Tool to add to the sampling area in order to fill the selected region.";

/// Acts on what the workspace reported since the last call: the host's answer to OK, the events of its passes and
/// the colour the picker child returned. Every pass may call it: what it drains is gone for the next call, so a
/// second pass in one frame finds nothing to apply twice.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(id) = app.caf.as_ref().map(|c| c.id) else { return };
    let Some(hosted) = host(app, id) else {
        // Finished (OK answered, Cancel, or cancelled from outside): the workspace remembered its settings.
        let _ = app.framework.dialogs.take_by_id::<Hosted>(id);
        app.caf = None;
        return;
    };
    let events = hosted.take_events();
    let ok = hosted.take_ok();
    let custom = hosted.workspace().settings().sampling == craft::imaging::ui::fill::SamplingMode::Custom;

    // `rebased`: the plan's inputs outside the session changed (the document, Sample All Layers).
    let (mut replan, mut rebased, mut committed) = (false, false, false);
    let (mut applies, mut picks, mut custom_chosen) = (Vec::new(), Vec::new(), false);
    if let Some(c) = app.caf.as_mut() {
        for event in events {
            match event {
                FillEvent::Stroke(FillStrokeEvent::Begin(s)) => c.begin_stroke(&s),
                FillEvent::Stroke(FillStrokeEvent::Update(s)) => c.extend_stroke(&s),
                FillEvent::Stroke(FillStrokeEvent::Commit(s)) => {
                    c.commit_stroke(&s);
                    committed = true;
                }
                // The painted part is taken back: the planes are rebuilt from the session.
                FillEvent::Stroke(_) => {
                    c.last = None;
                    c.dirty = true;
                    replan |= !c.known.strokes.is_empty() || c.mask != c.base_px;
                }
                // Choosing Custom (again) clears the sampling area and asks for the Sampling Brush (#14249).
                FillEvent::Reset(craft::imaging::ui::fill::FillReset::Sampling) if custom => (replan, custom_chosen) = (true, true),
                FillEvent::FillArea(_) | FillEvent::Reset(_) => replan = true,
                FillEvent::SettingsChanged(settings) => {
                    if settings.sample_all_layers != c.sample_all {
                        c.sample_all = settings.sample_all_layers;
                        (replan, rebased) = (true, true);
                    }
                }
                FillEvent::Apply(command) => applies.push(command),
                FillEvent::PickColor(request) => picks.push(request),
                _ => {}
            }
        }
    }

    // A fill the engine refused stays reported: the replan that follows (the document changed under the workspace)
    // starts from a clean error otherwise.
    let mut refused = None;
    for command in applies {
        let answer = apply(app, &command);
        if let Err(e) = &answer {
            refused = Some(e.clone());
        }
        if let Some(h) = host(app, id) {
            h.workspace_mut().answer_apply(answer);
        }
    }
    if let Some(command) = ok {
        let answer = apply(app, &command);
        if let Err(e) = &answer {
            refused = Some(e.clone());
        }
        let answer = answer.map_err(Label::from);
        if let Some(h) = host(app, id) {
            h.answer_ok(answer);
        }
    }

    if sync_document(app) {
        (replan, rebased) = (true, true);
    }
    reconcile(app, id, replan, rebased, committed);
    if let (Some(e), Some(c)) = (refused, app.caf.as_mut()) {
        c.error = Some(e);
    }

    for request in picks {
        crate::color_picker_ui::open_for_request(app, id, request);
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(h) = host(app, id)
    {
        h.workspace_mut().set_color(&key, color);
    }

    // The workspace reads a new source only when the planes or the document changed (its pending key follows).
    let limit = ctx.input(|i| i.max_texture_side);
    if let Some(c) = app.caf.as_mut()
        && std::mem::take(&mut c.dirty)
    {
        let source = c.source(limit);
        if let Some(h) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
            h.set_source(source);
        }
        ctx.request_repaint();
    }

    if custom_chosen {
        let _ = crate::dialogs::alerts::alert_once(app, tl!(CUSTOM_SAMPLING_ALERT), CUSTOM_SAMPLING_PREF);
    }
}

/// The document changed under the open workspace (an Apply): a new composite and document. `true` when it did, for
/// a new plan.
fn sync_document(app: &mut PhotocraftApp) -> bool {
    let Some(c) = app.caf.as_ref() else { return false };
    let Some(st) = app.session.active() else { return false };
    if st.doc.id != c.document || st.revision == c.revision {
        return false;
    }
    let (revision, doc, k) = (st.revision, Arc::clone(&st.doc), c.k as u32);
    let image = composite(app, c.pw, c.ph);
    let Some(c) = app.caf.as_mut() else { return false };
    c.revision = revision;
    c.doc = doc;
    if let Some(image) = image {
        c.image.replace(image, k);
    }
    c.dirty = true;
    true
}

/// Brings the planes to the workspace's session. Undo and redo change the session without an event, so it is
/// compared with the one the planes were made for: strokes alone repaint, a changed sampling option or fill area
/// (and `replan`) plan again. A plan that refuses a lasso edit (an empty fill area) is reported back to the
/// workspace, which takes the edit out; the refusal stays shown while a plan of the same session over the same inputs
/// (`rebased` false) follows, as nothing it could answer changed.
fn reconcile(app: &mut PhotocraftApp, id: DialogId, replan: bool, rebased: bool, committed: bool) {
    let Some((session, command)) = app.framework.dialogs.get_by_id::<Hosted>(id).map(|h| (h.workspace().session().clone(), h.workspace().command())) else {
        return;
    };
    let Some(c) = app.caf.as_mut() else { return };
    if !replan && session == c.known {
        return;
    }
    c.dirty = true;
    if !replan && session.sampling == c.known.sampling && session.fill_edits == c.known.fill_edits {
        // A committed stroke was painted as it was drawn; undo and redo of strokes repaint.
        if !committed {
            c.restate_all(&session.strokes);
        }
        c.error = None;
        c.known = session;
        return;
    }
    match compute_base(&c.doc, c.layer, &command) {
        Ok(base) => {
            c.set_base(base, &session.strokes);
            if rebased || session != c.known {
                c.error = None;
            }
            c.known = session;
        }
        Err(e) => {
            c.error = Some(e.clone());
            let edited = session.fill_edits != c.known.fill_edits;
            match app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
                Some(h) if edited => {
                    h.workspace_mut().refuse_fill_edit(e);
                    let now = h.workspace().session().clone();
                    c.restate_all(&now.strokes);
                    c.known = now;
                }
                _ => {
                    c.restate_all(&session.strokes);
                    c.known = session;
                }
            }
        }
    }
}
