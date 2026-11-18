//! Create Rectangle / Ellipse / Triangle / Polygon: a single click (no drag) with a Shape tool opens
//! the framework's `CreateShapeDialog` on the dialog host. Its OK draws the shape with the options
//! bar's fill and stroke, as a drag does. The dialog owns the fields, their validation and the
//! remembered values.

use craft::behavior::kurbo::Point;
use craft::draw::ui::shape_drag::{CreateShapeDialog, CreateShapeOptions, ShapeKind};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::form_dialogs::GuideUnits;

use crate::PhotocraftApp;

/// Opens the creation dialog of `kind` for a click at `at` (document px). `None` for a kind without
/// one (Line, Custom Shape) or a click that is not a point.
pub(crate) fn open(app: &mut PhotocraftApp, kind: ShapeKind, at: Point) -> Option<DialogId> {
    if !CreateShapeDialog::supports(kind) || !(at.x.is_finite() && at.y.is_finite()) {
        return None;
    }
    let ur = &app.session.prefs().units_and_rulers;
    let ruler = crate::sizing::pref_unit(ur.rulers);
    let points = ur.point_size.per_inch();
    let (dpi, size) = app
        .session
        .active()
        .map_or((72.0, [1.0, 1.0]), |d| (f64::from(d.doc.resolution_dpi.max(1.0)), [f64::from(d.doc.size.width), f64::from(d.doc.size.height)]));
    let o = &app.ui.tool_options;
    let radius = f64::from(o.corner_radius.max(0.0));
    let sides = o.polygon_sides.clamp(3, 100);
    let units = GuideUnits::new(ruler, size[0], size[1], dpi).points_per_inch(points);
    // The options bar has no star ratio: a polygon opens as a plain polygon.
    let options = CreateShapeOptions::initial(kind, at).units(units).tool(radius, sides, 1.0);
    Some(app.framework.dialogs.open(CreateShapeDialog::new(options).alert_title(tl!("PhotoCraft"))))
}

/// Whether a click at `at` (document px) with the Rectangle tool in Shape mode lies inside the active live
/// rectangle's outline: the click leaves the shape alone and opens no dialog (#13340). Rounded corners use
/// the outline, not the box.
pub(crate) fn click_inside_active_rectangle(app: &PhotocraftApp, kind: ShapeKind, at: [f64; 2]) -> bool {
    use photocraft_doc::{LayerContent, LiveShape};

    if kind != ShapeKind::Rectangle || !matches!(app.ui.tool_options.shape_mode, craft::draw::ui::VectorMode::Shape) {
        return false;
    }
    let Some(layer) = app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))) else { return false };
    let LayerContent::Shape(shape) = &layer.content else { return false };
    let Some(LiveShape::Rect { rect: [x, y, w, h], radii }) = shape.live else { return false };
    let (left, top) = (x.min(x + w), y.min(y + h));
    let (right, bottom) = (x.max(x + w), y.max(y + h));
    let [px, py] = at;
    if !((left..=right).contains(&px) && (top..=bottom).contains(&py)) {
        return false;
    }
    let (l, r, t, b) = (px - left, right - px, py - top, bottom - py);
    [(l, t), (r, t), (r, b), (l, b)]
        .into_iter()
        .zip(photocraft_vector::shapes::clamp_radii(w.abs(), h.abs(), radii))
        .all(|((dx, dy), radius)| dx >= radius || dy >= radius || (radius - dx).hypot(radius - dy) <= radius)
}

/// Opens the alert of a size typed past its range, once, then draws the shape of a Create Shape dialog that
/// finished OK this pass.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    if let Some(alert) = app.framework.dialogs.get_mut::<CreateShapeDialog>().and_then(CreateShapeDialog::take_range_alert) {
        let _ = super::alerts::show(app, alert);
    }
    let Some(DialogResult::Ok(shape)) = app.framework.dialogs.take::<CreateShapeDialog>() else { return };

    if let Err(e) = crate::vector_ui::create_geometry(app, &shape.geometry, shape.kind, crate::vector_ui::tool_history_name(shape.kind)) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests;
