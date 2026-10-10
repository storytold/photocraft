//! Patterns: pattern presets.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "patterns",
    title: "Patterns",
    icon: "grid-2x2",
    menu_ids: &["window.panel.patterns"],
    siblings: &["color", "swatches", "gradients"],
    size: Size::new(190.0, 130.0, 80.0),
    fills: false,
    body: crate::preset_panels::patterns_panel,
    menu: None,
};
