//! [`Host`] over a shared [`photocraft_engine::Session`] on the same thread.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::Session;
use serde_json::Value;

use crate::host::{Host, HostError};

/// Runs commands on a session the caller also holds. Every command of the run carries the same
/// `coalesce` key, so consecutive edits of one document share one history step.
pub struct SessionHost {
    session: Rc<RefCell<Session>>,
    coalesce: String,
    output: Rc<RefCell<Vec<String>>>,
}

impl SessionHost {
    /// `run_key` must be unique per run (for example a counter): it becomes the history step key.
    pub fn new(session: Rc<RefCell<Session>>, run_key: &str) -> Self {
        Self { session, coalesce: format!("script:{run_key}"), output: Rc::default() }
    }

    /// The lines the script wrote, shared with the caller.
    pub fn output(&self) -> Rc<RefCell<Vec<String>>> {
        self.output.clone()
    }
}

/// Make the document with `serial` active, for a command that names it with
/// [`crate::commands::ON_DOCUMENT`].
///
/// # Errors
/// The document is no longer open.
pub fn focus(session: &mut Session, serial: u64) -> Result<(), String> {
    if session.active().is_some_and(|d| d.serial == serial) {
        return Ok(());
    }
    session.execute("document.activate", serde_json::json!({ "serial": serial })).map(|_| ()).map_err(|_| "the document is no longer open".to_owned())
}

impl Host for SessionHost {
    fn execute(&mut self, id: &str, mut params: Value) -> Result<Value, HostError> {
        let target = crate::commands::take_document(&mut params);
        if let Value::Object(map) = &mut params {
            map.entry("coalesce").or_insert_with(|| Value::String(self.coalesce.clone()));
        }
        let mut session = self.session.try_borrow_mut().map_err(|_| HostError("the session is busy".into()))?;
        if let Some(serial) = target {
            focus(&mut session, serial).map_err(HostError)?;
        }
        session.execute(id, params).map_err(|e| HostError(e.to_string()))
    }

    fn write_line(&mut self, line: &str) {
        self.output.borrow_mut().push(line.to_owned());
    }
}
