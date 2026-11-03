//! Edit › Generative Fill (#41): fill the selection from a prompt through the model the person
//! chose in the Craft suite's shared AI layer, reached through the app's [`GenerativeAi`]
//! ([`Session::set_generative_ai`]). With none (the web build, the CLI) or when the shared layer
//! says the tool can't run, the command is disabled with the layer's reason and nothing is sent.
//!
//! The selection's bounds plus some surrounding context are rendered (all visible layers, in
//! sRGB) and sent with a mask of the selection and the prompt, once per variation (three by
//! default, as Photoshop makes), the variations as separate jobs side by side. The results land in
//! a new group masked by the selection, one layer per variation with the first showing, so nothing
//! under it changes and one undo removes it. `layer.generative.selectVariation` flips between them
//! and `layer.generative.more` asks for more.
//!
//! Consent: before anything is sent, the shared layer's plan says whether the person must agree to
//! the destination first. Then the fill waits in the `consent` state (nothing sent) until
//! `layer.generative.agree` records the person's answer through the app's own prompt, or
//! `layer.generative.discard` drops it. Automation can't run these commands (they send document
//! pixels off the computer), so only a person answers.
//!
//! Fills don't lock the document: each one's job edits it with [`crate::jobs::run_on`], so while
//! one is generating the person selects the next area and starts another. Up to [`MAX_PARALLEL`]
//! talk to the service at once and the rest wait their turn (`queue`). A started fill is
//! remembered until its layer lands (above the layer that was active when it started, without
//! making it active) or it is discarded; one that failed waits for `layer.generative.retry` or
//! `layer.generative.discard`, and `layer.generative.fills` lists them for the shell's progress.
//! `area` fills one of the selection's separate areas (`areas`), so several objects selected
//! together are generated side by side.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{DocId, Document, Layer, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::genai::{AiErrorKind, AiFailure, AiStatus, ConsentTerm, Disclosure, FillPlan, FillRequest, SharedAi};
use crate::jobs::{JobCtx, JobId};
use crate::{EngineError, Result, Session};

mod areas;
mod diag;
mod pixels;
mod queue;
mod remote;

pub use areas::MAX_AREAS;
pub use diag::SAVE_DIAGNOSTICS;
pub use queue::MAX_PARALLEL;

pub use pixels::{ColourMatch, Framing};
use pixels::{Placed, Reference, other};

pub const FILL: &str = "edit.generativeFill";
pub const SELECT_VARIATION: &str = "layer.generative.selectVariation";
pub const MORE: &str = "layer.generative.more";
pub const RETRY: &str = "layer.generative.retry";
pub const AGREE: &str = "layer.generative.agree";
pub const DISCARD: &str = "layer.generative.discard";
pub const FILLS: &str = "layer.generative.fills";
pub const AREAS: &str = "layer.generative.areas";
/// The commands that send document pixels to an AI service: automation may not run them.
pub const SENDS: [&str; 4] = [FILL, MORE, RETRY, AGREE];
/// The commands only the person may run: Save Fill Diagnostics writes the pictures a fill sent and
/// got back, and automation must not be able to copy document pixels out through it.
pub const PERSON_ONLY: [&str; 1] = [SAVE_DIAGNOSTICS];
const LABEL: &str = "Generative Fill";
/// Why the tool is off where no app supplied a [`GenerativeAi`] (the web build, the CLI).
pub const NO_AI: &str = "Generative Fill needs the PhotoCraft desktop app.";
/// Longest side sent; larger areas are scaled down for the request and the result is scaled back
/// up.
const UPLOAD_SIDE: u32 = 2048;
/// Variations a fill makes when not told: three, as Photoshop makes, each its own request.
pub const DEFAULT_VARIATIONS: usize = 3;
const MAX_VARIATIONS: u64 = 4;
/// Longest prompt, in bytes.
pub const MAX_PROMPT: usize = 8_000;
/// What is asked for when the prompt is empty.
pub const CONTINUE_PROMPT: &str = "Fill the masked area so the surrounding picture continues through it seamlessly, matching its content, lighting, texture and perspective. Add no new objects.";
/// What a fill waiting for the person's agreement reports as its error.
const NEEDS_CONSENT: &str = "waiting for your agreement to send this";

impl Session {
    /// Install (or with `None`, remove) generative AI. The desktop app installs its link to the
    /// shared AI layer; headless and test sessions have none unless they set one.
    pub fn set_generative_ai(&mut self, ai: Option<SharedAi>) {
        self.genai = ai;
    }

    pub fn generative_ai(&self) -> Option<&SharedAi> {
        self.genai.as_ref()
    }

    /// Can Generative Fill run now? Asked afresh each time (the person may have changed their
    /// choice in another Craft app meanwhile).
    pub fn generative_status(&self) -> AiStatus {
        match &self.genai {
            Some(ai) => ai.status(),
            None => AiStatus::Unavailable { reason: NO_AI.into() },
        }
    }

    /// The fills started in this session that haven't landed: generating, waiting their turn,
    /// waiting for consent, or failed (waiting for Retry or Discard), oldest first.
    pub fn generative_fills(&self) -> Vec<FillInfo> {
        self.genai_state.fills.iter().filter_map(|f| f.info(self)).collect()
    }

    /// The selection fill `id` fills (its shape, for the shell to draw while it generates).
    pub fn generative_fill_mask(&self, id: u64) -> Option<&Surface> {
        self.genai_state.fills.iter().find(|f| f.id == id).map(|f| &f.input.sel)
    }

    /// What fill `id` would send and where, while it waits for the person's agreement.
    pub fn generative_fill_disclosure(&self, id: u64) -> Option<Disclosure> {
        let f = self.genai_state.fills.iter().find(|f| f.id == id)?;
        if !f.status.consent.load(Ordering::Relaxed) {
            return None;
        }
        f.status.plan().map(|p| p.disclosure().clone())
    }

    /// Forget fills whose job ended without a result or an error (cancelled) and fills on closed
    /// documents. The shell calls it when a job ends.
    pub fn tidy_generative_fills(&mut self) {
        let fills = std::mem::take(&mut self.genai_state.fills);
        let (keep, gone): (Vec<_>, Vec<_>) = fills.into_iter().partition(|f| f.info(self).is_some());
        self.genai_state.fills = keep;
        for f in gone {
            if let Some(job) = f.job {
                self.cancel_job(job);
            }
        }
    }
}

/// The session's generative state.
#[derive(Default)]
pub(crate) struct GenState {
    /// What each generative layer made in this session was generated from.
    sources: std::collections::HashMap<LayerId, Arc<Source>>,
    /// Fills that haven't landed.
    fills: Vec<Fill>,
    next_fill: u64,
    /// Turns at the service, shared by every fill and Generate More.
    turns: Arc<queue::Turns>,
    /// How each variation made in this session was lined up and colour-matched, by its layer.
    notes: std::collections::HashMap<LayerId, VariationNote>,
    /// What the last few landed fills sent and got back, for Save Fill Diagnostics (memory only).
    captures: diag::Captures,
}

/// How a variation was placed: how the returned picture was lined up with the area sent, and
/// whether its colours were matched to the photo around the selection (and if not, why).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VariationNote {
    pub framing: Framing,
    pub colour: ColourMatch,
}

impl VariationNote {
    /// How `p` was placed.
    fn of(p: &Placed) -> VariationNote {
        VariationNote { framing: p.framing.clone(), colour: p.colour.clone() }
    }
}

/// A fill that hasn't landed (Generate More included).
struct Fill {
    id: u64,
    /// Its current job (none yet, or the one that ended).
    job: Option<JobId>,
    doc: DocId,
    /// The layer its result goes above: the active layer when it started (for Generate More,
    /// the generative layer it adds to).
    anchor: Option<LayerId>,
    status: Arc<FillStatus>,
    input: Arc<FillInput>,
}

/// What a fill's job reports back while it runs.
#[derive(Default)]
struct FillStatus {
    /// Place in line for a turn at the service (1 = next), 0 when not waiting.
    queued: AtomicUsize,
    /// Why it failed, when it did (it waits for Retry or Discard then), and how the AI layer
    /// classed it.
    error: Mutex<Option<(String, Option<AiErrorKind>)>>,
    /// The plan it sends, kept so the person's agreement applies to exactly what they saw.
    plan: Mutex<Option<Arc<dyn FillPlan>>>,
    /// Waiting for the person to agree to sending it (nothing sent yet).
    consent: AtomicBool,
    /// What a failed fill sent and got back, for Save Fill Diagnostics (memory only).
    capture: Mutex<Option<Arc<diag::Capture>>>,
}

impl FillStatus {
    /// Why it failed, and how the AI layer classed it.
    fn error(&self) -> Option<(String, Option<AiErrorKind>)> {
        self.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_error(&self, e: Option<(String, Option<AiErrorKind>)>) {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = e;
    }

    fn plan(&self) -> Option<Arc<dyn FillPlan>> {
        self.plan.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep (or forget) the plan it sends.
    fn set_plan(&self, plan: Option<Arc<dyn FillPlan>>) {
        *self.plan.lock().unwrap_or_else(PoisonError::into_inner) = plan;
    }

    /// What a failed fill sent and got back, if kept.
    fn capture(&self) -> Option<Arc<diag::Capture>> {
        self.capture.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep (or forget) what a failed fill sent and got back.
    fn set_capture(&self, c: Option<diag::Capture>) {
        *self.capture.lock().unwrap_or_else(PoisonError::into_inner) = c.map(Arc::new);
    }

    /// The error kind of a failure, kept for the shell (it offers setup for `auth`).
    fn fail(&self, f: &AiFailure) {
        self.set_error(Some((f.message.clone(), Some(f.kind))));
    }
}

/// What is sent for a fill, made once from the document as it was when the fill started and
/// kept so Retry and Generate More send it again.
struct Prepared {
    request: FillRequest,
    reference: Reference,
}

/// Everything a fill's job needs.
struct FillInput {
    /// The document as it was when the fill started: what is sent comes from it.
    doc: Arc<Document>,
    /// The selection (or the area of it) filled; the result is masked by it.
    sel: Surface,
    /// Its bounds on the canvas.
    bounds: Rect,
    /// The canvas when it started: the result lands only on a canvas of that size.
    canvas: Rect,
    area: Rect,
    size: (u32, u32),
    prompt: String,
    name: String,
    count: usize,
    /// Generate More: the generative layer the variations join.
    into: Option<LayerId>,
    /// Made by the job the first time it runs (given at the start for Generate More).
    prepared: Mutex<Option<Arc<Prepared>>>,
    /// Make the result the active layer: run inline (scripts, the CLI), the next command works
    /// on it. In the background the person has moved on, so the active layer stays.
    select: bool,
}

impl FillInput {
    /// What is sent: rendered from the document the first time, the same bytes afterwards.
    fn prepared(&self, ctx: &JobCtx) -> Result<Arc<Prepared>> {
        if let Some(p) = self.prepared.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return Ok(p);
        }
        let (w, h) = self.size;
        ctx.progress(0.05, "Preparing the picture");
        let image = pixels::scaled(&pixels::srgb_composite(&self.doc, self.area)?, self.area, w, h);
        let (source_png, mask_png, reference) = pixels::request_images(&image, &self.sel, self.area, w, h)?;
        // Image models need words: an empty prompt asks for the picture to carry on through the
        // selection, which removes what was there.
        let prompt = if self.prompt.is_empty() { CONTINUE_PROMPT.to_owned() } else { self.prompt.clone() };
        let p = Arc::new(Prepared { request: FillRequest { source_png, mask_png, prompt }, reference });
        *self.prepared.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&p));
        Ok(p)
    }
}

/// A fill that hasn't landed, as `layer.generative.fills` and the shell see it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FillInfo {
    pub id: u64,
    pub job: Option<JobId>,
    pub document: DocId,
    /// The layer its result goes above (`None`, or a layer since deleted: above the active one).
    pub anchor: Option<LayerId>,
    /// The name its layer gets (from the prompt).
    pub name: String,
    pub prompt: String,
    /// The selection's bounds, in document pixels: `[x0, y0, x1, y1]`.
    pub bounds: [i32; 4],
    /// `queued` (waiting for a turn), `running`, `consent` (waiting for the person to agree to
    /// sending it; nothing sent) or `failed`.
    pub state: &'static str,
    /// Place in line when queued (1 = next).
    pub queue_position: usize,
    pub progress: f32,
    pub elapsed_ms: f64,
    pub error: Option<String>,
    /// How the AI layer classed the error (`auth`, `quota`, `network`, …), when it did.
    pub error_kind: Option<AiErrorKind>,
    /// Generate More for an existing generative layer.
    pub more: bool,
    /// A failed fill whose request and results Save Fill Diagnostics can write.
    pub diagnostics: bool,
}

impl Fill {
    /// How it stands; `None` once there is nothing to show (it was cancelled, or its document
    /// closed).
    fn info(&self, s: &Session) -> Option<FillInfo> {
        s.documents().iter().find(|st| st.doc.id == self.doc)?;
        let error = self.status.error();
        let job = self.job.and_then(|j| s.job(j));
        if job.is_none() && error.is_none() {
            return None;
        }
        let consent = self.status.consent.load(Ordering::Relaxed);
        let queued = self.status.queued.load(Ordering::Relaxed);
        let state = if consent && job.is_none() {
            "consent"
        } else if error.is_some() && job.is_none() {
            "failed"
        } else if queued > 0 {
            "queued"
        } else {
            "running"
        };
        let b = self.input.bounds;
        let (error, error_kind) = match (state, error) {
            ("failed", Some((e, k))) => (Some(e), k),
            _ => (None, None),
        };
        Some(FillInfo {
            id: self.id,
            job: self.job,
            document: self.doc,
            anchor: self.anchor,
            name: self.input.name.clone(),
            prompt: self.input.prompt.clone(),
            bounds: [b.x0, b.y0, b.x1, b.y1],
            state,
            queue_position: if state == "queued" { queued } else { 0 },
            progress: job.as_ref().map_or(0.0, |j| j.progress),
            elapsed_ms: job.as_ref().map_or(0.0, |j| j.elapsed_ms),
            error,
            error_kind,
            more: self.input.into.is_some(),
            diagnostics: state == "failed" && self.status.capture().is_some(),
        })
    }
}

/// A bad-params error for Generative Fill.
fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: FILL.into(), msg: msg.into() }
}

/// Generative AI is installed and the shared layer says the tool can run: the layer's own reason
/// when it can't, shown as it is.
fn ai_ready(s: &Session) -> std::result::Result<SharedAi, String> {
    let ai = s.generative_ai().cloned().ok_or(NO_AI)?;
    match ai.status() {
        AiStatus::Ready { .. } => Ok(ai),
        AiStatus::Unavailable { reason } => Err(reason),
    }
}

/// Generative Fill needs a document, a selection on the canvas and a tool that can run.
fn fill_enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let sel = st.doc.selection.as_ref().ok_or("no selection")?;
    if sel.content_bounds().intersect(&st.doc.bounds()).is_empty() {
        return Err("the selection is empty".into());
    }
    ai_ready(s).map(|_| ())
}

/// `key` as a variation count, 1 to 4 (3 when missing).
fn variations_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(DEFAULT_VARIATIONS),
        Some(v) => match v.as_u64() {
            Some(n @ 1..=MAX_VARIATIONS) => Ok(n as usize),
            _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("`{key}` must be a whole number from 1 to {MAX_VARIATIONS}") }),
        },
    }
}

/// `area`: which of the selection's separate areas to fill (0-based), if not all of it.
fn area_param(p: &Value) -> Result<Option<usize>> {
    match p.get("area") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|n| Some(usize::try_from(n).unwrap_or(usize::MAX)))
            .ok_or_else(|| bad("`area` must be an area number from 0 (see layer.generative.areas)")),
    }
}

/// The new layer's name: Photoshop names a generative layer after its prompt.
fn layer_name(prompt: &str) -> String {
    let p = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if p.is_empty() {
        return LABEL.into();
    }
    match p.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", p.get(..i).unwrap_or(&p)),
        None => p,
    }
}

/// What a generative layer was made from (what was sent, the prompt and where the result goes),
/// so Generate More can ask again without re-rendering a document that now holds the earlier
/// results. Kept in the session only: a reopened file's generative layer is an ordinary group.
pub(crate) struct Source {
    prompt: String,
    area: Rect,
    prepared: Arc<Prepared>,
}

/// Edit › Generative Fill: check the params, take the selection (or one area of it) and start the fill.
fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = match p.get("prompt") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.trim().to_owned(),
        Some(_) => return Err(bad("`prompt` must be text")),
    };
    if prompt.len() > MAX_PROMPT {
        return Err(bad(format!("the prompt is longer than {MAX_PROMPT} bytes")));
    }
    let count = variations_param(p, "variations", FILL)?;
    let area_index = area_param(p)?;
    fill_enabled(s).map_err(bad)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let mut sel = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
    let canvas = doc.bounds();
    if let Some(k) = area_index {
        let areas = areas::Areas::find(&sel, canvas);
        let n = areas.len();
        sel = areas.mask(&sel, k).ok_or_else(|| bad(format!("the selection has {n} separate areas; `area` counts them from 0")))?;
    }
    let bounds = sel.content_bounds().intersect(&canvas);
    let area = pixels::context_area(bounds, canvas, pixels::request_shape);
    let k = (f64::from(UPLOAD_SIDE) / f64::from(area.width().max(area.height()).max(1))).min(1.0);
    let (w, h) = (((f64::from(area.width()) * k).round() as u32).max(1), ((f64::from(area.height()) * k).round() as u32).max(1));
    let (doc_id, anchor) = (doc.id, st.active_layer);
    let input = FillInput {
        name: layer_name(&prompt),
        bounds,
        doc,
        sel,
        canvas,
        area,
        size: (w, h),
        prompt,
        count,
        into: None,
        prepared: Mutex::new(None),
        select: !s.spawning(),
    };
    let id = add_fill(s, doc_id, anchor, input);
    start(s, id)
}

/// Remember a new fill and return its id.
fn add_fill(s: &mut Session, doc: DocId, anchor: Option<LayerId>, input: FillInput) -> u64 {
    let g = &mut s.genai_state;
    g.next_fill += 1;
    let id = g.next_fill;
    g.fills.push(Fill { id, job: None, doc, anchor, status: Arc::default(), input: Arc::new(input) });
    id
}

/// Run fill `id`'s job: in the background (it stays listed until it lands), or inline.
fn start(s: &mut Session, id: u64) -> Result<Value> {
    let ai = s.generative_ai().cloned().ok_or_else(|| bad(NO_AI))?;
    let fill = s.genai_state.fills.iter().find(|f| f.id == id).ok_or_else(|| bad(format!("no Generative Fill {id} is waiting")))?;
    let (doc, input, status, turns) = (fill.doc, Arc::clone(&fill.input), Arc::clone(&fill.status), Arc::clone(&s.genai_state.turns));
    status.set_error(None);
    status.set_capture(None);
    status.consent.store(false, Ordering::Relaxed);
    let spawning = s.spawning();
    let worker = Arc::clone(&status);
    let r = crate::jobs::run_on(
        s,
        LABEL,
        doc,
        move |ctx| {
            let r = fill_work(ctx, &input, ai.as_ref(), &turns, &worker);
            if let Err(e) = &r
                && !matches!(e, EngineError::Cancelled)
                && worker.error().is_none()
            {
                worker.set_error(Some((e.to_string(), None)));
            }
            r
        },
        move |s, made| land(s, id, made),
    );
    let started = match &r {
        Ok(v) if spawning => v.get("job").and_then(Value::as_u64).map(|j| (v.clone(), JobId(j))),
        _ => None,
    };
    match started {
        Some((mut v, job)) => {
            if let Some(f) = s.genai_state.fills.iter_mut().find(|f| f.id == id) {
                f.job = Some(job);
            }
            if let Some(o) = v.as_object_mut() {
                o.insert("fill".into(), json!(id));
            }
            Ok(v)
        }
        // Inline it landed (and was forgotten) or failed; one waiting for consent stays, so the
        // caller can agree to it. A job that couldn't start is forgotten.
        None => {
            let waits = s.genai_state.fills.iter().any(|f| f.id == id && f.status.consent.load(Ordering::Relaxed));
            if !waits {
                s.genai_state.fills.retain(|f| f.id != id);
            }
            match r {
                Err(_) if waits => Err(EngineError::Other(format!("Generative Fill {id} is {NEEDS_CONSENT} (layer.generative.agree)"))),
                r => r,
            }
        }
    }
}

/// What a fill's job made: the pictures placed, what was sent, and the capture of what came back.
struct Made {
    placed: Vec<Placed>,
    prepared: Arc<Prepared>,
    capture: diag::Capture,
}

/// A fill's AI part: what is sent, the plan (and consent), a turn at the service, the results
/// placed.
fn fill_work(ctx: &JobCtx, input: &FillInput, ai: &dyn crate::genai::GenerativeAi, turns: &queue::Turns, status: &FillStatus) -> Result<Made> {
    let prepared = input.prepared(ctx)?;
    ctx.check()?;
    // The plan the person agreed to, or a fresh one from the model chosen now.
    let plan = match status.plan() {
        Some(p) => p,
        None => {
            let p = ai.plan(prepared.request.clone()).map_err(|f| failed(status, &f))?;
            status.set_plan(Some(Arc::clone(&p)));
            p
        }
    };
    if plan.needs_consent() {
        status.consent.store(true, Ordering::Relaxed);
        status.set_error(Some((NEEDS_CONSENT.into(), Some(AiErrorKind::ConsentRequired))));
        return Err(EngineError::Other(NEEDS_CONSENT.into()));
    }
    let _turn = turns.take(ctx, &status.queued)?;
    let mut capture = diag::Capture {
        prepared: Arc::clone(&prepared),
        area: input.area,
        typed: input.prompt.clone(),
        disclosure: Some(plan.disclosure().clone()),
        more: input.into.is_some(),
        returned: Vec::new(),
        failure: None,
        group: input.into,
    };
    let results = match remote::run_jobs(ctx, plan.as_ref(), input.count) {
        Err(remote::Stopped::Failed(f)) if f.kind == AiErrorKind::ConsentRequired => {
            status.consent.store(true, Ordering::Relaxed);
            status.set_error(Some((NEEDS_CONSENT.into(), Some(AiErrorKind::ConsentRequired))));
            return Err(EngineError::Other(NEEDS_CONSENT.into()));
        }
        Err(remote::Stopped::Failed(f)) => {
            capture.failure = Some(f.message.clone());
            status.set_capture(Some(capture));
            return Err(failed(status, &f));
        }
        r => remote::into_result(r)?,
    };
    ctx.check()?;
    ctx.progress(0.9, "Placing the results");
    // A picture that can't be placed (unreadable, framed differently) is dropped like a failed
    // variation, as long as another one is placed.
    let mut placed = Vec::new();
    for bytes in results {
        let outcome = pixels::place(&bytes, input.area, &prepared.reference).map(|p| {
            let note = VariationNote::of(&p);
            placed.push(p);
            note
        });
        capture.returned.push(diag::Returned { bytes, outcome, layer: None });
    }
    if placed.is_empty() {
        let why = capture.returned.iter().find_map(|r| r.outcome.as_ref().err().cloned()).unwrap_or_else(|| "the AI service returned no picture".into());
        capture.failure = Some(why.clone());
        status.set_capture(Some(capture));
        return Err(failed(status, &AiFailure::new(AiErrorKind::Provider, why)));
    }
    Ok(Made { placed, prepared, capture })
}

/// Keep `f`'s kind for the shell and turn it into the job's error.
fn failed(status: &FillStatus, f: &AiFailure) -> EngineError {
    status.fail(f);
    EngineError::Other(f.message.clone())
}

/// Fill `id`'s results arrive (on its document): a new group masked by the selection, one layer
/// per variation with the first showing, above the layer that was active when it started; or,
/// for Generate More, more variations in the generative layer. One undo step.
fn land(s: &mut Session, id: u64, made: Made) -> Result<Value> {
    let Made { placed, prepared, capture } = made;
    // Discarded meanwhile.
    let i = s.genai_state.fills.iter().position(|f| f.id == id).ok_or(EngineError::Cancelled)?;
    let fill = s.genai_state.fills.remove(i);
    let input = &fill.input;
    let canvas = s.active().map(|st| st.doc.bounds()).ok_or(EngineError::NoDocument)?;
    if canvas != input.canvas {
        return Err(other("the canvas changed size while Generative Fill ran; its result was discarded"));
    }
    if let Some(group) = input.into {
        let ids = s.edit("Generate More", |doc, _| {
            let have = doc.layer(group).and_then(Layer::children).map(<[Layer]>::len).ok_or_else(|| other("the generative layer is gone"))?;
            let new = placed.iter().enumerate().map(|(i, p)| pixels::variation_layer(doc, &p.surface, have + i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
            let ids: Vec<u64> = new.iter().map(|l| l.id.0).collect();
            let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
            // The first new variation shows, as when the layer was made.
            for l in layers.iter_mut() {
                l.visible = false;
            }
            layers.extend(new);
            Ok(ids)
        })?;
        let matching = note_variations(s, &ids, &placed);
        keep_capture(s, capture, group, &ids);
        forget_gone(s);
        return Ok(json!({"layer": group.0, "variations": ids, "matching": matching, "fill": id}));
    }
    let (group, ids) = s.edit(LABEL, |doc, active| {
        // Photoshop's generative layer holds its variations; here that is a group masked by
        // the selection with one layer per variation, the first one showing.
        let children = placed.iter().enumerate().map(|(i, p)| pixels::variation_layer(doc, &p.surface, i + 1, i == 0)).collect::<Result<Vec<_>>>()?;
        let ids: Vec<u64> = children.iter().map(|l| l.id.0).collect();
        let mut group = Layer::group(input.name.clone(), children);
        group.mask = Some(LayerMask { surface: input.sel.clone(), ..LayerMask::reveal_all() });
        let above = fill.anchor.filter(|a| doc.layer(*a).is_some()).or(*active);
        let id = doc.insert_above(above, group);
        crate::layer_multi_cmds::check_group_depth(doc, LABEL)?;
        if input.select {
            *active = Some(id);
        }
        Ok((id, ids))
    })?;
    let area = input.area;
    s.genai_state.sources.insert(group, Arc::new(Source { prompt: input.prompt.clone(), area, prepared }));
    let matching = note_variations(s, &ids, &placed);
    keep_capture(s, capture, group, &ids);
    forget_gone(s);
    Ok(json!({"layer": group.0, "variations": ids, "bounds": [area.x0, area.y0, area.width(), area.height()], "matching": matching, "fill": id}))
}

/// Forget what generative layers were made from, and how variations were placed, once no open
/// document holds their layer: not now, and not in a state undo, redo or a History snapshot can
/// bring back. Run when a fill lands (what holds the memory) and when a document closes, so a
/// session's fills don't keep their requests' pixels after their layers are gone for good.
pub(crate) fn forget_gone(s: &mut Session) {
    let g = &s.genai_state;
    if g.sources.is_empty() && g.notes.is_empty() {
        return;
    }
    let docs: Vec<Arc<Document>> = s
        .documents()
        .iter()
        .flat_map(|st| std::iter::once(&st.doc).chain(st.history.held_docs()).chain(st.snapshots.iter().map(|x| &x.doc)))
        .cloned()
        .collect();
    let held = |id: &LayerId| docs.iter().any(|d| d.layer(*id).is_some());
    s.genai_state.sources.retain(|id, _| held(id));
    s.genai_state.notes.retain(|id, _| held(id));
}

/// Keep what fill landed in `group` as variations `ids` sent and got back, each placed picture
/// tied to its layer, for Save Fill Diagnostics.
fn keep_capture(s: &mut Session, mut capture: diag::Capture, group: LayerId, ids: &[u64]) {
    capture.group = Some(group);
    let mut ids = ids.iter();
    for r in capture.returned.iter_mut().filter(|r| r.outcome.is_ok()) {
        r.layer = ids.next().map(|id| LayerId(*id));
    }
    s.genai_state.captures.push(capture);
}

/// Keep how each new variation (layer `ids`, in `placed`'s order) was placed, for Properties, and
/// return it for the command's result.
fn note_variations(s: &mut Session, ids: &[u64], placed: &[Placed]) -> Vec<VariationNote> {
    let notes: Vec<VariationNote> = placed.iter().map(VariationNote::of).collect();
    for (id, note) in ids.iter().zip(&notes) {
        s.genai_state.notes.insert(LayerId(*id), note.clone());
    }
    notes
}

/// `fill` as a fill id.
fn fill_param(p: &Value, cmd: &str) -> Result<u64> {
    p.get("fill")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "`fill` must be a fill id (see layer.generative.fills)".into() })
}

/// Retry needs a tool that can run.
fn retry_enabled(s: &Session) -> std::result::Result<(), String> {
    ai_ready(s).map(|_| ())
}

/// Send a failed fill again, as it was started, planned afresh (the person may have fixed the
/// setup or chosen another model meanwhile).
fn retry(s: &mut Session, p: &Value) -> Result<Value> {
    let id = fill_param(p, RETRY)?;
    let bad = |msg: String| EngineError::BadParams { cmd: RETRY.into(), msg };
    let fill = s.genai_state.fills.iter().find(|f| f.id == id).ok_or_else(|| bad(format!("no Generative Fill {id} is waiting")))?;
    if fill.status.error().is_none() || fill.job.and_then(|j| s.job(j)).is_some() {
        return Err(bad(format!("Generative Fill {id} hasn't failed")));
    }
    fill.status.set_plan(None);
    start(s, id)
}

/// `fill`, or a list of them as `fills`.
fn fills_param(p: &Value, cmd: &str) -> Result<Vec<u64>> {
    let bad = || EngineError::BadParams { cmd: cmd.into(), msg: "`fill` must be a fill id, or `fills` a list of them (see layer.generative.fills)".into() };
    match (p.get("fill"), p.get("fills")) {
        (Some(v), None) => Ok(vec![v.as_u64().ok_or_else(bad)?]),
        (None, Some(Value::Array(list))) if !list.is_empty() => list.iter().map(|v| v.as_u64().ok_or_else(bad)).collect(),
        _ => Err(bad()),
    }
}

/// Agreeing needs the app's generative AI.
fn agree_enabled(s: &Session) -> std::result::Result<(), String> {
    s.generative_ai().map(|_| ()).ok_or_else(|| NO_AI.into())
}

/// The person agreed to sending a fill that waits for consent, for `term` (`this_time`,
/// `one_hour`, `one_day` or `until_revoked`): it starts, with the plan they saw. (One fill per
/// command, as a command starts at most one job: the shell agrees to each fill it asked about.)
fn agree(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: AGREE.into(), msg };
    let term = p
        .get("term")
        .and_then(Value::as_str)
        .and_then(ConsentTerm::from_id)
        .ok_or_else(|| bad("`term` must be \"this_time\", \"one_hour\", \"one_day\" or \"until_revoked\"".into()))?;
    let id = fill_param(p, AGREE)?;
    let fill = s
        .genai_state
        .fills
        .iter()
        .find(|f| f.id == id && f.status.consent.load(Ordering::Relaxed) && f.job.and_then(|j| s.job(j)).is_none())
        .ok_or_else(|| bad(format!("Generative Fill {id} isn't waiting for your agreement")))?;
    let plan = fill.status.plan().ok_or_else(|| bad(format!("Generative Fill {id} isn't waiting for your agreement")))?;
    plan.agree(term).map_err(|f| EngineError::Other(f.message))?;
    let mut r = start(s, id)?;
    if let Some(o) = r.as_object_mut() {
        o.insert("term".into(), json!(term.id()));
    }
    Ok(r)
}

/// Stop a fill that is generating or waiting its turn, or forget one that failed or waits for
/// consent (nothing of it is sent then).
fn discard(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = fills_param(p, DISCARD)?;
    for id in &ids {
        let i = s
            .genai_state
            .fills
            .iter()
            .position(|f| f.id == *id)
            .ok_or_else(|| EngineError::BadParams { cmd: DISCARD.into(), msg: format!("no Generative Fill {id} is waiting") })?;
        let fill = s.genai_state.fills.remove(i);
        if let Some(job) = fill.job {
            s.cancel_job(job);
        }
    }
    Ok(json!({"discarded": ids}))
}

/// `layer.generative.fills`: the fills that haven't landed, and whether the tool can run.
fn fills(s: &mut Session, _: &Value) -> Result<Value> {
    s.tidy_generative_fills();
    Ok(json!({"fills": s.generative_fills(), "parallel": MAX_PARALLEL, "status": s.generative_status()}))
}

/// The separate areas of the active document's selection, top to bottom.
pub fn selection_areas(s: &Session) -> Vec<Rect> {
    let Some(st) = s.active() else { return Vec::new() };
    let Some(sel) = st.doc.selection.as_ref() else { return Vec::new() };
    areas::Areas::find(sel, st.doc.bounds()).rects().to_vec()
}

/// Listing areas needs a selection.
fn areas_enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    st.doc.selection.as_ref().map(|_| ()).ok_or_else(|| "no selection".into())
}

/// `layer.generative.areas`: the selection's separate areas.
fn areas(s: &mut Session, _: &Value) -> Result<Value> {
    let rects: Vec<[i32; 4]> = selection_areas(s).iter().map(|r| [r.x0, r.y0, r.x1, r.y1]).collect();
    Ok(json!({"areas": rects}))
}

/// The generative layer `p["layer"]` names, or the active one: the group itself or one of its
/// variations.
fn generative_target(s: &Session, p: &Value) -> Option<LayerId> {
    let st = s.active()?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => st.active_layer?,
    };
    let is_generative = |id: LayerId| s.genai_state.sources.contains_key(&id) && st.doc.layer(id).is_some_and(Layer::is_group);
    if is_generative(id) {
        return Some(id);
    }
    let path = st.doc.path_of(id)?;
    let parent = st.doc.layer_at(path.get(..path.len().checked_sub(1)?)?)?.id;
    is_generative(parent).then_some(parent)
}

impl Session {
    /// The active generative layer (a Generative Fill group, or the active variation's group), for
    /// the Properties panel's variations.
    pub fn generative_layer(&self) -> Option<LayerId> {
        generative_target(self, &Value::Null)
    }

    /// The prompt a generative layer was made with.
    pub fn generative_prompt(&self, id: LayerId) -> Option<&str> {
        self.genai_state.sources.get(&id).map(|s| s.prompt.as_str())
    }

    /// How variation layer `id` was placed (lined up and colour-matched), when it was made in this
    /// session.
    pub fn generative_variation_note(&self, id: LayerId) -> Option<&VariationNote> {
        self.genai_state.notes.get(&id)
    }
}

/// The variation commands need an active generative layer.
fn select_enabled(s: &Session) -> std::result::Result<(), String> {
    generative_target(s, &Value::Null).map(|_| ()).ok_or_else(|| "select a generative layer first".into())
}

/// Show variation `index` of the generative layer alone, in one undo step.
fn select_variation(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: SELECT_VARIATION.into(), msg: msg.into() };
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first"))?;
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("`index` must be a variation number from 0"))?;
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let children = s.active().and_then(|st| st.doc.layer(group)).and_then(Layer::children).map_or(0, <[Layer]>::len);
    if index >= children {
        return Err(bad(&format!("this generative layer has {children} variations")));
    }
    s.edit("Generative Variation", |doc, _| {
        let layers = doc.layer_mut(group).and_then(Layer::children_mut).ok_or_else(|| other("the generative layer is gone"))?;
        for (i, l) in layers.iter_mut().enumerate() {
            l.visible = i == index;
        }
        Ok(())
    })?;
    Ok(json!({"layer": group.0, "index": index}))
}

/// Generate More needs an active generative layer and a tool that can run.
fn more_enabled(s: &Session) -> std::result::Result<(), String> {
    select_enabled(s)?;
    ai_ready(s).map(|_| ())
}

/// Generate More: the same request again, as a fill whose variations join the generative layer.
fn more(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: MORE.into(), msg };
    let count = variations_param(p, "count", MORE)?;
    // The layer asked for (`layer`), else the active one: not only the active one.
    let group = generative_target(s, p).ok_or_else(|| bad("select a generative layer first".into()))?;
    ai_ready(s).map_err(bad)?;
    let source = s.genai_state.sources.get(&group).cloned().ok_or_else(|| bad("select a generative layer first".into()))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let canvas = doc.bounds();
    // Its shape on the canvas while it generates: the generative layer's mask.
    let sel = doc.layer(group).and_then(|l| l.mask.as_ref()).map(|m| m.surface.clone()).ok_or_else(|| bad("the generative layer has no mask".into()))?;
    let bounds = sel.content_bounds().intersect(&canvas);
    let input = FillInput {
        name: layer_name(&source.prompt),
        bounds,
        doc: Arc::clone(&doc),
        sel,
        canvas,
        area: source.area,
        size: (0, 0),
        prompt: source.prompt.clone(),
        count,
        into: Some(group),
        prepared: Mutex::new(Some(Arc::clone(&source.prepared))),
        select: false,
    };
    let id = add_fill(s, doc.id, Some(group), input);
    start(s, id)
}

/// Generative Fill's commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILL,
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r#"{"prompt":str?,"variations":1..4=3,"area":uint?} fills the selection with the model chosen for Generative Fill in the Craft AI setup: a new group masked by the selection holds one layer per variation, the first showing. The selected area is blurred out first, so the model fills a hole (an empty prompt continues the surrounding picture through it, removing what was there). `area` fills only that one of the selection's separate areas (0 = the first, see layer.generative.areas). Started in the background it doesn't lock the document: it returns {job, fill} at once, at most 5 fills talk to the service at a time (the rest wait their turn), and the group lands above the layer that was active when it started, without becoming active (layer.generative.fills lists those still running, waiting for consent or failed). Sends the selected area, the picture around it, the selection's shape and the prompt to the chosen model, once per variation; a model off this computer needs the person's agreement first (layer.generative.agree). Not available to automation."#,
            enabled: fill_enabled,
            run: fill,
            journal: true,
        },
        CommandSpec {
            id: SELECT_VARIATION,
            label: "Generative Variation",
            menu: &[],
            shortcut: None,
            params: r#"{"index":uint,"layer":id?} shows variation `index` (0 = the first) of the active generative layer and hides the others."#,
            enabled: select_enabled,
            run: select_variation,
            journal: true,
        },
        CommandSpec {
            id: MORE,
            label: "Generate More",
            menu: &[],
            shortcut: None,
            params: r#"{"count":1..4=3,"layer":id?} asks the chosen model for more variations of the active generative layer, with the same picture, mask and prompt, and adds them to it. Sends that area again. Not available to automation."#,
            enabled: more_enabled,
            run: more,
            journal: true,
        },
        CommandSpec {
            id: RETRY,
            label: "Retry Generative Fill",
            menu: &[],
            shortcut: None,
            params: r#"{"fill":id} sends a Generative Fill that failed again, as it was started (the same area and prompt, from the document as it was then), with the model chosen now. Not available to automation."#,
            enabled: retry_enabled,
            run: retry,
            journal: false,
        },
        CommandSpec {
            id: AGREE,
            label: "Agree to Send",
            menu: &[],
            shortcut: None,
            params: r#"{"fill":id,"term":"this_time"|"one_hour"|"one_day"|"until_revoked"} the person agreed to sending a Generative Fill that waits for consent, to the destination they were shown, for that long (only "this_time" is not remembered); it starts. Not available to automation."#,
            enabled: agree_enabled,
            run: agree,
            journal: false,
        },
        CommandSpec {
            id: DISCARD,
            label: "Discard Generative Fill",
            menu: &[],
            shortcut: None,
            params: r#"{"fill":id|"fills":[id]} stops a Generative Fill that is generating or waiting its turn (its result is dropped), or forgets one that failed or waits for consent (nothing is sent)."#,
            enabled: |_| Ok(()),
            run: discard,
            journal: false,
        },
        CommandSpec {
            id: FILLS,
            label: "Generative Fills",
            menu: &[],
            shortcut: None,
            params: r#"{} lists the Generative Fills that haven't landed and whether the tool can run: {fills:[{id, job, document, anchor, name, prompt, bounds:[x0,y0,x1,y1], state:"queued"|"running"|"consent"|"failed", queue_position, progress, elapsed_ms, error, error_kind, more}], parallel, status:{state:"ready"|"unavailable", …}}."#,
            enabled: |_| Ok(()),
            run: fills,
            journal: false,
        },
        CommandSpec {
            id: SAVE_DIAGNOSTICS,
            label: "Save Fill Diagnostics…",
            menu: &[],
            shortcut: None,
            params: r#"{"folder":text,"layer":id?,"fill":id?} writes what a Generative Fill sent and got back into a new folder inside `folder`: the area as rendered before the selection was cleared, the picture, mask and prompt exactly as sent, each picture as the service returned it, each placed variation composited, and summary.json (model, connection, sizes, how each picture was lined up, whether colour matching ran or why not). For the active generative layer (or `layer`), or a failed fill (`fill`). Kept in memory only for the last 8 fills of the session; nothing is written unless this runs. Desktop only; not available to automation."#,
            enabled: diag::save_enabled,
            run: diag::save,
            journal: false,
        },
        CommandSpec {
            id: AREAS,
            label: "Selection Areas",
            menu: &[],
            shortcut: None,
            params: r#"{} the separate areas of the selection, top to bottom: {areas:[[x0,y0,x1,y1]]}. Generative Fill's `area` fills one of them."#,
            enabled: areas_enabled,
            run: areas,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
