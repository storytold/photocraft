//! Type tool: click to add point text, drag to add paragraph text, edit inline with a caret.
//!
//! Every edit is an engine `type.*` command carrying the session's `coalesce` key, so a whole
//! typing session is one "Edit Type" history step and automation sees exactly what the user does.
//! Offsets in [`TextEdit`] are character indices (the engine's unit); the layout works in bytes.

use std::sync::{Arc, PoisonError};

use photocraft_doc::{Document, LayerContent, LayerId, TextLayer};
use photocraft_geom::{Affine, Point};
use photocraft_text::TextLayout;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::TextEdit;

mod controls;
mod defaults;
mod editing;
mod fonts;
mod host;
mod insertion_style;
mod layout;
mod options_bar;
mod panel;
mod pointer;
mod values;
mod vertical_cursor;

pub(crate) use controls::canvas_controls;
#[cfg(test)]
pub(crate) use craft::text::ui::editor::CharToggle;
#[cfg(test)]
use defaults::apply;
#[cfg(test)]
use defaults::{default_style_params, default_styles, shown_scale};
pub use defaults::{foreground_changed, open_color_picker};
pub(crate) use defaults::{formatting_params, target, target_text};
pub(crate) use editing::commit_before_selecting;
pub use editing::{cancel, commit, commit_with, edit_active, edit_at, end_if_left};
#[cfg(test)]
pub(crate) use editing::{step_char_metric, toggle_char_style, toggle_hyphenate};
#[cfg(test)]
use fonts::preview_text;
pub use fonts::{families, style_label, styles, warm_fonts};
pub(crate) use fonts::{finish_font_state, font_state, selected_style};
pub(crate) use host::LayerHost;
pub use layout::layout;
pub(crate) use layout::{hit_layer, offset_at, shows_own_layout};
pub use options_bar::options_bar;
pub(crate) use options_bar::styles_at;
#[cfg(test)]
pub(crate) use panel::toggle_opentype;
pub use panel::{character_panel, type_properties};
#[cfg(test)]
pub(crate) use pointer::{PathClick, path_target};
pub(crate) use pointer::{TypeState, box_shape, drain, tool};
pub(crate) use values::doc_unit;

pub const PLACEHOLDER: &str = "Lorem Ipsum";

fn text_layer(doc: &Document, id: LayerId) -> Option<&TextLayer> {
    match &doc.layer(id)?.content {
        LayerContent::Text(t) => Some(t),
        _ => None,
    }
}

fn byte_of(text: &str, ci: usize) -> usize {
    photocraft_text::byte_index(text, ci)
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn session_key(app: &mut PhotocraftApp) -> String {
    format!("type-{}", app.ui.alloc_id())
}

fn active_doc_id(app: &PhotocraftApp) -> Option<u64> {
    app.session.active().map(|st| st.doc.id.0)
}

fn current_text(app: &PhotocraftApp, id: LayerId) -> Option<String> {
    Some(text_layer(&app.session.active()?.doc, id)?.text.clone())
}

/// The selected characters, clamped after undo/redo may have shortened the text.
pub(crate) fn selected_range(app: &PhotocraftApp) -> Option<[usize; 2]> {
    let ed = app.ui.text_edit.as_ref()?;
    let n = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b).then_some([a, b])
}

/// The byte range of characters `a..b` of `text`.
fn byte_range(text: &str, a: usize, b: usize) -> (usize, usize) {
    let at = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    (at(a.min(b)), at(a.max(b)))
}

#[cfg(test)]
mod vertical_align_buttons_tests;

#[cfg(test)]
mod vertical_cursor_tests;

#[cfg(test)]
mod canvas_tests;
#[cfg(test)]
mod ctrl_period_cancel_tests;

#[cfg(test)]
mod font_commit_tests;
#[cfg(test)]
mod font_picker_tests;
#[cfg(test)]
mod panels_toggle_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mask_replace_selection_tests;

#[cfg(test)]
mod commit_tests;
#[cfg(test)]
mod locked_layer_alert_tests;

#[cfg(test)]
mod ime_tests;
#[cfg(test)]
mod mask_escape_tests;

#[cfg(test)]
mod unchanged_edit_history_tests;
