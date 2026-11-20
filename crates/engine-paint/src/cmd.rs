//! The painting commands, one module per tool family, and their command-table fragments.

macro_rules! cmd {
    ($($spec:tt)*) => { photocraft_engine_core::command_spec!($($spec)*) };
}

pub mod brush_cmds;
pub mod brush_key_cmds;
pub mod eraser_cmds;
pub mod face_cmds;
pub mod fill_cmds;
pub mod fill_key_cmds;
#[cfg(test)]
mod gradient_alpha_method_tests;
pub mod paint_cmds;
pub mod pick_cmds;
pub mod redeye_cmds;
pub mod retouch_cmds;
pub mod sampler_cmds;
pub mod stamp_cmds;
pub mod table;
