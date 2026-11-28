//! The PhotoCraft engine for apps: [`photocraft_engine_core`] and the engine crates built on it,
//! re-exported at the paths the engine has always had, with every engine command installed.
//!
//! Commands run through one registry per process, which a process installs before its first
//! command. [`Session::new`], [`install`], [`command_specs`] and [`commands::find`] install every
//! engine command first, so a frontend uses the engine as one crate.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ops::{Deref, DerefMut};

/// The session type the engine hands to callbacks (a job's `apply`, for example). [`Session`]
/// dereferences to it, so `&mut Session` passes wherever `&mut CoreSession` is asked for.
/// `CoreSession::new()` and `default()` install no commands: an app constructs [`Session`].
pub use photocraft_engine_core::Session as CoreSession;
pub use photocraft_engine_core::*;
// The painting commands' modules, at the paths they had inside the engine.
pub use photocraft_engine_paint::{
    adjustment_brush_cmds, brush_cmds, brush_key_cmds, eraser_cmds, face_cmds, fill_cmds, fill_key_cmds, paint_cmds, pick_cmds, redeye_cmds, retouch_cmds,
    sampler_cmds, stamp_cmds,
};
// The vector-area commands' modules, likewise.
pub use photocraft_engine_vector::{
    align_cmds, comps_cmds, layer_css_cmds, layer_xmp_cmds, notes_cmds, path_edit_cmds, rasterize_style_cmds, smart_svg_cmds, trap_cmds, type_area_cmds,
    type_caret_cmds, type_mask_cmds, type_styles_cmds,
};

mod install;

pub use install::install;

/// The engine's commands (`photocraft_engine_core::cmd`), with [`command_specs`](commands::command_specs)
/// and [`find`](commands::find) installing every engine command first.
pub mod commands {
    pub use crate::{command_specs, find};
    pub use photocraft_engine_core::cmd::*;
    pub use photocraft_engine_paint::cmd::{
        adjustment_brush_cmds, brush_cmds, brush_key_cmds, eraser_cmds, face_cmds, fill_cmds, fill_key_cmds, paint_cmds, pick_cmds, redeye_cmds, retouch_cmds,
        sampler_cmds, stamp_cmds,
    };
    pub use photocraft_engine_vector::cmd::{
        align_cmds, comps_cmds, layer_css_cmds, layer_xmp_cmds, notes_cmds, path_edit_cmds, rasterize_style_cmds, smart_svg_cmds, trap_cmds, type_area_cmds,
        type_caret_cmds, type_mask_cmds, type_styles_cmds,
    };
}
pub use commands as cmd;

/// Every engine command in registry order, installed first: for callers that read the registry
/// before any session exists (menus, docs, tests).
pub fn command_specs() -> &'static [CommandSpec] {
    // A conflicting install makes every read report the conflict.
    let _ = install();
    registry::command_specs()
}

/// The engine command `id`, installed first (see [`command_specs`]).
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let _ = install();
    registry::find(id)
}

/// A session with every engine command installed.
pub struct Session(CoreSession);

impl Session {
    /// A new session, after installing every engine command for the process.
    pub fn new() -> Self {
        // A conflicting install reports itself at the session's first command.
        let _ = install();
        Self(CoreSession::new())
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Session {
    type Target = CoreSession;

    fn deref(&self) -> &CoreSession {
        &self.0
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut CoreSession {
        &mut self.0
    }
}

/// A session with every engine command installed; the same as [`Session::new`].
pub fn session() -> Session {
    Session::new()
}
