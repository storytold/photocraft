use crate::cmd::{CommandSpec, can_redo, can_undo, json};

/// Edit menu: Undo and Redo.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, |s, _| Ok(json!(s.undo()))),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, |s, _| Ok(json!(s.redo()))),
    ]
}
