//! The engine's full command registry: every command table, in registry order, and the install
//! that puts it in place for the process.

use std::sync::OnceLock;

use crate::Result;
use crate::registry::{self, CommandSpec, Install, Table};

/// Every command table with its name, in registry order. The order is part of the behaviour
/// (menus list commands in it) and `command_ids.txt` pins it. The `table` fragments are grouped by
/// the crate that will own them; a menu split between owners has one fragment per contiguous run.
const TABLES: &[Table] = &[
    ("store::file_menu", crate::cmd::table::store::file_menu::specs),
    ("core::file_menu", crate::cmd::table::core::file_menu::specs),
    ("core::edit_menu", crate::cmd::table::core::edit_menu::specs),
    ("paint::edit_menu", crate::cmd::table::paint::edit_menu::specs),
    ("model::edit_menu", crate::cmd::table::model::edit_menu::specs),
    ("core::select_menu", crate::cmd::table::core::select_menu::specs),
    ("model::layer_menu", crate::cmd::table::model::layer_menu::specs_before_gradient),
    ("store::layer_menu", crate::cmd::table::store::layer_menu::specs),
    ("model::layer_menu", crate::cmd::table::model::layer_menu::specs_after_gradient),
    ("model::image_menu", crate::cmd::table::model::image_menu::specs),
    ("image::image_menu", crate::cmd::table::image::image_menu::specs),
    ("paint::stroke", crate::cmd::table::paint::stroke::specs),
    ("core::tools", crate::cmd::table::core::tools::specs),
    ("core::queries", crate::cmd::table::core::queries::specs),
    ("image::adjustments", crate::cmd::table::image::adjustments::specs),
    ("layer_style", crate::layer_style::specs),
    ("contour_lib_cmds", crate::contour_lib_cmds::specs),
    ("layer_list_cmds", crate::layer_list_cmds::specs),
    ("filters", crate::filters::specs),
    ("filters_ext", crate::filters_ext::specs),
    ("color_to_alpha_cmds", crate::color_to_alpha_cmds::specs),
    ("gallery_cmds", crate::gallery_cmds::specs),
    ("gradient_fill_cmds", crate::gradient_fill_cmds::specs),
    ("solid_fill_cmds", crate::solid_fill_cmds::specs),
    ("pattern_fill_cmds", crate::pattern_fill_cmds::specs),
    ("type_cmds", crate::type_cmds::specs),
    ("type_mask_cmds", crate::type_mask_cmds::specs),
    ("transform_cmds", crate::transform_cmds::specs),
    ("float_cmds", crate::float_cmds::specs),
    ("vector_cmds", crate::vector_cmds::specs),
    ("path_edit_cmds", crate::path_edit_cmds::specs),
    ("smartselect_cmds", crate::smartselect_cmds::specs),
    ("cutout_cmds", crate::cutout_cmds::specs),
    ("symmetry_cmds", crate::symmetry_cmds::specs),
    ("edit_cmds", crate::edit_cmds::specs),
    ("smart_svg_cmds", crate::smart_svg_cmds::specs),
    ("color_cmds", crate::color_cmds::specs),
    ("brush_cmds", crate::brush_cmds::specs),
    ("brush_preset_cmds", crate::brush_preset_cmds::specs),
    ("eraser_cmds", crate::eraser_cmds::specs),
    ("preset_import_cmds", crate::preset_import_cmds::specs),
    ("style_asl_cmds", crate::style_asl_cmds::specs),
    ("atn_cmds", crate::atn_cmds::specs),
    ("actions_edit_cmds", crate::actions_edit_cmds::specs),
    ("swatch_cmds", crate::swatch_cmds::specs),
    ("retouch_cmds", crate::retouch_cmds::specs),
    ("adjustment_brush_cmds", crate::adjustment_brush_cmds::specs),
    ("redeye_cmds", crate::redeye_cmds::specs),
    ("image_cmds", crate::image_cmds::specs),
    ("perspective_crop_cmds", crate::perspective_crop_cmds::specs),
    ("selection_cmds", crate::selection_cmds::specs),
    ("magnetic_cmds", crate::magnetic_cmds::specs),
    ("select_extra_cmds", crate::select_extra_cmds::specs),
    ("paint_cmds", crate::paint_cmds::specs),
    ("extra_cmds", crate::extra_cmds::specs),
    ("background_cmds", crate::background_cmds::specs),
    ("file_cmds", crate::file_cmds::specs),
    ("exr_cmds", crate::exr_cmds::specs),
    ("type_extra_cmds", crate::type_extra_cmds::specs),
    ("type_area_cmds", crate::type_area_cmds::specs),
    ("type_styles_cmds", crate::type_styles_cmds::specs),
    ("type_spell_cmds", crate::type_spell_cmds::specs),
    ("type_caret_cmds", crate::type_caret_cmds::specs),
    ("smart_cmds", crate::smart_cmds::specs),
    ("layer_multi_cmds", crate::layer_multi_cmds::specs),
    ("layer_copy_cmds", crate::layer_copy_cmds::specs),
    ("prefs", crate::prefs::specs),
    ("edit_menu_cmds", crate::edit_menu_cmds::specs),
    ("fill_key_cmds", crate::fill_key_cmds::specs),
    ("sample_cmds", crate::sample_cmds::specs),
    ("face_cmds", crate::face_cmds::specs),
    ("sampler_cmds", crate::sampler_cmds::specs),
    ("brush_key_cmds", crate::brush_key_cmds::specs),
    ("stamp_cmds", crate::stamp_cmds::specs),
    ("align_cmds", crate::align_cmds::specs),
    ("photo_cmds", crate::photo_cmds::specs),
    ("lens_cmds", crate::lens_cmds::specs),
    ("vp_cmds", crate::vp_cmds::specs),
    ("channel_cmds", crate::channel_cmds::specs),
    ("adjust_cmds", crate::adjust_cmds::specs),
    ("layer_menu_cmds", crate::layer_menu_cmds::specs),
    ("layer_label_cmds", crate::layer_label_cmds::specs),
    ("layer_css_cmds", crate::layer_css_cmds::specs),
    ("layer_xmp_cmds", crate::layer_xmp_cmds::specs),
    ("mode_cmds", crate::mode_cmds::specs),
    ("multichannel_cmds", crate::multichannel_cmds::specs),
    ("pattern_cmds", crate::pattern_cmds::specs),
    ("warp_cmds", crate::warp_cmds::specs),
    ("comps_cmds", crate::comps_cmds::specs),
    ("artboard_cmds", crate::artboard_cmds::specs),
    ("distort_cmds", crate::distort_cmds::specs),
    ("analysis_cmds", crate::analysis_cmds::specs),
    ("notes_cmds", crate::notes_cmds::specs),
    ("font_library_cmds", crate::font_library_cmds::specs),
    ("history_cmds", crate::history_cmds::specs),
    ("history_options_cmds", crate::history_options_cmds::specs),
    ("history_menu_cmds", crate::history_menu_cmds::specs),
    ("history_source_cmds", crate::history_source_cmds::specs),
    ("history_select_cmds", crate::history_select_cmds::specs),
    ("proof_sim", crate::proof_sim::specs),
    ("presets", crate::presets::specs),
    ("document_preset_cmds", crate::document_preset_cmds::specs),
    ("export_preset_cmds", crate::export_preset_cmds::specs),
    ("render_cmds", crate::render_cmds::specs),
    ("slice_cmds", crate::slice_cmds::specs),
    ("web_cmds", crate::web_cmds::specs),
    ("automate_cmds", crate::automate_cmds::specs),
    ("print_cmds", crate::print_cmds::specs),
    ("installed_profiles", crate::installed_profiles::specs),
    ("pick_cmds", crate::pick_cmds::specs),
    ("layer_nav_cmds", crate::layer_nav_cmds::specs),
    ("frame_cmds", crate::frame_cmds::specs),
    ("migrate_cmds", crate::migrate_cmds::specs),
    ("trap_cmds", crate::trap_cmds::specs),
    ("timeline_cmds", crate::timeline_cmds::specs),
    ("video_cmds", crate::video_cmds::specs),
    ("jobs", crate::jobs::specs),
    ("wia_cmds", crate::wia_cmds::specs),
    ("rasterize_style_cmds", crate::rasterize_style_cmds::specs),
    ("variables_cmds", crate::variables_cmds::specs),
    ("plugin_cmds", crate::plugin_cmds::specs),
    ("group_view_cmds", crate::group_view_cmds::specs),
    ("fx_view_cmds", crate::fx_view_cmds::specs),
    ("fx_visibility_cmds", crate::fx_visibility_cmds::specs),
    ("mask_view_cmds", crate::mask_view_cmds::specs),
    ("layer_mask_props_cmds", crate::layer_mask_props_cmds::specs),
    ("actions_cmds", crate::actions_cmds::specs),
];

/// Install every engine command for this process. Later calls return the first call's result.
pub fn install() -> Result<()> {
    static INSTALLED: OnceLock<Result<()>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            let mut install = Install::default();
            install.commands.extend_from_slice(TABLES);
            registry::install(install)
        })
        .clone()
}

/// Every engine command in registry order, installed first: for callers that read the registry
/// before any session exists (menus, docs, tests).
pub fn command_specs() -> &'static [CommandSpec] {
    // A conflicting install leaves the registry empty, and every command reports the conflict.
    let _ = install();
    registry::command_specs()
}

/// The engine command `id`, installed first (see [`command_specs`]).
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let _ = install();
    registry::find(id)
}

#[cfg(test)]
mod order_tests;
