//! The script host's side of a run, over any channel to the app: a process's standard input and
//! output on the desktop (`serve`), and a web Worker's messages for the web build. The
//! channel only moves [`FromHost`] and [`ToHost`]
//! messages; everything else here is the same on both.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;

use super::protocol::{FromHost, MAX_LINE, ToHost, cut};
use crate::host::{Host, HostError};
use crate::{ModalKind, RunOptions, Runner, Step, Unlimited};

/// A channel to the app. `recv` blocks until the app answers.
pub(crate) trait Link {
    /// Send a message to the app.
    ///
    /// # Errors
    /// The message could not be sent (too large, or the app is gone).
    fn send(&mut self, msg: &FromHost) -> Result<(), String>;

    /// Wait for the app's next message.
    ///
    /// # Errors
    /// The app closed the run.
    fn recv(&mut self) -> Result<ToHost, String>;
}

/// The run's [`Host`]: every call is a message to the app.
struct LinkHost {
    link: Rc<RefCell<dyn Link>>,
}

impl LinkHost {
    fn send(&self, msg: &FromHost) -> Result<(), String> {
        self.link.try_borrow_mut().map_err(|_| "internal error: the channel to the app is busy".to_owned())?.send(msg)
    }

    fn recv(&self) -> Result<ToHost, String> {
        self.link.try_borrow_mut().map_err(|_| "internal error: the channel to the app is busy".to_owned())?.recv()
    }
}

impl Host for LinkHost {
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, HostError> {
        self.send(&FromHost::Execute { id: id.to_owned(), params }).map_err(|e| HostError(format!("the command could not be sent to the app: {e}")))?;
        match self.recv() {
            Ok(ToHost::Executed { result }) => result.map_err(HostError),
            Ok(_) => Err(HostError("internal error: the app answered a command with something else".into())),
            // The transport's own reason: a closed channel says "the app closed the run", and a
            // browser answer the Worker refuses says why.
            Err(e) => Err(HostError(e)),
        }
    }

    fn write_line(&mut self, line: &str) {
        let head = cut(line, MAX_LINE);
        let line = if head.len() < line.len() { format!("{head} [line cut: {} more bytes]", line.len() - head.len()) } else { line.to_owned() };
        let _ = self.send(&FromHost::Output { line });
    }

    /// The host has nothing else to do: wait for the answer here, so the script is not replayed
    /// from its start after every dialog.
    fn modal(&mut self, kind: ModalKind, args: &Value) -> Option<Result<Value, HostError>> {
        if let Err(e) = self.send(&FromHost::Dialog { kind: kind.into(), args: args.clone() }) {
            return Some(Err(HostError(format!("The dialog could not be shown: {e}"))));
        }
        Some(match self.recv() {
            Ok(ToHost::Dialog { answer }) => Ok(answer),
            Ok(_) => Err(HostError("internal error: the app answered a dialog with something else".into())),
            Err(e) => Err(HostError(e)),
        })
    }

    fn window(&mut self, id: u32, window: Option<&Value>) -> Result<(), HostError> {
        // It needs no answer, but one that can't be sent (larger than a message may be) is the
        // script's error, not a window silently left as it was.
        self.send(&FromHost::Window { id, window: window.cloned() }).map_err(|e| HostError(format!("the window could not be shown: {e}")))
    }
}

/// Run `source` against the app at the other end of `link` until it ends: `Ok` when the script
/// finished, else the message for the user. The app owns the clock and stops the host when the
/// time limit passes, so the run itself has none.
pub(crate) fn run(link: Rc<RefCell<dyn Link>>, source: &str, app_name: Option<String>, script_name: Option<String>) -> Result<(), String> {
    let host = LinkHost { link: link.clone() };
    let options = RunOptions { app_name, script_name, unbounded_loops: true, unguarded_nesting: true };
    let mut runner = Runner::new(source, Box::new(host), Box::new(Unlimited), options)?;
    let link = LinkHost { link };
    loop {
        match runner.step() {
            Step::Done => return Ok(()),
            Step::Failed(m) => return Err(m),
            // A dialog the host could not wait at (never, here): ask, and answer the replay.
            Step::Paused(modal) => {
                link.send(&FromHost::Dialog { kind: modal.kind.into(), args: modal.args }).map_err(|e| format!("The dialog could not be shown: {e}"))?;
                match link.recv() {
                    Ok(ToHost::Dialog { answer }) => runner.answer(answer)?,
                    Ok(_) => return Err("internal error: the app answered a dialog with something else".into()),
                    Err(e) => return Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link whose answers can't be read, for its own reason.
    struct Unreadable;

    impl Link for Unreadable {
        fn send(&mut self, _: &FromHost) -> Result<(), String> {
            Ok(())
        }

        fn recv(&mut self) -> Result<ToHost, String> {
            Err("the answer holds more values than a script reads".into())
        }
    }

    /// The transport's reason reaches the script; it isn't turned into "the app closed the run".
    #[test]
    fn a_receive_error_keeps_the_transports_reason() {
        let link: Rc<RefCell<dyn Link>> = Rc::new(RefCell::new(Unreadable));
        let mut host = LinkHost { link };
        let err = host.execute("session.inspect", serde_json::json!({})).unwrap_err();
        assert_eq!(err.0, "the answer holds more values than a script reads");
        let Some(Err(err)) = host.modal(ModalKind::Alert, &serde_json::json!(["x"])) else { panic!("no answer") };
        assert_eq!(err.0, "the answer holds more values than a script reads");
    }
}
