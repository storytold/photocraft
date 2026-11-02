//! Reports the stock toolbar boundary so panel chrome paints a single seam.
pub(crate) fn record_docked(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("tools-palette").with("docked-frame"), frame));
}
pub(crate) fn docked_this_frame(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(egui::Id::unique("tools-palette").with("docked-frame"))) == Some(ctx.cumulative_frame_nr())
}

#[cfg(test)]
mod tests {
    #[test]
    fn marker_can_be_recorded_in_a_ui_pass_and_expires_on_the_next_frame() {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            ctx.run_ui(Default::default(), |ui| {
                assert!(!super::docked_this_frame(ui.ctx()));
                super::record_docked(ui.ctx());
                assert!(super::docked_this_frame(ui.ctx()));
            })
            .textures_delta
            .clear();
        }
    }
}
