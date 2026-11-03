//! Shell capture ownership while an immutable action preview waits for step dialogs.
use std::sync::Arc;

use super::{Action, Recording, Result, Session, Value, bad};

#[derive(Debug)]
enum Destination {
    Classic(Action),
    Draft(u64),
}

/// Move-only ownership of a suspended recorder or an unpublished preview. Dropping a stale
/// dialog token cannot target a later recorder; the shell explicitly finishes every exit.
#[derive(Debug)]
pub struct Owner {
    serial: u64,
    take: u64,
    destination: Option<Destination>,
    preview: Option<(u64, Arc<Action>)>,
    call: Option<(String, usize)>,
}

fn classic_index(s: &Session, action: &Action) -> Option<usize> {
    let mut matches = s.actions.list.iter().enumerate().filter(|(_, candidate)| *candidate == action).map(|(index, _)| index);
    let index = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// Definition edits cannot invalidate a shell-owned capture, even through direct Session calls.
pub fn admit(s: &Session, id: &str) -> Result<()> {
    if s.actions.held_playback.is_some()
        && s.actions.playing == 0
        && id.starts_with("actions.")
        && !matches!(id, "actions.list" | "actions.get" | "actions.editor.get" | "actions.save")
    {
        return Err(bad(id, "confirm or cancel the open action step dialog first"));
    }
    Ok(())
}

fn suspend(s: &mut Session, preview: Option<(u64, Arc<Action>)>, call: Option<(String, usize)>) -> Result<Option<Owner>> {
    if s.actions.held_playback.is_some() {
        return Err(bad("actions.play", "another action owns a suspended playback"));
    }
    if preview.is_none() && !s.actions.recording_active() {
        return Ok(None);
    }
    // A job started in the old capture must land before its cursor is moved past preview work.
    // Jobs started by the paused step have no recorder and retain the normal foreign-take guard.
    if s.has_jobs() {
        return Err(bad("actions.play", "finish the running jobs before previewing or suspending capture"));
    }
    let serial = s.actions.playback_nonce.checked_add(1).ok_or_else(|| bad("actions.play", "playback identities exhausted"))?;
    super::flush_recording(s)?;
    let destination = match s.actions.take_recording() {
        Some(Recording::Classic(_, index)) => {
            // Valid Classic capture always names a current action. A missing destination must
            // fail before acquiring a lease rather than resume into an unrelated row.
            match s.actions.list.get(index).cloned() {
                Some(action) => Some(Destination::Classic(action)),
                None => {
                    s.actions.recording = Some((s.journal.len(), index));
                    return Err(bad("actions.play", "the recording destination no longer exists"));
                }
            }
        }
        Some(Recording::Draft { owner, .. }) => Some(Destination::Draft(owner)),
        None => None,
    };
    s.actions.playback_nonce = serial;
    s.actions.held_playback = Some(serial);
    Ok(Some(Owner { serial, take: s.actions.take, destination, preview, call }))
}

/// Resolve an ordinary named library call and suspend either recording destination once.
pub fn library(s: &mut Session, params: &Value) -> Result<(Arc<Action>, usize, Option<Owner>)> {
    admit(s, "actions.play")?;
    let (action, from) = super::playback_plan(s, params)?;
    let owner = suspend(s, None, Some((action.name.clone(), from)))?;
    Ok((Arc::new(action), from, owner))
}

/// Snapshot an unpublished draft without inserting it into the library or committing it.
/// `from` is an original step index; disabled/dialog indices stay attached to that snapshot.
pub fn preview(s: &mut Session, draft: u64, from: usize) -> Result<(Arc<Action>, Owner)> {
    super::not_playing(s, "actions.editor.preview")?;
    let action = Arc::new(super::editor::preview_value(s, draft)?);
    if from > action.steps.len() {
        return Err(bad("actions.editor.preview", "preview step is outside the draft"));
    }
    let owner = suspend(s, Some((draft, action.clone())), None)?.ok_or_else(|| bad("actions.editor.preview", "preview ownership is unavailable"))?;
    Ok((action, owner))
}

impl Owner {
    pub fn is_preview(&self) -> bool {
        self.preview.is_some()
    }

    pub fn draft_id(&self) -> Option<u64> {
        self.preview.as_ref().map(|(id, _)| *id)
    }

    /// Check before executing a confirmed command, not only before its later continuation.
    pub fn validate(&self, s: &Session) -> Result<()> {
        if s.actions.held_playback != Some(self.serial) || s.actions.take != self.take || s.actions.recording_active() {
            return Err(bad("actions.play", "this playback no longer owns its capture"));
        }
        let destination_exists = match &self.destination {
            Some(Destination::Classic(action)) => classic_index(s, action).is_some(),
            Some(Destination::Draft(id)) => s.actions.editor.as_ref().is_some_and(|draft| draft.id == *id),
            None => true,
        };
        if !destination_exists {
            return Err(bad("actions.play", "the recording destination has changed or no longer exists"));
        }
        if let Some((id, snapshot)) = &self.preview
            && !super::editor::preview_matches(s, *id, snapshot)
        {
            return Err(bad("actions.editor.preview", "the preview draft has changed"));
        }
        Ok(())
    }

    /// Release exactly once at completion, cancel or failure. Preview never records a self-call.
    /// Normal document work remains; only capture ownership and its cursor are restored.
    pub fn finish(self, s: &mut Session) {
        if s.actions.held_playback != Some(self.serial) {
            return;
        }
        s.actions.held_playback = None;
        if s.actions.take != self.take || s.actions.recording_active() {
            return;
        }
        match self.destination {
            Some(Destination::Classic(action)) => {
                // A reordered identical owner is legitimate; a reused name/index is not.
                if let Some(index) = classic_index(s, &action) {
                    s.actions.recording = Some((s.journal.len(), index));
                }
            }
            Some(Destination::Draft(id)) => {
                s.actions.restore_capture(Some(Recording::Draft { from: s.journal.len(), owner: id, take: self.take }));
            }
            None => {}
        }
        if s.actions.recording_active()
            && let Some((action, from)) = self.call
        {
            super::journal_call(s, &action, from);
        }
    }
}
