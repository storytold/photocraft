//! Layers: the document's layer stack. It fills the column by default.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "layers",
    title: "Layers",
    icon: "layers",
    menu_ids: &["window.panel.layers"],
    siblings: &["channels", "paths"],
    size: Size { default: 320.0, compact: 200.0, min: 180.0, fill: 500.0 },
    fills: true,
    body: crate::panels::layers,
    menu: Some(crate::layer_row_ui::panel_menu),
};
