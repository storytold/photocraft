//! Paths: work paths, saved paths and vector masks.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "paths",
    title: "Paths",
    icon: "pen-tool",
    menu_ids: &["window.panel.paths"],
    siblings: &["layers", "channels"],
    size: Size::new(320.0, 200.0, 140.0),
    fills: false,
    body: crate::vector_ui::paths_panel,
    menu: None,
};
