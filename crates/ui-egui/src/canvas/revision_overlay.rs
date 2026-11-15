//! Revision markers on the canvas: a red box over content that still must be changed, turning
//! green when `revision.detect` finds the framed content differs from the snapshot saved when the
//! marker was added. The box is drawn in document space like guides, so it pans, zooms and
//! rotates with the view, and follows a bound layer when that layer moves (see
//! `photocraft_engine`'s `move_layers`).

use egui::{Color32, FontId, Rect, Stroke, pos2, vec2};
use photocraft_doc::Document;

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::canvas::ViewXform;

/// Red: content still to be changed (or not yet detected).
pub const TODO: Color32 = Color32::from_rgb(235, 70, 70);
/// Green: detected changed since the snapshot.
pub const DONE: Color32 = Color32::from_rgb(60, 200, 90);

/// Draw every revision marker of the active document, clamped to the canvas. The marker the
/// Revision Marker tool has picked reads heavier, and a marker being dragged shows a ghost frame at
/// the place it would land.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    if !app.ui.view.shows(app.ui.view.show.revision_markers) || doc.revision_markers.is_empty() {
        return;
    }
    let tool_state = &app.revision_tool;
    let accent = Tokens::get(painter.ctx()).accent;
    // Preferences ▸ Workspace ▸ Revision marker opacity: how much of the frame the artwork
    // shows through. The chip's text keeps its colour so the name still reads.
    let opacity = app.session.prefs().workspace.revision_opacity();
    let canvas = doc.bounds();
    let clip = painter.clip_rect();
    for (i, m) in doc.revision_markers.iter().enumerate() {
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
        let color = faded(if m.done { DONE } else { TODO }, opacity);
        // The picked marker is the one the options bar edits and the one a drag would move.
        let picked = tool_state.selected.contains(&i);
        let width = if picked { 3.0 } else { 2.0 };
        // Dashed box reads as "annotation" rather than selection; solid enough to survive
        // screenshotting the manual.
        painter.extend(egui::Shape::dashed_line(&[a, pos2(b.x, a.y), b, pos2(a.x, b.y), a], Stroke::new(width, color), 6.0, 3.0));
        if picked {
            // The pick stays readable at any opacity, or the frame being edited is lost.
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, faded(accent, opacity.max(0.5))), egui::StrokeKind::Outside);
        }
        draw_label(painter, rect, &m.name, color, m.done, opacity);
    }
    // The frames a drag is moving: where each would land, drawn in the accent colour so the drop
    // reads before the release commits it. A set or a whole group moves as one, so every frame of it
    // gets its ghost.
    if let Some(preview) = &tool_state.preview {
        let ghost = faded(accent, opacity.max(0.5));
        for (index, pos, size) in &preview.targets {
            let _ = index;
            let a = xf.to_screen((pos[0] + preview.dx) as f32, (pos[1] + preview.dy) as f32);
            let b = xf.to_screen((pos[0] + preview.dx + size[0]) as f32, (pos[1] + preview.dy + size[1]) as f32);
            let rect = Rect::from_two_pos(a, b);
            if rect.intersects(clip) {
                painter.extend(egui::Shape::dashed_line(&[a, pos2(b.x, a.y), b, pos2(a.x, b.y), a], Stroke::new(2.0, ghost), 4.0, 3.0));
            }
        }
        // The smart guides the drag lined up with: one line each, spanning the frames and whatever
        // they snapped to, so it reads which edges are now on each other.
        for g in &preview.guides {
            let stroke = Stroke::new(1.0, faded(accent, opacity.max(0.7)));
            if g.vertical {
                let x = xf.to_screen(g.at as f32, 0.0).x;
                let y0 = xf.to_screen(0.0, g.from as f32).y;
                let y1 = xf.to_screen(0.0, g.to as f32).y;
                painter.extend(egui::Shape::dashed_line(&[pos2(x, y0), pos2(x, y1)], stroke, 4.0, 3.0));
            } else {
                let y = xf.to_screen(0.0, g.at as f32).y;
                let x0 = xf.to_screen(g.from as f32, 0.0).x;
                let x1 = xf.to_screen(g.to as f32, 0.0).x;
                painter.extend(egui::Shape::dashed_line(&[pos2(x0, y), pos2(x1, y)], stroke, 4.0, 3.0));
            }
        }
    }
}

/// `color` at `opacity` (its own alpha kept), for the frames drawn over the artwork.
fn faded(color: Color32, opacity: f32) -> Color32 {
    let a = (f32::from(color.a()) * opacity).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// The marker's name chip at the box's top-left corner (moved above the box when it would run
/// off the top of the screen).
fn draw_label(painter: &egui::Painter, rect: Rect, name: &str, color: Color32, done: bool, opacity: f32) {
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
    // The chip's plate follows the frames' opacity; its text and outline keep the marker's own
    // colour, so the name stays legible over the artwork it now lets through.
    painter.rect(bg, 2.0, Color32::from_black_alpha((180.0 * opacity).round() as u8), Stroke::new(1.0, color), egui::StrokeKind::Middle);
    painter.galley(at + pad, galley, color);
}
