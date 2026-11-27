//! This crate's command tables and hooks, and the install for a process that uses this crate
//! and core (this crate's own tests). `photocraft_engine::install()` installs them with every
//! other engine crate's.

use std::sync::OnceLock;

use photocraft_engine_core::registry::{self, Install, Table};

use crate::{Result, Session};

/// This crate's command tables with their names, in the app's registry order
/// (`crates/engine/src/install/command_ids.txt`).
const TABLES: &[Table] = &[
    ("type_mask_cmds", crate::type_mask_cmds::specs),
    ("path_edit_cmds", crate::path_edit_cmds::specs),
    ("smart_svg_cmds", crate::smart_svg_cmds::specs),
    ("type_area_cmds", crate::type_area_cmds::specs),
    ("type_styles_cmds", crate::type_styles_cmds::specs),
    ("type_caret_cmds", crate::type_caret_cmds::specs),
    ("align_cmds", crate::align_cmds::specs),
    ("layer_css_cmds", crate::layer_css_cmds::specs),
    ("layer_xmp_cmds", crate::layer_xmp_cmds::specs),
    ("comps_cmds", crate::comps_cmds::specs),
    ("notes_cmds", crate::notes_cmds::specs),
    ("trap_cmds", crate::trap_cmds::specs),
    ("rasterize_style_cmds", crate::rasterize_style_cmds::specs),
];

/// This crate's commands whose doc does not yet name every key they read (the engine's own tests
/// pass them more): not checked until their doc is completed. The list may only shrink.
const INCOMPLETE: &[&str] = &["notes.delete"];

/// This crate's command tables.
pub fn command_tables() -> &'static [Table] {
    TABLES
}

/// This crate's pipeline hooks. Its recorded step is looked up by command id, so the order the
/// engine crates' `register`s run in doesn't change what it does.
pub fn register(install: &mut Install) {
    install.incomplete.push(INCOMPLETE);
    let hooks = &mut install.hooks;
    hooks.recorded_steps.push(("layerComp.apply", |s, _, p, _| Some(crate::comps_cmds::recorded_apply(s, p.clone()))));
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
