//! Recording into a document's design timeline ([`photocraft_doc::design`]): every outermost
//! command that changes a document with a timeline becomes a step, with the targets it acted on
//! and the layers it created and changed.

use std::sync::Arc;

use photocraft_doc::design::{Checkpoint, DesignStep, Prelude, StepState, layer_changes};
use photocraft_doc::{DocId, Document, LayerId};
use serde_json::Value;

use crate::Session;

/// A document with a timeline as it was before a command.
pub(crate) struct Before {
    doc: DocId,
    /// The document has a timeline. One without is captured only when the command runs in it, to
    /// tell a stroke (a document edit) from a tool setting.
    timeline: bool,
    state: Arc<Document>,
    active: Option<LayerId>,
    selected: Vec<LayerId>,
    history_row: Option<u64>,
    /// The command started in this document (not another one, like Save Contents in the child).
    was_active: bool,
    /// A job's command left this document as it was when it started the job (`job_started`).
    untouched: bool,
    /// The tool settings run since this document's last step: the step's prelude. Taken when the
    /// command starts, so one changed while a job runs goes with the next step instead.
    prelude: Vec<Prelude>,
    /// The foreground and background colours when the command started, as `tools.setColors` takes them.
    colors: Value,
    /// The document's painting symmetry when the command started, as the command that sets it
    /// (None for a symmetry path, which only its own command can set).
    symmetry: Option<Prelude>,
}

/// The prelude command a step's colours are pinned with ([`with_pins`]).
const COLORS: &str = "tools.setColors";

/// The commands that set a document's painting symmetry; the first two pin it ([`with_pins`]).
const SYMMETRY: &[&str] = &["paint.setSymmetry", "paint.symmetryDisable", "paint.symmetryFromPath"];

/// Commands that change no document but feed the steps after them (tool colours and options).
/// Only these are kept as a step's prelude: anything else that changes no document (saving,
/// undo, navigation) must never run again on replay.
const PRELUDE: &[&str] = &["tools.", "tool.", "brush.", "gradient.", "pattern.", "cloneSource.", "paint.", "type.", "shape.", "swatches.", "color."];

/// Commands that don't replay identically (found by the `design_replay_audit` test): their steps
/// keep the layers they produced and replay those instead of running again. Most read session
/// state a replay session doesn't have (the clipboards, the Fade source, the journal), and copying today's
/// clipboard would paste something other than what was pasted when the step was recorded.
pub const BAKED: &[&str] = &[
    "select.reselect",
    "edit.paste",
    "edit.pasteSpecial.pasteInPlace",
    "edit.pasteSpecial.pasteInto",
    "edit.pasteSpecial.pasteOutside",
    "layer.layerStyle.pasteLayerStyle",
    "path.style.pasteFill",
    "path.style.pasteStroke",
    "edit.fade",
    "filter.lastFilter",
    "edit.transform.again",
    "edit.transform.againCopy",
];

/// Whether a step of `id` run with `params` keeps its result instead of running again on replay.
fn bakes(id: &str, params: &Value) -> bool {
    // A Vanishing Point paste without a `layer` pastes the clipboard.
    let vp_clip = || params.get("paste").and_then(Value::as_array).is_some_and(|ps| ps.iter().any(|p| p.get("layer").is_none()));
    // These paint from a History panel state or snapshot, which a replay session doesn't hold.
    let from_history = matches!(id, "paint.historyBrush" | "paint.artHistoryBrush")
        || (id == "edit.fill" && params.get("contents").and_then(Value::as_str) == Some("history"));
    BAKED.contains(&id) || (id == crate::vp_cmds::VP && vp_clip()) || from_history
}

/// The scratch session's gate: a timeline may come from a crafted file, so a replay stays inside
/// the document, nested steps (`actions.play`) and smart-object renders included. Steps recorded
/// with such a command are baked; one that isn't fails instead of running. Not the desktop
/// variant: a mode change resolving the user's own working profile is no path a file can choose.
fn replay_gate(id: &str, params: &Value) -> crate::Result<()> {
    if crate::untrusted::reaches_outside(id, params) {
        return Err(crate::EngineError::Other(format!("`{id}` reaches files outside the document, so the design timeline doesn't replay it")));
    }
    Ok(())
}

/// Whether a baked step's stored result can be out of date after an earlier step changes. Not
/// for one run from another document that only added layers (a layer copied across): its pixels
/// came from there. A paste can, though: where it lands and Paste Into's mask follow the selection.
fn can_go_stale(step: &DesignStep) -> bool {
    step.baked.as_ref().is_some_and(|b| bakes(&step.command, &step.params) || !step.changed.is_empty() || b.selection.is_some())
}

/// Every document with a timeline, and the active document, before a command runs (none when no
/// document has a timeline).
pub(crate) fn capture(s: &Session) -> Vec<Before> {
    let active = s.active().map(|st| st.doc.id);
    if s.docs.iter().all(|st| st.doc.design_timeline.is_none()) {
        return Vec::new();
    }
    s.docs
        .iter()
        .filter(|st| st.doc.design_timeline.is_some() || active == Some(st.doc.id))
        .map(|st| Before {
            timeline: st.doc.design_timeline.is_some(),
            was_active: active == Some(st.doc.id),
            doc: st.doc.id,
            state: st.doc.clone(),
            active: st.active_layer,
            selected: st.selected_layers.clone(),
            history_row: st.history.row_id(st.history.past_len()),
            untouched: false,
            prelude: s.timeline_prelude.get(&st.doc.id).cloned().unwrap_or_default(),
            colors: serde_json::json!({"foreground": s.tools.foreground, "background": s.tools.background}),
            symmetry: match &st.symmetry {
                None => Some(Prelude { command: "paint.symmetryDisable".into(), params: serde_json::json!({}) }),
                Some(p @ crate::symmetry_cmds::PaintingSymmetry::Preset(_)) => Some(Prelude { command: "paint.setSymmetry".into(), params: p.inspect() }),
                Some(crate::symmetry_cmds::PaintingSymmetry::Path(_)) => None,
            },
        })
        .collect()
}

/// Notes which documents of `before` the command hasn't changed by the time it starts a job.
pub(crate) fn job_started(s: &Session, before: &mut [Before]) {
    for b in before {
        b.untouched = s.docs.iter().any(|st| st.doc.id == b.doc && Arc::ptr_eq(&st.doc, &b.state));
    }
}

/// As a job's result applies: its untouched documents as they are now, so an edit made in one
/// while the job ran isn't recorded as the job's.
pub(crate) fn job_applying(s: &Session, before: &mut [Before]) {
    for b in before.iter_mut().filter(|b| b.untouched) {
        if let Some(st) = s.docs.iter().find(|st| st.doc.id == b.doc) {
            b.state = st.doc.clone();
            b.history_row = st.history.row_id(st.history.past_len());
        }
    }
}

/// Record command `id` (run under id-scope `step`) into each document of `before` it changed.
/// `force_bake` keeps the result of an edit that can't run again (a file placed from bytes).
pub(crate) fn record(s: &mut Session, before: Vec<Before>, step: u64, id: &str, params: &Value, force_bake: bool) {
    // Queries and view-only commands (video frame navigation) aren't steps.
    if !tracks(&before) || id.starts_with("design.") || !crate::commands::find(id).is_some_and(|c| c.journal) {
        return;
    }
    let pinned = pin_color_settings(s, id, params);
    let params = pinned.as_ref().unwrap_or(params);
    // Per document: (it took its prelude this many entries long, or it took none).
    let mut took: Vec<(DocId, Option<usize>)> = Vec::new();
    // It changed a document without a timeline (a stroke there).
    let mut edited_untracked = false;
    // The document the command ran in: a step it made in another one names it as the source.
    let source = s.active().map(|st| st.doc.name.clone()).unwrap_or_default();
    for b in before {
        let Some(st) = s.docs.iter_mut().find(|st| st.doc.id == b.doc) else { continue };
        if !b.timeline {
            edited_untracked |= !Arc::ptr_eq(&st.doc, &b.state);
            continue;
        }
        took.push((b.doc, None));
        if Arc::ptr_eq(&st.doc, &b.state) {
            continue;
        }
        // Undo, redo and revert swap in another state with its own timeline: not a new step.
        let same_timeline = matches!((&st.doc.design_timeline, &b.state.design_timeline), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
        if !same_timeline {
            continue;
        }
        let (created, changed) = layer_changes(&b.state, &st.doc);
        // A command run from another document (Save Contents, closing an Edit Contents document)
        // can't run again here: keep what it did. Nor can one reaching outside the document
        // (`replay_gate`).
        // A stroke mirrored across a symmetry path keeps its result: no pin restores the path.
        let path_mirrored = b.symmetry.is_none() && matches!(id, "paint.stroke" | "paint.pencil");
        let baked = (force_bake || bakes(id, params) || crate::untrusted::reaches_outside(id, params) || !b.was_active || path_mirrored)
            .then(|| Arc::new(photocraft_doc::design::Baked::capture(&b.state, &st.doc, &created, &changed)));
        let entry = DesignStep {
            step,
            command: id.to_string(),
            params: resolve_names(&b.state, params),
            label: step_label(st.history.undo_label().unwrap_or(id), id, params, (!b.was_active).then_some(source.as_str())),
            prelude: with_pins(&b, st.doc.design_timeline.as_deref()),
            active: b.active,
            selected: b.selected,
            created,
            changed,
            suppressed: false,
            joins_previous: st.history.row_id(st.history.past_len()) == b.history_row,
            baked,
            rerecord: false,
            state: StepState::Ok,
            // Without its timeline: that would hold every earlier checkpoint out of the budget's reach.
            checkpoint: Checkpoint(Some(Arc::new(Document { design_timeline: None, ..(*st.doc).clone() }))),
        };
        let current = st.doc.clone();
        let doc = Arc::make_mut(&mut st.doc);
        let Some(tl) = doc.design_timeline.as_mut() else { continue };
        let tl = Arc::make_mut(tl);
        let at = tl.in_effect();
        keep_successor_pins(tl, at, &entry);
        tl.steps.insert(at, entry);
        if tl.marker.is_some() {
            tl.marker = Some(at + 1);
            // Steps after an inserted one replay from it when rolled forward.
            for later in tl.steps.iter_mut().skip(at + 1) {
                later.checkpoint = Checkpoint(None);
            }
        }
        budget_checkpoints(&st.history, &current, tl);
        if let Some(t) = took.last_mut() {
            t.1 = Some(b.prelude.len());
        }
    }
    // Each document's prelude runs from its own last step: one recorded elsewhere leaves it be.
    // A tool command that changed a document (a stroke) is that document's step, no setting.
    // Pattern commands manage the shared library, whichever documents' copies they also edit.
    let tool = PRELUDE.iter().any(|p| id.starts_with(p)) && took.iter().all(|t| t.1.is_none()) && (!edited_untracked || id.starts_with("pattern."));
    for (doc, n) in took {
        let pending = s.timeline_prelude.entry(doc).or_default();
        match n {
            Some(n) => drop(pending.drain(..n.min(pending.len()))),
            None if tool => pending.push(Prelude { command: id.to_string(), params: params.clone() }),
            None => {}
        }
    }
    let docs = &s.docs;
    s.timeline_prelude.retain(|d, p| !p.is_empty() && docs.iter().any(|st| st.doc.id == *d));
}

/// `b`'s prelude with the colours and painting symmetry it started with pinned when they aren't
/// those in effect at the marker of `tl`: both also change outside the prelude (targeting a mask
/// resets the colours, a crop moves the symmetry), and the first step must not take today's.
/// Every pin sets the whole value, so the last one in effect says what is, and an explicit setting
/// in the prelude is left out for it. A replay starts without symmetry.
fn with_pins(b: &Before, tl: Option<&photocraft_doc::design::DesignTimeline>) -> Vec<Prelude> {
    let last = |cmds: &[&str]| {
        tl.and_then(|tl| tl.steps.iter().take(tl.in_effect()).rev().flat_map(|st| st.prelude.iter().rev()).find(|p| cmds.contains(&p.command.as_str())))
    };
    // A symmetry path has no pin: its own command stays in the prelude.
    let pinned = |p: &&Prelude| p.command != COLORS && (b.symmetry.is_none() || !SYMMETRY.contains(&p.command.as_str()));
    let mut prelude: Vec<Prelude> = b.prelude.iter().filter(pinned).cloned().collect();
    let colors = Prelude { command: COLORS.to_string(), params: b.colors.clone() };
    if last(&[COLORS]) != Some(&colors) {
        prelude.push(colors);
    }
    if let Some(symmetry) = &b.symmetry {
        let off = Prelude { command: "paint.symmetryDisable".into(), params: serde_json::json!({}) };
        if last(SYMMETRY).unwrap_or(&off) != symmetry {
            prelude.push(symmetry.clone());
        }
    }
    prelude
}

/// Before `entry` goes in at `at` (a rolled-back marker): give the step after it the colour and
/// symmetry pins in effect there, where `entry` pins one and it doesn't, so it keeps running
/// with what it was recorded with rather than `entry`'s.
fn keep_successor_pins(tl: &mut photocraft_doc::design::DesignTimeline, at: usize, entry: &DesignStep) {
    let Some(next) = tl.steps.get(at) else { return };
    let off = Prelude { command: "paint.symmetryDisable".into(), params: serde_json::json!({}) };
    let mut keep = Vec::new();
    for (kind, default) in [(&[COLORS][..], None), (SYMMETRY, Some(off))] {
        let sets = |prelude: &[Prelude]| prelude.iter().any(|p| kind.contains(&p.command.as_str()));
        if sets(&entry.prelude) && !sets(&next.prelude) {
            let before = tl.steps.iter().take(at).rev().flat_map(|st| st.prelude.iter().rev()).find(|p| kind.contains(&p.command.as_str()));
            keep.extend(before.cloned().or(default));
        }
    }
    if let Some(next) = tl.steps.get_mut(at) {
        next.prelude.splice(0..0, keep);
    }
}

/// Whether a capture holds a document with a timeline: the command may be a step.
pub(crate) fn tracks(before: &[Before]) -> bool {
    before.iter().any(|b| b.timeline)
}

/// A mode change's `params` with the Color Settings it ran with filled in: a replay (in a scratch
/// session, or after the file moves to another machine) converts the same way. A working space
/// given by `.icc` path then reaches outside the document, so the step is baked.
fn pin_color_settings(s: &Session, id: &str, params: &Value) -> Option<Value> {
    use photocraft_doc::ColorMode;
    let mode = match id {
        "image.mode.rgb" => ColorMode::Rgb,
        "image.mode.grayscale" => ColorMode::Grayscale,
        "image.mode.cmyk" => ColorMode::Cmyk,
        "image.mode.lab" => ColorMode::Lab,
        // Conditional Mode Change passes these on to the mode change it runs.
        "file.automate.conditionalModeChange" => match params.get("to").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
            Some("rgb") => ColorMode::Rgb,
            Some("gray" | "grayscale") => ColorMode::Grayscale,
            Some("cmyk") => ColorMode::Cmyk,
            Some("lab") => ColorMode::Lab,
            _ => return None,
        },
        _ => return None,
    };
    let mut p = params.as_object().cloned().unwrap_or_default();
    // No profile, or one naming a working space: what the Color Settings resolve it to here.
    let working = match p.get("profile").map(|v| v.as_str()) {
        None | Some(Some("working" | "default")) => Some(mode),
        Some(Some("working-rgb" | "workingRgb")) => Some(ColorMode::Rgb),
        Some(Some("working-cmyk" | "workingCmyk")) => Some(ColorMode::Cmyk),
        Some(Some("working-gray" | "workingGray")) => Some(ColorMode::Grayscale),
        Some(_) => None,
    };
    if let Some(working) = working
        && let Some((spec, _)) = s.color.working_setting(working)
    {
        // A built-in named another way ("Adobe RGB (1998)") is kept by its id, as every version reads it.
        let spec = photocraft_cms::Builtin::from_id(spec).map_or(spec, |b| b.id());
        p.insert("profile".into(), spec.into());
    }
    p.entry("intent").or_insert_with(|| s.color.settings.intent().id().into());
    p.entry("bpc").or_insert_with(|| s.color.settings.bpc.into());
    Some(Value::Object(p))
}

/// A step's label: the history step's, naming where its content came from. A layer copied from
/// another document is "Copy from <document>", any other step made from one "<label> from
/// <document>", and a placed file "<label> <file>".
pub(crate) fn step_label(label: &str, id: &str, params: &Value, from: Option<&str>) -> String {
    match from {
        Some(doc) if id == "layer.copyToDocument" => format!("Copy from {doc}"),
        Some(doc) => format!("{label} from {doc}"),
        None => match params.get("path").and_then(Value::as_str).filter(|_| id.starts_with("file.place")) {
            Some(path) => format!("{label} {}", crate::file_cmds::file_name(path)),
            None => label.to_string(),
        },
    }
}

/// `params` with a `"layer"` given by name replaced by that layer's id, so replay doesn't pick
/// another layer that later took the name.
fn resolve_names(doc: &Document, params: &Value) -> Value {
    let mut p = params.clone();
    if let Some(slot) = p.get_mut("layer")
        && let Some(name) = slot.as_str()
        && let Some(id) = crate::commands::layer_named(doc, name)
    {
        *slot = Value::from(id.0);
    }
    p
}

/// A timeline replayed from some step on.
pub(crate) struct Replayed {
    pub doc: Document,
    pub timeline: photocraft_doc::design::DesignTimeline,
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

/// A session to replay in: `doc` alone, with this session's tool, preset and preference state,
/// so the live session's history, jobs and journal are untouched.
/// What a replay needs from the session, taken on its thread so the replay can run on a worker.
pub(crate) struct Seed {
    tools: crate::ToolState,
    prefs: crate::prefs::PrefsStore,
    patterns: crate::pattern_cmds::PatternLibrary,
    presets: crate::presets::PresetState,
    type_defaults: Option<(photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle)>,
    quick_mask_options: crate::channel_cmds::QuickMaskOptions,
}

impl Seed {
    /// Take what a replay needs from `s`.
    pub(crate) fn of(s: &Session) -> Self {
        Self {
            tools: s.tools.clone(),
            prefs: s.prefs.clone(),
            patterns: s.patterns.clone(),
            presets: s.presets.clone(),
            type_defaults: s.type_defaults.clone(),
            quick_mask_options: s.quick_mask_options,
        }
    }
}

/// A session holding only `doc` (without its timeline) and the settings commands read.
fn scratch(seed: Seed, doc: &Document) -> Session {
    let mut r = Session::new();
    r.tools = seed.tools;
    r.prefs = seed.prefs;
    r.patterns = seed.patterns;
    r.presets = seed.presets;
    r.type_defaults = seed.type_defaults;
    r.quick_mask_options = seed.quick_mask_options;
    r.authorize = Some(replay_gate);
    let mut doc = doc.clone();
    doc.design_timeline = None;
    r.add_document(doc, None);
    r
}

/// The step a replay of `tl` from `from` starts at: the first at or before it with a checkpoint
/// before it.
fn resume_at(tl: &photocraft_doc::design::DesignTimeline, from: usize) -> usize {
    let mut start = from.min(tl.in_effect());
    while start > 0 && tl.steps.get(start - 1).is_none_or(|st| st.checkpoint.0.is_none()) {
        start -= 1;
    }
    start
}

/// How many steps a replay of `tl` from `from` runs.
pub(crate) fn steps_to_run(tl: &photocraft_doc::design::DesignTimeline, from: usize) -> usize {
    tl.in_effect() - resume_at(tl, from)
}

/// The document `tl` produces from the first step at or before `from` with a checkpoint to the
/// marker, with every replayed step's outcome and checkpoint updated.
/// Baked steps after `changed` (the step the change was made at) are marked stale.
/// Reports progress per step on `ctx` and stops when it is cancelled.
pub(crate) fn replay(
    seed: Seed,
    tl: &photocraft_doc::design::DesignTimeline,
    from: usize,
    changed: Option<usize>,
    ctx: &crate::jobs::JobCtx,
) -> crate::Result<Replayed> {
    let mut tl = tl.clone();
    let end = tl.in_effect();
    let start = resume_at(&tl, from);
    let resume = match start {
        0 => tl.base.clone(),
        k => tl
            .steps
            .get(k - 1)
            .and_then(|st| st.checkpoint.0.clone())
            .ok_or_else(|| crate::EngineError::Other("internal error: no checkpoint to replay from".into()))?,
    };
    let mut r = scratch(seed, &resume);
    // The tool settings the steps before `start` set, as a replay from the base has them.
    for p in tl.steps.iter().take(start).flat_map(|st| &st.prelude) {
        let _ = r.execute(&p.command, p.params.clone());
    }
    for (i, step) in tl.steps.iter_mut().enumerate().take(end).skip(start) {
        ctx.check()?;
        ctx.progress((i - start) as f32 / (end - start) as f32, &format!("Step {} of {end}", i + 1));
        let stale = can_go_stale(step) && (step.state == StepState::Stale || changed.is_some_and(|c| i > c));
        run_step(&mut r, step);
        if stale && !step.state.is_failure() {
            step.state = StepState::Stale;
        }
    }
    // Steps past the marker resume from it when rolled forward.
    for later in tl.steps.iter_mut().skip(end) {
        later.checkpoint = Checkpoint(None);
    }
    let st = r.active().ok_or(crate::EngineError::NoDocument)?;
    Ok(Replayed { doc: (*st.doc).clone(), active: st.active_layer, selected: st.selected_layers.clone(), timeline: tl })
}

/// Replay one step in `r`, setting its outcome and checkpoint.
fn run_step(r: &mut Session, step: &mut DesignStep) {
    let Some(pre) = r.active().map(|st| st.doc.clone()) else { return };
    let finish = |r: &mut Session, step: &mut DesignStep, state: StepState| {
        step.state = state;
        step.checkpoint = Checkpoint(r.active().map(|st| st.doc.clone()));
    };
    // Tool settings feed the later steps too, so they run even when the step doesn't.
    for p in &step.prelude {
        // A tool setting that no longer applies leaves the tool as it is.
        let _ = r.execute(&p.command, p.params.clone());
    }
    if step.suppressed {
        return finish(r, step, StepState::Ok);
    }
    let missing: Vec<LayerId> = step.changed.iter().copied().filter(|id| pre.layer(*id).is_none()).collect();
    // A step whose params were edited to name another layer acts on that one, if it exists.
    let named = step.params.get("layer").and_then(|v| v.as_u64().map(LayerId).or_else(|| v.as_str().and_then(|n| crate::commands::layer_named(&pre, n))));
    let replaced = step.rerecord && named.is_some_and(|l| pre.layer(l).is_some());
    if !missing.is_empty() && !replaced {
        return finish(r, step, StepState::MissingReference { layers: missing });
    }
    let placed_elsewhere = step.active.is_some_and(|a| pre.layer(a).is_none()) && !step.created.is_empty();
    if let Some(st) = r.active_mut() {
        st.active_layer = step.active.filter(|a| pre.layer(*a).is_some());
        st.selected_layers = step.selected.iter().copied().filter(|id| pre.layer(*id).is_some()).collect();
        st.layer_anchor = st.active_layer;
    }
    let ran = match step.baked.clone().filter(|_| !step.rerecord) {
        Some(baked) => apply_baked(r, step, &baked),
        None => {
            let _scope = photocraft_doc::IdScope::new(step.step).owned_by(r.scope_owner.0).enter();
            r.execute(&step.command, step.params.clone()).map(|_| ())
        }
    };
    let restore = |r: &mut Session| {
        if let Some(st) = r.active_mut() {
            st.doc = pre.clone();
        }
    };
    if let Err(e) = ran {
        restore(r);
        return finish(r, step, StepState::Error { message: e.to_string() });
    }
    let Some(post) = r.active().map(|st| st.doc.clone()) else { return };
    let (created, changed) = layer_changes(&pre, &post);
    if step.rerecord {
        if step.baked.is_some() {
            step.baked = Some(Arc::new(photocraft_doc::design::Baked::capture(&pre, &post, &created, &changed)));
        }
        (step.created, step.changed, step.rerecord) = (created, changed, false);
        return finish(r, step, StepState::Ok);
    }
    let foreign: Vec<LayerId> = changed.into_iter().filter(|id| !step.changed.contains(id)).collect();
    if !foreign.is_empty() {
        restore(r);
        return finish(r, step, StepState::Retargeted { layers: foreign });
    }
    finish(r, step, if placed_elsewhere { StepState::PlacedElsewhere } else { StepState::Ok });
}

/// Put a baked step's stored result in place of running it.
fn apply_baked(r: &mut Session, step: &DesignStep, baked: &photocraft_doc::design::Baked) -> crate::Result<()> {
    let st = r.active_mut().ok_or(crate::EngineError::NoDocument)?;
    let mut doc = (*st.doc).clone();
    baked.apply(&mut doc, &step.created, &step.changed);
    st.doc = Arc::new(doc);
    Ok(())
}

/// Drop the oldest checkpoints while the pixels only they hold exceed a quarter of the history
/// budget. The base and the newest checkpoint are kept.
pub(crate) fn budget_checkpoints(history: &photocraft_ops::History, current: &Document, tl: &mut photocraft_doc::design::DesignTimeline) {
    // No History byte limit (0: scratch disks hold the pixels), so none for checkpoints either.
    if history.max_bytes == 0 {
        return;
    }
    let cap = history.max_bytes / 4;
    loop {
        let held: Vec<&Document> = tl.steps.iter().filter_map(|st| st.checkpoint.0.as_deref()).collect();
        if held.len() <= 1 || history.extra_bytes(current, &held) <= cap {
            return;
        }
        if let Some(oldest) = tl.steps.iter_mut().find(|st| st.checkpoint.0.is_some()) {
            oldest.checkpoint = Checkpoint(None);
        }
    }
}
