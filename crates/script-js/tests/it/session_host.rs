//! A [`Host`] over a [`Session`] the test also holds, for runs in the test's own process. It does
//! what the app does with a run's commands: a command naming its document runs there, every
//! command carries the run's history key, and files are written only where the user's answers
//! to the run's file dialogs allow ([`PathGrants`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use photocraft_engine::Session;
use photocraft_script_js::grants::PathGrants;
use photocraft_script_js::{Host, HostError, ModalKind};
use serde_json::{Value, json};

/// What the user does while a dialog is open.
type AtDialog = Box<dyn FnMut(&mut Session)>;

/// Runs commands on the shared session, keeps the script's output and the dialogs it opened,
/// and answers each dialog with the next of its answers (`null` once they run out), after
/// running `at_dialog` on the session (what the user does while the dialog is open).
pub(crate) struct SessionHost {
    session: Rc<RefCell<Session>>,
    output: Rc<RefCell<Vec<String>>>,
    dialogs: Rc<RefCell<Vec<Value>>>,
    at_dialog: Option<AtDialog>,
    answers: VecDeque<Value>,
    grants: PathGrants,
}

impl SessionHost {
    pub(crate) fn new(session: Rc<RefCell<Session>>) -> Self {
        Self { session, output: Rc::default(), dialogs: Rc::default(), at_dialog: None, answers: VecDeque::new(), grants: PathGrants::default() }
    }

    /// Run `f` on the session each time the script opens a dialog.
    pub(crate) fn at_dialog(mut self, f: impl FnMut(&mut Session) + 'static) -> Self {
        self.at_dialog = Some(Box::new(f));
        self
    }

    /// Answer the run's dialogs with `answers`, in order.
    pub(crate) fn answering(mut self, answers: &[Value]) -> Self {
        self.answers = answers.iter().cloned().collect();
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
    fn execute(&mut self, id: &str, mut params: Value) -> Result<Value, HostError> {
        self.grants.check(id, &params).map_err(HostError)?;
        let checked = params.clone();
        let target = photocraft_script_js::commands::take_document(&mut params);
        if let Value::Object(map) = &mut params {
            map.insert("coalesce".into(), Value::String("script:test".into()));
        }
        let mut session = self.session.try_borrow_mut().map_err(|_| HostError("the session is busy".into()))?;
        if let Some(serial) = target
            && !session.active().is_some_and(|d| d.serial == serial)
        {
            session.execute("document.activate", json!({ "serial": serial })).map_err(|_| HostError("the document is no longer open".into()))?;
        }
        let result = session.execute(id, params).map_err(|e| HostError(e.to_string()))?;
        self.grants.wrote(id, &checked);
        Ok(result)
    }

    fn write_line(&mut self, line: &str) {
        self.output.borrow_mut().push(line.to_owned());
    }

    fn modal(&mut self, kind: ModalKind, args: &Value) -> Result<Value, HostError> {
        self.dialogs.borrow_mut().push(args.clone());
        if let Some(f) = &mut self.at_dialog {
            let mut session = self.session.try_borrow_mut().map_err(|_| HostError("the session is busy".into()))?;
            f(&mut session);
        }
        let answer = self.answers.pop_front().unwrap_or(Value::Null);
        self.grants.record(kind, &answer);
        Ok(answer)
    }
}
