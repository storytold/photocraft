//! What the framework Actions panel asks of PhotoCraft while it draws.

use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::{ActionsHost, ShortcutChoice};
use photocraft_engine::{Session, actions_cmds};
use serde_json::Value;

use super::{label_of, shortcut_id};

/// The panel's host over the session: step words, shortcuts and the recording state.
pub(crate) struct PhotoActions<'a> {
    session: &'a Session,
    /// A Step by Step or Pause For wait is running.
    paced: bool,
    recording: Option<usize>,
    /// The journal since the recording began, collected once per pass.
    pending: Vec<ActionStep>,
}

impl<'a> PhotoActions<'a> {
    /// The host for this pass; `paced` is whether a paced playback waits for its next step.
    pub(crate) fn new(session: &'a Session, paced: bool) -> Self {
        let recording = session.recording.target.map(|(_, action)| action);
        let pending = recording
            .map(|index| actions_cmds::pending_steps(session, index).map(|(id, params)| ActionStep::new(id.clone(), params.clone())).collect())
            .unwrap_or_default();
        Self { session, paced, recording, pending }
    }
}

impl ActionsHost for PhotoActions<'_> {
    fn step_label(&self, step: &ActionStep) -> String {
        let (id, params) = (step.command.as_str(), &step.params);
        if id == "actions.play" {
            let target = params.get("action").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
            format!("{}: {target}", label_of(id))
        } else if id == "layer.select" {
            let target = params
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| params.get("layer").and_then(Value::as_str))
                .map(str::to_owned)
                .or_else(|| params.get("layer").and_then(Value::as_u64).map(|id| format!("ID {id}")));
            target.map_or_else(|| label_of(id), |target| format!("{} ({target})", label_of(id)))
        } else if id == "actions.conditional" {
            // "If hasLayers: FS8-Layers / FS8-No-Layers"
            let cond = params.get("if").and_then(Value::as_str).unwrap_or("?");
            let branch = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("–").to_string();
            format!("{}: {cond} → {} / {}", label_of(id), branch("then"), branch("else"))
        } else if id == photocraft_engine::atn_cmds::MENU_ITEM {
            photocraft_engine::atn_cmds::menu_item_command(params).map_or_else(|_| label_of(id), label_of)
        } else if id == photocraft_engine::atn_cmds::STOP_MESSAGE {
            format!("{}: {}", label_of(id), params.get("message").and_then(Value::as_str).unwrap_or_default())
        } else if id == photocraft_engine::atn_cmds::PHOTOSHOP_STEP {
            // A step from a .atn file PhotoCraft can't play yet: its Photoshop name, marked.
            let name = params.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("?");
            crate::i18n::fmt(tl!("{name} (Photoshop)"), &[("name", name)])
        } else {
            label_of(id)
        }
    }

    fn shortcut(&self, action: &Action) -> Option<ShortcutChoice> {
        let text = self.session.prefs().shortcuts.get(&shortcut_id(&action.name))?;
        let (key, shift, command) = photocraft_engine::atn_cmds::parse_function_key(text)?;
        // The framework's chord names F1 to F12 only.
        u8::try_from(key).ok().filter(|k| *k <= 12).map(|key| ShortcutChoice::new(key, shift, command))
    }

    fn recording(&self) -> Option<usize> {
        self.recording
    }

    fn pending_steps(&self, action: usize) -> &[ActionStep] {
        if self.recording == Some(action) { &self.pending } else { &[] }
    }

    fn playing(&self) -> bool {
        self.session.recording.playing > 0 || self.paced
    }
}
