//! Every engine crate's commands, session slots and pipeline hooks, installed together for the app.

use std::sync::OnceLock;

use photocraft_engine_core::registry::{self, Install, Table};

use crate::Result;

/// The app's registry order: every engine command id, one per line. Menus, the Command Palette
/// and `command.list` list commands in it; a new command's id goes after its predecessor's.
const ORDER: &str = include_str!("install/command_ids.txt");

/// Every engine crate's command tables, core first, then the crates above it.
fn crate_tables() -> [&'static [Table]; 3] {
    [photocraft_engine_core::command_tables(), photocraft_engine_paint::command_tables(), photocraft_engine_vector::command_tables()]
}

/// Install every engine command for this process. Later calls return the first call's result.
pub fn install() -> Result<()> {
    static INSTALLED: OnceLock<Result<()>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let mut install = Install::default();
            install.order = Some(ORDER);
            for tables in crate_tables() {
                install.commands.extend_from_slice(tables);
            }
            // Each stage runs its hooks in this order; core's own list keeps today's after-command order.
            photocraft_engine_core::register(&mut install);
            photocraft_engine_paint::register(&mut install);
            photocraft_engine_vector::register(&mut install);
            registry::install(install)
        })
        .clone()
}

#[cfg(test)]
mod order_tests;
