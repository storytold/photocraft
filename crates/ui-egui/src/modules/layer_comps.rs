//! Layer Comps: saved visibility, position and style states of the layers.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "layerComps",
    title: "Layer Comps",
    icon: "copy",
    menu_ids: &["window.panel.layerComps"],
    siblings: &["history", "actions"],
    size: Size::new(200.0, 130.0, 80.0),
    fills: false,
    body: crate::comps_ui::panel,
    menu: None,
};
