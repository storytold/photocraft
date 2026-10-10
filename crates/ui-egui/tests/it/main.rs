//! Integration tests of `photocraft-ui-egui`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod support;

mod adjust_preview_gpu;
mod arbitrary_rotation_preview;
mod camera_raw_histogram;
mod camera_raw_localization;
mod camera_raw_scope;
mod camera_raw_zoom;
mod canvas_16f;
mod checker_canvas;
mod color_managed_canvas;
mod crop_beyond_canvas;
mod drag_preview_canvas;
mod export_as_scaling;
mod full_res_preview;
mod gpu_device_loss;
mod layout_perf;
mod live_stroke_canvas;
mod paint_bucket_all_layers;
mod system_locale;
mod variables_data_sets;
mod workspace_presets;
