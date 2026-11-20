//! The Place Quick Actions contextual task bar: after a file is dropped / placed on the canvas
//! or whenever a placed/raster layer is active, a floating bar appears below the layer with quick
//! action buttons matching Photoshop's Contextual Task Bar:
//!
//! 1. the grip (widgets::task_bar);
//! 2. Select subject (quick selection);
//! 3. Remove background (scissors / mask cutout);
//! 4. Adjust colors (Hue/Saturation adjustment layer);
//! 5. Free Transform (vector-shape / quad with handles);
//! 6. Create new fill or adjustment layer (adjustment-layer popup menu);
//! 7. Place Embedded… / Add image (image);
//! 8. the ⋯ menu (widgets::task_bar::MoreMenu).

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

/// Whether `layer_id` is an image layer that should show the image Quick Actions contextual task bar.
///
/// Returns true for:
/// - Smart Objects (placed/embedded/linked image assets);
/// - Non-empty pixel/raster layers (unless it is the canvas background of a newly created sheet).
pub fn is_image_layer(st: &photocraft_engine::DocState, layer_id: photocraft_doc::LayerId) -> bool {
    let Some(layer) = st.doc.layer(layer_id) else { return false };
    match &layer.content {
        photocraft_doc::LayerContent::Smart(_) => true,
        photocraft_doc::LayerContent::Raster(_) => {
            // A brand new sheet (File › New): the initial Background layer is just the blank canvas,
            // not an image, so quick actions like Select Subject or Remove Background do not apply.
            let is_new_sheet = st.history.entries().first().is_some_and(|e| e == "New");
            if is_new_sheet && st.doc.is_background(layer_id) {
                return false;
            }
            // An empty raster layer (zero content pixels) is not an image.
            !bounds(&st.doc, layer).is_empty()
        }
        _ => false,
    }
}

/// All conditions for the bar to show: the toggle is on, no dialog, no transform box open,
/// no modal tool (Crop or Type) task bar active, and an active image layer is selected.
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

    let Some(active_id) = st.active_layer else { return false };

    if let Some(bar) = &app.place_action_bar
        && bar.doc == st.doc.id
        && bar.layer == active_id
        && st.doc.layer(bar.layer).is_some()
    {
        return true;
    }

    is_image_layer(st, active_id)
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
    let active_id = st.active_layer?;
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
    let target_layer = app.session.active().and_then(|s| s.active_layer);
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

    // 5. Create new fill or adjustment layer
    let adj = crate::icons::button_with_icon_size_tip(
        ui,
        "adjustment-layer",
        task_bar::BUTTON,
        14.0,
        false,
        tl!("Create new fill or adjustment layer"),
        false,
    );
    let mut actions: Vec<(&str, serde_json::Value)> = Vec::new();
    crate::press_menu::show(&adj, |ui, menu| {
        ui.set_min_width(190.0);
        for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")) {
            if menu.item(ui, tl!(c.label).trim_end_matches('…')) {
                actions.push((c.id, json!({})));
            }
        }
        ui.separator();
        if menu.item(ui, tl!("Solid Color…")) {
            actions.push(("layer.newFillLayer.solidColor", json!({})));
        }
        if menu.item(ui, tl!("Gradient…")) {
            actions.push(("layer.newFillLayer.gradient", json!({})));
        }
        if menu.item(ui, tl!("Pattern…")) {
            actions.push(("layer.newFillLayer.pattern", json!({})));
        }
    });
    for (cmd, params) in actions {
        set_active(app);
        if let Err(e) = app.run(cmd, params) {
            crate::notices::error(app, e);
        }
    }

    // 6. Place Embedded…
    if crate::icons::button_with_icon_size_tip(
        ui,
        "image",
        task_bar::BUTTON,
        14.0,
        false,
        tl!("Place Embedded…"),
        false,
    )
    .clicked()
    {
        set_active(app);
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, "file.placeEmbedded", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 7. More options (⋯: Hide bar, Reset bar position, Pin bar position)
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

    fn app_with_image() -> PhotocraftApp {
        let mut app = app_with_doc();
        app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn eligible_false_when_context_task_bar_off() {
        let mut app = app_with_image();
        app.ui.panels.context_task_bar = false;
        assert!(!eligible(&app));
    }

    #[test]
    fn eligible_false_on_new_empty_sheet() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        assert!(!eligible(&app), "new empty sheet without image must not show image quick actions");
    }

    #[test]
    fn eligible_true_with_image_layer() {
        let mut app = app_with_image();
        app.ui.panels.context_task_bar = true;
        assert!(eligible(&app));
    }

    #[test]
    fn eligible_false_while_transforming() {
        let mut app = app_with_image();
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

    #[test]
    fn free_transform_transitions_to_transform_task_bar() {
        let mut app = app_with_image();
        app.ui.panels.context_task_bar = true;
        assert!(eligible(&app));
        assert!(!crate::tools::transform_task_bar::eligible(&app));

        // Invoke Free Transform as the button does
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
        assert!(app.ui.transform.is_some());
        assert!(!eligible(&app));
        assert!(crate::tools::transform_task_bar::eligible(&app));

        // Commit Free Transform returns to place action bar
        crate::transform_tool::commit(&mut app);
        assert!(app.ui.transform.is_none());
        assert!(eligible(&app));
        assert!(!crate::tools::transform_task_bar::eligible(&app));
    }
}
