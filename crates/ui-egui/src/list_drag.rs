//! Reorderable list rows vs touch scrolling.
//!
//! On a mouse, a press-drag reorders the row (Photoshop desktop). On touch or pen the same
//! gesture must scroll the list: a single finger drag is scrolling. Reorder is armed only by a
//! double-press that stays held (tap, then press-and-hold on the same row), after which a drag
//! moves the item. Used by layer/Actions rows and both preset browsers (Brushes plus Swatches,
//! Gradients, Patterns, Styles, Shapes) wherever cells take [`Sense::click_and_drag`].

use egui::{Context, Id, Pos2, Response, Sense};

use crate::stylus::PointerSource;

const GATE: &str = "pc-list-drag-gate";
// Finger taps are less precise than mouse clicks. These are egui points, so they
// follow UI/device scaling. Keep ordinary egui click/double-click settings intact.
const DOUBLE_PRESS_DELAY: f64 = 0.5;

fn tap_slop(source: PointerSource) -> f32 {
    if source == PointerSource::Touch { 18.0 } else { 12.0 }
}

#[derive(Clone, Debug)]
struct Tap {
    item: Id,
    source: PointerSource,
    time: f64,
    origin: Pos2,
    last_pos: Pos2,
    max_distance: f32,
}

#[derive(Clone, Debug, Default)]
struct Gate {
    /// Last completed click: item, time (seconds), position.
    last_click: Option<(Id, PointerSource, f64, Pos2)>,
    /// First press, tracked independently of egui's mouse-sized click tolerance.
    tap: Option<Tap>,
    /// Item whose current held press is the second half of a double-tap (drag allowed).
    armed: Option<Id>,
    /// Logical item and egui widget owning the drag, retained through its release frame.
    drag_widget: Option<(Id, Id)>,
}

/// Touch and pen should scroll list rows by default; mouse keeps click-and-drag reorder.
pub fn prefers_scroll(source: PointerSource) -> bool {
    matches!(source, PointerSource::Touch | PointerSource::Pen)
}

/// Sense for one reorderable list row this frame.
pub fn row_sense(ctx: &Context, source: PointerSource, item: Id) -> Sense {
    if !prefers_scroll(source) {
        return Sense::click_and_drag();
    }
    if is_armed(ctx, item) { Sense::click_and_drag() } else { Sense::click() }
}

/// Whether `item` is armed for reorder on the current held press.
pub fn is_armed(ctx: &Context, item: Id) -> bool {
    ctx.data(|d| d.get_temp::<Gate>(Id::new(GATE)))
        .is_some_and(|g| g.armed == Some(item) || g.drag_widget.is_some_and(|(logical, widget)| logical == item && ctx.drag_stopped_id() == Some(widget)))
}

/// Whether the current touch/pen drag passed the gate and may be received by another row.
pub fn drag_is_armed(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Gate>(Id::new(GATE))).is_some_and(|g| {
        g.armed.is_some() || g.drag_widget.is_some_and(|(_, widget)| ctx.dragged_id() == Some(widget) || ctx.drag_stopped_id() == Some(widget))
    })
}

/// Update the gate after the row's response is known. Use the returned response so
/// drag-start notifications from an ownership transfer are visible in this frame.
pub fn after_row(ctx: &Context, source: PointerSource, item: Id, resp: &Response) -> Response {
    if !prefers_scroll(source) {
        ctx.data_mut(|d| d.remove::<Gate>(Id::new(GATE)));
        return resp.clone();
    }
    let mut gate = ctx.data(|d| d.get_temp::<Gate>(Id::new(GATE))).unwrap_or_default();
    let (time, down, pressed, released, pos) =
        ctx.input(|i| (i.time, i.pointer.primary_down(), i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.interact_pos()));
    let max_tap_duration = ctx.options(|o| o.input_options.max_click_duration);
    let slop = tap_slop(source);

    if !down {
        gate.armed = None;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.multi_touch().is_some_and(|t| t.num_touches > 1)) {
        gate.armed = None;
        gate.drag_widget = None;
        gate.last_click = None;
        gate.tap = None;
    }

    if !down && !ctx.input(|i| i.pointer.primary_released()) {
        gate.drag_widget = None;
    }

    if let Some(tap) = &mut gate.tap
        && let Some(pos) = pos
    {
        tap.max_distance = tap.max_distance.max(pos.distance(tap.origin));
        tap.last_pos = pos;
    }
    if released && gate.tap.as_ref().is_some_and(|tap| tap.item == item) {
        if let Some(tap) = gate.tap.take() {
            let release_pos = pos.unwrap_or(tap.last_pos);
            if tap.source == source && time - tap.time <= max_tap_duration && tap.max_distance <= slop && resp.rect.contains(release_pos) {
                // Touch adapters emit PointerGone on lift, which can clear interact_pos
                // before the row sees the release. Keep the last contact position.
                gate.last_click = Some((item, source, time, release_pos));
            } else {
                gate.last_click = None;
            }
        } else {
            gate.last_click = None;
        }
    } else if resp.clicked() {
        // Also handle a complete tap delivered within one frame.
        let pos = resp.interact_pointer_pos().unwrap_or_else(|| resp.rect.center());
        gate.last_click = Some((item, source, time, pos));
    }

    // Match a completed tap and the next held press on the same item. Do not use
    // max_click_dist (6 mouse points), or small finger jitter becomes scrolling.
    if pressed && down && resp.is_pointer_button_down_on() {
        let pos = ctx.input(|i| i.pointer.press_origin()).unwrap_or_else(|| resp.rect.center());
        if gate.last_click.is_some_and(|(id, previous_source, t0, p0)| {
            id == item && previous_source == source && time - t0 <= DOUBLE_PRESS_DELAY && pos.distance(p0) <= slop * 2.0
        }) {
            gate.armed = Some(item);
            gate.tap = None;
        } else {
            gate.tap = Some(Tap { item, source, time, origin: pos, last_pos: pos, max_distance: 0.0 });
        }
        gate.last_click = None;
    }

    // egui chooses potential_drag_id on the press using LAST frame's widget senses.
    // Merely switching a click-only row to click-and-drag next frame leaves the
    // ScrollArea owning the gesture. Transfer ownership using the actual widget id
    // (allocated rows may have a different id from the logical item).
    let armed = gate.armed == Some(item) && down;
    if armed {
        gate.drag_widget = Some((item, resp.id));
    }
    ctx.data_mut(|d| d.insert_temp(Id::new(GATE), gate));
    if armed {
        // A stationary touch hold can trigger egui's long-touch context-menu path,
        // which clears drag ownership. Keep the armed gesture attached to this row.
        if ctx.dragged_id() != Some(resp.id) {
            ctx.set_dragged_id(resp.id);
            ctx.request_repaint();
        }
        let mut response = ctx.read_response(resp.id).unwrap_or_else(|| resp.clone());
        // Keep a normal double-tap clickable, but a held second press belongs to
        // dragging rather than the touch context menu.
        response.flags.remove(egui::response::Flags::LONG_TOUCHED);
        response
    } else {
        resp.clone()
    }
}

/// Whether a reorder / transfer drag may begin from this response.
pub fn may_begin_drag(ctx: &Context, source: PointerSource, item: Id, resp: &Response) -> bool {
    if !resp.drag_started() {
        return false;
    }
    if !prefers_scroll(source) {
        return true;
    }
    is_armed(ctx, item)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn mouse_keeps_click_and_drag_and_touch_prefers_scroll() {
        let ctx = Context::default();
        let item = Id::new("row-a");
        assert_eq!(row_sense(&ctx, PointerSource::Mouse, item), Sense::click_and_drag());
        assert!(!prefers_scroll(PointerSource::Mouse));
        assert!(prefers_scroll(PointerSource::Touch));
        assert!(prefers_scroll(PointerSource::Pen));
        assert_eq!(row_sense(&ctx, PointerSource::Touch, item), Sense::click());
    }

    #[test]
    fn armed_item_gets_click_and_drag_sense() {
        let ctx = Context::default();
        let item = Id::new("layer-1");
        let other = Id::new("layer-2");
        ctx.data_mut(|d| {
            d.insert_temp(
                Id::new(GATE),
                Gate { last_click: Some((item, PointerSource::Touch, 1.0, pos2(10.0, 10.0))), armed: Some(item), ..Default::default() },
            );
        });
        assert!(is_armed(&ctx, item));
        assert!(!is_armed(&ctx, other));
        assert_eq!(row_sense(&ctx, PointerSource::Touch, item), Sense::click_and_drag());
        assert_eq!(row_sense(&ctx, PointerSource::Touch, other), Sense::click());
        assert_eq!(row_sense(&ctx, PointerSource::Pen, item), Sense::click_and_drag());
    }

    #[test]
    fn may_begin_drag_requires_drag_started() {
        // Without a live Response.drag_started(), the gate alone is not enough — covered by
        // layer_row_ui mouse drag tests remaining green for PointerSource::Mouse.
        let ctx = Context::default();
        let item = Id::new("layer-3");
        assert!(!prefers_scroll(PointerSource::Mouse));
        assert!(prefers_scroll(PointerSource::Touch));
        assert_eq!(row_sense(&ctx, PointerSource::Mouse, item), Sense::click_and_drag());
    }

    #[test]
    fn preset_grid_cell_uses_same_gate_as_list_rows() {
        // Swatches (and Gradients/Patterns) share this gate for grid cells and list rows.
        let ctx = Context::default();
        let cell = Id::new(("swatches", 0usize, 3usize));
        assert_eq!(row_sense(&ctx, PointerSource::Touch, cell), Sense::click());
        assert_eq!(row_sense(&ctx, PointerSource::Pen, cell), Sense::click());
        assert_eq!(row_sense(&ctx, PointerSource::Mouse, cell), Sense::click_and_drag());
        ctx.data_mut(|d| {
            d.insert_temp(Id::new(GATE), Gate { last_click: Some((cell, PointerSource::Touch, 1.0, pos2(8.0, 8.0))), armed: Some(cell), ..Default::default() });
        });
        assert!(is_armed(&ctx, cell));
        assert_eq!(row_sense(&ctx, PointerSource::Touch, cell), Sense::click_and_drag());
        assert_eq!(row_sense(&ctx, PointerSource::Touch, Id::new(("swatches", 0usize, 4usize))), Sense::click());
    }
}
