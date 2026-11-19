//! The crate's integration tests, linked into one test binary.

mod actions_duplicate;
mod actions_panel_menu;
mod batch_export_options;
mod brush_reset_defaults;
mod camera_raw_histogram;
mod camera_raw_scope;
mod clipboard_svg;
mod crop_ctrl_straighten;
mod crop_defaults;
mod crop_options_bar_chrome;
mod crop_task_bar;
mod display_names;
mod eraser_reset_defaults;
mod flatpak_updates;
mod gpu_device_loss;
mod healing_reset_tip;
// The interactive bench's `#[cfg(test)]` reporting tests: CI runs `--tests`, never examples.
// Embedding the example leaves its other entrypoints unused in this binary.
#[path = "../../examples/interactive_bench.rs"]
#[allow(dead_code)]
mod interactive_bench_reporting;
mod layout_perf;
mod magnetic_lasso_right_click;
mod paint_bucket_all_layers;
mod pending_remove_pointer;
mod radial_blur_center;
mod save_memory_gate;
mod system_locale;
mod tone_targets;
mod variables_data_sets;
mod workspace_presets;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The simulated free memory is process-wide: tests that set it take turns.
pub(crate) static HEADROOM: Mutex<()> = Mutex::new(());

/// Takes one of the locks above; a failed test that held it does not fail the next.
pub(crate) fn lock(lock: &'static Mutex<()>) -> MutexGuard<'static, ()> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

// Tests that draw on a GPU device. nextest runs every test in a `gpu_tests` module in its `gpu`
// test group (.config/nextest.toml); the rest run on the CPU canvas.
mod gpu_tests {
    mod adjust_preview_gpu;
    mod arbitrary_rotation_preview;
    mod canvas_16f;
    mod checker_canvas;
    mod color_managed_canvas;
    mod display_shadows;
    mod drag_preview_canvas;
    mod full_res_preview;
    mod live_stroke_canvas;
    mod pattern_preview_canvas;
    mod ui_scale_canvas;
}
