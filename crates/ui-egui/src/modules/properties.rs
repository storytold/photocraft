//! Properties: the active layer's (or its adjustment's, mask's, type's) settings.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "properties",
    title: "Properties",
    icon: "sliders-horizontal",
    menu_ids: &["window.panel.properties"],
    siblings: &["adjustments"],
    size: Size::new(250.0, 160.0, 80.0),
    fills: false,
    body: crate::panels::properties_body,
    menu: None,
};
