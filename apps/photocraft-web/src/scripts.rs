//! JavaScript runs in the browser: each run in its own Worker (`photocraft-script-worker`), which
//! the page checks and stops through [`WebJob`], driven by the interface through [`ScriptRun`].
//!
//! The page needs cross-origin isolation for the Worker's shared memory (the COOP and COEP
//! headers, see `packaging/web/README.md`); without it scripts stay off and say why.
//!
//! The Worker answers by message, which the page reads between frames: a run's commands go one
//! per frame or so, slower than on the desktop, where the app waits for the next command within
//! the frame's budget.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use photocraft_script_js::ModalKind;
use photocraft_script_js::grants::PathGrants;
use photocraft_script_js::isolate::web_job::{Transport, WebEvent, WebJob, WebOptions};
use photocraft_ui_egui::{ScriptCommands, ScriptDialog, ScriptEvent, ScriptRun, ScriptStart};
use serde_json::Value;

/// How long a run may compute, not counting time at its dialogs, as on the desktop.
const TIME_LIMIT: Duration = Duration::from_secs(30 * 60);
/// Bytes a run's Worker heap may hold. A wasm32 Worker addresses 4 GiB at most, and the page
/// holds its own documents beside it.
const MEMORY_LIMIT: u64 = 1 << 30;
/// Longest the page may take to run one command, and to see one dialog answered.
const REPLY_LIMITS: (Duration, Duration) = (Duration::from_secs(30 * 60), Duration::from_secs(12 * 60 * 60));
/// The Worker's loader script, next to the page (Trunk's `data-loader-shim`).
#[cfg(target_arch = "wasm32")]
const WORKER_URL: &str = "./photocraft-script-worker_loader.js";

/// Gives each run its own history step key.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The browser script service.
#[cfg(target_arch = "wasm32")]
pub fn service() -> photocraft_ui_egui::StartScriptFn {
    Box::new(|start: ScriptStart| {
        let transport = photocraft_script_js::isolate::WorkerTransport::new(WORKER_URL)?;
        Ok(Box::new(start_run(transport, start)?) as Box<dyn ScriptRun>)
    })
}

/// Start `start` over `transport`.
fn start_run<T: Transport>(transport: T, start: ScriptStart) -> Result<WebRun<T>, String> {
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    let options = WebOptions {
        app_name: start.app_name,
        script_name: Some(start.name),
        time_limit: TIME_LIMIT,
        memory_limit: MEMORY_LIMIT,
        coalesce: format!("script:web:{run}"),
        reply_limits: REPLY_LIMITS,
    };
    let job = WebJob::spawn(transport, start.source, options)?;
    Ok(WebRun { job, waiting: None, grants: PathGrants::default() })
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

/// A run in a Worker.
struct WebRun<T: Transport> {
    job: WebJob<T>,
    /// The dialog it waits at.
    waiting: Option<ModalKind>,
    /// Where the run may write. Kept on the page, not in the Worker: the Worker is not trusted.
    grants: PathGrants,
}

impl<T: Transport> ScriptRun for WebRun<T> {
    fn pump(&mut self, commands: &mut dyn ScriptCommands, budget: Duration) -> Vec<ScriptEvent> {
        let start = web_time::Instant::now();
        let mut out = Vec::new();
        while let Some(event) = self.job.poll() {
            match event {
                WebEvent::Execute { id, params } => {
                    let result = match self.grants.check(&id, &params) {
                        Ok(()) => commands.execute(&id, params),
                        Err(e) => Err(e),
                    };
                    // An answer the Worker can't take ends the run; that comes as `Finished`.
                    let _ = self.job.reply_executed(result);
                }
                WebEvent::Output(line) => out.push(ScriptEvent::Output(line)),
                WebEvent::Window { id, window } => out.push(ScriptEvent::Window { id, window }),
                WebEvent::Dialog(modal) => {
                    self.waiting = Some(modal.kind);
                    out.push(ScriptEvent::Dialog { kind: dialog_kind(modal.kind), args: modal.args });
                    break;
                }
                WebEvent::Finished(result) => {
                    self.waiting = None;
                    out.push(ScriptEvent::Finished(result));
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
        self.job.reply_dialog(answer)
    }

    fn cancel(&mut self) {
        self.waiting = None;
        self.job.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_script_js::isolate::protocol::FromHost;
    use photocraft_script_js::isolate::web_job::Incoming;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A Worker that has sent `inbox` and records what the page answers.
    #[derive(Clone, Default)]
    struct Fake {
        inbox: Rc<RefCell<Vec<Incoming>>>,
        answers: Rc<RefCell<Vec<String>>>,
        terminated: Rc<RefCell<bool>>,
    }

    impl Fake {
        fn send(&self, msg: &FromHost) {
            self.inbox.borrow_mut().push(Incoming::Message(serde_json::to_string(msg).unwrap(), web_time::Instant::now()));
        }
    }

    impl Transport for Fake {
        fn start(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn take(&mut self) -> Vec<Incoming> {
            std::mem::take(&mut *self.inbox.borrow_mut())
        }
        fn answer(&mut self, answer: &str) -> Result<(), String> {
            self.answers.borrow_mut().push(answer.to_owned());
            Ok(())
        }
        fn terminate(&mut self) {
            *self.terminated.borrow_mut() = true;
        }
    }

    #[derive(Default)]
    struct Commands(Vec<(String, Value)>);

    impl ScriptCommands for Commands {
        fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
            self.0.push((id.to_owned(), params));
            Ok(json!({"ok": true}))
        }
    }

    fn run(fake: &Fake) -> WebRun<Fake> {
        start_run(fake.clone(), ScriptStart { name: "t.jsx".into(), source: "1".into(), app_name: None }).unwrap()
    }

    const BUDGET: Duration = Duration::from_secs(5);

    #[test]
    fn commands_output_and_the_end_reach_the_interface_in_order() {
        let fake = Fake::default();
        let mut r = run(&fake);
        fake.send(&FromHost::Output { line: "hello".into() });
        fake.send(&FromHost::Execute { id: "session.inspect".into(), params: json!({}) });
        let mut commands = Commands::default();
        assert_eq!(r.pump(&mut commands, BUDGET), [ScriptEvent::Output("hello".into())]);
        assert_eq!(commands.0.len(), 1, "the command ran on the session");
        assert!(commands.0[0].1.get("coalesce").is_some(), "with the run's history key: {:?}", commands.0[0].1);
        assert_eq!(fake.answers.borrow().len(), 1, "and its result went back to the Worker");
        fake.send(&FromHost::Finished { error: None });
        assert_eq!(r.pump(&mut commands, BUDGET), [ScriptEvent::Finished(Ok(()))]);
    }

    #[test]
    fn a_save_dialog_answer_grants_that_path_and_only_that_path() {
        let fake = Fake::default();
        let mut r = run(&fake);
        fake.send(&FromHost::Dialog { kind: ModalKind::SaveFile.into(), args: json!({}) });
        let mut commands = Commands::default();
        let events = r.pump(&mut commands, BUDGET);
        assert!(matches!(events.as_slice(), [ScriptEvent::Dialog { kind: ScriptDialog::SaveFile, .. }]), "{events:?}");
        // Absolute on the platform the tests run on.
        let (chosen, elsewhere) = if cfg!(windows) { (r"C:\out\chosen.png", r"C:\etc\elsewhere.png") } else { ("/out/chosen.png", "/etc/elsewhere.png") };
        r.answer(json!(chosen)).unwrap();
        // A write elsewhere is refused on the page, before the session sees it; the chosen path
        // goes through.
        fake.send(&FromHost::Execute { id: "file.saveACopy".into(), params: json!({"path": elsewhere}) });
        assert_eq!(r.pump(&mut commands, BUDGET), [], "the run goes on");
        assert!(commands.0.is_empty(), "{:?}", commands.0);
        assert!(fake.answers.borrow().last().unwrap().contains("only save to a file or folder you chose"), "{:?}", fake.answers.borrow());
        fake.send(&FromHost::Execute { id: "file.saveACopy".into(), params: json!({"path": chosen}) });
        r.pump(&mut commands, BUDGET);
        assert_eq!(commands.0.len(), 1, "{:?}", fake.answers.borrow());
        assert_eq!(commands.0[0].0, "file.saveACopy");
    }

    #[test]
    fn a_cancelled_run_stops_its_worker_and_says_so() {
        let fake = Fake::default();
        let mut r = run(&fake);
        r.cancel();
        let events = r.pump(&mut Commands::default(), BUDGET);
        assert!(matches!(events.as_slice(), [ScriptEvent::Finished(Err(m))] if m == photocraft_script_js::isolate::CANCELLED), "{events:?}");
        assert!(*fake.terminated.borrow());
    }
}
