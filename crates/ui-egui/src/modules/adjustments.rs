//! Adjustments: one click adds an adjustment layer.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "adjustments",
    title: "Adjustments",
    icon: "adjustment-layer",
    menu_ids: &["window.panel.adjustments"],
    siblings: &["properties"],
    size: Size::new(250.0, 160.0, 80.0),
    fills: false,
    body: crate::panels::adjustments_grid,
    menu: None,
};
