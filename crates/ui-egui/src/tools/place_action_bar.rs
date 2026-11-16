//! The Place Quick Actions contextual task bar: after a file is dropped / placed on the canvas
//! or whenever a placed/raster layer is active, a floating bar appears below the layer with quick
//! action buttons matching Photoshop's Contextual Task Bar:
//!
//! 1. the grip (widgets::task_bar);
//! 2. Select subject (quick selection);
//! 3. Remove background (scissors / mask cutout);
//! 4. Adjust colors (Hue/Saturation adjustment layer);
//! 5. Free Transform (vector-shape / quad with handles);
//! 6. the ⋯ menu (widgets::task_bar::MoreMenu).

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::widgets::task_bar;

/// The placed layer that just had its Free Transform committed or is targeted by the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaceActionBar {
    /// The placed layer's id.
    pub layer: photocraft_doc::LayerId,
    /// The document the layer belongs to.
    pub doc: photocraft_doc::DocId,
}

// ── guards ──────────────────────────────────────────────────────────────────

/// All conditions for the bar to show: the toggle is on, no dialog, no transform box open,
/// no modal tool (Crop or Type) task bar active, and an active raster or smart-object layer.
fn wanted(app: &PhotocraftApp) -> bool {
    if !app.ui.panels.context_task_bar {
        return false;
    }
    if !app.ui.dialogs.is_empty() {
        return false;
    }
    if app.ui.transform.is_some() {
        return false;
    }
    if app.ui.tool == crate::state::Tool::Crop && app.ui.crop_rect.is_some() {
        return false;
    }
    if app.ui.tool.is_type() && app.ui.text_edit.is_some() {
        return false;
    }
    let Some(st) = app.session.active() else { return false };

    // Check if place_action_bar is set and still matches active doc
    if let Some(bar) = &app.place_action_bar
        && st.doc.id == bar.doc && st.doc.layer(bar.layer).is_some()
    {
        return true;
    }

    // Or if the active layer is a pixel or smart-object layer
    let Some(active_id) = st.active_layer else { return false };
    let Some(layer) = st.doc.layer(active_id) else { return false };
    matches!(layer.content, photocraft_doc::LayerContent::Raster(_) | photocraft_doc::LayerContent::Smart(_))
}

/// Whether the bar can show now — called from `ui.inspect` so a bar hidden by `ui.set` before
/// the next draw no longer reports last frame's rect.
pub fn eligible(app: &PhotocraftApp) -> bool {
    wanted(app)
}

// ── anchor ──────────────────────────────────────────────────────────────────

fn bounds(doc: &photocraft_doc::Document, l: &photocraft_doc::Layer) -> photocraft_geom::Rect {
    if let Some(a) = l.artboard() {
        return a.rect;
    }
    let content = |s| photocraft_compose::bounds::content_bounds(s);
    match &l.content {
        photocraft_doc::LayerContent::Group(_) | photocraft_doc::LayerContent::Adjustment(_) => photocraft_geom::Rect::EMPTY,
        photocraft_doc::LayerContent::Fill(_) => l.mask.as_ref().filter(|m| m.enabled).map_or(doc.bounds(), |m| content(&m.surface)),
        _ => l.surface().map_or(photocraft_geom::Rect::EMPTY, content),
    }
}

/// The screen-space bounding box to anchor the bar below: the layer's visible bounds
/// mapped through the view transform. Falls back to a horizontal strip centred in the canvas
/// when the layer has no content bounds (an empty or brand-new layer).
fn anchor(app: &PhotocraftApp, xf: &ViewXform) -> Option<egui::Rect> {
    let st = app.session.active()?;
    let active_id = app.place_action_bar.as_ref().filter(|b| b.doc == st.doc.id).map(|b| b.layer).or(st.active_layer)?;
    let layer = st.doc.layer(active_id)?;

    let doc_rect = bounds(&st.doc, layer);
    let doc_rect = if doc_rect.is_empty() { st.doc.bounds() } else { doc_rect };

    if doc_rect.is_empty() {
        let d = st.doc.bounds();
        let cx = (d.x0 + d.x1) as f32 / 2.0;
        let cy = (d.y0 + d.y1) as f32 / 2.0;
        let p = xf.to_screen(cx, cy);
        return Some(egui::Rect::from_center_size(p, egui::vec2(1.0, 1.0)));
    }

    Some(xf.doc_rect(doc_rect))
}

// ── draw ─────────────────────────────────────────────────────────────────────

/// Draws the bar under the placed or active layer's screen bounding box.
/// Called from the canvas overlay pass for the active document's tile
/// (canvas.rs, alongside crop_task_bar and type_tool::draw_task_bar).
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    if !wanted(app) {
        return;
    }

    let Some(anchor_rect) = anchor(app, xf) else { return };

    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = task_bar::show(&ctx, &mut place, anchor_rect, xf.rect, |ui, more| {
        contents(app, ui, more);
    });
    app.ui.panels.task_bar = place;

    let Some(shown) = shown else { return };
    if shown.hide {
        app.place_action_bar = None;
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = shown.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);
}

// ── contents ─────────────────────────────────────────────────────────────────

fn action_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut atoms = egui::Atoms::default();
    atoms.push_right(crate::icons::image(icon, 14.0, t.icon));
    atoms.push_right(label.to_owned());
    ui.add(egui::Button::new(atoms).min_size(egui::vec2(0.0, task_bar::BUTTON)))
}

/// The controls inside the bar:
/// - Select subject
/// - Remove background
/// - Adjust colors
/// - Free Transform
/// - More options (⋯)
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, more: &mut task_bar::MoreMenu) {
    let target_layer = app.place_action_bar.as_ref().map(|b| b.layer).or_else(|| app.session.active().and_then(|s| s.active_layer));
    let layer_param = if let Some(id) = target_layer {
        json!({"layer": id.0})
    } else {
        json!({})
    };

    let set_active = |app: &mut PhotocraftApp| {
        if let Some(id) = target_layer
            && let Some(st) = app.session.active_mut()
        {
            st.active_layer = Some(id);
        }
    };

    // 1. Select subject
    if action_button(ui, "quick-selection", tl!("Select subject")).clicked() {
        set_active(app);
        if let Err(e) = app.run("select.subject", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 2. Remove background
    if action_button(ui, "scissors", tl!("Remove background")).clicked() {
        set_active(app);
        if let Err(e) = app.run("layer.removeBackground", layer_param.clone()) {
            crate::notices::error(app, e);
        }
    }

    // 3. Adjust colors
    if action_button(ui, "sliders-horizontal", tl!("Adjust colors")).clicked() {
        set_active(app);
        if let Err(e) = app.run("layer.newAdjustmentLayer.hueSaturation", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 4. Free Transform
    if crate::icons::button_with_icon_size_tip(ui, "vector-shape", task_bar::BUTTON, 14.0, false, tl!("Free Transform"), false).clicked() {
        set_active(app);
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, "edit.freeTransform", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 5. More options (⋯: Hide bar, Reset bar position, Pin bar position)
    more.show(ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_doc() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn eligible_false_when_context_task_bar_off() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = false;
        assert!(!eligible(&app));
    }

    #[test]
    fn eligible_true_with_doc_and_raster_layer() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        assert!(eligible(&app));
    }

    #[test]
    fn eligible_false_while_transforming() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        app.ui.transform = Some(crate::state::TransformSession {
            session: 1,
            layer: 0,
            rect: [0.0, 0.0, 100.0, 100.0],
            quad: [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            pivot: [50.0, 50.0],
            interpolation: "nearest".into(),
            warp: None,
            selection: false,
            target: None,
            path: None,
            made: None,
            mode: Default::default(),
            content_aware: None,
            skew_v: 0.0,
            layers: Vec::new(),
        });
        assert!(!eligible(&app));
    }
}
