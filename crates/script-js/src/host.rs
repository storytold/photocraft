//! What a run can reach outside the engine.

use std::fmt;

use serde_json::Value;

use crate::ModalKind;

/// A failed host call. A failed command is a catchable `Error` with this message; a failed
/// dialog ends the run with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}

/// The application side of a run.
pub trait Host {
    /// Run a session command by id (the same ids and params as automation).
    ///
    /// # Errors
    /// The command's error.
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, HostError>;

    /// A line of script output (`$.writeln`).
    fn write_line(&mut self, line: &str);

    /// Show a modal dialog and wait for its answer: `null` for an alert or a cancelled prompt, a
    /// boolean for confirm, a string for prompt. The run waits here; its time limit does not
    /// count the wait.
    ///
    /// # Errors
    /// The dialog could not be shown or answered; the run ends with this message.
    fn modal(&mut self, kind: ModalKind, args: &Value) -> Result<Value, HostError>;
}
