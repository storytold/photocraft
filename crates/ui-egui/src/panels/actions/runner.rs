//! Playing recorded actions in the shell: the [`ShellRunner`] that gives the framework's
//! [`ActionPlayer`] its step semantics (dialogs, Stop messages, shell saves and views), and the
//! paused playbacks that continue after a step dialog, a Stop message or a Step by Step wait.

use craft::automation::actions::{ActionPlayer, ActionRunner, ActionStep, PlaybackBudget, PlaybackRefusal, PlaybackReport, StepOutcome};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedCanvasSize, HostedImageSize, HostedParamDialog};
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};
use web_time::Duration;

use photocraft_engine::actions_cmds::{self, Action};

use super::{dialogs, label_of, playback, record_again};
use crate::PhotocraftApp;

/// Play `targets` one after another, stopping at the first that fails or can't start. One that
/// pauses for a step dialog of its own hands the rest to that dialog, which plays them after its OK
/// (Cancel stops them all, as it stops one action), also from inside an action it calls: that one
/// continues into its callers first. A Stop with its dialog off ends the set. Returns the last result with `ran` counting the steps
/// of all of them plus `before` (steps the same Play ran before a pause), or `None` when there was nothing to play.
/// `from` is the step the first target starts at (the footer's Play on a selected step); the rest start at 0.
pub(crate) fn play_targets(app: &mut PhotocraftApp, targets: &[usize], from: usize, before: u64) -> Result<Option<Value>, String> {
    // A new Play ends a Step by Step or Pause For playback that is waiting for its next step.
    playback::stop(app);
    let (mut last, mut ran) = (None, before);
    for (n, &i) in targets.iter().enumerate() {
        let name = app.session.actions().list.action(i).map(|a| a.name.clone());
        let params = if n == 0 && from > 0 { json!({"action": i, "from": from}) } else { json!({"action": i}) };
        let mut v = app.run("actions.play", params)?;
        ran = ran.saturating_add(v.get("ran").and_then(Value::as_u64).unwrap_or(0));
        let failed = v.get("failed").is_some_and(Value::is_object) || v.get("stopped").is_some_and(Value::is_object);
        let paused = v.get("paused").filter(|p| p.is_object()).and_then(waiting_dialog).cloned();
        if let Some(o) = v.as_object_mut() {
            o.insert("ran".into(), json!(ran));
        }
        last = Some(v);
        if let Some(dialog) = paused {
            // As they are now, not by index: the list can be reordered while the dialog is open
            // (`actions.move`), and one deleted and made again under its name isn't the same action.
            let list = &app.session.actions().list;
            let rest: Vec<Action> = targets.get(n + 1..).unwrap_or_default().iter().filter_map(|i| list.action(*i).cloned()).collect();
            let token = paused_token(app, &dialog);
            if let Some(entry) = token.and_then(|t| app.paused_playbacks.paused.get_mut(&t))
                && name.as_deref() == Some(entry.outermost())
            {
                entry.then = rest;
                entry.ran = ran;
            }
            break;
        }
        if failed {
            break;
        }
    }
    Ok(last)
}

/// The dialog a paused playback waits at: a step dialog by id, Check Spelling, or Step by Step and
/// Pause For's wait before the next step.
fn waiting_dialog(paused: &Value) -> Option<&Value> {
    paused.get("dialog").filter(|d| d.is_u64() || matches!(d.as_str(), Some(crate::type_panels_ui::SPELLING | playback::PLAYBACK_TIMER)))
}

/// The token of the paused playback waiting at `dialog` (see [`waiting_dialog`]).
fn paused_token(app: &PhotocraftApp, dialog: &Value) -> Option<u64> {
    match dialog.as_u64() {
        Some(id) => app.paused_playbacks.paused.iter().find(|(_, p)| p.dialog.is_some_and(|d| d.value() == id)).map(|(token, _)| *token),
        None if dialog.as_str() == Some(playback::PLAYBACK_TIMER) => playback::waiting(app),
        None => app.ui.type_panels.spell.as_ref().and_then(|d| d.resume),
    }
}

/// The token of the paused playback that opened `dialog`: the one lookup from a step dialog to its
/// continuation. `None` for a dialog no paused playback owns (a dialog opened by hand).
pub(crate) fn token_of(app: &PhotocraftApp, dialog: DialogId) -> Option<u64> {
    paused_token(app, &json!(dialog.value()))
}

/// Replay edits synchronously, with view steps handled by the shell in the same order.
/// Engine steps keep the headless action semantics (no dialogs or background jobs).
pub(crate) fn play(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    // A new Play ends a Step by Step or Pause For playback that is waiting for its next step (none
    // waits while a playback runs its steps, so a Play Action step never ends its own).
    playback::stop(app);
    play_owned(app, params, None, &[], &mut PlaybackBudget::new())
}

/// A Conditional step: test its condition as the engine does, then play the branch it picks
/// like a Play Action step (inside the same `budget`), so the branch's steps keep their pacing, step
/// dialogs and Stop messages. The result is the engine command's, with the branch's `paused` or
/// `stopped`. Malformed params go to the engine command, which refuses them with its own error.
fn conditional(app: &mut PhotocraftApp, p: &Value, budget: &mut PlaybackBudget) -> Result<Value, String> {
    let branch = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(n)) => Some(Some(n.clone()).filter(|n| !n.trim().is_empty())),
        Some(_) => None,
    };
    let (Some(condition), Some(then), Some(otherwise)) = (p.get("if").and_then(Value::as_str), branch("then"), branch("else")) else {
        return app.session.execute("actions.conditional", p.clone()).map_err(|e| e.to_string());
    };
    let Ok(holds) = photocraft_engine::actions_edit_cmds::evaluate(&app.session, condition) else {
        return app.session.execute("actions.conditional", p.clone()).map_err(|e| e.to_string());
    };
    let played = if holds { then } else { otherwise };
    let mut out = json!({"if": condition, "holds": holds, "played": played});
    if let Some(name) = played {
        let r = play_owned(app, &json!({"action": name}), None, &[], budget)?;
        for key in ["failed", "paused", "stopped"] {
            if let Some(v) = r.get(key) {
                out[key] = v.clone();
            }
        }
        out["ran"] = r.get("ran").cloned().unwrap_or(json!(0));
        out["action"] = json!(name);
    }
    Ok(out)
}

/// Step `i` of `action` is a Stop: with its dialog toggle on it shows its message and pauses
/// (Continue, when the step allows it, resumes after it); with the toggle off it ends the playback
/// without a word, as in the reference application.
fn stop_step(app: &mut PhotocraftApp, action: &Action, i: usize, p: &Value) -> (StepOutcome, Value) {
    let id = photocraft_engine::atn_cmds::STOP_MESSAGE;
    if !action.dialogs.contains(&i) {
        return (StepOutcome::Stopped, json!({"step": i, "id": id}));
    }
    let token = app.paused_playbacks.pause(action.name.clone(), i + 1, app.session.authorize);
    let allow = p.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    let message = p.get("message").and_then(Value::as_str).unwrap_or_default();
    let dialog = dialogs::open_stop_message(app, message, allow, token);
    if let Some(entry) = app.paused_playbacks.paused.get_mut(&token) {
        entry.allow_continue = allow;
        entry.dialog = Some(dialog);
    }
    (StepOutcome::Paused, json!({"step": i, "id": id, "dialog": dialog.value()}))
}

/// Let the engine handle programmatic playback and stateless items. Interactive Play opens
/// the settings editor and retains its command, document and receiver until confirmation.
fn pause_menu_item(app: &mut PhotocraftApp, action: &str, from: usize, id: &str, params: &Value) -> Result<Option<DialogId>, String> {
    use photocraft_engine::atn_cmds::{MENU_ITEM, menu_item_command, menu_item_needs_settings};

    if id != MENU_ITEM || !playback::is_interactive(app) {
        return Ok(None);
    }
    let command = menu_item_command(params).map_err(|error| error.to_string())?;
    if !menu_item_needs_settings(command) {
        return Ok(None);
    }
    let defaults = json!({});
    authorize_owned(app, command, &defaults, app.session.authorize, app.automation_input)?;
    app.session.precheck(command, &defaults).map_err(|error| error.to_string())?;
    let receiver = app.session.active().and_then(|st| st.active_layer);
    let doc = app.session.active().map(|st| st.doc.id);
    let dialog = open_step_editor(app, command, &defaults)?;
    hold_editor_ok(app, dialog);
    let token = app.paused_playbacks.pause(action.to_owned(), from, app.session.authorize);
    if let Some(p) = app.paused_playbacks.paused.get_mut(&token) {
        p.dialog = Some(dialog);
        p.menu_editor = Some((dialog, command.to_owned()));
        p.filter_receiver = Some(receiver);
        p.doc = doc;
    }
    Ok(Some(dialog))
}

/// Route playback and Record Again to the same settings editor used by the command. A sizing
/// command opens on `values`; the other editors take theirs from [`open_recorded_editor`].
fn open_step_editor(app: &mut PhotocraftApp, command: &str, values: &Value) -> Result<DialogId, String> {
    if crate::sizing::is_sizing(command) {
        crate::sizing::open_with(app, command, values).ok_or_else(|| tl!("No document.").into())
    } else if crate::filter_dialog::has_dialog(command)
        || crate::adjust_dialog::has_dialog(command)
        || (command == crate::equalize_ui::COMMAND && crate::equalize_ui::asks(app))
    {
        crate::dialogs::open_command_dialog(app, command, &label_of(command))
    } else {
        crate::filter_dialog::open(app, command)
    }
}

/// Initialize a recorded editor in its target's space before attaching playback or Record Again state.
pub(crate) fn open_recorded_editor(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<DialogId, String> {
    let adjustment = crate::adjust_dialog::has_dialog(id);
    if adjustment && !params.is_object() && !params.is_null() {
        return Err("adjustment parameters must be an object or null".into());
    }
    let mut values = params.clone();
    if let Some(object) = values.as_object_mut() {
        // Private keys belong to the dialog, not the recorded Action step.
        object.retain(|key, _| !key.starts_with("__"));
    }
    match crate::adjust_dialog::open_with_params_checked(app, id, &values) {
        // Adjustment dialogs normalized their effective target already. Raw parameters here
        // would restore discarded Lab/CMYK records or override the normalized tone controls.
        Ok(dialog) => return Ok(dialog),
        // The editor refuses these parameters (a CMYK Channel Mixer, #8047): say why instead of
        // opening the generic step editor over them.
        Err(Some(reason)) => return Err(reason),
        Err(None) => {}
    }
    let dialog = open_step_editor(app, id, &values)?;
    if !crate::sizing::is_sizing(id) {
        fill_editor(app, dialog, &values);
    }
    Ok(dialog)
}

/// Put a recorded step's values into the open editor `dialog`, field by field, as `ui.dialog.set`
/// does. A field the editor refuses is not one of its values and stays out, as the editor's own
/// fields are the only ones its confirmation reads.
fn fill_editor(app: &mut PhotocraftApp, dialog: DialogId, values: &Value) {
    let Some(object) = values.as_object() else { return };
    let ctx = app.ctx().clone();
    for (key, value) in object {
        let _ = app.framework.dialogs.set_json_field(&ctx, dialog, key, value.clone());
    }
}

/// Open a recorded step's dialog and bind it to the paused playback.
fn pause_for_dialog(app: &mut PhotocraftApp, action: &Action, step: usize, id: &str, params: &Value) -> Result<DialogId, String> {
    let active_layer = app.session.active().and_then(|st| st.active_layer);
    let dialog = open_recorded_editor(app, id, params)?;
    let token = app.paused_playbacks.pause(action.name.clone(), step + 1, app.session.authorize);
    // A dialog on a document target may switch to a layer-owned one, so remember the receiver of
    // every implicit step for run_step_dialog's confirmation check.
    let receiver = params.get("layer").is_none_or(Value::is_null).then_some(active_layer);
    if let Some(paused) = app.paused_playbacks.paused.get_mut(&token) {
        paused.dialog = Some(dialog);
        paused.filter_receiver = receiver;
        paused.explicit_layer = params.get("layer").filter(|layer| !layer.is_null()).cloned();
    }
    Ok(dialog)
}

/// `suspended` is the recording a paused part of the same call took ([`PausedPlayback::record`]); it comes
/// back, with that named call journaled, only when the whole call ends. `inner` are the called
/// actions a resumed playback replays first, outermost first, each `(action, from)`: the first is
/// the one this action's step before `from` called ([`resume`]). `budget` is the one the whole
/// playback tree shares: a called action plays on its caller's.
fn play_owned(
    app: &mut PhotocraftApp,
    params: &Value,
    suspended: Option<Value>,
    inner: &[(String, usize)],
    budget: &mut PlaybackBudget,
) -> Result<Value, String> {
    // An action automation wrote plays under automation's gate even when the user starts it (its
    // key, the panel, Button Mode); nested calls, a paused step's dialog and the steps' own
    // commands see that gate too.
    let kept = actions_cmds::playback_plan(&app.session, params).ok().and_then(|(_, action, _)| actions_cmds::kept_gate(&app.session, &action));
    if kept.is_some() {
        app.session.authorize = kept;
    }
    let played = play_steps(app, params, suspended, inner, budget);
    if kept.is_some() {
        app.session.authorize = None;
        // A pause under the kept gate records it as this action's own, not the set's.
        if let Ok(v) = &played
            && let Some(dialog) = v.get("paused").and_then(waiting_dialog)
            && let Some(token) = paused_token(app, dialog)
            && let Some(entry) = app.paused_playbacks.paused.get_mut(&token)
        {
            entry.kept = true;
        }
    }
    // The call journaled into a recording while automation's gate is installed marks the recorded
    // action, as the engine's `actions.play` does (the kept gate is already removed).
    actions_cmds::note_automation_step(&mut app.session, "actions.play", params);
    played
}

/// How a step that called another action left its caller.
enum Called {
    /// The called action finished: the caller goes on.
    Done,
    /// It failed with this message, and the called action's own result when that carries detail.
    Failed(String, Option<Value>),
    /// It waits at a dialog that now carries this caller as well.
    Paused(Value),
    /// A Stop with its dialog off ended it, and the caller with it.
    Stopped(Value),
}

/// Read the result of step `i` (`id`) of `caller`: a nested failure, a pause (the caller joins the
/// paused playback's [`PausedPlayback::callers`], so it continues after the called action) or a
/// silent stop end the caller there.
fn called(app: &mut PhotocraftApp, caller: &str, i: usize, id: &str, value: &Value, target: Option<photocraft_doc::DocId>) -> Called {
    // A child that waits at a dialog carries its caller, even when its result also reports the
    // failure that opened the Continue / Stop message (#10769): the pause is checked first so the
    // caller goes on after the answer instead of failing here.
    if let Some(inner) = value.get("paused").filter(|p| p.is_object()) {
        if let Some(token) = waiting_dialog(inner).and_then(|dialog| paused_token(app, dialog))
            && let Some(entry) = app.paused_playbacks.paused.get_mut(&token)
        {
            if entry.callers.len() >= MAX_CALLERS {
                return Called::Failed(format!("action nesting exceeds {MAX_CALLERS} levels"), None);
            }
            entry.callers.push((caller.to_string(), i.saturating_add(1)));
        }
        return Called::Paused(inner.clone());
    }
    if let Some(error) = actions_cmds::nested_failure(id, value).or_else(|| actions_cmds::save_target_closed(&app.session, id, target)) {
        // Keep the same child-result detail as headless Actions and Scripts.
        return Called::Failed(error, Some(value.clone()));
    }
    if matches!(id, "actions.play" | "actions.conditional")
        && let Some(stop) = value.get("stopped").filter(|s| s.is_object())
    {
        return Called::Stopped(json!({"step": i, "id": id, "stopped": stop}));
    }
    Called::Done
}

/// The `failed` entry of a playback report: step `step` (`id`) failed with `error`, and `result` is
/// the nested command's own result when it carries detail (a Batch's file outcomes).
fn failure(step: usize, id: &str, error: &str, result: Option<Value>) -> Value {
    let mut failed = json!({"step": step, "id": id, "error": error});
    if let Some(result) = result {
        failed["result"] = result;
    }
    failed
}

/// The message and resume step for a failed step of `action` that names a layer the playing
/// document does not have: the Continue / Stop the reference application shows when an action
/// plays on a document without the recorded names (#10769, docs/atn.md › Playing on another
/// document). It reports a Select of a missing layer as the command being unavailable, and steps
/// that name a layer as the layer object being unavailable. The ask also fires when the layer was
/// deleted from the original document: the test is against the playing document as it stands, not
/// against names it never had. `None` keeps today's plain failure: steps naming no layer (target
/// steps, numeric ids kept for ambiguous names, a rename's new name), names the playing document
/// can resolve for that step's own selector kind, as the engine's lookup for that form does (the
/// step failed for another reason), failures
/// reported for a step other than the one named in them (a player refusal), and step indexes the
/// action does not have.
pub(super) fn missing_layer_prompt(app: &PhotocraftApp, action: &Action, failed: &Value) -> Option<(String, usize)> {
    let index = failed.get("step").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())?;
    let step = action.steps.get(index)?;
    // Only a failure reported for this step itself: the player's refusals come back at the step
    // Play starts at, under the Play label (`actions.play`), and must not be read as an ask.
    if failed.get("id").and_then(Value::as_str) != Some(step.command.as_str()) {
        return None;
    }
    let selector = named_step_layer(&step.command, &step.params)?;
    let name = selector.name();
    let doc = &app.session.active()?.doc;
    // The presence test follows the lookup THIS kind uses in the engine. A `layer` param goes
    // through `explicit_layer`: the exact name, else the same name whatever its case, so a step
    // naming "background" finds a "Background" layer and its failure is unrelated to a missing
    // layer. A Select step's `name` (`selection_target`) and a recorded visibility step's
    // `_actionLayer` (`playback_params`) are matched exactly, so over a document holding "b" the
    // step naming "B" is a layer that document does not have and the ask follows the failure.
    let exact = |want: &str| doc.walk().into_iter().any(|(_, _, layer)| layer.name == want);
    let resolved = exact(name) || selector.folds_case() && doc.walk().into_iter().any(|(_, _, layer)| layer.name.to_lowercase() == name.to_lowercase());
    if resolved {
        return None;
    }
    let message = if step.command == "layer.select" {
        tl!("The command 'Select' is not currently available").to_string()
    } else {
        crate::i18n::fmt(tl!("The object 'Layer \"{name}\"' is not currently available"), &[("name", name)])
    };
    Some((message, index.saturating_add(1)))
}

/// The layer a recorded step names, as the kind of selector it is: a `layer` the recorder made
/// into a name or a source layer's `{"name": …}` object, a Select step's `name`, and a visibility
/// step's recorded target. A numeric `layer` names no layer: the recorder kept that id for an
/// ambiguous name.
fn named_step_layer<'a>(id: &str, params: &'a Value) -> Option<StepLayer<'a>> {
    if let Some(layer) = params.get("layer")
        && let Some(name) = layer.as_str().or_else(|| layer.get("name").and_then(Value::as_str))
    {
        return Some(StepLayer::Param(name));
    }
    if id == "layer.select" {
        return params.get("name").and_then(Value::as_str).map(StepLayer::SelectName);
    }
    params.get("_actionLayer").and_then(|target| target.get("name")).and_then(Value::as_str).map(StepLayer::Recorded)
}

/// The kind of layer selector a recorded step carries. Each form is resolved by its own engine
/// lookup, and only some of those lookups fall back to a case-insensitive name match, so the
/// missing-layer ask tests presence per kind rather than with one rule for every name.
enum StepLayer<'a> {
    /// A `layer` param: a name string or a source layer's `{"name": …}` object, resolved by
    /// `explicit_layer` (the exact name, else the same name whatever its case).
    Param(&'a str),
    /// A Select step's `name`, resolved by `selection_target`: the exact name, at `occurrence`.
    SelectName(&'a str),
    /// A recorded visibility step's `_actionLayer`, resolved by `playback_params`: the exact name,
    /// at the recorded ordinal.
    Recorded(&'a str),
}

impl<'a> StepLayer<'a> {
    /// The name the step gives its layer, whatever its kind.
    fn name(&self) -> &'a str {
        match self {
            Self::Param(name) | Self::SelectName(name) | Self::Recorded(name) => name,
        }
    }

    /// Whether this kind's engine lookup also accepts the name in another case.
    fn folds_case(&self) -> bool {
        matches!(self, Self::Param(_))
    }
}

/// How long Step by Step or Pause For waits after step `i` of `action`, or `None` when it goes on at
/// once: no pacing is set, or no enabled step follows.
fn wait_after(app: &PhotocraftApp, action: &Action, i: usize) -> Option<Duration> {
    playback::step_pause(app).filter(|_| action.enabled_steps().any(|(j, _)| j > i))
}

/// Gives the framework's player the shell's meaning of a step: dialogs and Stop messages that
/// pause the playback, nested actions, shell saves and views, and the engine's commands for the
/// rest. One runner plays one action; a called action gets its own on the same budget.
struct ShellRunner<'a> {
    app: &'a mut PhotocraftApp,
    /// Whether a person is playing: only then do dialog toggles, Stop messages and pacing apply.
    interactive: bool,
    /// The params of the step being run after [`ActionRunner::authorize`] resolved them.
    resolved: Option<Value>,
    /// The `paused` or `stopped` entry of the step that ended the playback without a failure.
    ending: Option<Value>,
    /// The nested command's own result, when the step failed because of it.
    detail: Option<Value>,
    /// The wait after the step that returned [`StepOutcome::Wait`].
    wait: Option<Duration>,
}

impl ShellRunner<'_> {
    /// End the playback at a dialog; `entry` is its `paused` value.
    fn pause(&mut self, entry: Value) -> StepOutcome {
        self.ending = Some(entry);
        StepOutcome::Paused
    }

    /// The outcome of a nested call's result, as [`called`] reads it.
    fn after_call(&mut self, action: &Action, i: usize, id: &str, result: Result<Value, String>, target: Option<photocraft_doc::DocId>) -> StepOutcome {
        let value = match result {
            Ok(value) => value,
            Err(error) => return StepOutcome::Failed(error),
        };
        match called(self.app, &action.name, i, id, &value, target) {
            // Step by Step and Pause For: the canvas shows this step before the next one runs.
            Called::Done => match wait_after(self.app, action, i) {
                Some(wait) => {
                    self.wait = Some(wait);
                    StepOutcome::Wait(value)
                }
                None => StepOutcome::Done(value),
            },
            Called::Failed(error, result) => {
                self.detail = result;
                StepOutcome::Failed(error)
            }
            Called::Paused(entry) => self.pause(entry),
            Called::Stopped(entry) => {
                self.ending = Some(entry);
                StepOutcome::Stopped
            }
        }
    }
}

impl ActionRunner for ShellRunner<'_> {
    /// The engine's counter is the only one: scripts, batches and droplets charge it too.
    fn charge(&mut self, _budget: &mut PlaybackBudget) -> bool {
        actions_cmds::consume_playback_step(&mut self.app.session)
    }

    /// The gate judges the step before its dialog opens: a denied step neither runs nor pauses.
    fn authorize(&mut self, step: &ActionStep) -> Result<(), String> {
        let resolved = actions_cmds::playback_params(&self.app.session, &step.command, &step.params).map_err(|e| e.to_string())?;
        if let Some(gate) = self.app.session.authorize {
            gate(&step.command, &resolved).map_err(|e| e.to_string())?;
        }
        self.resolved = Some(resolved);
        Ok(())
    }

    fn is_noop(&self, step: &ActionStep) -> bool {
        actions_cmds::noop_step(&self.app.session, &step.command, &step.params)
    }

    fn run_step(&mut self, action: &Action, i: usize, step: &ActionStep, show_dialog: bool, budget: &mut PlaybackBudget) -> StepOutcome {
        let id = step.command.as_str();
        let p = self.resolved.take().unwrap_or_else(|| step.params.clone());
        let interactive = self.interactive;
        // A Stop shows its message or ends the playback (`stop_step`). Agents keep the headless
        // semantics of the engine command.
        if id == photocraft_engine::atn_cmds::STOP_MESSAGE && interactive {
            let (outcome, entry) = stop_step(self.app, action, i, &p);
            self.ending = Some(entry);
            return outcome;
        }
        match pause_menu_item(self.app, &action.name, i + 1, id, &p) {
            Ok(Some(dialog)) => return self.pause(json!({"step": i, "id": id, "dialog": dialog.value()})),
            Err(error) => return StepOutcome::Failed(error),
            Ok(None) => {}
        }
        // The dialog toggle: show the step's dialog with its recorded values and continue from the
        // next step when it is confirmed (`resume`). Agents play headlessly.
        if show_dialog
            && interactive
            && (crate::filter_dialog::has_dialog(id)
                || crate::adjust_dialog::has_dialog(id)
                || (id == crate::equalize_ui::COMMAND && crate::equalize_ui::asks(self.app)))
        {
            return match pause_for_dialog(self.app, action, i, id, &p) {
                Ok(dialog) => self.pause(json!({"step": i, "id": id, "dialog": dialog.value()})),
                Err(error) => StepOutcome::Failed(error),
            };
        }
        // Check Spelling's dialog opens with Check All Layers as recorded; Done continues the action.
        // While the dialog is open (for another action or by hand), this step fails and the open
        // one keeps its place and ignored words.
        // A correction (`action` set) opens no dialog and runs below.
        if show_dialog && interactive && id == "edit.checkSpelling" && p.get("action").is_none() {
            if let Some(open) = self.app.ui.type_panels.spell.as_ref() {
                let error = if open.resume.is_some() { "Check Spelling is already open for another action" } else { "Check Spelling is already open" };
                return StepOutcome::Failed(error.into());
            }
            if let Err(error) = crate::type_panels_ui::open_spelling(self.app, p.get("allLayers").and_then(Value::as_bool).unwrap_or(true)) {
                return StepOutcome::Failed(error);
            }
            let token = self.app.paused_playbacks.pause(action.name.clone(), i + 1, self.app.session.authorize);
            let doc = self.app.session.active().map(|d| d.doc.id);
            if let Some(entry) = self.app.paused_playbacks.paused.get_mut(&token) {
                entry.doc = doc;
            }
            if let Some(d) = self.app.ui.type_panels.spell.as_mut() {
                d.resume = Some(token);
            }
            return self.pause(json!({"step": i, "id": id, "dialog": crate::type_panels_ui::SPELLING}));
        }
        let app = &mut *self.app;
        let target = app.session.active().map(|d| d.doc.id);
        let result = match id {
            "actions.play" => play_owned(app, &p, None, &[], budget),
            "actions.conditional" => conditional(app, &p, budget),
            _ if actions_cmds::saves_smart_contents(&app.session, id) => app.session.execute(actions_cmds::SAVE_CONTENTS, json!({})).map_err(|e| e.to_string()),
            _ if actions_cmds::shell_save_command(id) => {
                // Write synchronously, so the next step doesn't meet a document locked by a
                // background save.
                let background = std::mem::replace(&mut app.background_jobs, false);
                let r = app.replay_save(id, &p);
                app.background_jobs = background;
                r
            }
            _ if actions_cmds::shell_view_command(id) => {
                app.sync_views();
                app.run(id, p.clone())
            }
            // An explicit vector paste needs the original OS source, not just an old pixel copy.
            "edit.paste" if p.get("as").is_some() => app.run(id, p.clone()),
            _ => app.session.execute(id, p.clone()).map_err(|e| e.to_string()),
        };
        // A called action that paused for a dialog continues here after it once confirmed.
        self.after_call(action, i, id, result, target)
    }
}

/// Why the player refused a playback the engine's plan let through.
fn refusal_text(refusal: &PlaybackRefusal) -> String {
    match refusal {
        PlaybackRefusal::Recursive(name) => format!("recursive action call to `{name}`"),
        PlaybackRefusal::TooDeep => "action nesting exceeds 16 levels".into(),
        _ => "action playback was refused".into(),
    }
}

/// Play an action's steps in the shell (dialogs, shell saves and views), under whatever gate
/// [`play_owned`] installed; pauses at a step whose dialog is toggled on. `inner` is as in
/// [`play_owned`].
fn play_steps(
    app: &mut PhotocraftApp,
    params: &Value,
    suspended: Option<Value>,
    inner: &[(String, usize)],
    budget: &mut PlaybackBudget,
) -> Result<Value, String> {
    let planned = actions_cmds::playback_plan(&app.session, params).map_err(|e| e.to_string());
    let (index, action, from) = match planned {
        Ok(plan) => plan,
        Err(e) => {
            actions_cmds::finish_pending_history(&mut app.session);
            restore_recording(app, suspended.as_ref());
            return Err(e);
        }
    };
    let recording = match actions_cmds::begin_playback_from(&mut app.session, &action, from) {
        Ok(r) => r,
        Err(e) => {
            actions_cmds::finish_pending_history(&mut app.session);
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let owner = recording
        .map(|(_, idx)| {
            let into = app.session.actions().list.action(idx).map(|a| a.name.clone());
            json!({"index": idx, "into": into, "action": action.name, "from": from})
        })
        .or(suspended);
    let mut ran = 0u64;
    let interactive = playback::is_interactive(app);
    let (mut failed, mut paused, mut stopped) = (None, None, None);
    let mut ended = false;
    // A resumed playback goes on in the action this one called first (the call was step `from - 1`)
    // inside this one's playback frame, so its recursion checks still see this caller.
    if let Some(((name, child_from), rest)) = inner.split_first() {
        let step = from.saturating_sub(1);
        let target = app.session.active().map(|d| d.doc.id);
        let outcome = match play_owned(app, &json!({"action": name, "from": child_from}), None, rest, budget) {
            Ok(value) => called(app, &action.name, step, "actions.play", &value, target),
            Err(error) => Called::Failed(error, None),
        };
        match outcome {
            Called::Done => {
                ran = 1;
                // A paced Play waits after the call as after any other step.
                if let Some(wait) = wait_after(app, &action, step) {
                    paused = Some(playback::pace(app, &action.name, step, "actions.play", ran, wait));
                    ended = true;
                }
            }
            Called::Failed(error, result) => {
                failed = Some(failure(step, "actions.play", &error, result));
                ended = true;
            }
            Called::Paused(entry) => {
                paused = Some(entry);
                ended = true;
            }
            Called::Stopped(entry) => {
                stopped = Some(entry);
                ended = true;
            }
        }
    }
    if !ended {
        // The runner borrows the app for the whole play, so the player reads its own copy of the playable list.
        let list = app.session.actions().playable().clone();
        let mut runner = ShellRunner { app: &mut *app, interactive, resolved: None, ending: None, detail: None, wait: None };
        let report: PlaybackReport = ActionPlayer::play(&list, index, from, &mut runner, budget);
        let ShellRunner { ending, detail, wait, .. } = runner;
        ran = ran.saturating_add(report.ran);
        if let Some(f) = report.failed {
            failed = Some(failure(f.step, &f.command, &f.message, detail));
        } else if let Some(refusal) = &report.refused {
            // A refusal is the whole call's, not one step's: it is reported at the step Play
            // starts at, under the Play label. `playback_plan` pre-refuses a recursion or a 17th
            // level at the calling step (which fails as the call's own step, with that label), so
            // the player's refusal is what a resumed or rebased playback reports.
            failed = Some(failure(from, "actions.play", &refusal_text(refusal), None));
        } else if report.waiting {
            if let (Some(at), Some(wait)) = (report.paused_at, wait) {
                let id = action.steps.get(at).map(|s| s.command.clone()).unwrap_or_default();
                paused = Some(playback::pace(app, &action.name, at, &id, ran, wait));
            }
        } else if report.paused_at.is_some() {
            paused = ending;
        } else if report.stopped_at.is_some() {
            stopped = ending;
        }
    }
    // "Playing on another document": when a step that names a layer fails because the playing
    // document does not have it, the playback stops there and asks a person Continue (skip the
    // step, play on from the next) or Stop (#10769). The edits so far stay applied. Agents and
    // paced failures that name no missing layer keep today's plain failure report.
    let mut prompted = false;
    if let Some(f) = failed.as_ref().filter(|_| interactive && paused.is_none() && stopped.is_none())
        && let Some((message, from)) = missing_layer_prompt(app, &action, f)
    {
        let token = app.paused_playbacks.pause(action.name.clone(), from, app.session.authorize);
        let dialog = dialogs::open_stop_message(app, &message, true, token);
        if let Some(entry) = app.paused_playbacks.paused.get_mut(&token) {
            entry.allow_continue = true;
            entry.dialog = Some(dialog);
            // Continue skips the failed step: the pause's own step never ran and is not counted.
            entry.timed = true;
        }
        paused = Some(json!({"step": f.get("step"), "id": f.get("id"), "dialog": dialog.value()}));
        prompted = true;
    }
    let history = if (failed.is_none() || prompted) && paused.is_some() { actions_cmds::suspend_history(&mut app.session) } else { None };
    actions_cmds::finish_playback(&mut app.session, None, &action, from);
    match paused.as_ref().and_then(waiting_dialog).cloned() {
        // Paused for a dialog: the call goes on after it, so recording stays suspended.
        Some(dialog) if failed.is_none() || prompted => {
            let token = paused_token(app, &dialog);
            let entry = token.and_then(|t| app.paused_playbacks.paused.get_mut(&t));
            match entry {
                Some(p) => {
                    p.record = owner;
                    p.history = history;
                    // Innermost first: each frame of the paused call adds the action it played.
                    p.played.push(action.clone());
                }
                // No continuation to carry the recording: the call ends here.
                _ => {
                    drop(history);
                    restore_recording(app, owner.as_ref());
                }
            }
        }
        _ => {
            drop(history);
            restore_recording(app, owner.as_ref());
        }
    }
    Ok(match (failed, paused, stopped) {
        // The report of a Continue / Stop ask keeps both: the failed step and the message the
        // playback waits at, so `play_targets` carries the rest of a set like a dialog pause does.
        (Some(f), Some(p), _) if prompted => json!({"action": action.name, "ran": ran, "failed": f, "paused": p}),
        (Some(f), _, _) => json!({"action": action.name, "ran": ran, "failed": f}),
        (None, Some(p), _) => json!({"action": action.name, "ran": ran, "paused": p}),
        (None, None, Some(s)) => json!({"action": action.name, "ran": ran, "stopped": s}),
        (None, None, None) => json!({"action": action.name, "ran": ran}),
    })
}

/// Runs view command `id` (a view row of `shortcut_dispatch::RUNTIME_COMMANDS`: Zoom In, Fit on Screen, 100%, …) for
/// app code (menus, action steps, automation): asks the active document's canvas for the framework's command, which its
/// next pass runs as it runs the command's chord, and journals it now, so an action step plays and records in order.
/// The canvas runs it on the document as the steps before it left it.
pub(crate) fn run_view(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    let command = crate::shortcut_dispatch::view_command(id).ok_or_else(|| format!("unsupported view command: {id}"))?;
    app.sync_views();
    let i = app.session.active_index().ok_or("no document")?;
    app.ui.views.get_mut(i).ok_or("no document view")?.request(command);
    journal_view(app, id, params);
    Ok(Value::Null)
}

/// Journals the view command `id` that ran, which records it into the action being recorded.
pub(crate) fn journal_view(app: &mut PhotocraftApp, id: &str, params: Value) {
    actions_cmds::note_automation_step(&mut app.session, id, &params);
    app.session.journal.push((id.into(), params));
}

/// Show the outcome of a playback report (`{action, ran, failed | paused | stopped}`) in the status bar.
pub(crate) fn report_play(app: &mut PhotocraftApp, v: &Value) {
    if v.get("stopped").is_some_and(Value::is_object) {
        app.ui.status = tl!("Action playback stopped").to_string();
        app.ui.status_error = false;
        return;
    }
    // A Continue / Stop ask carries `paused` for its message but reports the failed step below:
    // the failure line, not "waiting for its dialog", stands behind the message.
    if let Some(paused) = v.get("paused").filter(|p| p.is_object() && v.get("failed").is_none()) {
        if paused.get("dialog").and_then(Value::as_str) == Some(playback::PLAYBACK_TIMER) {
            // Step by Step or Pause For between two steps: the last report comes at the end.
            return;
        }
        let step = paused.get("step").and_then(Value::as_u64).unwrap_or(0);
        let template = "Step {step} is waiting for its dialog";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string())]);
        app.ui.status_error = false;
        return;
    }
    let Some(failed) = v.get("failed").filter(|f| f.is_object()) else {
        let ran = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
        app.ui.status = crate::i18n::trn(crate::i18n::current(), ran, "Played {n} step", "Played {n} steps");
        app.ui.status_error = false;
        return;
    };
    let step = failed.get("step").and_then(Value::as_u64).unwrap_or(0);
    let id = failed.get("id").and_then(Value::as_str).unwrap_or("");
    let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
    let template = "Step {step} ({id}) failed: {error}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string()), ("id", id), ("error", error)]);
    app.ui.status_error = true;
}

/// Resume the recording a paused call suspended and journal that call, once.
fn restore_recording(app: &mut PhotocraftApp, owner: Option<&Value>) {
    let Some(owner) = owner else { return };
    let (Some(idx), Some(into), Some(name)) = (
        owner.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()),
        owner.get("into").and_then(Value::as_str),
        owner.get("action").and_then(Value::as_str),
    ) else {
        return;
    };
    let from = owner.get("from").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    // `suspended_guard` refuses what would move the recorded action, but another command may
    // still replace the list: the call goes to the action of that name, never to whatever now
    // sits at the old index, and is dropped if that action is gone.
    let list = app.session.actions().list.actions();
    let target = if list.get(idx).is_some_and(|a| a.name == into) { Some(idx) } else { list.iter().position(|a| a.name == into) };
    if let Some(idx) = target
        && app.session.recording.target.is_none()
    {
        actions_cmds::restore_recording(&mut app.session, idx, name, from);
    }
}

/// Whether a paused playback holds a suspended recording.
fn suspended(app: &PhotocraftApp) -> bool {
    app.paused_playbacks.paused.values().any(|p| p.record.is_some())
}

/// While a step dialog holds a suspended recording, refuse what would lose its call: a new
/// recording (the restore would find recording taken), deleting/moving actions or changing the
/// whole library (the recorded action could go away or change place). The engine checks these
/// against `recording`, which is `None` then.
pub(crate) fn suspended_guard(app: &PhotocraftApp, id: &str) -> Result<(), String> {
    if matches!(
        id,
        "actions.record"
            | "actions.delete"
            | "actions.move"
            | "actions.duplicate"
            | "actions.recordAgain"
            | "actions.allowToolRecording"
            | "actions.clearAll"
            | "actions.reset"
            | "actions.replace"
            | "actions.undoLibrary"
    ) && suspended(app)
    {
        return Err(tl!("Confirm or cancel the open action step dialog first").into());
    }
    Ok(())
}

/// A step dialog's command failed: the action ends there.
pub fn abandoned(app: &mut PhotocraftApp, token: u64) {
    let paused = app.paused_playbacks.take(token);
    let record = paused.and_then(|p| {
        drop(p.history);
        p.record
    });
    actions_cmds::finish_pending_history(&mut app.session);
    restore_recording(app, record.as_ref());
}

/// The automation gate a session installs ([`photocraft_engine::Session::authorize`]).
pub(crate) type Gate = fn(&str, &Value) -> photocraft_engine::Result<()>;

/// Deferred Actions dialogs retain their caller's gate and also honor current permissions.
pub(crate) fn authorize_owned(app: &PhotocraftApp, id: &str, params: &Value, gate: Option<Gate>, automated: bool) -> Result<(), String> {
    if let Some(gate) = gate {
        gate(id, params).map_err(|e| e.to_string())?;
    }
    if let Some(current) = app.session.authorize {
        current(id, params).map_err(|e| e.to_string())?;
    }
    if (automated || app.automation_input || app.session.authorize.is_some())
        && let Some(check) = app.services.automation_command.as_ref()
    {
        check(id, params)?;
    }
    Ok(())
}

/// A function-pointer dispatch gate cannot carry two different policies into nested jobs.
/// Changed gates and distinct simultaneous Session/service gates both end this continuation;
/// hosts that need both policies must supply one composed gate at both boundaries.
fn continuation_gate(captured: Option<Gate>, live: Option<Gate>) -> Result<Option<Gate>, String> {
    if captured.zip(live).is_some_and(|(before, now)| !std::ptr::fn_addr_eq(before, now)) {
        return Err("the authorization context changed or uses policies that cannot be combined for this paused action".into());
    }
    Ok(live.or(captured))
}

/// A playback paused at a step dialog: the action and step it continues from, and the gate it
/// was playing under (`None` for one a person started).
#[derive(Clone, Debug)]
pub(crate) struct PausedPlayback {
    pub(super) action: String,
    pub(super) from: usize,
    pub(super) gate: Option<Gate>,
    /// The dialog the playback waits at (a step editor or a Stop message), found by
    /// [`token_of`]. `None` for Check Spelling, which holds its token itself, and for a playback
    /// parked by Step by Step or Pause For.
    pub(super) dialog: Option<DialogId>,
    /// The recording the paused call suspended: `{index, into, action, from}` (`into` is the
    /// name of the action being recorded). A step dialog only closes through the dialog host's
    /// confirm (OK, `↩`, automation) or cancel (Cancel, Esc, the close box, automation), which
    /// end the call through `resume`, `abandoned` or `stopped`, so the recording comes back
    /// exactly once.
    pub(super) record: Option<Value>,
    pub(super) history: Option<actions_cmds::ActionHistoryContinuation>,
    /// The actions still to play after this one, as they were when Play started a whole set.
    /// Actions have no id, so the whole action stands in for one: an action deleted and made again
    /// under the same name, or edited, while the dialog is open no longer matches.
    pub(super) then: Vec<Action>,
    /// The steps that Play ran before this pause (earlier actions of the set, and this one's
    /// steps before the dialog), so the final count covers the whole Play.
    pub(super) ran: u64,
    /// The document a pause at Check Spelling was playing on. That dialog doesn't block the
    /// document tabs, so Done ([`resume_spelling`]) continues only on this document.
    pub(super) doc: Option<photocraft_doc::DocId>,
    /// An implicit filter/adjustment step resolves this active layer at the pause. The outer Option marks
    /// that binding even when there was no active layer; explicit-layer and other dialogs omit it.
    pub(super) filter_receiver: Option<Option<photocraft_doc::LayerId>>,
    /// The layer the recorded step names. The settings editor has no row for it, so the confirmed command takes it
    /// from here: the step keeps its receiver, whatever layer is active at OK.
    pub(super) explicit_layer: Option<Value>,
    /// Descriptor-free menu playback owns this settings dialog and the concrete command it opened.
    pub(super) menu_editor: Option<(DialogId, String)>,
    /// The actions that called this one through Play Action steps, nearest first, each with the
    /// step it continues from: confirming the dialog finishes `action`, then each of them in turn
    /// (at most [`MAX_CALLERS`]).
    pub(super) callers: Vec<(String, usize)>,
    /// A Stop step's Allow Continue, kept here rather than in the dialog, which automation may
    /// change: without it, confirming the message only ends the playback.
    pub(super) allow_continue: bool,
    /// Parked by Step by Step or Pause For after a step that ran (counted in `ran`), not at a dialog
    /// whose own step runs on confirm.
    pub(super) timed: bool,
    /// The paused action and each of its callers as they were when it paused, innermost first.
    /// The continuation goes on by name, so an action edited, deleted or replaced by a new one of
    /// the same name meanwhile ends it there instead of playing the new action's tail ([`resume`]).
    pub(super) played: Vec<Action>,
    /// `gate` is the kept gate of an action automation wrote that the user played
    /// ([`actions_cmds::kept_gate`]), not the gate of the whole call: it still judges the dialog's
    /// own command ([`run_step_dialog`]), but [`resume`] lets each such action take it again as it
    /// plays, so its callers and the rest of the set keep the gate they had.
    pub(super) kept: bool,
}

/// The longest caller chain a paused playback keeps; [`actions_cmds::playback_plan`] refuses deeper
/// nesting anyway.
pub(super) const MAX_CALLERS: usize = 16;

impl PausedPlayback {
    /// The action the person (or agent) started: the last caller, else the paused action.
    fn outermost(&self) -> &str {
        self.callers.last().map_or(self.action.as_str(), |(name, _)| name.as_str())
    }
}

/// The paused playbacks, by token. A dialog finds its token through [`token_of`]; what plays
/// next, and under which gate, is kept here.
#[derive(Debug, Default)]
pub(crate) struct PausedPlaybacks {
    last: u64,
    pub(super) paused: std::collections::HashMap<u64, PausedPlayback>,
    /// The Step by Step or Pause For playback waiting for its next step ([`playback::tick`]).
    pub(super) timer: Option<playback::Timer>,
}

impl PausedPlaybacks {
    /// Allocate an opaque token retaining the action, resume position and captured authorization gate.
    pub(super) fn pause(&mut self, action: String, from: usize, gate: Option<Gate>) -> u64 {
        self.last = self.last.wrapping_add(1);
        self.paused.insert(
            self.last,
            PausedPlayback {
                action,
                from,
                gate,
                record: None,
                history: None,
                then: Vec::new(),
                ran: 0,
                doc: None,
                dialog: None,
                filter_receiver: None,
                explicit_layer: None,
                menu_editor: None,
                callers: Vec::new(),
                allow_continue: false,
                timed: false,
                played: Vec::new(),
                kept: false,
            },
        );
        self.last
    }

    /// Consume the continuation `token` names; an expired token identifies no playback.
    fn take(&mut self, token: u64) -> Option<PausedPlayback> {
        self.paused.remove(&token)
    }
}

/// Whether a dialog command's `target` belongs to the document rather than to a layer: alpha
/// channels and Quick Mask do; masks and colour components still belong to a layer.
fn document_target(params: &Value) -> bool {
    params.get("target").is_some_and(|target| target.as_str() == Some("quickMask") || target.get("channel").and_then(Value::as_u64).is_some())
}

/// Before a preview or confirmation evaluates settings, check the actual editor and its permissions.
pub(crate) fn authorize_panel_preview(app: &PhotocraftApp, dialog: DialogId, command: &str, params: &Value) -> Result<(), String> {
    record_again::authorize_preview(app, dialog, command, params)?;
    if let Some(paused) = app.paused_playbacks.paused.values().find(|p| p.menu_editor.as_ref().is_some_and(|(id, _)| *id == dialog)) {
        check_menu_editor(app, paused, command, params)?;
        authorize_owned(app, command, params, paused.gate, paused.gate.is_some())?;
    }
    Ok(())
}

/// Makes the open editor `dialog` hold its OK, so [`check_editor`] judges the answer before the editor closes: a
/// parameter form or a sizing dialog files it for the host; an adjustment editor holds its own.
pub(crate) fn hold_editor_ok(app: &mut PhotocraftApp, dialog: DialogId) {
    let dialogs = &mut app.framework.dialogs;
    if let Some(form) = dialogs.get_mut_by_id::<HostedParamDialog>(dialog) {
        form.hold_ok();
    } else if let Some(size) = dialogs.get_mut_by_id::<HostedImageSize>(dialog) {
        size.hold_ok();
    } else if let Some(size) = dialogs.get_mut_by_id::<HostedCanvasSize>(dialog) {
        size.hold_ok();
    }
}

/// Whether `dialog` is an editor whose OK a step judges: the one Record Again opened or the one a paused menu step opened.
pub(crate) fn is_editor(app: &PhotocraftApp, dialog: DialogId) -> bool {
    record_again::owns_dialog(app, dialog) || app.paused_playbacks.paused.values().any(|p| p.menu_editor.as_ref().is_some_and(|(id, _)| *id == dialog))
}

/// The held OK of the editor `dialog` with the answer `command` and `params`: what the editor's Record Again or paused
/// menu step requires before it takes the answer. A refusal keeps the editor open.
pub(crate) fn check_editor(app: &PhotocraftApp, dialog: DialogId, command: &str, params: &Value) -> Result<(), String> {
    if record_again::owns_dialog(app, dialog) {
        record_again::check(app, dialog, command, params)
    } else {
        authorize_panel_preview(app, dialog, command, params)
    }
}

/// Check that preview and confirmation still target the command and receiver that opened the editor.
fn check_menu_editor(app: &PhotocraftApp, paused: &PausedPlayback, command: &str, params: &Value) -> Result<(), String> {
    let actions = app.session.actions().list.actions();
    if let Some((_, original)) = &paused.menu_editor
        && (original != command
            || paused.played.iter().any(|played| actions.iter().find(|action| action.name == played.name) != Some(played))
            || app.session.active().map(|st| st.doc.id) != paused.doc
            || (!document_target(params) && paused.filter_receiver != Some(app.session.active().and_then(|st| st.active_layer))))
    {
        return Err("the menu item command or its document changed".into());
    }
    Ok(())
}

/// Admit the final dialog command before resuming history or starting a worker. The captured
/// policy judges the edited parameters; a current live gate still governs dispatch and nested
/// work. With no live gate, reinstall the captured gate for job admission and rendering.
pub(crate) fn run_step_dialog(app: &mut PhotocraftApp, dialog: DialogId, command: &str, params: Value) -> Result<Value, String> {
    match token_of(app, dialog) {
        Some(token) => run_paused_command(app, token, command, params),
        None => app.run(command, params),
    }
}

/// [`run_step_dialog`] for the playback `token` names, which Check Spelling holds itself.
fn run_paused_command(app: &mut PhotocraftApp, token: u64, command: &str, params: Value) -> Result<Value, String> {
    let paused = app.paused_playbacks.paused.get(&token).ok_or("this dialog doesn't belong to a paused action")?;
    let mut params = params;
    if let (Some(layer), Some(object)) = (&paused.explicit_layer, params.as_object_mut()) {
        object.entry("layer").or_insert_with(|| layer.clone());
    }
    check_menu_editor(app, paused, command, &params)?;
    if let Some(receiver) = paused.filter_receiver
        && !document_target(&params)
        && app.session.active().and_then(|st| st.active_layer) != receiver
    {
        return Err("the paused command's receiver changed while its dialog was open".into());
    }
    let captured = paused.gate;
    let saved = app.session.authorize;
    let service = if app.automation_input { app.services.automation_authorize } else { None };
    for authorize in [captured, saved, service].into_iter().flatten() {
        authorize(command, &params).map_err(|e| e.to_string())?;
    }
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(command, &params)?;
    }
    let gate = continuation_gate(captured, continuation_gate(saved, service)?)?;
    prepare_history(app, token)?;
    app.session.authorize = gate;
    let result = app.run(command, params);
    app.session.authorize = saved;
    result
}

/// Bind the dialog's own edit to its suspended action before it runs or starts a worker.
pub(crate) fn prepare_history(app: &mut PhotocraftApp, token: u64) -> Result<(), String> {
    let history = app.paused_playbacks.paused.get_mut(&token).ok_or("this dialog doesn't belong to a paused action")?.history.take();
    if let Some(history) = history
        && let Err(e) = history.resume(&mut app.session)
    {
        if let Some(paused) = app.paused_playbacks.paused.get_mut(&token) {
            paused.history = Some(history);
        }
        return Err(e.to_string());
    }
    Ok(())
}

/// Check Spelling owns each correction only while it runs; the open window owns a bound token.
pub(crate) fn spelling_change(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    let Some(token) = app.ui.type_panels.spell.as_ref().and_then(|dialog| dialog.resume) else {
        return app.run("edit.checkSpelling", params);
    };
    let paused = app.paused_playbacks.paused.get(&token).ok_or("this dialog doesn't belong to a paused action")?;
    if paused.doc.is_some_and(|doc| Some(doc) != app.session.active().map(|state| state.doc.id)) {
        return Err("the active document changed while Check Spelling was open".into());
    }
    let history = paused.history.clone();
    let result = run_paused_command(app, token, "edit.checkSpelling", params);
    // A rejected admission did not resume this continuation. Keep the original token intact.
    if app.paused_playbacks.paused.get(&token).is_some_and(|paused| paused.history.is_some()) {
        return result;
    }
    if let Some(history) = history {
        match history.suspend_after_edit(&mut app.session) {
            Ok(history) => {
                let paused = app.paused_playbacks.paused.get_mut(&token).ok_or("the spelling continuation ended during its correction")?;
                paused.history = Some(history);
            }
            Err(error) => {
                if let Some(paused) = app.paused_playbacks.paused.get_mut(&token) {
                    paused.history = Some(history);
                }
                return Err(error.to_string());
            }
        }
    }
    result
}

/// Check Spelling's Done for the action paused at its step: the action continues when the document
/// it was playing on is still the active one, and otherwise ends there with the reason in the
/// status bar. Returns `Ok` when nothing failed.
pub(crate) fn resume_spelling(app: &mut PhotocraftApp, token: u64) -> Result<(), String> {
    let Some((from, doc)) = app.paused_playbacks.paused.get(&token).map(|p| (p.from, p.doc)) else {
        return resume(app, token);
    };
    if doc.is_none() || doc == app.session.active().map(|d| d.doc.id) {
        return resume(app, token);
    }
    abandoned(app, token);
    let error = "the active document changed while Check Spelling was open";
    report_play(app, &json!({"failed": {"step": from.saturating_sub(1), "id": "edit.checkSpelling", "error": error}}));
    Err(error.into())
}

/// A step dialog confirmed (or a paced wait over): continue the playback `token` names after it.
///
/// The remaining steps run under the gate the playback paused with, however the dialog was
/// confirmed: a control request, synthetic Enter or OK, or a click.
pub fn resume(app: &mut PhotocraftApp, token: u64) -> Result<(), String> {
    let Some(paused) = app.paused_playbacks.take(token) else {
        return Err("this dialog doesn't belong to a paused action".into());
    };
    let owner = paused.record.clone();
    if let Some(history) = &paused.history
        && let Err(e) = history.resume(&mut app.session)
    {
        restore_recording(app, owner.as_ref());
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
        app.ui.status_error = true;
        return Err(e.to_string());
    }
    // The dialog's command may run as a background job; the next step needs its result.
    if let Some(doc) = app.session.active().map(|d| d.doc.id)
        && let Some(job) = app.session.job_on(doc)
        && let Err(e) = app.session.wait_job(job.id)
    {
        actions_cmds::finish_pending_history(&mut app.session);
        restore_recording(app, owner.as_ref());
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
        app.ui.status_error = true;
        return Err(e.to_string());
    }
    let saved = app.session.authorize;
    let service = if app.automation_input { app.services.automation_authorize } else { None };
    let remaining = !paused.then.is_empty()
        || !paused.callers.is_empty()
        || app
            .session
            .actions()
            .list
            .actions()
            .iter()
            .find(|action| action.name == paused.action)
            .is_some_and(|action| action.enabled_steps().any(|(i, _)| i >= paused.from));
    let gate =
        if remaining { continuation_gate(saved, service).and_then(|live| continuation_gate(paused.gate, live)) } else { Ok(saved.or(service).or(paused.gate)) };
    let gate = match gate {
        Ok(gate) => gate,
        Err(error) => {
            // The admitted dialog edit has completed; preserve it while rejecting the tail.
            actions_cmds::finish_pending_history(&mut app.session);
            restore_recording(app, owner.as_ref());
            app.ui.status = error.clone();
            app.ui.status_error = true;
            return Err(error);
        }
    };
    // A kept gate belongs to the action automation wrote: it takes the gate again as it plays
    // ([`play_owned`]), while its callers and the rest of the set keep the gate they had.
    let gate = if paused.kept { service.or(saved) } else { gate };
    // The rest plays by name: refuse it when an action of the paused call changed meanwhile, so a
    // new action of the same name never plays from the old one's step. With nothing left to play,
    // the call simply ends. (Another document coming to the front fails `history.resume` above.)
    let list = app.session.actions().list.actions();
    if remaining && paused.played.iter().any(|a| list.iter().find(|b| b.name == a.name) != Some(a)) {
        let error = "the action changed while it was paused";
        actions_cmds::finish_pending_history(&mut app.session);
        restore_recording(app, owner.as_ref());
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", error)]);
        app.ui.status_error = true;
        return Err(error.into());
    }
    // The paused action goes on, then each action that called it, inside its callers' frames:
    // replay starts at the outermost, which replays the inner ones first.
    let frames: Vec<(String, usize)> = paused.callers.iter().rev().cloned().chain(std::iter::once((paused.action.clone(), paused.from))).collect();
    let Some(((outer, outer_from), inner)) = frames.split_first() else {
        return Err("this dialog doesn't belong to a paused action".into());
    };
    let params = json!({"action": outer, "from": outer_from});
    // A dialog's own step runs on its confirm; a paced wait's step already ran and was counted.
    let own = u64::from(!paused.timed);
    app.session.authorize = gate;
    let mut played = play_owned(app, &params, owner, inner, &mut PlaybackBudget::new());
    // Paused again at a later dialog of the same call: that dialog carries the rest of the set.
    if let Ok(v) = &played
        && let Some(dialog) = v.get("paused").and_then(waiting_dialog)
        && let Some(token) = paused_token(app, dialog)
        && let Some(entry) = app.paused_playbacks.paused.get_mut(&token)
        && entry.outermost() == paused.outermost()
    {
        entry.then = paused.then.clone();
        entry.ran = paused.ran.saturating_add(own).saturating_add(v.get("ran").and_then(Value::as_u64).unwrap_or(0));
    }
    // The count covers the steps this Play ran before the pause, and the step the dialog ran.
    let ran = paused.ran.saturating_add(own).saturating_add(played.as_ref().ok().and_then(|v| v.get("ran")).and_then(Value::as_u64).unwrap_or(0));
    if let Ok(Some(o)) = played.as_mut().map(Value::as_object_mut)
        && o.contains_key("ran")
    {
        o.insert("ran".into(), json!(ran));
    }
    // The rest of a set Play started: under the same gate, and only if this action finished. An
    // action of it deleted, renamed, edited or replaced by a new one of the same name meanwhile
    // ends the set there.
    if let Ok(v) = &played
        && !paused.then.is_empty()
        && v.get("failed").is_none()
        && v.get("paused").is_none()
        && v.get("stopped").is_none()
    {
        let list = app.session.actions().list.actions();
        let rest: Vec<usize> = paused.then.iter().map_while(|then| list.iter().position(|a| a == then)).collect();
        match play_targets(app, &rest, 0, ran) {
            Ok(Some(r)) => played = Ok(r),
            Ok(None) => {}
            Err(e) => played = Err(e),
        }
    }
    app.session.authorize = saved;
    match played {
        Ok(v) => {
            report_play(app, &v);
            Ok(())
        }
        Err(e) => {
            let template = "Action playback failed: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
            app.ui.status_error = true;
            Err(e)
        }
    }
}

/// A step dialog cancelled: the action stops there, as in Photoshop.
pub fn stopped(app: &mut PhotocraftApp, token: u64) {
    abandoned(app, token);
    app.ui.status = tl!("Action playback stopped").to_string();
    app.ui.status_error = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::actions::stop_tests::{answer, assert_idle, count, stop_app, stop_prompt};

    /// A paused call that already has the longest caller chain refuses one more caller, and a
    /// chain whose caller is gone ends the resumed playback with an error.
    #[test]
    fn a_caller_chain_is_bounded() {
        let mut app = stop_app(true, true);
        let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
        let dialog = stop_prompt(&app);
        let token = token_of(&app, dialog).unwrap();
        app.paused_playbacks.paused.get_mut(&token).unwrap().callers = vec![("Missing".into(), 1); MAX_CALLERS];
        let r = called(&mut app, "One more", 0, "actions.play", &json!({"paused": v["paused"].clone()}), None);
        assert!(matches!(r, Called::Failed(..)), "a caller past the bound fails the call");
        // A caller that no longer exists ends the resumed playback with an error, nothing else runs.
        app.paused_playbacks.paused.get_mut(&token).unwrap().callers = vec![("Missing".into(), 1)];
        answer(&mut app, dialog, true).unwrap();
        assert!(app.ui.status_error, "{}", app.ui.status);
        assert_eq!(count(&app, "Suffix"), 0);
        assert_idle(&app);
    }
}
