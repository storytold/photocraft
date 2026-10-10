//! Paragraph: alignment, indents and spacing of the selected text.

use super::{Module, Size};
use crate::PhotocraftApp;

pub const MODULE: Module = Module {
    id: "paragraph",
    title: "Paragraph",
    icon: "align-left",
    menu_ids: &["window.panel.paragraph", "type.panels.paragraph"],
    siblings: &["character"],
    size: Size::new(270.0, 160.0, 80.0),
    fills: false,
    body,
    menu: None,
};

fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::type_tool::character_panel(app, ui, true);
}
