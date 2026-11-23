//! Playback Options: the Actions panel menu's dialog, and the Step by Step and Pause For pacing of a
//! person's Play.
//!
//! A paced Play is still one playback: after each step the runner parks the rest of the action like a
//! step dialog does (a [`super::PausedPlayback`] token holding the suspended history and recording), and
//! [`tick`] resumes it once the wait is over, so the steps share one history state and the canvas
//! redraws between them. Esc, the panel's Stop button, an edit of the action list or a new recording end
//! it there, keeping the steps already applied. Programmatic playback never waits.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use web_time::{Duration, Instant};

use photocraft_engine::prefs::{Actions, PlaybackPerformance};

use crate::PhotocraftApp;
use crate::dialogs::params::{self, ParamFamily};

/// The marker a paced playback leaves where a step dialog's id would be (see [`super::waiting_dialog`]).
pub(crate) const PLAYBACK_TIMER: &str = "__playbackTimer";

/// The key of the Playback Options dialog.
const KEY: &str = "actionsPanel.playbackOptions";

/// The Playback Options dialog's result handler. OK is refused for seconds the field cannot show.
pub(crate) const FAMILY: ParamFamily = ParamFamily { check: Some(check), ..ParamFamily::new(|key| key == KEY, run) };

/// Step by Step's gap between two steps: the reference application takes about 1.15 s per step
/// with nothing to redraw (measured 2026-11-05), and adds Pause For's seconds to it.
pub(crate) const STEP_GAP: Duration = Duration::from_secs(1);

/// The paced playback waiting for its next step: its [`super::PausedPlaybacks`] token, when it goes
/// on, and the action list's revision it was parked at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timer {
    pub(crate) token: u64,
    pub(crate) due: Instant,
    rev: u64,
}

/// Native Play and control-channel menu clicks may open editors and wait between steps.
pub(super) fn is_interactive(app: &PhotocraftApp) -> bool {
    !app.automation_input && !app.programmatic_playback
}

/// How long a person's Play waits after a step, or `None` when it goes straight on (Accelerated,
/// and always for agents).
pub(super) fn step_pause(app: &PhotocraftApp) -> Option<Duration> {
    if !is_interactive(app) {
        return None;
    }
    let a = &app.session.prefs().actions;
    match a.performance {
        PlaybackPerformance::Accelerated => None,
        PlaybackPerformance::StepByStep => Some(STEP_GAP),
        PlaybackPerformance::Pause => Some(STEP_GAP.saturating_add(Duration::from_secs(u64::from(a.pause_seconds())))),
    }
}

/// Park `action` after its step `i` (`ran` steps of it so far): it continues from the next step
/// after `wait`. Returns the `paused` value the runner reports.
pub(super) fn pace(app: &mut PhotocraftApp, action: &str, i: usize, id: &str, ran: u64, wait: Duration) -> Value {
    let rev = app.session.actions.list.revision();
    let gate = app.session.authorize;
    let pb = &mut app.paused_playbacks;
    let token = pb.pause(action.to_string(), i.saturating_add(1), gate);
    if let Some(entry) = pb.paused.get_mut(&token) {
        entry.timed = true;
        entry.ran = ran;
    }
    let due = Instant::now().checked_add(wait).unwrap_or_else(Instant::now);
    // One paced playback at a time: Play ends a waiting one before it starts (`stop`).
    if let Some(old) = pb.timer.replace(Timer { token, due, rev }) {
        pb.paused.remove(&old.token);
    }
    json!({"step": i, "id": id, "dialog": PLAYBACK_TIMER})
}

/// The token of the paced playback that is waiting, if any.
pub(super) fn waiting(app: &PhotocraftApp) -> Option<u64> {
    app.paused_playbacks.timer.map(|t| t.token)
}

/// Whether a paced playback is waiting for its next step.
pub(crate) fn pending(app: &PhotocraftApp) -> bool {
    app.paused_playbacks.timer.is_some()
}

/// End a waiting paced playback where it is: the steps it applied stay, in one history state when
/// the option is on. Returns whether one was waiting.
pub(crate) fn stop(app: &mut PhotocraftApp) -> bool {
    let Some(timer) = app.paused_playbacks.timer.take() else { return false };
    super::runner::stopped(app, timer.token);
    true
}

/// Once per frame: play the next step when its wait is over, or end the playback on Esc, an edit of
/// the action list or a new recording. A frame's Esc belongs to its first pass; a second pass of the frame finds
/// the wait already ended (or no Esc) and applies only its own input.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(timer) = app.paused_playbacks.timer else { return };
    let esc = !app.dialog_open() && crate::window_keys::take_escape(ctx);
    let changed = timer.rev != app.session.actions.list.revision() || app.session.recording.target.is_some();
    if esc || changed || !app.paused_playbacks.paused.contains_key(&timer.token) {
        stop(app);
        return;
    }
    let now = Instant::now();
    // A dialog opened meanwhile (Playback Options itself, say) holds the next step until it closes.
    if now < timer.due || app.dialog_open() {
        ctx.request_repaint_after(timer.due.saturating_duration_since(now).max(Duration::from_millis(100)));
        return;
    }
    app.paused_playbacks.timer = None;
    // `resume` reports the outcome in the status bar.
    let _ = super::runner::resume(app, timer.token);
    ctx.request_repaint();
}

/// Pause For's seconds as the engine takes them: a whole number within [`Actions::PAUSE_SECONDS`].
fn whole_seconds(seconds: f64) -> Option<u8> {
    let range = Actions::PAUSE_SECONDS;
    let whole = seconds.is_finite() && seconds.fract() == 0.0 && seconds >= f64::from(*range.start()) && seconds <= f64::from(*range.end());
    whole.then_some(seconds as u8)
}

/// The dialog's rows over the saved options: Performance (Accelerated, Step by Step, Pause For: n seconds, whose
/// seconds field is on only with Pause For) and History (Create a single history state).
fn rows(performance: &str, seconds: f64, single: bool) -> Vec<Param> {
    let modes = [
        (PlaybackPerformance::Accelerated, Label::from(tl!("Accelerated"))),
        (PlaybackPerformance::StepByStep, Label::from(tl!("Step by Step"))),
        (PlaybackPerformance::Pause, Label::from(tl!("Pause For:"))),
    ];
    let options: Vec<(String, Label)> = modes.into_iter().map(|(mode, label)| (mode.name().to_owned(), label)).collect();
    let range = Actions::PAUSE_SECONDS;
    let (least, most) = (f64::from(*range.start()), f64::from(*range.end()));
    vec![
        Param::new("performance", tl!("Performance"), ParamKind::radio(options, Some(performance.to_owned()))),
        Param::new("seconds", tl!("seconds"), ParamKind::number(Some(least), Some(most), seconds, 1.0, 0))
            .enabled_when("performance", PlaybackPerformance::Pause.name()),
        Param::new("singleState", tl!("Create a single history state"), ParamKind::bool(single)),
    ]
}

/// Playback Options…: the saved options, Pause For's seconds kept while another mode is chosen.
pub(super) fn open_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let saved = &app.session.prefs().actions;
    let (performance, seconds, single) = (saved.performance.name().to_owned(), f64::from(saved.pause_seconds()), saved.create_single_history_state);
    let id = params::open_param(app, KEY, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        ParamDialog::command(KEY, tl!("Playback Options"), rows(&performance, seconds, single)).values(Bind::bound(fields)).alert_title(tl!("PhotoCraft"))
    });
    Ok(json!({"dialog": id.value()}))
}

/// Refuses seconds that are not a whole number in range, which the field cannot show.
fn check(_app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    match result.fields.number("seconds").and_then(whole_seconds) {
        Some(_) => Ok(()),
        None => Err(Label::from(tl!("Pause For takes a whole number of seconds"))),
    }
}

/// OK: stores the options (`actions.setPlayback`) as the dialog holds them.
fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let fields = &result.fields;
    let request = json!({
        "performance": fields.text("performance"),
        "seconds": fields.number("seconds").and_then(whole_seconds),
        "singleState": fields.bool("singleState"),
    });
    if let Err(error) = app.run("actions.setPlayback", request) {
        super::dialogs::report(app, error);
    }
}
