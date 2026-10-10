//! Histogram: the tonal distribution of the image.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "histogram",
    title: "Histogram",
    icon: "chart-column",
    menu_ids: &["window.panel.histogram"],
    siblings: &["navigator", "info"],
    size: Size::new(210.0, 140.0, 80.0),
    fills: false,
    body: crate::tone::histogram_panel,
    menu: None,
};
