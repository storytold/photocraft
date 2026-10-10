//! Actions: recorded command sequences.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "actions",
    title: "Actions",
    icon: "play",
    menu_ids: &["window.panel.actions"],
    siblings: &["history", "layerComps"],
    size: Size::new(200.0, 130.0, 80.0),
    fills: false,
    body: crate::actions::panel,
    menu: None,
};
