//! Character: type settings of the selected text.

use super::{Module, Size};
use crate::PhotocraftApp;

pub const MODULE: Module = Module {
    id: "character",
    title: "Character",
    icon: "type",
    menu_ids: &["window.panel.character", "type.panels.character"],
    siblings: &["paragraph"],
    size: Size::new(270.0, 160.0, 80.0),
    fills: false,
    body,
    menu: None,
};

fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::type_tool::character_panel(app, ui, false);
}
