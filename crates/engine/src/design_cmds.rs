//! Design timeline commands: turn a document's timeline on or off and list its steps. Each
//! change is an ordinary undoable history step.

use std::sync::Arc;

use photocraft_doc::Document;
use photocraft_doc::design::DesignTimeline;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
pub use crate::timeline::BAKED;
use crate::{EngineError, Result, Session};

/// The active document's timeline, or why there is none.
fn timeline_of(s: &Session) -> std::result::Result<&Arc<DesignTimeline>, String> {
    s.active().ok_or("no document open")?.doc.design_timeline.as_ref().ok_or_else(|| "the design timeline is off".into())
}

/// Start's precondition: a document whose timeline is off, and no job running: one started
/// before the timeline (a print with a Script Event) could still edit it unrecorded.
fn can_start(s: &Session) -> std::result::Result<(), String> {
    match s.active().ok_or("no document open")?.doc.design_timeline {
        Some(_) => Err("the design timeline is already on".into()),
        None if s.has_jobs() => Err("wait for the running jobs to finish before starting the design timeline".into()),
        None => Ok(()),
    }
}

/// `design.start`: record from the current document on, as one undoable edit.
fn start(s: &mut Session, _: &Value) -> Result<Value> {
    can_start(s).map_err(EngineError::Other)?;
    forget_prelude(s);
    s.edit("Start Design Timeline", |doc, _| {
        doc.design_timeline = Some(Arc::new(DesignTimeline::new(Arc::new(doc.clone()))));
        Ok(())
    })?;
    Ok(json!({"started": true}))
}

/// `design.stop`: drop the timeline and its steps, as one undoable edit.
fn stop(s: &mut Session, _: &Value) -> Result<Value> {
    timeline_of(s).map_err(EngineError::Other)?;
    s.edit("Stop Design Timeline", |doc, _| {
        doc.design_timeline = None;
        Ok(())
    })?;
    Ok(json!({"stopped": true}))
}

/// Drop the active document's pending tool settings as a timeline starts: an earlier timeline's
/// (stopped, or its start undone), which the new one must not take. Its colours and symmetry are
/// pinned.
fn forget_prelude(s: &mut Session) {
    if let Some(id) = s.active().map(|st| st.doc.id) {
        s.timeline_prelude.remove(&id);
    }
}

/// `design.list`: every step with its outcome, the marker and how many steps are in effect.
fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let tl = timeline_of(s).map_err(EngineError::Other)?;
    let steps: Vec<Value> = tl
        .steps
        .iter()
        .map(|st| {
            json!({
                "step": st.step, "command": st.command, "label": st.label, "params": st.params,
                "suppressed": st.suppressed, "joinsPrevious": st.joins_previous, "baked": st.baked.is_some(),
                "created": st.created, "changed": st.changed, "state": st.state,
            })
        })
        .collect();
    Ok(json!({"steps": steps, "marker": tl.marker, "inEffect": tl.in_effect()}))
}

/// A parameter error for `cmd`.
fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The index of the step named by `"step"` (its id, as `design.list` gives it).
fn step_index(cmd: &str, tl: &DesignTimeline, p: &Value) -> Result<usize> {
    let id = p.get("step").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`step` must be a step id from design.list"))?;
    tl.index_of(id).ok_or_else(|| bad(cmd, format!("no step {id} in the design timeline")))
}

/// `a` and `b` are the same document, whatever their timelines hold.
fn same_content(a: &Document, b: &Document) -> bool {
    // The name is file identity (a save renames the document), never a recorded step.
    let strip = |d: &Document| Document { design_timeline: None, name: String::new(), ..d.clone() };
    strip(a) == strip(b)
}

/// Change the active document's timeline with `f` (which returns the first step it affects),
/// replay from there and commit the result as one undo step named `label`.
/// `f` returns the first step to replay from and whether that step itself changed (so baked steps
/// after it may be out of date).
fn change(s: &mut Session, cmd: &str, label: &str, f: impl FnOnce(&mut DesignTimeline) -> Result<(usize, bool)>) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let tl = st.doc.design_timeline.clone().ok_or_else(|| EngineError::Other("the design timeline is off".into()))?;
    // Replay rebuilds the document from recorded steps: a change made around them would be lost.
    let last = match tl.in_effect() {
        0 => Some(tl.base.clone()),
        n => tl.steps.get(n - 1).and_then(|st| st.checkpoint.0.clone()),
    };
    if last.is_some_and(|last| !same_content(&last, &st.doc)) {
        return Err(EngineError::Other(format!(
            "`{cmd}`: the document has changes the design timeline didn't record; stop and restart the timeline to keep them"
        )));
    }
    let mut next = (*tl).clone();
    let (from, changed) = f(&mut next)?;
    let steps = crate::timeline::steps_to_run(&next, from);
    let seed = crate::timeline::Seed::of(s);
    let work = move |ctx: &crate::jobs::JobCtx| crate::timeline::replay(seed, &next, from, changed.then_some(from), ctx);
    let label_owned = label.to_string();
    let apply = move |s: &mut Session, replayed| commit(s, &label_owned, replayed);
    // Replaying steps can take seconds: a background start runs it as a job (progress, Cancel).
    // Restoring a checkpoint runs none, so it lands at once, before a dialog opens on it.
    if steps == 0 {
        return apply(s, work(&crate::jobs::JobCtx::new())?);
    }
    crate::jobs::run(s, label, true, work, apply)
}

/// Put a replay's document and timeline in place as one undoable edit, keeping the document's identity.
fn commit(s: &mut Session, label: &str, mut replayed: crate::timeline::Replayed) -> Result<Value> {
    if let Some(st) = s.active() {
        crate::timeline::budget_checkpoints(&st.history, &st.doc, &mut replayed.timeline);
    }
    let crate::timeline::Replayed { doc: result, timeline, active, selected } = replayed;
    s.edit(label, |doc, act| {
        let (id, name) = (doc.id, std::mem::take(&mut doc.name));
        *doc = result;
        doc.id = id;
        // The replayed checkpoint may predate a save that renamed the document.
        doc.name = name;
        doc.design_timeline = Some(Arc::new(timeline));
        *act = active;
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.selected_layers = selected;
        st.layer_anchor = st.active_layer;
    }
    list(s, &Value::Null)
}

/// `design.suppress` / `design.unsuppress`: skip or restore a step, then replay from it.
fn suppress(s: &mut Session, p: &Value, on: bool) -> Result<Value> {
    let (cmd, label) = if on { ("design.suppress", "Suppress Step") } else { ("design.unsuppress", "Unsuppress Step") };
    change(s, cmd, label, |tl| {
        let i = step_index(cmd, tl, p)?;
        if let Some(st) = tl.steps.get_mut(i) {
            st.suppressed = on;
        }
        Ok((i, true))
    })
}

/// `design.delete`: remove a step, then replay from where it was.
fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    // The last step's tool settings go with the next step recorded instead.
    let tail = timeline_of(s).ok().and_then(|tl| {
        let i = step_index("design.delete", tl, p).ok()?;
        (i + 1 == tl.steps.len()).then(|| tl.steps.get(i).map(|st| st.prelude.clone())).flatten()
    });
    let doc = s.active().map(|st| st.doc.id);
    let r = change(s, "design.delete", "Delete Step", |tl| {
        let i = step_index("design.delete", tl, p)?;
        let removed = tl.steps.remove(i);
        // Its tool settings and pattern versions still feed the steps after it.
        if let Some(next) = tl.steps.get_mut(i) {
            next.prelude.splice(0..0, removed.prelude);
            let own: Vec<String> = next.patterns.iter().map(|p| p.id.clone()).collect();
            next.patterns.splice(0..0, removed.patterns.into_iter().filter(|p| !own.contains(&p.id)));
        }
        if let Some(m) = tl.marker.as_mut()
            && i < *m
        {
            *m -= 1;
        }
        Ok((i, true))
    })?;
    if let (Some(prelude), Some(doc)) = (tail, doc) {
        s.timeline_prelude.entry(doc).or_default().splice(0..0, prelude);
    }
    Ok(r)
}

/// `design.rollTo`: move the marker (`at` steps in effect, null = all) and rebuild the document there.
fn roll_to(s: &mut Session, p: &Value) -> Result<Value> {
    change(s, "design.rollTo", "Move Timeline Marker", |tl| {
        let was = tl.in_effect();
        let at = match p.get("at") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n <= tl.steps.len())
                    .ok_or_else(|| bad("design.rollTo", format!("`at` must be a step count from 0 to {} (null = the end)", tl.steps.len())))?,
            ),
        };
        tl.marker = at.filter(|n| *n < tl.steps.len());
        Ok((was.min(tl.in_effect()), false))
    })
}

/// `design.editStep`: give a step new parameters and replay from it.
fn edit_step(s: &mut Session, p: &Value) -> Result<Value> {
    change(s, "design.editStep", "Edit Step", |tl| {
        let i = step_index("design.editStep", tl, p)?;
        let params = p.get("params").filter(|v| v.is_object()).ok_or_else(|| bad("design.editStep", "`params` must be an object"))?;
        if let Some(st) = tl.steps.get_mut(i) {
            st.params = params.clone();
            st.rerecord = true;
        }
        // Edit Parameters puts the marker back in the same replay rather than a second one.
        if let Some(at) = p.get("restore") {
            tl.marker = match at {
                Value::Null => None,
                v => Some(
                    v.as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|n| *n <= tl.steps.len())
                        .ok_or_else(|| bad("design.editStep", format!("`restore` must be a step count from 0 to {} (null = the end)", tl.steps.len())))?,
                ),
            }
            .filter(|n| *n < tl.steps.len());
        }
        Ok((i, true))
    })
}

/// `design.replay`: rebuild the document from the base through every step in effect.
fn replay_all(s: &mut Session, _: &Value) -> Result<Value> {
    change(s, "design.replay", "Replay Timeline", |_| Ok((0, false)))
}

/// The step commands' precondition: the timeline is on.
fn has_timeline(s: &Session) -> std::result::Result<(), String> {
    timeline_of(s).map(|_| ())
}

macro_rules! step_cmd {
    ($id:expr, $label:expr, $params:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: has_timeline, run: $run, journal: true }
    };
}

/// The design timeline commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "design.start",
            label: "Start Design Timeline",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: can_start,
            run: start,
            journal: true,
        },
        CommandSpec {
            id: "design.stop",
            label: "Stop Design Timeline",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_timeline,
            run: stop,
            journal: true,
        },
        CommandSpec {
            id: "design.list",
            label: "List Design Timeline Steps",
            menu: &[],
            shortcut: None,
            params: "{} → {steps:[{step,command,label,params,suppressed,joinsPrevious,baked,created,changed,state}],marker,inEffect}",
            enabled: has_timeline,
            run: list,
            journal: false,
        },
        step_cmd!("design.suppress", "Suppress Step", "{step} → design.list (replays the steps after it)", |s, p| suppress(s, p, true)),
        step_cmd!("design.unsuppress", "Unsuppress Step", "{step} → design.list", |s, p| suppress(s, p, false)),
        step_cmd!("design.delete", "Delete Step", "{step} → design.list", delete),
        step_cmd!("design.rollTo", "Move Timeline Marker", "{at: steps in effect, null = the end} → design.list (new edits go in at the marker)", roll_to),
        step_cmd!(
            "design.editStep",
            "Edit Step",
            "{step, params, restore?} → design.list (replays the step with new params; `restore` then moves the marker, null = the end)",
            edit_step
        ),
        step_cmd!("design.replay", "Replay Timeline", "{} → design.list (re-runs every step)", replay_all),
    ]
}

#[cfg(test)]
#[path = "design_cmds_tests.rs"]
mod tests;
