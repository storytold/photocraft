//! Revision markers on the canvas: a red box over content that still must be changed, turning
//! green when `revision.detect` finds the framed content differs from the snapshot saved when the
//! marker was added. The box is drawn in document space like guides, so it pans, zooms and
//! rotates with the view, and follows a bound layer when that layer moves (see
//! `photocraft_engine`'s `move_layers`).

use egui::{Color32, FontId, Rect, Stroke, pos2, vec2};
use photocraft_doc::Document;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// Red: content still to be changed (or not yet detected).
pub const TODO: Color32 = Color32::from_rgb(235, 70, 70);
/// Green: detected changed since the snapshot.
pub const DONE: Color32 = Color32::from_rgb(60, 200, 90);

/// Draw every revision marker of the active document, clamped to the canvas.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    if !app.ui.view.shows(app.ui.view.show.revision_markers) || doc.revision_markers.is_empty() {
        return;
    }
    let canvas = doc.bounds();
    let clip = painter.clip_rect();
    for m in &doc.revision_markers {
        // Clamp the box to the canvas in document space (a marker bound to a moved layer can
        // edge past it) and skip boxes outside the viewport.
        let x0 = m.pos[0].clamp(f64::from(canvas.x0), f64::from(canvas.x1));
        let y0 = m.pos[1].clamp(f64::from(canvas.y0), f64::from(canvas.y1));
        let x1 = (m.pos[0] + m.size[0]).clamp(f64::from(canvas.x0), f64::from(canvas.x1));
        let y1 = (m.pos[1] + m.size[1]).clamp(f64::from(canvas.y0), f64::from(canvas.y1));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let a = xf.to_screen(x0 as f32, y0 as f32);
        let b = xf.to_screen(x1 as f32, y1 as f32);
        let rect = Rect::from_two_pos(a, b);
        if !rect.intersects(clip) || rect.width() < 1.0 || rect.height() < 1.0 {
            continue;
        }
        let color = if m.done { DONE } else { TODO };
        // Dashed box reads as "annotation" rather than selection; solid enough to survive
        // screenshotting the manual.
        painter.extend(egui::Shape::dashed_line(&[a, pos2(b.x, a.y), b, pos2(a.x, b.y), a], Stroke::new(2.0, color), 6.0, 3.0));
        draw_label(painter, rect, &m.name, color, m.done);
    }
}

/// The marker's name chip at the box's top-left corner (moved above the box when it would run
/// off the top of the screen).
fn draw_label(painter: &egui::Painter, rect: Rect, name: &str, color: Color32, done: bool) {
    let mark = if done { "✓ " } else { "◉ " };
    let text = format!("{mark}{name}");
    let font = FontId::proportional(11.0);
    let mut at = pos2(rect.left(), rect.top());
    if at.y < 14.0 {
        at = pos2(rect.left(), rect.bottom() + 2.0);
    }
    let galley = painter.layout_no_wrap(text, font, color);
    let pad = vec2(3.0, 2.0);
    let bg = Rect::from_min_size(at, galley.size() + pad * 2.0);
    painter.rect(bg, 2.0, Color32::from_black_alpha(180), Stroke::new(1.0, color), egui::StrokeKind::Middle);
    painter.galley(at + pad, galley, color);
}
