//! A [`Host`] over a [`Session`] the test also holds, for runs in the test's own process.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::Session;
use photocraft_script_js::{Host, HostError, ModalKind};
use serde_json::Value;

/// What the user does while a dialog is open.
type AtDialog = Box<dyn FnMut(&mut Session)>;

/// Runs commands on the shared session, keeps the script's output and the dialogs it opened,
/// and answers every dialog with `null`, after running `at_dialog` on the session (what the
/// user does while the dialog is open).
pub(crate) struct SessionHost {
    session: Rc<RefCell<Session>>,
    output: Rc<RefCell<Vec<String>>>,
    dialogs: Rc<RefCell<Vec<Value>>>,
    at_dialog: Option<AtDialog>,
}

impl SessionHost {
    pub(crate) fn new(session: Rc<RefCell<Session>>) -> Self {
        Self { session, output: Rc::default(), dialogs: Rc::default(), at_dialog: None }
    }

    /// Run `f` on the session each time the script opens a dialog.
    pub(crate) fn at_dialog(mut self, f: impl FnMut(&mut Session) + 'static) -> Self {
        self.at_dialog = Some(Box::new(f));
        self
    }

    /// The lines the script wrote.
    pub(crate) fn output(&self) -> Rc<RefCell<Vec<String>>> {
        self.output.clone()
    }

    /// The arguments of each dialog the script opened.
    pub(crate) fn dialogs(&self) -> Rc<RefCell<Vec<Value>>> {
        self.dialogs.clone()
    }
}

impl Host for SessionHost {
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, HostError> {
        let mut session = self.session.try_borrow_mut().map_err(|_| HostError("the session is busy".into()))?;
        session.execute(id, params).map_err(|e| HostError(e.to_string()))
    }

    fn write_line(&mut self, line: &str) {
        self.output.borrow_mut().push(line.to_owned());
    }

    fn modal(&mut self, _: ModalKind, args: &Value) -> Result<Value, HostError> {
        self.dialogs.borrow_mut().push(args.clone());
        if let Some(f) = &mut self.at_dialog {
            let mut session = self.session.try_borrow_mut().map_err(|_| HostError("the session is busy".into()))?;
            f(&mut session);
        }
        Ok(Value::Null)
    }
}
