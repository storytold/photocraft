//! Pipeline hooks: the engine crates' parts of running a command.
//!
//! Core runs every command (`Session::dispatch`, a job's apply, the bookkeeping after both). Where
//! a step belongs to a crate above core (a floating selection drops, Last Filter runs the filter it
//! repeats, an action records the step), core calls the functions that crate registered for that
//! stage. Each crate's `register` adds its hooks to the [`Install`](crate::registry::Install) that
//! also carries its command tables; the process installs both once.

use photocraft_doc::{DocId, LayerId};
use serde_json::Value;

use crate::commands::CommandSpec;
use crate::{Result, Session};

/// Replaces a request's params before the authorization gates see them.
pub type RequestParams = fn(&Session, &str, &Value) -> Option<Value>;
/// Runs another command in this one's place, with its params.
pub type Redirect = fn(&Session, &str, &Value) -> Option<(String, Value)>;
/// Refuses params the command can't take.
pub type CheckParams = fn(&Session, &str, &Value) -> Result<()>;
/// Refuses a command the session can't run now, after core's own admission checks.
pub type Admit = fn(&Session, &str, &Value) -> Result<()>;
/// Runs once the params are valid, before the command: `Some` is the command's result, and the
/// command itself doesn't run.
pub type BeforeCommand = fn(&mut Session, &str, &CommandSpec, &Value) -> Result<Option<Value>>;
/// Rewrites the params of a command that ran, before a job or the journal takes them.
pub type RunParams = fn(&Session, &str, &mut Value);
/// Runs after every command or job apply that succeeded, with the params it ran with.
pub type AfterCommand = fn(&mut Session, &str, &Value);
/// The params a recording keeps for one command, given the layers selected before it ran;
/// `None` leaves the step to [`Hooks::recorded_step`].
pub type RecordedStepFor = fn(&Session, &str, &Value, Option<&[LayerId]>) -> Option<Value>;
/// The params a recording keeps for any command without its own entry in
/// [`Hooks::recorded_steps`].
pub type RecordedStep = fn(&Session, &str, Value, Option<&[LayerId]>) -> Value;
/// A step joined the recording; `true` when its request ran under automation's gate.
pub type StepRecorded = fn(&mut Session, bool, &str, &Value);
/// Folds the step into the journal's last entry instead of adding it; `true` when it did.
pub type CoalesceJournal = fn(&mut Session, &str, &Value) -> bool;
/// Commits the steps recorded so far to the action being recorded.
pub type FlushRecording = fn(&mut Session) -> Result<()>;
/// Applies the preferences to the crate's own state.
pub type PrefsApplied = fn(&mut Session);
/// The document at this index is about to close.
pub type DocumentClosing = fn(&mut Session, usize);
/// Whether the crate still refers to a document id, so a newly opened document must not take it.
pub type DocIdReserved = fn(&Session, DocId) -> bool;
/// An edit changed the active document's pixels.
pub type PixelsEdited = fn(&mut Session);

/// The hooks of every installed crate, per stage, in the order the crates registered them.
#[derive(Default)]
pub struct Hooks {
    /// Before the authorization gates; the first `Some` replaces the params.
    pub request_params: Vec<RequestParams>,
    /// Once authorized; the first `Some` names the command that runs instead.
    pub redirect: Vec<Redirect>,
    /// After the documented-keys check, in `dispatch` and `precheck`; the first error refuses.
    pub check_params: Vec<CheckParams>,
    /// At the end of `admit` (in `precheck`, and in `dispatch` before and after a floating
    /// selection drops); the first error refuses.
    pub admit: Vec<Admit>,
    /// Before the command runs; the first `Some` finishes it.
    pub before_command: Vec<BeforeCommand>,
    /// After a successful run, each in turn.
    pub run_params: Vec<RunParams>,
    /// After a successful command or job apply, each in turn.
    pub after_command: Vec<AfterCommand>,
    /// While recording: a command's own recorded step, by command id.
    pub recorded_steps: Vec<(&'static str, RecordedStepFor)>,
    /// While recording: the recorded step of every other command. None keeps the params.
    pub recorded_step: Option<RecordedStep>,
    /// While recording, once a step is journaled, each in turn.
    pub step_recorded: Vec<StepRecorded>,
    /// Before a step is journaled; the first `true` keeps it out of the journal.
    pub coalesce_journal: Vec<CoalesceJournal>,
    /// Before a job applies its step while a recording it didn't start runs, each in turn.
    pub flush_recording: Vec<FlushRecording>,
    /// After the preferences changed and core applied its own, each in turn.
    pub prefs_applied: Vec<PrefsApplied>,
    /// Before a document closes, each in turn; the document is still open.
    pub document_closing: Vec<DocumentClosing>,
    /// When a document is added; any `true` gives it a fresh id.
    pub doc_id_reserved: Vec<DocIdReserved>,
    /// After an edit that changed pixels (or left the whole document to redraw), each in turn.
    pub pixels_edited: Vec<PixelsEdited>,
}

impl Hooks {
    /// The params a recording keeps for command `id`: its own entry in
    /// [`Self::recorded_steps`], else [`Self::recorded_step`].
    pub(crate) fn recorded(&self, s: &Session, id: &str, params: Value, pre_selected: Option<&[LayerId]>) -> Value {
        let own = self.recorded_steps.iter().find(|(cmd, _)| *cmd == id).and_then(|(_, f)| f(s, id, &params, pre_selected));
        match (own, self.recorded_step) {
            (Some(p), _) => p,
            (None, Some(f)) => f(s, id, params, pre_selected),
            (None, None) => params,
        }
    }
}
