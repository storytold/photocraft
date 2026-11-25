//! The PhotoCraft engine's vector-area commands: alignment, layer comps, layer CSS and XMP, notes,
//! path editing, rasterized layer styles, smart-object SVG, traps and the type area, caret, mask
//! and style commands.
//!
//! Built on [`photocraft_engine_core`]; [`install`] installs core's commands and this crate's, and
//! [`session`] makes a session after it. Apps use `photocraft-engine`, which installs every engine crate.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// The core items this crate's code names as `crate::x`, so the code reads as it did inside core
// (photocraft#15710).
use photocraft_engine_core::{
    EngineError, Result, Session, active_layer_of, allocation, analysis_cmds, channel_cmds, commands, error, extra_cmds, file_cmds, layer_multi_cmds,
    layer_place, photo_cmds, smart_cmds, transform_cmds, type_cmds, type_extra_cmds, vector_cmds,
};
#[cfg(test)]
use photocraft_engine_core::{actions_cmds, command_specs, inspect};

pub mod cmd;
mod install;

pub use cmd::align_cmds;
pub use cmd::comps_cmds;
pub use cmd::layer_css_cmds;
pub use cmd::layer_xmp_cmds;
pub use cmd::notes_cmds;
pub use cmd::path_edit_cmds;
pub use cmd::rasterize_style_cmds;
pub use cmd::smart_svg_cmds;
pub use cmd::trap_cmds;
pub use cmd::type_area_cmds;
pub use cmd::type_caret_cmds;
pub use cmd::type_mask_cmds;
pub use cmd::type_styles_cmds;
pub use install::{command_tables, install, register, session};
