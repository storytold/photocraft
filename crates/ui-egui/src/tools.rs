pub(super) mod align_bar;
pub mod analysis;
pub mod artboard_tool;
pub mod bg_eraser_popup;
pub(crate) mod brush_cursor;
pub mod brush_feedback;
pub mod brush_picker;
pub mod brush_refusal;
pub mod brush_resize;
pub(super) mod cam_transform;
pub mod cas_transform;
pub mod crop_presets;
pub mod crop_task_bar;
pub mod crop_ui;
pub mod direct_select;
pub mod eraser_ui;
pub mod eyedropper_ui;
pub mod frame_tool;
pub mod gradient_ui;
pub mod magnetic_lasso_ui;
pub mod move_box;
pub mod move_distance;
pub mod move_hover;
pub mod move_lock;
pub mod move_mods;
pub mod object_finder;
pub mod option_cells;
pub(super) mod paint_picker;
pub mod path_components;
pub mod perspective_crop_ui;
pub mod perspective_ui;
pub mod puppet_ui;
pub mod retouch_ui;
pub mod shape_controls;
pub(super) mod shape_gradient_ui;
pub(super) mod shape_options;
pub(super) mod shape_paint_ui;
pub mod shape_stroke_ui;
pub mod slice_ui;
pub mod stroke_constraint;
pub mod stroke_trace;
pub mod stroke_trail;
pub mod symmetry_ui;
pub(super) mod tool_reset;
pub mod transform_fx;
pub mod transform_tool;
pub(super) mod type_box_preview;
pub(super) mod type_font_preview;
pub mod type_path_placement;
pub mod type_tool;
pub(super) mod type_transform;
pub mod vector_ui;

#[cfg(test)]
mod art_history_default_style_tests;
#[cfg(test)]
mod gradient_mode_greyed_tests;

#[cfg(test)]
mod count_show_extras_tests;
#[cfg(test)]
mod crop_mode_tests;
#[cfg(test)]
mod gradient_options_bar_mode_tests;
#[cfg(test)]
mod line_tool_defaults_tests;
#[cfg(test)]
mod object_selection_mode_tests;
#[cfg(test)]
mod path_selection_ctrl_direct_tests;
#[cfg(test)]
mod path_selection_shape_over_saved_path_tests;
