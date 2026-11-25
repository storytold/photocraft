//! The PhotoCraft engine's painting commands: brushes, the Mixer Brush, erasers, retouching tools,
//! fills, gradients, red eye, color samplers, stamps and the Move tool's layer picking.
//!
//! Built on [`photocraft_engine_core`]; [`install`] installs core's commands and this crate's, and
//! [`session`] makes a session after it. Apps use `photocraft-engine`, which installs every engine crate.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The core items this crate's code names as `crate::x`, so the code reads as it did inside core
// (photocraft#15710).
use photocraft_engine_core::{
    DocState, EngineError, Result, Session, active_layer_of, adjust_params, allocation, brush_defs, brush_preset_cmds, channel_cmds, color_channels,
    color_cmds, commands, content_sampling, extra_cmds, fix_selection, float_cmds, history_source_cmds, jobs, layer_multi_cmds, layer_place, pattern_cmds,
    pixel_fill, pixels_mut, preset_store, presets, sample_cmds, selection_cmds, symmetry_cmds, transform_cmds,
};
#[cfg(test)]
use photocraft_engine_core::{command_specs, hidden_target, param_schema, prefs};

pub mod cmd;
mod history_source;
mod install;

pub use cmd::adjustment_brush_cmds;
pub use cmd::brush_cmds;
pub use cmd::brush_key_cmds;
pub use cmd::eraser_cmds;
pub use cmd::face_cmds;
pub use cmd::fill_cmds;
pub use cmd::fill_key_cmds;
pub use cmd::paint_cmds;
pub use cmd::pick_cmds;
pub use cmd::redeye_cmds;
pub use cmd::retouch_cmds;
pub use cmd::sampler_cmds;
pub use cmd::stamp_cmds;
pub use install::{command_tables, install, register, session};

#[cfg(test)]
mod brush_clear_background_tests;
#[cfg(test)]
mod brush_pressure_size_default_tests;
#[cfg(test)]
mod eraser_gray_background_tests;
#[cfg(test)]
mod mixer_indexed_bitmap_tests;
#[cfg(test)]
mod mixer_toggle_commands_tests;
#[cfg(test)]
mod pencil_auto_erase_exact_tests;
