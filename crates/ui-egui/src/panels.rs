//! Chrome around the canvas: title bar, options bar, toolbar, status bar, dock cards, Properties.

use craft::ui::patterns::blend_keys::{BlendKeyEntry, BlendShortcut};
#[cfg(test)]
use egui::{Rect, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::LayerId;
use photocraft_engine::history_source_cmds::HistorySource;
use serde_json::{Value, json};

use crate::state::Tool;
#[cfg(test)]
use crate::theme;
use crate::{PhotocraftApp, icons};

pub mod actions;
pub mod adjust_ui;
pub mod apply_color;
pub mod apply_data_set_ui;
pub(crate) mod blend_preview;
#[cfg(test)]
mod blend_rows_tests;
pub(super) mod brand;
pub mod brush_panel;
pub mod brush_preview;
pub mod brush_sections;
mod brush_settings;
mod brush_settings_events;
pub mod channels_panel;
pub mod chrome_ui;
pub mod clip_line_ui;
pub(crate) mod color;
pub mod color_hud;
pub mod color_panel_ui;
pub mod comps_ui;
pub mod doc_props_ui;
pub mod histogram_panel;
pub(crate) mod history;
pub mod history_menu_ui;
pub(crate) mod info;
pub mod layer_filter_ui;
pub mod layer_menu_ui;
pub mod layer_props_ui;
pub(super) mod layer_reveal;
pub mod layer_row_ui;
pub(super) mod layer_transfer;
mod layers;
pub(crate) mod layers_events;
pub mod layers_panel_ui;
pub(crate) mod layers_tree;
pub(super) mod mask_props_ui;
pub mod mask_thumbs_ui;
mod navigator;
pub(crate) mod options_bar;
pub(super) mod panel_docking;
pub(crate) mod preset_browser;
pub mod preset_panels;
mod properties;
pub(crate) mod region_versions;
pub(crate) mod registry;
pub mod smart_ui;
pub(crate) mod status_bar;
pub mod swatches_ui;
pub(super) mod timeline_ui;
pub(crate) mod title_bar;
pub(crate) mod tool_commands;
pub(crate) mod toolbar;
pub mod type_panels_ui;
pub(super) mod variables_ui;

#[cfg(test)]
pub(crate) use history::select_snapshot;
pub(crate) use history::selected_snapshot;
pub(crate) use info::{SamplerReadouts, sampler_readouts};
pub(crate) use layers::blend_options;
pub(crate) use layers_tree::listed_filter;
pub use options_bar::options_bar;
pub(crate) use status_bar::document_window_status;
pub use status_bar::status_bar;
pub use title_bar::{title_bar, window_title};
pub(crate) use toolbar::QUICK_MASK;
#[cfg(test)]
use toolbar::color_chips;
pub use toolbar::toolbar;

/// Photoshop's Shift+Alt letter position of blend mode `m`; `None` for the modes without a letter (Darker Color,
/// Lighter Color, Subtract, Divide) and for PhotoCraft's own extra modes.
pub(crate) fn blend_shortcut(m: BlendMode) -> Option<BlendShortcut> {
    Some(match m {
        BlendMode::Normal => BlendShortcut::Normal,
        BlendMode::Dissolve => BlendShortcut::Dissolve,
        BlendMode::PassThrough => BlendShortcut::PassThrough,
        BlendMode::Darken => BlendShortcut::Darken,
        BlendMode::Multiply => BlendShortcut::Multiply,
        BlendMode::ColorBurn => BlendShortcut::ColorBurn,
        BlendMode::LinearBurn => BlendShortcut::LinearBurn,
        BlendMode::Lighten => BlendShortcut::Lighten,
        BlendMode::Screen => BlendShortcut::Screen,
        BlendMode::ColorDodge => BlendShortcut::ColorDodge,
        BlendMode::LinearDodge => BlendShortcut::LinearDodge,
        BlendMode::Overlay => BlendShortcut::Overlay,
        BlendMode::SoftLight => BlendShortcut::SoftLight,
        BlendMode::HardLight => BlendShortcut::HardLight,
        BlendMode::VividLight => BlendShortcut::VividLight,
        BlendMode::LinearLight => BlendShortcut::LinearLight,
        BlendMode::PinLight => BlendShortcut::PinLight,
        BlendMode::HardMix => BlendShortcut::HardMix,
        BlendMode::Difference => BlendShortcut::Difference,
        BlendMode::Exclusion => BlendShortcut::Exclusion,
        BlendMode::Hue => BlendShortcut::Hue,
        BlendMode::Saturation => BlendShortcut::Saturation,
        BlendMode::Color => BlendShortcut::Color,
        BlendMode::Luminosity => BlendShortcut::Luminosity,
        _ => return None,
    })
}

/// The entry of blend mode `m` under the app's `key`, for every blend list the framework builds.
pub(crate) fn blend_entry<K>(key: K, m: BlendMode) -> BlendKeyEntry<K> {
    match blend_shortcut(m) {
        Some(s) => BlendKeyEntry::new(key, s),
        None => BlendKeyEntry::unlettered(key),
    }
}

#[cfg(test)]
mod color_tests;

#[cfg(test)]
mod fx_drag_tests;

#[cfg(test)]
mod unsupported_marker_tests;

#[cfg(test)]
mod swatch_type_tests {
    use super::*;
    use egui_kittest::Harness;

    /// The Color panel only acts on an edit. A colour's round trip through the panel's models isn't
    /// exact for every colour (#D8452E isn't), and writing it back every frame would recolour
    /// selected type the moment it was selected.
    #[test]
    fn idle_color_panel_leaves_the_foreground_and_selected_type_alone() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("tools.setColors", json!({"foreground": "#d8452e"})).unwrap();
        let fg = app.session.tools.foreground;
        app.ui.tool = Tool::Type;
        app.ui.text_edit = Some(crate::state::TextEdit { layer: id, document: None, caret: 0, anchor: 5, session: "s".into(), created: false, mask: false });
        let mut h = Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(vec2(300.0, 300.0))
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::panels::color::panel(app, ui), app);
        h.run_steps(4);
        assert_eq!(h.state().session.tools.foreground, fg);
        let st = h.state().session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        assert_eq!(t.char_runs().len(), 1, "still one white run");
    }
}

#[cfg(test)]
mod type_flyout_tests;

#[cfg(test)]
mod clipping_indicator_tests;

#[cfg(test)]
mod mask_properties_tests;

#[cfg(test)]
mod history_panel_select_tests;
