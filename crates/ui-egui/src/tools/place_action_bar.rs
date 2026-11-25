//! The Place Quick Actions contextual task bar: after a file is dropped / placed on the canvas
//! or whenever a placed/raster layer is active, a floating bar appears below the canvas with quick
//! action buttons matching Photoshop's Contextual Task Bar:
//!
//! 1. Remove background (text button);
//! 2. Select Subject (quick selection icon);
//! 3. Free Transform (vector-shape / quad with handles icon);
//! 4. Add image (image icon menu with "Add from device");
//! 5. the ••• menu (MoreMenu with "More Properties", separator, and Hide/Reset/Pin).
//!
//! Variants:
//! - For an empty pixel layer, "Add from device" shows as a text button.
//! - In Indexed Color, Remove background, Select Subject and Free Transform are greyed.
//! - With two or more layers or a group selected, no bar is shown.
//! - On the Background layer, Free Transform is greyed.

use craft::behavior::IconId;
use craft::behavior::kurbo::Point;
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas_controls::{TaskBar, TaskBarCx};
use craft::ui::contract::{Bind, Entry, PopupKind, Props as _, Show as _, Trigger};
use craft::ui::menu::{Menu, PanelRow};
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
/// - Pixel/raster layers (including the Background layer and empty pixel layers).
pub fn is_image_layer(st: &photocraft_engine::DocState, layer_id: photocraft_doc::LayerId) -> bool {
    let Some(layer) = st.doc.layer(layer_id) else { return false };
    matches!(
        layer.content,
        photocraft_doc::LayerContent::Smart(_) | photocraft_doc::LayerContent::Raster(_)
    )
}

/// All conditions for the bar to show: the toggle is on, no dialog, no transform box open,
/// no modal tool (Crop or Type) task bar active, exactly one active image layer is selected,
/// and it is not a group.
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

    // With two or more layers or a group selected, there should be no bar.
    if st.selected_layers().len() != 1 {
        return false;
    }

    let Some(active_id) = st.active_layer else { return false };
    let Some(layer) = st.doc.layer(active_id) else { return false };
    if matches!(layer.content, photocraft_doc::LayerContent::Group(_)) {
        return false;
    }

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

/// Draws the image Quick Actions contextual task bar below the canvas bottom edge.
/// Called from the active-document canvas overlay (`canvas::document_display`).
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, area: egui::Rect) {
    if !wanted(app) {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(active_id) = st.active_layer else { return };
    if st.doc.layer(active_id).is_none() {
        return;
    }

    // The reference centres the bar under the canvas's bottom edge for a pixel layer,
    // Background or smart object.
    let b = st.doc.bounds();
    let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    let corners = [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)];

    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = TaskBar::new(tl!("Contextual Task Bar"), &corners, Bind::bound(&mut place))
        .more_rows(vec![PanelRow::Command(CommandRef::new("window.panel.properties"))])
        .show(ui, area, |ui, cx| {
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
/// Left to right:
/// 1. Remove background (text button)
/// 2. Select Subject (icon)
/// 3. Free Transform (icon)
/// 4. Add image (icon menu whose entry Add from device runs file.placeEmbedded)
/// 5. The ••• menu (More Properties, separator, Hide/Reset/Pin)
///
/// For an empty pixel layer, "Add from device" shows as a text button.
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

    let Some(st) = app.session.active() else { return };
    let Some(active_id) = st.active_layer else { return };
    let Some(layer) = st.doc.layer(active_id) else { return };

    let is_empty_pixel = matches!(layer.content, photocraft_doc::LayerContent::Raster(_))
        && !st.doc.is_background(active_id)
        && bounds(&st.doc, layer).is_empty();

    if is_empty_pixel {
        // Variant: For an empty pixel layer, Add from device should show as a text button.
        if Button::new(tl!("Add from device"))
            .icon(IconSpec::svg(IconId::new("image")))
            .show(ui)
            .clicked()
        {
            set_active(app);
            if let Err(e) = crate::menus::invoke(app, ctx, "file.placeEmbedded", json!({})) {
                crate::notices::error(app, e);
            }
        }
    } else {
        // In Indexed Color, Remove background, Select Subject and Free Transform should be greyed.
        let not_indexed = st.doc.mode != photocraft_doc::ColorMode::Indexed;
        let can_cutout = not_indexed;
        // On Background, Free Transform is greyed.
        let can_transform = not_indexed && !st.doc.is_background(active_id);

        // 1. Remove background (text button)
        if Button::new(tl!("Remove background"))
            .icon(IconSpec::svg(IconId::new("scissors")))
            .enabled(can_cutout)
            .show(ui)
            .clicked()
        {
            set_active(app);
            if let Err(e) = app.run("layer.removeBackground", layer_param) {
                crate::notices::error(app, e);
            }
        }

        // 2. Select Subject icon
        if IconButton::new(IconId::new("quick-selection"))
            .label(tl!("Select Subject"))
            .tooltip(tl!("Select Subject"))
            .enabled(can_cutout)
            .show(ui)
            .clicked()
        {
            set_active(app);
            if let Err(e) = app.run("select.subject", json!({})) {
                crate::notices::error(app, e);
            }
        }

        // 3. Free Transform icon
        if IconButton::new(IconId::new("vector-shape"))
            .label(tl!("Free Transform"))
            .tooltip(tl!("Free Transform"))
            .enabled(can_transform)
            .show(ui)
            .clicked()
        {
            set_active(app);
            if let Err(e) = crate::menus::invoke(app, ctx, "edit.freeTransform", json!({})) {
                crate::notices::error(app, e);
            }
        }

        // 4. Add image icon menu whose entry Add from device runs file.placeEmbedded
        let add_btn = IconButton::new(IconId::new("image"))
            .label(tl!("Add image"))
            .tooltip(tl!("Add image"))
            .popup(PopupKind::Menu)
            .show(ui);

        let entries = [
            Entry::new("add_from_device", tl!("Add from device").to_string()),
        ];
        if let Some(picked) = Menu::new(tl!("Add image").to_string())
            .opened_by(&add_btn, Trigger::Click)
            .pick(ui, &entries)
            .picked
            && picked == "add_from_device"
        {
            set_active(app);
            if let Err(e) = crate::menus::invoke(app, ctx, "file.placeEmbedded", json!({})) {
                crate::notices::error(app, e);
            }
        }
    }

    // 5. The ••• menu
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
    fn eligible_true_on_new_sheet_background() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;
        assert!(eligible(&app), "new sheet with Background layer shows contextual task bar");
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

    #[test]
    fn eligible_false_with_multiple_layers_selected() {
        let mut app = app_with_doc();
        let bg = app.session.active().unwrap().active_layer.unwrap().0;
        let l1 = app.session.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        app.session.execute("layer.select", json!({"layer": bg})).unwrap();
        app.session.execute("layer.select", json!({"layer": l1, "mode": "add"})).unwrap();
        app.sync_views();
        assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
        assert!(!eligible(&app), "two or more selected layers must not show image quick actions");
    }

    #[test]
    fn eligible_false_with_group_selected() {
        let mut app = app_with_doc();
        app.session.execute("layer.new.group", json!({})).unwrap();
        app.sync_views();
        let group_id = app.session.active().unwrap().active_layer.unwrap();
        assert!(matches!(app.session.active().unwrap().doc.layer(group_id).unwrap().content, photocraft_doc::LayerContent::Group(_)));
        assert!(!eligible(&app), "group selection must not show image quick actions");
    }

    #[test]
    fn eligible_true_on_empty_pixel_layer() {
        let mut app = app_with_doc();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        assert!(eligible(&app), "empty pixel layer is eligible for task bar");
    }

    #[test]
    fn eligible_true_on_indexed_color() {
        let mut app = app_with_image();
        let d = app.session.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(d.doc_mut());
        doc.mode = photocraft_doc::ColorMode::Indexed;
        assert!(eligible(&app), "indexed color image layer is eligible for task bar");
    }
}
