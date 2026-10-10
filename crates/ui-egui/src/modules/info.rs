//! Info: colour under the pointer, position and selection size.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "info",
    title: "Info",
    icon: "info",
    menu_ids: &["window.panel.info"],
    siblings: &["navigator", "histogram"],
    size: Size::new(210.0, 140.0, 80.0),
    fills: false,
    body: crate::panels::info_panel,
    menu: None,
};
