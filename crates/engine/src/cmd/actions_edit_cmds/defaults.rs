//! PhotoCraft's original, document-local starter actions; no external assets or file effects.
use crate::actions_cmds::Action;
use serde_json::json;

pub(super) const NAME: &str = "PhotoCraft Basics";

/// Small useful examples using existing native adjustments and ordinary Undo.
/// Folder occurrence identity belongs to the Actions library, not these names.
pub(super) fn actions() -> Vec<Action> {
    [("Invert Colors", "image.adjustments.invert"), ("Desaturate", "image.adjustments.desaturate")]
        .into_iter()
        .map(|(name, command)| {
            let mut action = Action::new(name, vec![(command.into(), json!({}))]);
            action.set = Some(NAME.into());
            action
        })
        .collect()
}
