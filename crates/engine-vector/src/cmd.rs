//! The vector-area commands, one module per area.

pub mod align_cmds;
pub mod comps_cmds;
pub mod layer_css_cmds;
pub mod layer_xmp_cmds;
pub mod notes_cmds;
#[cfg(test)]
mod notes_stored_values_tests;
pub mod path_edit_cmds;
pub mod rasterize_style_cmds;
pub mod smart_svg_cmds;
pub mod trap_cmds;
pub mod type_area_cmds;
pub mod type_caret_cmds;
pub mod type_mask_cmds;
pub mod type_styles_cmds;
