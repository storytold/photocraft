//! Integration tests of `photocraft-engine`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod affinity;
mod background_layer_consistency;
mod brush_smoothing;
mod camera_raw_noise_clamp;
mod close_document;
mod command_lookup;
mod current_affinity_artboards;
mod duplicate_layer_comps;
mod empty_selection_combine;
mod eraser_locked;
mod fill_lock_transparency;
mod filter_locked;
mod flatten_background;
mod gallery_colours;
mod gauss_small_sigma;
mod guide_layout_limit;
mod hdr_gamma_direction;
mod jpeg_exif_resolution;
mod keyboard_shortcut_bulk;
mod kys_import;
mod layer_delete_last;
mod panic_hunt;
mod path_blur_speed_clamp;
mod pdn;
mod photoshop_oracles;
mod pixel_lock_edits;
mod prefs_usage;
mod rasterize_selected_layers;
mod rendering_preferences;
mod selection_layer_commands;
mod smart_duplicates;
mod smart_psd;
mod svg_smart_object;
mod type_format_targets;
mod undo_keeps_active_layer;
mod unicode_colors;
mod wand_transparent_copy;
