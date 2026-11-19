//! What a run can reach outside the engine.

use std::fmt;

use serde_json::Value;

use crate::ModalKind;

/// A failed host call. Shown to the script as a catchable `Error` with this message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}

/// The application side of a run. Calls arrive only from the first attempt that reaches them;
/// replays are answered from the tape.
pub trait Host {
    /// Run a session command by id (the same ids and params as automation).
    ///
    /// # Errors
    /// The command's error.
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, HostError>;

    /// A line of script output (`$.writeln`).
    fn write_line(&mut self, line: &str);

    /// Show a modal dialog and wait for its answer. `None`, the default, means this host cannot
    /// wait: the run pauses, and a later attempt replays the script up to the dialog. A host
    /// that can block (the script host process, which has a thread to itself) answers here
    /// instead, so the script is not run again from the start after every dialog: replays cost
    /// the script's whole running time each, and that time counts against its limit.
    ///
    /// # Errors
    /// The dialog could not be shown or answered; the run ends with this message.
    fn modal(&mut self, kind: ModalKind, args: &Value) -> Option<Result<Value, HostError>> {
        let _ = (kind, args);
        None
    }

    /// Show or update a ScriptUI window that does not wait for the user (a palette, or a dialog
    /// whose `show()` is between events), or hide it with `None`. Needs no answer. `id` is the
    /// window's id in `window`'s controls. The default ignores it.
    ///
    /// # Errors
    /// The window could not be sent; the script gets this as a catchable error.
    fn window(&mut self, id: u32, window: Option<&Value>) -> Result<(), HostError> {
        let _ = (id, window);
        Ok(())
    }
}
