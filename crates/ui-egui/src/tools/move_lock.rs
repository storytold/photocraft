//! A blocked Move drag shows "Layer is Locked" as a brief canvas overlay.
//! A plain click only selects the layer; locks and pixels are left untouched.

use crate::PhotocraftApp;
use crate::state::Tool;

pub const MESSAGE: &str = "Layer is Locked";

/// Would a press at document point `p` with `tool` in effect and `mods` held be a move that
/// fails because of a lock?
pub fn blocked(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> bool {
    let Some(st) = app.session.active() else { return false };
    let floating = photocraft_engine::float_cmds::floating(st).is_some();
    // The selected pixels: only a fresh cut can be refused.
    let pixels_locked =
        || !floating && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| photocraft_engine::float_cmds::locked_for_float(&st.doc, l));
    // The layers: any of them locked in place refuses the whole move.
    let layers_locked = || {
        photocraft_engine::layer_multi_cmds::move_targets(&st.doc, &st.selected_layers()).iter().any(|id| {
            let locks = st.doc.effective_locks(*id);
            locks.position || locks.all
        })
    };
    if tool == Tool::Move {
        // `moves_selected_pixels_with` is false for a layer whose pixels can't float: then the Move tool
        // moves the layer, which its position lock refuses too.
        if crate::move_ui::moves_selected_pixels_with(app, tool) {
            return pixels_locked();
        }
        return !floating && st.doc.selection.is_some() && pixels_locked() || layers_locked();
    }
    if crate::canvas::command_moves_layer(app, tool, p, mods) {
        return layers_locked();
    }
    crate::canvas::selection_drag_kind(app, tool, p, mods) == Some(true) && mods.command && pixels_locked()
}

/// A brief canvas overlay: locking must not silently turn Move into a marquee,
/// and dismissing a modal dialog should not be necessary to continue working.
pub fn prompt(app: &mut PhotocraftApp) {
    app.move_lock_until = Some(crate::gpu_canvas::now_ms() + 2000.0);
}

pub(crate) fn overlay(app: &mut PhotocraftApp, painter: &egui::Painter, canvas: egui::Rect) {
    let Some(until) = app.move_lock_until else { return };
    let left = until - crate::gpu_canvas::now_ms();
    if left <= 0.0 {
        app.move_lock_until = None;
        return;
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    let galley = painter.layout_no_wrap(tl!(MESSAGE).to_string(), egui::FontId::proportional(15.0), t.text);
    let size = galley.size() + egui::vec2(48.0, 22.0);
    let rect = egui::Rect::from_center_size(egui::pos2(canvas.center().x, canvas.top() + 34.0), size);
    painter.rect_filled(rect, t.radius_sm, t.card);
    painter.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    painter.galley(rect.min + egui::vec2(24.0, 11.0), galley, t.text);
    painter.ctx().request_repaint_after(std::time::Duration::from_millis(left.ceil() as u64));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;

    #[test]
    fn merged_locked_layer_warns_without_moving_or_selecting_pixels() {
        for auto in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("layer.mergeLayers", json!({})).unwrap();
            app.sync_views();
            app.ui.tool = Tool::Move;
            app.ui.tool_options.move_auto_select = auto;
            let before = app.session.active().unwrap().doc.clone();
            let steps = app.session.active().unwrap().history.past_len();
            for ev in
                [ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, ToolEvent::Move { x: 30.0, y: 25.0, pressure: 1.0 }, ToolEvent::Up { x: 30.0, y: 25.0 }]
            {
                tool_event(&mut app, ev, egui::Modifiers::NONE);
            }
            assert!(app.move_lock_until.is_some());
            assert!(app.layer_box.is_none() && app.drag.is_none());
            assert!(app.ui.dialogs.is_empty());
            assert_eq!(*app.session.active().unwrap().doc, *before);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        }
    }

    #[test]
    fn lock_overlay_expires_and_can_be_rendered_at_multiple_display_scales() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.move_lock_until = Some(crate::gpu_canvas::now_ms() - 1.0);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| overlay(&mut app, ui.painter(), ui.max_rect()));
        out.textures_delta.clear();
        assert!(app.move_lock_until.is_none());
        let Some(dir) = std::env::var_os("PHOTOCRAFT_BATCH_SNAPSHOTS") else { return };
        let _gpu = photocraft_testkit::gpu_lock();
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for scale in [1.0, 2.0] {
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
                app
            });
            h.run_steps(3);
            prompt(h.state_mut());
            h.run_steps(1);
            h.render().unwrap().save(dir.join(format!("layer-locked-{scale}x.png"))).unwrap();
        }
    }
}
