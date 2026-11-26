//! JavaScript runs on the desktop: each run in its own script host process (the
//! `photocraft-script-host` executable next to this one), driven by the interface through
//! [`ScriptRun`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use photocraft_script_js::isolate::{
    COMMAND_REPLY_LIMIT, DEFAULT_MEMORY_LIMIT, DIALOG_REPLY_LIMIT, Event, HelperCommand, IsolateOptions, IsolatedJob, QUEUE_GRACE, Reply,
};
use photocraft_script_js::{ModalKind, RunOptions};
use photocraft_ui_egui::i18n::{fmt, t};
use photocraft_ui_egui::{ScriptCommands, ScriptDialog, ScriptEvent, ScriptRun, ScriptStart, StartScriptFn};
use serde_json::Value;

/// How long a run may compute, not counting time at its dialogs. Scripts automate long jobs, and
/// the user can stop one at any time; the limit catches a run that never ends on its own.
const TIME_LIMIT: Duration = Duration::from_secs(30 * 60);

/// The script host's file name.
const HELPER: &str = "photocraft-script-host";

/// Gives each run its own History key.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The script host: next to this executable, where every package puts it.
fn helper() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| fmt(t("Couldn't find the script host: {error}"), &[("error", &e.to_string())]))?;
    helper_beside(&exe)
}

/// The script host beside `exe`.
pub(crate) fn helper_beside(exe: &Path) -> Result<PathBuf, String> {
    let path = exe.with_file_name(format!("{HELPER}{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        let template = "Couldn't find the script host ({path}): this installation of PhotoCraft is missing it.";
        Err(fmt(t(template), &[("path", &path.display().to_string())]))
    }
}

/// The desktop script service.
pub fn service() -> StartScriptFn {
    Box::new(|start: ScriptStart| start_run(helper()?, start))
}

/// Start a run in the script host at `helper`.
pub(crate) fn start_run(helper: PathBuf, start: ScriptStart) -> Result<Box<dyn ScriptRun>, String> {
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    let options = IsolateOptions {
        run: RunOptions { app_name: start.app_name, script_name: Some(start.name), ..RunOptions::default() },
        time_limit: TIME_LIMIT,
        memory_limit: DEFAULT_MEMORY_LIMIT,
        coalesce: format!("script:{}:{run}", std::process::id()),
        helper: HelperCommand::new(helper),
        reply_limits: (COMMAND_REPLY_LIMIT, DIALOG_REPLY_LIMIT),
        queue_grace: QUEUE_GRACE,
    };
    let job = IsolatedJob::spawn(start.source, options).map_err(|f| f.message)?;
    Ok(Box::new(HostRun { job, waiting: false }))
}

fn dialog_kind(kind: ModalKind) -> ScriptDialog {
    match kind {
        ModalKind::Alert => ScriptDialog::Alert,
        ModalKind::Confirm => ScriptDialog::Confirm,
        ModalKind::Prompt => ScriptDialog::Prompt,
    }
}

struct HostRun {
    job: IsolatedJob,
    /// It waits at a dialog: nothing to pump until it is answered.
    waiting: bool,
}

impl ScriptRun for HostRun {
    fn pump(&mut self, commands: &mut dyn ScriptCommands, budget: Duration) -> Vec<ScriptEvent> {
        let start = Instant::now();
        let mut out = Vec::new();
        // Right after a reply the host is about to ask again: wait for that within the budget,
        // rather than leaving every command of a script to its own frame.
        let mut replied = false;
        loop {
            let wait = if replied { budget.saturating_sub(start.elapsed()) } else { Duration::ZERO };
            let Some(event) = self.job.poll(wait) else { break };
            replied = false;
            // At a dialog the host waits for the answer and sends nothing, but it may still end
            // (killed, out of memory): that ends the run and its dialog.
            if self.waiting && !matches!(event, Event::Finished(_)) {
                continue;
            }
            match event {
                Event::Execute { id, params } => {
                    let result = commands.execute(&id, params);
                    // A reply the host can't take ends the run; that arrives as `Finished`.
                    replied = self.job.reply(Reply::Executed(result)).is_ok();
                }
                // The host goes on running: its next message is likely close behind.
                Event::Output(line) => {
                    out.push(ScriptEvent::Output(line));
                    replied = true;
                }
                Event::Dialog(modal) => {
                    self.waiting = true;
                    out.push(ScriptEvent::Dialog { kind: dialog_kind(modal.kind), args: modal.args });
                    break;
                }
                Event::Finished(result) => {
                    self.waiting = false;
                    out.push(ScriptEvent::Finished(result.map_err(|f| f.message)));
                    break;
                }
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        out
    }

    fn answer(&mut self, answer: Value) -> Result<(), String> {
        self.waiting = false;
        self.job.reply(Reply::Dialog(answer))
    }

    fn cancel(&mut self) {
        self.waiting = false;
        self.job.cancel();
    }
}
