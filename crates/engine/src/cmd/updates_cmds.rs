//! Release-check state and commands. The desktop injects a non-blocking transport; the engine
//! owns the result so CLI, MCP and the GUI query the same state without network dependencies.

use std::sync::mpsc::{Receiver, TryRecvError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

pub const RELEASES: &str = "https://github.com/storytold/photocraft/releases";
pub type CheckResult = std::result::Result<Option<Release>, String>;
/// Starts background work and returns immediately. No document or personal data is sent.
pub type CheckFn = Box<dyn FnMut() -> std::result::Result<Receiver<CheckResult>, String> + Send>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        release: Release,
    },
    Failed {
        message: String,
    },
}

#[derive(Default)]
pub struct Updates {
    pub check: Option<CheckFn>,
    pub status: Status,
    pending: Option<Receiver<CheckResult>>,
}

impl Updates {
    /// Consume a completed check; a disconnected worker becomes an actionable error.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The update check stopped before returning a result".into()),
        };
        self.pending = None;
        self.status = match result {
            Ok(Some(release)) => Status::Available { release },
            Ok(None) => Status::UpToDate,
            Err(message) => Status::Failed { message },
        };
        true
    }
}

fn empty_params(p: &Value) -> Result<()> {
    if p.as_object().is_some_and(|p| p.is_empty()) { Ok(()) } else { Err(EngineError::Other("update commands take an empty object".into())) }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.updates.check.as_ref().map(|_| ()).ok_or_else(|| "Update checks are unavailable in this frontend".into())
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    empty_params(p)?;
    // Repeated automation requests must not spawn an unbounded number of network workers.
    if s.updates.pending.is_none() {
        let start = s.updates.check.as_mut().ok_or_else(|| EngineError::Other("Update checks are unavailable in this frontend".into()))?;
        match start() {
            Ok(rx) => {
                s.updates.pending = Some(rx);
                s.updates.status = Status::Checking;
            }
            Err(message) => {
                s.updates.status = Status::Failed { message: message.clone() };
                return Err(EngineError::Other(message));
            }
        }
    }
    Ok(json!(s.updates.status))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "help.checkForUpdates",
            label: "Check for Updates…",
            menu: &["Help"],
            shortcut: None,
            params: "{}",
            enabled,
            run: check,
            journal: false,
        },
        CommandSpec {
            id: "updates.status",
            label: "Update Check Status",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, p| {
                empty_params(p)?;
                s.updates.poll();
                Ok(json!({"supported": s.updates.check.is_some(), "currentVersion": crate::build_info::VERSION, "result": s.updates.status}))
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_nonblocking_deduplicated_and_queryable_without_a_document() {
        let mut s = Session::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut rx = Some(rx);
        s.updates.check = Some(Box::new(move || Ok(rx.take().expect("only one worker"))));
        for _ in 0..3 {
            assert_eq!(s.execute("help.checkForUpdates", json!({})).unwrap()["status"], "checking");
        }
        tx.send(Ok(Some(Release { version: "1.0.0".into(), url: format!("{RELEASES}/tag/v1.0.0") }))).unwrap();
        let status = s.execute("updates.status", json!({})).unwrap();
        assert_eq!(status["result"]["status"], "available");
        assert_eq!(status["result"]["release"]["version"], "1.0.0");
        assert!(s.journal.is_empty());
    }

    #[test]
    fn graceful_failures_and_preference_compatibility() {
        let mut s = Session::new();
        assert!(s.execute("help.checkForUpdates", json!({})).is_err());
        for p in [Value::Null, json!([]), json!({"url": "evil"}), json!("bad")] {
            assert!(s.execute("updates.status", p.clone()).is_err());
            assert!(check(&mut s, &p).is_err());
        }
        s.updates.check = Some(Box::new(|| Err("Could not start worker".into())));
        assert!(s.execute("help.checkForUpdates", json!({})).is_err());
        assert!(matches!(s.updates.status, Status::Failed { .. }));
        s.updates.check = Some(Box::new(|| {
            let (_tx, rx) = std::sync::mpsc::channel();
            Ok(rx)
        }));
        s.execute("help.checkForUpdates", json!({})).unwrap();
        assert_eq!(s.execute("updates.status", json!({})).unwrap()["result"]["status"], "failed");
        s.load_prefs_json("{}").unwrap();
        assert!(s.prefs().general.automatically_check_updates);
        s.execute("prefs.set", json!({"path": "general.automaticallyCheckUpdates", "value": false})).unwrap();
        let saved = s.prefs_to_json();
        let mut restored = Session::new();
        restored.load_prefs_json(&saved).unwrap();
        assert!(!restored.prefs().general.automatically_check_updates);
    }
}
