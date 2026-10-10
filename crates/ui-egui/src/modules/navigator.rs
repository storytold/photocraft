//! Navigator: a thumbnail of the document with the visible area, and zoom.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "navigator",
    title: "Navigator",
    icon: "navigation",
    menu_ids: &["window.panel.navigator"],
    siblings: &["histogram", "info"],
    size: Size::new(210.0, 140.0, 80.0),
    fills: false,
    body: crate::panels::navigator,
    menu: None,
};
