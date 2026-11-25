//! The Place Quick Actions contextual task bar: after a file is dropped / placed on the canvas
//! or whenever a placed/raster layer is active, a floating bar appears below the layer with quick
//! action buttons matching Photoshop's Contextual Task Bar:
//!
//! 1. Select subject (quick selection);
//! 2. Remove background (scissors / mask cutout);
//! 3. Adjust colors (Hue/Saturation adjustment layer);
//! 4. Free Transform (vector-shape / quad with handles);
//! 5. Place Embedded… / Add image (image);
//! 6. the ⋯ menu (MoreMenu).

use craft::behavior::IconId;
use craft::behavior::kurbo::Point;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas_controls::{TaskBar, TaskBarCx};
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::primitives::IconSpec;
use serde_json::json;

use crate::PhotocraftApp;

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
    if app.dialog_open() {
        return false;
    }
    if app.ui.transform.is_some() {
        return false;
    }
    if app.ui.tool == crate::state::Tool::Crop && app.canvas_tools.crop.tool.frame().is_some() {
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
    match &l.content {
        photocraft_doc::LayerContent::Group(_) | photocraft_doc::LayerContent::Adjustment(_) => photocraft_geom::Rect::EMPTY,
        photocraft_doc::LayerContent::Fill(_) => l.mask.as_ref().filter(|m| m.enabled).map_or(doc.bounds(), |m| m.surface.content_bounds()),
        _ => l.surface().map_or(photocraft_geom::Rect::EMPTY, |s| s.content_bounds()),
    }
}

// ── show ─────────────────────────────────────────────────────────────────────

/// Draws the image Quick Actions contextual task bar below the active image layer.
/// Called from the active-document canvas overlay (`canvas::document_display`).
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, area: egui::Rect) {
    if !wanted(app) {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(active_id) = st.active_layer else { return };
    let Some(layer) = st.doc.layer(active_id) else { return };

    let mut b = bounds(&st.doc, layer);
    if b.is_empty() {
        b = st.doc.bounds();
    }
    let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    let corners = [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];

    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = TaskBar::new(tl!("Contextual Task Bar"), &corners, Bind::bound(&mut place)).show(ui, area, |ui, cx| {
        contents(app, ui, cx, &ctx);
    });
    app.ui.panels.task_bar = place;

    if shown.output.hide {
        app.place_action_bar = None;
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = shown.output.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);
}


// ── contents ─────────────────────────────────────────────────────────────────

/// The controls inside the bar:
/// - Select subject
/// - Remove background
/// - Adjust colors
/// - Free Transform
/// - Place Embedded…
/// - More options (⋯)
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, cx: &mut TaskBarCx<'_>, ctx: &egui::Context) {
    let target_layer = app.session.active().and_then(|s| s.active_layer);
    let layer_param = if let Some(id) = target_layer {
        json!({"layer": id.0})
    } else {
        json!({})
    };

    let set_active = |app: &mut PhotocraftApp| {
        if let Some(id) = target_layer {
            app.session.set_active_layer(Some(id));
        }
    };

    // 1. Select subject
    if Button::new(tl!("Select subject"))
        .icon(IconSpec::svg(IconId::new("quick-selection")))
        .show(ui)
        .clicked()
    {
        set_active(app);
        if let Err(e) = app.run("select.subject", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 2. Remove background
    if Button::new(tl!("Remove background"))
        .icon(IconSpec::svg(IconId::new("scissors")))
        .show(ui)
        .clicked()
    {
        set_active(app);
        if let Err(e) = app.run("layer.removeBackground", layer_param) {
            crate::notices::error(app, e);
        }
    }

    // 3. Adjust colors
    if Button::new(tl!("Adjust colors"))
        .icon(IconSpec::svg(IconId::new("sliders-horizontal")))
        .show(ui)
        .clicked()
    {
        set_active(app);
        if let Err(e) = app.run("layer.newAdjustmentLayer.hueSaturation", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 4. Free Transform
    if IconButton::new(IconId::new("vector-shape"))
        .label(tl!("Free Transform"))
        .tooltip(tl!("Free Transform"))
        .show(ui)
        .clicked()
    {
        set_active(app);
        if let Err(e) = crate::menus::invoke(app, ctx, "edit.freeTransform", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 5. Place Embedded…
    if IconButton::new(IconId::new("image"))
        .label(tl!("Place Embedded…"))
        .tooltip(tl!("Place Embedded…"))
        .show(ui)
        .clicked()
    {
        set_active(app);
        if let Err(e) = crate::menus::invoke(app, ctx, "file.placeEmbedded", json!({})) {
            crate::notices::error(app, e);
        }
    }

    // 6. More options (⋯)
    cx.more_menu(ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_doc() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    fn app_with_image() -> Box<PhotocraftApp> {
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
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
        assert!(app.ui.transform.is_some());
        assert!(!eligible(&app));
    }

    #[test]
    fn free_transform_transitions_to_transform_task_bar() {
        let mut app = app_with_image();
        app.ui.panels.context_task_bar = true;
        assert!(eligible(&app));
        assert!(!crate::tools::transform_task_bar::eligible(&app));

        // Invoke Free Transform as the button does
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
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
