//! The engine's commands and the helpers they share. Ids follow Photoshop's menu structure (see
//! the parity checklist).

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, inspect, pixels};

/// A journaled [`CommandSpec`] from its id, label, menu path, shortcut, params, `enabled` and
/// `run`: the form the engine crates' command tables use, as `cmd!`.
#[macro_export]
macro_rules! command_spec {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        $crate::registry::CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

macro_rules! cmd {
    ($($spec:tt)*) => { $crate::command_spec!($($spec)*) };
}

pub mod actions_cmds;
pub mod actions_edit_cmds;
pub mod adjust_cmds;
mod adjust_params;
pub mod analysis_cmds;
pub mod artboard_cmds;
pub mod atn_cmds;
pub mod automate_cmds;
pub mod background_cmds;
pub mod brush_preset_cmds;
pub mod channel_cmds;
pub mod color_cmds;
pub mod color_to_alpha_cmds;
pub mod contour_lib_cmds;
#[cfg(test)]
mod count_delete_group_tests;
#[cfg(test)]
mod count_history_names_tests;
#[cfg(test)]
mod count_record_label_tests;
#[cfg(test)]
mod count_resize_tests;
pub mod cutout_cmds;
pub mod distort_cmds;
pub mod document_preset_cmds;
pub mod edit_cmds;
pub mod edit_menu_cmds;
mod enabled;
pub mod export_preset_cmds;
pub mod exr_cmds;
pub mod extra_cmds;
pub mod file_cmds;
pub mod float_cmds;
pub mod font_library_cmds;
pub mod frame_cmds;
pub mod fx_view_cmds;
pub mod fx_visibility_cmds;
pub mod gallery_cmds;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod history_cmds;
pub mod history_menu_cmds;
pub mod history_options_cmds;
pub mod history_select_cmds;
pub mod history_source_cmds;
pub mod image_cmds;
pub mod layer_copy_cmds;
#[cfg(test)]
mod layer_duplicate_new_document_history_tests;
#[cfg(test)]
mod layer_from_background_name_tests;
pub mod layer_label_cmds;
pub mod layer_list_cmds;
pub mod layer_mask_props_cmds;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
pub mod layer_nav_cmds;
mod layer_params;
mod layer_references;
pub mod lens_cmds;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
pub mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
#[cfg(test)]
mod new_document_history_name_tests;
mod ops;
mod params;
pub mod pattern_cmds;
pub mod pattern_fill_cmds;
pub mod perspective_crop_cmds;
#[cfg(test)]
mod perspective_crop_refuse_tests;
pub mod photo_cmds;
pub mod plugin_cmds;
pub mod preset_import_cmds;
pub mod print_cmds;
pub mod render_cmds;
#[cfg(test)]
mod rotate_canvas_bitmap_tests;

#[cfg(test)]
mod rotate_canvas_guides_tests;
pub mod sample_cmds;

pub mod select_extra_cmds;
pub mod selection_cmds;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smartselect_cmds;
pub mod solid_fill_cmds;
pub mod style_asl_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
pub mod table;
pub mod timeline_cmds;
pub mod transform_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
#[cfg(test)]
mod type_indexed_mode_tests;
pub mod type_spell_cmds;
pub mod variables_cmds;
mod vector_clipboard;
pub mod vector_cmds;
pub mod video_cmds;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
pub mod wia_cmds;

pub(crate) use adjust_params::inject_kind;
use adjust_params::{adjust_mode, destructive_adjust, doc_mode, new_adjustment};
pub use adjust_params::{adjustment_from_params, adjustment_kind};
pub use enabled::always;
pub(crate) use enabled::check_all_unlocked;
pub use enabled::check_pixels_unlocked;
pub use enabled::has_pixel_or_channel;
pub use enabled::is_mask_target;
pub use enabled::paint_surface;
use enabled::{can_redo, can_undo, has_doc, has_layer, has_selection};
pub use layer_params::explicit_layer;
pub use layer_params::layer_param;
pub use layer_params::mask_at_depth;
pub use layer_params::selection_mask;
pub(crate) use layer_params::{NEW_LAYER_PATH, layer_listing, layer_panel_option, mask_new_layer};
pub(crate) use layer_references::{recorded_drop, recorded_layer};
pub use ops::translate_layer;
use ops::{arrange, merge_group, move_mask, set_mask, set_mask_and_deselect};
pub(crate) use ops::{layer_copy, translate_layer_wrapped};
pub use params::color_param;
pub use params::parse_hex;
pub use params::u32_id;
pub use params::u32_id_param;
use params::{bad, set_clipped};
pub(crate) use params::{check_f32_values, cmyk_param, int, int_i32};
pub use photocraft_color::blend::parse_blend;

pub use crate::registry::{CommandSpec, list_entry, list_entry_matches, search_keywords};
pub use crate::registry::{command_specs, find};

#[cfg(test)]
mod set_props_tests {
    use super::*;

    #[test]
    fn opacity_and_fill_outside_zero_to_one_are_refused_not_clamped() {
        // #5242: `55` (a percentage, as the Layer Style commands take) used to become 100 %.
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.55, "fill": 0.4})).unwrap();
        let layer = |s: &Session| {
            let d = s.active().unwrap();
            let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
            (l.opacity, l.fill_opacity)
        };
        assert_eq!(layer(&s), (0.55, 0.4));
        for bad in [json!({"opacity": 55}), json!({"opacity": -0.1}), json!({"fill": 1.5}), json!({"opacity": "50%"}), json!({"opacity": 0.5, "fill": 40})] {
            let e = s.execute("layer.setProps", bad.clone()).unwrap_err().to_string();
            assert!(e.contains("0..1"), "{bad}: {e}");
            assert_eq!(layer(&s), (0.55, 0.4), "{bad} left the layer alone");
        }
        s.execute("layer.setProps", json!({"opacity": 0, "fill": 1})).unwrap();
        assert_eq!(layer(&s), (0.0, 1.0));
    }
}

#[cfg(test)]
mod parse_hex_tests {
    use super::parse_hex;

    #[test]
    fn signs_and_other_non_hex_characters_are_rejected() {
        for bad in ["#+f+f+f", "+f+f+f+f", "#-f0000", "#gg0000", "#0f0f+f"] {
            assert_eq!(parse_hex(bad), None, "{bad:?} is not hex digits");
        }
        let g = 15.0 / 255.0;
        assert_eq!(parse_hex("#0f0f0f"), Some([g, g, g, 1.0]));
        assert_eq!(parse_hex("#0f0f0fff"), Some([g, g, g, 1.0]));
    }
}
