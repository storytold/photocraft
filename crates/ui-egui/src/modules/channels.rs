//! Channels: the colour and alpha channels.

use super::{Module, Size};

pub const MODULE: Module = Module {
    id: "channels",
    title: "Channels",
    icon: "contrast",
    menu_ids: &["window.panel.channels"],
    siblings: &["layers", "paths"],
    size: Size::new(320.0, 200.0, 140.0),
    fills: false,
    body: crate::panels::channels,
    menu: None,
};
