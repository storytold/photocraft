//! Color: the foreground/background colour field (Pro) or picker.

use super::{Module, Size};
use crate::PhotocraftApp;

pub const MODULE: Module = Module {
    id: "color",
    title: "Color",
    icon: "palette",
    menu_ids: &["window.panel.color"],
    siblings: &["swatches", "gradients", "patterns"],
    size: Size::new(190.0, 130.0, 80.0),
    fills: false,
    body,
    menu: None,
};

fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if crate::theme::Tokens::get(ui.ctx()).pro {
        crate::panels::color_field(app, ui);
    } else {
        crate::panels::color_picker(app, ui);
    }
}
