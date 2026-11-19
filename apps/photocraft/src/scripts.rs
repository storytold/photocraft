//! JavaScript runs on the desktop: each run in its own script host process (this executable
//! started with `--script-host`), driven by the interface through [`ScriptRun`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use photocraft_script_js::grants::PathGrants;
use photocraft_script_js::isolate::{
    COMMAND_REPLY_LIMIT, DEFAULT_MEMORY_LIMIT, DIALOG_REPLY_LIMIT, Event, HelperCommand, IsolateOptions, IsolatedJob, QUEUE_GRACE, Reply,
};
use photocraft_script_js::{ModalKind, RunOptions};
use photocraft_ui_egui::{ScriptCommands, ScriptDialog, ScriptEvent, ScriptRun, ScriptStart, StartScriptFn};
use serde_json::Value;

/// How long a run may compute, not counting time at its dialogs. Scripts automate long jobs, and
/// the user can stop one at any time; the limit catches a run that never ends on its own.
const TIME_LIMIT: Duration = Duration::from_secs(30 * 60);

/// Gives each run its own history step key.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The desktop script service.
pub fn service() -> StartScriptFn {
    Box::new(|start: ScriptStart| {
        let helper = HelperCommand::current_exe().map_err(|e| format!("Couldn't start the script host: {e}"))?;
        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let options = IsolateOptions {
            run: RunOptions { app_name: start.app_name, script_name: Some(start.name), ..RunOptions::default() },
            time_limit: TIME_LIMIT,
            memory_limit: DEFAULT_MEMORY_LIMIT,
            coalesce: format!("script:{}:{run}", std::process::id()),
            helper,
            reply_limits: (COMMAND_REPLY_LIMIT, DIALOG_REPLY_LIMIT),
            queue_grace: QUEUE_GRACE,
        };
        let job = IsolatedJob::spawn(start.source, options).map_err(|f| f.message)?;
        Ok(Box::new(HostRun { job, waiting: None, grants: PathGrants::default() }) as Box<dyn ScriptRun>)
    })
}

fn dialog_kind(kind: ModalKind) -> ScriptDialog {
    match kind {
        ModalKind::Alert => ScriptDialog::Alert,
        ModalKind::Confirm => ScriptDialog::Confirm,
        ModalKind::Prompt => ScriptDialog::Prompt,
        ModalKind::Window => ScriptDialog::Window,
        ModalKind::OpenFile => ScriptDialog::OpenFile,
        ModalKind::SaveFile => ScriptDialog::SaveFile,
        ModalKind::SelectFolder => ScriptDialog::SelectFolder,
    }
}

struct HostRun {
    job: IsolatedJob,
    /// The dialog it waits at: nothing to pump until it is answered.
    waiting: Option<ModalKind>,
    /// Where the run may write. Kept here, not in the script host: the host is not trusted.
    grants: PathGrants,
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
            if self.waiting.is_some() && !matches!(event, Event::Finished(_)) {
                continue;
            }
            match event {
                Event::Execute { id, params } => {
                    let result = match self.grants.check(&id, &params) {
                        Ok(()) => commands.execute(&id, params),
                        Err(e) => Err(e),
                    };
                    // A reply the host can't take ends the run; that arrives as `Finished`.
                    replied = self.job.reply(Reply::Executed(result)).is_ok();
                }
                // The host goes on running: its next message is likely close behind.
                Event::Output(line) => {
                    out.push(ScriptEvent::Output(line));
                    replied = true;
                }
                Event::Window { id, window } => {
                    out.push(ScriptEvent::Window { id, window });
                    replied = true;
                }
                Event::Dialog(modal) => {
                    self.waiting = Some(modal.kind);
                    out.push(ScriptEvent::Dialog { kind: dialog_kind(modal.kind), args: modal.args });
                    break;
                }
                Event::Finished(result) => {
                    self.waiting = None;
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
        if let Some(kind) = self.waiting.take() {
            self.grants.record(kind, &answer);
        }
        self.job.reply(Reply::Dialog(answer))
    }

    fn cancel(&mut self) {
        self.waiting = None;
        self.job.cancel();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_interface_and_the_script_engine_name_a_commands_document_alike() {
        assert_eq!(photocraft_ui_egui::ON_DOCUMENT, photocraft_script_js::commands::ON_DOCUMENT);
    }
}
