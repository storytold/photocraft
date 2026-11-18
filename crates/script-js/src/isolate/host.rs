//! The script host: the process that runs one script for the app.
//!
//! It reads [`ToHost`] frames on standard input and writes [`FromHost`] frames on standard
//! output. A reader thread forwards input to the run and ends the process when the input ends,
//! so a host never outlives its app. The run itself executes on a thread with a large explicit
//! stack ([`STACK`]); replays after a dialog stay inside the same process.

use std::io::{self, Write};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use serde_json::Value;

use super::protocol::{FromHost, MAX_FROM_HOST, MAX_LINE, MAX_TO_HOST, ToHost, cut, read_frame, write_frame};
use crate::host::{Host, HostError};
use crate::{ModalKind, RunOptions, Runner, Step, Unlimited};

/// Stack of the script thread: the parser and some builtins recurse on nested input. Smaller on
/// a 32-bit system, whose 2 GiB of address space must also hold the heap.
pub const STACK: usize = if cfg!(target_pointer_width = "64") { 256 << 20 } else { 64 << 20 };

/// Exit status when the input was not a valid start of a run.
const BAD_INPUT: i32 = 2;

fn send(msg: &FromHost) -> Result<(), String> {
    let mut out = io::stdout().lock();
    match write_frame(&mut out, msg, MAX_FROM_HOST) {
        Ok(()) => Ok(()),
        // The app is gone; the reader thread ends the process.
        Err(e) => Err(e.to_string()),
    }
}

/// The run's [`Host`]: every call is a frame to the app.
struct PipeHost {
    inbox: Rc<Receiver<ToHost>>,
}

impl Host for PipeHost {
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, HostError> {
        send(&FromHost::Execute { id: id.to_owned(), params }).map_err(|e| HostError(format!("the command could not be sent to the app: {e}")))?;
        match self.inbox.recv() {
            Ok(ToHost::Executed { result }) => result.map_err(HostError),
            Ok(_) => Err(HostError("internal error: the app answered a command with something else".into())),
            Err(_) => Err(HostError("the app closed the run".into())),
        }
    }

    fn write_line(&mut self, line: &str) {
        let head = cut(line, MAX_LINE);
        let line = if head.len() < line.len() { format!("{head} [line cut: {} more bytes]", line.len() - head.len()) } else { line.to_owned() };
        let _ = send(&FromHost::Output { line });
    }

    /// This process has nothing else to do: wait for the answer here, so the script is not
    /// replayed from its start after every dialog.
    fn modal(&mut self, kind: ModalKind, args: &Value) -> Option<Result<Value, HostError>> {
        if let Err(e) = send(&FromHost::Dialog { kind: kind.into(), args: args.clone() }) {
            return Some(Err(HostError(format!("The dialog could not be shown: {e}"))));
        }
        Some(match self.inbox.recv() {
            Ok(ToHost::Dialog { answer }) => Ok(answer),
            Ok(_) => Err(HostError("internal error: the app answered a dialog with something else".into())),
            Err(_) => Err(HostError("the app closed the run".into())),
        })
    }

    fn window(&mut self, id: u32, window: Option<&Value>) -> Result<(), HostError> {
        // It needs no answer, but one that can't be sent (larger than a frame may be) is the
        // script's error, not a window silently left as it was.
        send(&FromHost::Window { id, window: window.cloned() }).map_err(|e| HostError(format!("the window could not be shown: {e}")))
    }
}

fn drive(source: &str, app_name: Option<String>, script_name: Option<String>, inbox: Receiver<ToHost>) -> Result<(), String> {
    let inbox = Rc::new(inbox);
    let host = PipeHost { inbox: inbox.clone() };
    // The app owns the clock: it kills this process when the time limit passes.
    let mut runner =
        Runner::new(source, Box::new(host), Box::new(Unlimited), RunOptions { app_name, script_name, unbounded_loops: true, unguarded_nesting: true })?;
    loop {
        match runner.step() {
            Step::Done => return Ok(()),
            Step::Failed(m) => return Err(m),
            Step::Paused(modal) => {
                send(&FromHost::Dialog { kind: modal.kind.into(), args: modal.args }).map_err(|e| format!("The dialog could not be shown: {e}"))?;
                match inbox.recv() {
                    Ok(ToHost::Dialog { answer }) => runner.answer(answer)?,
                    Ok(_) => return Err("internal error: the app answered a dialog with something else".into()),
                    Err(_) => return Err("the app closed the run".into()),
                }
            }
        }
    }
}

/// Serve one run on standard input and output; returns the process exit status. `heap` is the
/// executable's global allocator ([`super::limits::host_allocator`]), which the run's memory limit
/// caps.
pub fn serve(heap: &'static super::limits::HostAllocator) -> i32 {
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::Builder::new().name("script-host-input".into()).spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match read_frame::<_, ToHost>(&mut input, MAX_TO_HOST) {
                Ok(Some(msg)) => {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
                // The app closed the pipe or died: nothing can use this run any more.
                Ok(None) => std::process::exit(0),
                Err(e) => {
                    let _ = writeln!(io::stderr(), "script host: bad input: {e}");
                    std::process::exit(BAD_INPUT);
                }
            }
        }
    });
    if reader.is_err() {
        return BAD_INPUT;
    }
    let Ok(ToHost::Start { source, app_name, script_name, memory_limit }) = rx.recv() else {
        let _ = writeln!(io::stderr(), "script host: the first message was not a start");
        return BAD_INPUT;
    };
    let result = match super::limits::limit_memory(memory_limit, heap) {
        Err(e) => Err(format!("Couldn't limit the script's memory: {e}")),
        Ok(()) => match std::thread::Builder::new().name("script".into()).stack_size(STACK).spawn(move || drive(&source, app_name, script_name, rx)) {
            Err(e) => Err(format!("Couldn't start the script: {e}")),
            Ok(t) => t.join().unwrap_or_else(|_| Err("The script stopped unexpectedly: internal error in the script host.".into())),
        },
    };
    let _ = send(&FromHost::Finished { error: result.err() });
    // Wait for the app to close the pipe (the reader thread exits the process then).
    loop {
        std::thread::park();
    }
}
