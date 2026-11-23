//! This crate's command tables and hooks, and the install for a process that uses this crate
//! and core (this crate's own tests). `photocraft_engine::install()` installs them with every
//! other engine crate's.

use std::sync::OnceLock;

use photocraft_engine_core::registry::{self, Install, Table};

use crate::{Result, Session};

/// This crate's command tables with their names, in the app's registry order
/// (`crates/engine/src/install/command_ids.txt`).
const TABLES: &[Table] = &[
    ("paint::edit_menu", crate::cmd::table::paint::edit_menu::specs),
    ("paint::stroke", crate::cmd::table::paint::stroke::specs),
    ("brush_cmds", crate::brush_cmds::specs),
    ("eraser_cmds", crate::eraser_cmds::specs),
    ("retouch_cmds", crate::retouch_cmds::specs),
    ("adjustment_brush_cmds", crate::adjustment_brush_cmds::specs),
    ("redeye_cmds", crate::redeye_cmds::specs),
    ("paint_cmds", crate::paint_cmds::specs),
    ("fill_key_cmds", crate::fill_key_cmds::specs),
    ("face_cmds", crate::face_cmds::specs),
    ("sampler_cmds", crate::sampler_cmds::specs),
    ("brush_key_cmds", crate::brush_key_cmds::specs),
    ("stamp_cmds", crate::stamp_cmds::specs),
    ("pick_cmds", crate::pick_cmds::specs),
];

/// This crate's commands whose doc does not yet name every key they read (the engine's own tests
/// pass them more): not checked until their doc is completed. The list may only shrink.
const INCOMPLETE: &[&str] = &[
    "brush.mixer.sample",
    "edit.fill",
    "edit.fillForeground",
    "paint.artHistoryBrush",
    "paint.backgroundEraser",
    "paint.blur",
    "paint.bucket",
    "paint.burn",
    "paint.cloneStamp",
    "paint.colorReplacement",
    "paint.dodge",
    "paint.healingBrush",
    "paint.historyBrush",
    "paint.mixerBrush",
    "paint.patternStamp",
    "paint.pencil",
    "paint.redEye",
    "paint.sharpen",
    "paint.smudge",
    "paint.sponge",
    "paint.spotHealing",
    "paint.stroke",
];

/// This crate's command tables.
pub fn command_tables() -> &'static [Table] {
    TABLES
}

/// This crate's pipeline hooks. Each is the only hook of its stage, so the order the engine
/// crates' `register`s run in doesn't change what they do.
pub fn register(install: &mut Install) {
    install.incomplete.push(INCOMPLETE);
    let hooks = &mut install.hooks;
    hooks.coalesce_journal.push(crate::brush_cmds::coalesce_journal);
    // The Magic Eraser's alert for an Indexed Color or Bitmap document (#12032), the same error
    // its `run` gives: checked as the command is admitted too, so a floating selection stays
    // floating (#13169).
    hooks.admit.push(|s, id, _| match s.active().and_then(|st| crate::eraser_cmds::magic_eraser_refusal(&st.doc)) {
        Some(why) if id == "paint.magicEraser" => Err(crate::EngineError::Other(why.into())),
        _ => Ok(()),
    });
}

/// Install core's commands and this crate's for the process. Later calls return the first call's result.
pub fn install() -> Result<()> {
    static INSTALLED: OnceLock<Result<()>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let mut install = Install::default();
            install.commands.extend_from_slice(photocraft_engine_core::command_tables());
            install.commands.extend_from_slice(TABLES);
            photocraft_engine_core::register(&mut install);
            register(&mut install);
            registry::install(install)
        })
        .clone()
}

/// A session with core's commands and this crate's installed: what this crate's tests start from.
pub fn session() -> Session {
    // A conflicting install reports itself at the session's first command.
    let _ = install();
    Session::new()
}

#[cfg(test)]
mod tests {
    use photocraft_engine_core::param_schema;

    use super::{INCOMPLETE, install};

    #[test]
    fn incomplete_docs_are_registered_closed_docs() {
        install().unwrap();
        for id in INCOMPLETE {
            let spec = photocraft_engine_core::commands::find(id).unwrap_or_else(|| panic!("{id} is not registered"));
            assert!(param_schema::of(spec).is_some_and(|p| !p.open()), "{id}: open or unparsed docs are not checked anyway");
            assert!(photocraft_engine_core::is_incomplete(id), "{id} is on the installed list");
        }
        assert!(INCOMPLETE.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    }
}
