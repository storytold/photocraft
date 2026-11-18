//! The crate's integration tests, linked into one test binary.

mod actions_duplicate;
mod actions_panel_menu;
mod adjust_preview_gpu;
mod arbitrary_rotation_preview;
mod batch_export_options;
mod brush_reset_defaults;
mod camera_raw_histogram;
mod camera_raw_scope;
mod canvas_16f;
mod checker_canvas;
mod clipboard_svg;
mod color_managed_canvas;
mod crop_ctrl_straighten;
mod crop_defaults;
mod crop_options_bar_chrome;
mod crop_task_bar;
mod display_shadows;
mod drag_preview_canvas;
mod eraser_reset_defaults;
mod full_res_preview;
mod gpu_device_loss;
mod healing_reset_tip;
// The interactive bench's `#[cfg(test)]` reporting tests: CI runs `--tests`, never examples.
// Embedding the example leaves its other entrypoints unused in this binary.
#[path = "../../examples/interactive_bench.rs"]
#[allow(dead_code)]
mod interactive_bench_reporting;
mod layout_perf;
mod live_stroke_canvas;
mod magnetic_lasso_right_click;
mod paint_bucket_all_layers;
mod pattern_preview_canvas;
mod radial_blur_center;
mod remove_channel_target;
mod system_locale;
mod tone_targets;
mod ui_scale_canvas;
mod variables_data_sets;
mod workspace_presets;
