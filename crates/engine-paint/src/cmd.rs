//! The painting commands, one module per tool family, and their command-table fragments.

macro_rules! cmd {
    ($($spec:tt)*) => { photocraft_engine_core::command_spec!($($spec)*) };
}

pub mod adjustment_brush_cmds;
#[cfg(test)]
mod adjustment_brush_cmds_tests;
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

/// A layer that can be painted: pixels, or any layer with a mask (paint with `"target":"mask"`).
pub(crate) fn has_paintable(s: &crate::Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, photocraft_doc::LayerContent::Raster(_)) || l.mask.is_some() {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer without a mask", l.content.article(), l.content.kind_name()))
    }
}
