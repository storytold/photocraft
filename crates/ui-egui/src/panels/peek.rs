//! Auto-show hidden panels (#4096 rework): the pointer at the canvas edge brings back
//! panels Tab or Shift+Tab hid, as an overlay over the canvas.
//!
//! The dock peek is a second `DockArea` that renders only the dock column (no canvas)
//! inside a foreground `Area` anchored at the canvas rect's end edge. The canvas itself
//! never moves, so the image does not jump under the pointer. The hide state comes from
//! the framework shell: Tab hides everything (the shell is body-only) and Shift+Tab hides
//! the panels (`panels_hidden`); the frame's body closure records it for the peek, which
//! draws after the shell pass.
//!
//! Trigger and hold, per frame, at the frame's dock branch:
//! * Trigger: the preference is on, chrome is shown, no reveal is showing the panels, the
//!   pointer is in the 18-point band at the canvas edge, no button is down, and the point
//!   is not over a drawn scrollbar bar or the ruler band — except the outermost 3 points,
//!   which always trigger, so the edge itself is never dead.
//! * Hold: once active, the peek stays while the pointer is over the strip it drew
//!   (2-point tolerance), a drag that started in the strip is still down, a popup opened
//!   from it (the press that opened it went to the strip) is still open, the toolbar's tool
//!   flyout is open, or keyboard focus is on a widget it drew. A popup another surface
//!   opened (a canvas context menu) never holds it.
//! * Qualification: only panels Tab hid or the dock Shift+Tab hid peek back. A panel
//!   hidden any other way stays hidden.

use std::collections::HashSet;

use craft::behavior::dock::{DockEdge, Layout, Node};
use egui::{Align2, Id, Order, Pos2, Rect, Vec2};

use crate::PhotocraftApp;
use crate::panel_docking::{is_shown, show_column};
use crate::panels::registry::TOOLBAR;

/// What Auto-Show Hidden Panels is bringing back this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Peek {
    pub toolbar: bool,
    pub dock: bool,
}

/// How the shell hid the panels this frame: Tab hid everything, Shift+Tab hid the panels,
/// or a reveal is showing them. Recorded by the frame's body closure, read by
/// [`compute_peek`]; absent (a harness that draws no shell) means nothing is hidden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HideState {
    /// Tab hid everything: the shell is body-only and no reveal is showing the slots.
    pub tab: bool,
    /// Shift+Tab hid the panels, keeping the tools.
    pub shift: bool,
    /// A reveal (Tab or edge) is showing the slots: the framework already brings them back,
    /// so the peek stays out of the way.
    pub revealed: bool,
}

/// The temp key for the [`HideState`] the shell pass recorded.
fn hide_state_id() -> Id {
    Id::unique("panel-peek-hide-state")
}

/// Records the shell's hide state for this frame's peek.
pub(crate) fn record_hide_state(ctx: &egui::Context, hide: HideState) {
    ctx.data_mut(|d| d.insert_temp(hide_state_id(), hide));
}

/// The shell's hide state for this frame, or none hidden.
fn hide_state(ctx: &egui::Context) -> HideState {
    ctx.data(|d| d.get_temp::<HideState>(hide_state_id())).unwrap_or_default()
}

/// The second `DockArea`'s id: distinct from the main layout's, so its drag state never
/// collides with the main layout's.
pub(crate) fn peek_area_id() -> Id {
    Id::unique("photocraft-panel-docking-peek")
}

/// The strip the toolbar occupied when it last drew, for the edge hover below.
fn toolbar_strip_id() -> Id {
    Id::unique("toolbar-strip")
}

/// The strip the panel dock occupied when it last drew.
fn dock_strip_id() -> Id {
    Id::unique("dock-strip")
}

/// Widgets that held keyboard focus while the toolbar peek was active.
fn toolbar_focus_id() -> Id {
    Id::unique("toolbar-peek-focus")
}

/// Widgets that held keyboard focus while the dock peek was active.
fn dock_focus_id() -> Id {
    Id::unique("dock-peek-focus")
}

/// Whether the last press went to the toolbar peek: its strip, or a popup it opened.
fn toolbar_press_id() -> Id {
    Id::unique("toolbar-peek-press")
}

/// Whether the last press went to the dock peek: its strip, or a popup it opened.
fn dock_press_id() -> Id {
    Id::unique("dock-peek-press")
}

fn pressed_in_peek(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)
}

/// On a press, notes whether it went to the peek behind `strip_id`: its strip, or a popup
/// the peek opened (a press inside that popup, say on a submenu row, keeps it the peek's).
/// A popup opened by a press anywhere else, a canvas context menu say, is not the peek's,
/// so it never holds the peek open.
fn note_press(ctx: &egui::Context, strip_id: Id, press_id: Id, popup_open: bool) {
    let Some(origin) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None }) else {
        return;
    };
    let on_popup = ctx.layer_id_at(origin).is_some_and(|layer| matches!(layer.order, Order::Foreground | Order::Tooltip));
    let ours = over_strip(ctx, strip_id, origin) || (popup_open && on_popup && pressed_in_peek(ctx, press_id));
    ctx.data_mut(|d| d.insert_temp(press_id, ours));
}

fn clear_panel_strip(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|d| {
        d.insert_temp(id, None::<Rect>);
    });
}

/// Records the rect a peeked panel's overlay occupied, so the next frame's edge hover can
/// keep it revealed while the pointer is over the panel it just brought back.
fn record_panel_strip(ctx: &egui::Context, id: Id, rect: Rect) {
    ctx.data_mut(|d| {
        d.insert_temp(id, Some(rect));
    });
}

fn strip(ctx: &egui::Context, id: Id) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Option<Rect>>(id)).flatten()
}

fn note_focus(ctx: &egui::Context, strip_id: Id, focus_id: Id, p: Pos2, button_down: bool, press_origin: Option<Pos2>, focused: Option<Id>) {
    // Only focus gained through the peek itself joins the hold set: the pointer over its
    // strip, or a press that started in it. Focus gained elsewhere (a click on the canvas)
    // must never hold the peek open after the pointer leaves.
    let interacting =
        over_strip(ctx, strip_id, p) || (button_down && press_origin.is_some_and(|o| strip(ctx, strip_id).is_some_and(|r| r.expand(2.0).contains(o))));
    if interacting && let Some(f) = focused {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<HashSet<Id>>(focus_id).insert(f);
        });
    }
}

fn over_strip(ctx: &egui::Context, id: Id, p: Pos2) -> bool {
    strip(ctx, id).is_some_and(|r| r.expand(2.0).contains(p))
}

fn focus_set(ctx: &egui::Context, id: Id) -> HashSet<Id> {
    ctx.data(|d| d.get_temp::<HashSet<Id>>(id)).unwrap_or_default()
}

fn clear_focus_set(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|d| {
        d.insert_temp(id, HashSet::<Id>::new());
    });
}

/// The docked scrollbar's thickness in points: the framework's Docked look
/// (`tokens.size.handle.sm`, 12 points), the width of the bar at the canvas edge.
const BAR_THICKNESS: f32 = 12.0;

/// The ruler band's width in points: the framework Rulers strip thickness (the icon size,
/// 16 points), the band at the canvas's left edge while rulers show.
const RULER_WIDTH: f32 = 16.0;

/// The toolbar's width in points: the main dock's toolbar column is 60 pts wide, and the
/// rail fits in it. The peeked toolbar gets the same width so it stays a vertical rail.
const TOOLBAR_WIDTH: f32 = 60.0;

/// The narrowest a dock column draws, in points: the native dock's minimum, kept for the
/// peek's width when a column carries no width of its own.
const DOCK_MIN_WIDTH: f32 = 250.0;

/// How much of the window a dock may leave for the canvas, in points: the native dock's
/// rule, kept for the peek's width clamp.
const DOCK_LEAVES: f32 = 400.0;

/// The dock width range for a window `window_width` wide: at least [`DOCK_MIN_WIDTH`],
/// at most the window less [`DOCK_LEAVES`], and at least 520 points when the window allows.
fn dock_width_range(window_width: f32) -> std::ops::RangeInclusive<f32> {
    let max = (window_width - DOCK_LEAVES).max(520.0);
    DOCK_MIN_WIDTH..=max
}

/// The end-side dock column of `layout`, if any: a dock node is the root or follows only
/// same-side children of horizontal splits to its window edge.
fn end_dock_column(layout: &Layout<String>) -> Option<Node<String>> {
    fn find(node: &Node<String>) -> Option<Node<String>> {
        match node {
            Node::Dock { side, .. } if *side == DockEdge::End => Some(node.clone()),
            Node::Split { first, second, .. } => find(first).or_else(|| find(second)),
            _ => None,
        }
    }
    layout.root.as_ref().and_then(find)
}

/// The peek layout: only the end-side dock column, no canvas, no floating windows, plus
/// its width in points (the columns' widths, edge inwards). A layout with no dock column
/// at all gives nothing to peek. A column without a width of its own counts as the
/// minimum dock width.
pub(crate) fn peek_layout(app: &PhotocraftApp) -> Option<(Layout<String>, f32)> {
    let base = app.ui.docking.layout().clone();
    let dock = end_dock_column(&base)?;
    let width = match &dock {
        Node::Dock { columns, .. } => {
            columns.iter().map(|c| if c.iconic { c.iconic_width.or(c.width).unwrap_or(DOCK_MIN_WIDTH) } else { c.width.unwrap_or(DOCK_MIN_WIDTH) }).sum()
        }
        _ => return None,
    };
    Some((Layout::new(Some(dock), Vec::new()), width))
}

/// The edge trigger and hold state for this frame. Call after the dock branch drew, so
/// `last_canvas_rect` is this frame's. A strip recorded last frame is what keeps an active
/// peek alive: it only exists while the peek drew, so a stale strip can never reopen a
/// panel on its own. The hide state comes from the shell pass ([`record_hide_state`]).
pub(crate) fn compute_peek(app: &PhotocraftApp, ctx: &egui::Context, chrome: bool) -> Peek {
    let mut peek = Peek::default();
    let clear = |ctx: &egui::Context| {
        clear_panel_strip(ctx, toolbar_strip_id());
        clear_panel_strip(ctx, dock_strip_id());
        clear_focus_set(ctx, toolbar_focus_id());
        clear_focus_set(ctx, dock_focus_id());
        ctx.data_mut(|d| {
            d.remove::<bool>(toolbar_press_id());
            d.remove::<bool>(dock_press_id());
        });
    };
    let hide = hide_state(ctx);
    if !app.session.prefs().workspace.auto_show_hidden_panels || !chrome || hide.revealed {
        clear(ctx);
        return peek;
    }
    let Some(p) = ctx.input(|i| i.pointer.latest_pos()) else {
        clear(ctx);
        return peek;
    };
    let canvas = app.last_canvas_rect;
    if !canvas.is_positive() {
        clear(ctx);
        return peek;
    }
    let rtl = crate::direction::is_rtl();
    // A drag never starts a peek: the pointer crossing an edge mid-stroke must not move the
    // image under it. A peek the pointer is already inside stays alive, so a click on a
    // peeked panel does not dismiss it mid-click.
    let button_down = ctx.input(|i| i.pointer.any_down());
    let press_origin = ctx.input(|i| i.pointer.press_origin());
    // The canvas's bars always draw while a document exists (even when it fits), so the
    // vertical bar is the right-edge strip of the canvas, stopping above the horizontal
    // bar; the ruler band is the left-edge strip while rulers show.
    let v_bar = Rect::from_min_max(Pos2::new(canvas.right() - BAR_THICKNESS, canvas.top()), Pos2::new(canvas.right(), canvas.bottom() - BAR_THICKNESS));
    let over_vbar = v_bar.contains(p);
    let over_ruler = app.ui.extras.rulers && p.x < canvas.left() + RULER_WIDTH;
    // Only panels Tab hid come back on hover, plus the dock Shift+Tab hid (its own hide
    // source): a panel hidden any other way stays closed. The toolbar peeks only when it
    // is in the layout (a closed toolbar is not Tab-hidden); the dock peeks only when a
    // dock column exists to draw.
    // A popup the peek opened holds it: an egui popup, or a framework overlay popup
    // (a Select's listbox, a menu) — the framework's own layers, which egui's Popup
    // does not see. Without the second half a Select opened from the peek would
    // close with the peek the moment the pointer left the strip.
    let popup_open = egui::Popup::is_any_open(ctx) || craft::ui::contract::overlay::any_open(ctx, craft::ui::contract::overlay::OpenFilter::Keys);
    note_press(ctx, toolbar_strip_id(), toolbar_press_id(), popup_open);
    note_press(ctx, dock_strip_id(), dock_press_id(), popup_open);
    let focused = ctx.memory(|m| m.focused());
    // The hold needs an active peek behind it: the strip only exists while the peek drew.
    let hold = |ctx: &egui::Context, strip_id: Id, focus_id: Id, press_id: Id| {
        strip(ctx, strip_id).is_some()
            && (over_strip(ctx, strip_id, p)
                || (button_down && press_origin.is_some_and(|o| strip(ctx, strip_id).is_some_and(|r| r.expand(2.0).contains(o))))
                || (popup_open && pressed_in_peek(ctx, press_id))
                || focused.is_some_and(|f| focus_set(ctx, focus_id).contains(&f)))
    };
    const ZONE: f32 = 18.0;
    const EDGE_ALWAYS: f32 = 3.0;
    if is_shown(app, TOOLBAR) && hide.tab {
        let edge = if rtl { canvas.right() } else { canvas.left() };
        let blocked = if rtl { over_vbar } else { over_ruler };
        let dist = (p.x - edge).abs();
        let trigger = !button_down && dist <= ZONE && (dist <= EDGE_ALWAYS || !blocked);
        // The tool flyout is its own Area beside the toolbar: while it is open the toolbar
        // stays, and the flyout's own rule (any press outside it) closes it.
        let flyout = super::toolbar::tool_flyout_open(ctx) && strip(ctx, toolbar_strip_id()).is_some();
        peek.toolbar = trigger || flyout || hold(ctx, toolbar_strip_id(), toolbar_focus_id(), toolbar_press_id());
    }
    if hide.tab || hide.shift {
        let edge = if rtl { canvas.left() } else { canvas.right() };
        let blocked = if rtl { over_ruler } else { over_vbar };
        let dist = (p.x - edge).abs();
        let trigger = !button_down && dist <= ZONE && (dist <= EDGE_ALWAYS || !blocked);
        // A dock column must exist to draw: without one the peek is not active, so its
        // strip clears below instead of lingering.
        peek.dock = (trigger || hold(ctx, dock_strip_id(), dock_focus_id(), dock_press_id())) && peek_layout(app).is_some_and(|(_, width)| width > 0.0);
    }
    if peek.toolbar {
        note_focus(ctx, toolbar_strip_id(), toolbar_focus_id(), p, button_down, press_origin, focused);
    } else {
        clear_panel_strip(ctx, toolbar_strip_id());
        clear_focus_set(ctx, toolbar_focus_id());
        ctx.data_mut(|d| d.remove::<bool>(toolbar_press_id()));
        // A flyout the ended peek drew would come back with the next peek: forget it while
        // the toolbar is still hidden. When the panels are shown again the flyout belongs
        // to the real toolbar, so it stays.
        if hide.tab {
            super::toolbar::close_tool_flyout(ctx);
        }
    }
    if peek.dock {
        note_focus(ctx, dock_strip_id(), dock_focus_id(), p, button_down, press_origin, focused);
    } else {
        clear_panel_strip(ctx, dock_strip_id());
        clear_focus_set(ctx, dock_focus_id());
        ctx.data_mut(|d| d.remove::<bool>(dock_press_id()));
    }
    peek
}

/// Draws the peek overlays over the canvas. The dock column gets its own `DockArea` in a
/// foreground `Area` sized to the column width by the canvas height, anchored at the
/// canvas rect's end edge; the toolbar keeps the old content-sized strip, re-anchored at
/// the canvas rect's start edge. Call after the dock branch, so the canvas rect is this
/// frame's and the overlays land on top of it.
pub(crate) fn draw_peek(app: &mut PhotocraftApp, ctx: &egui::Context, peek: &Peek) {
    let canvas = app.last_canvas_rect;
    if !canvas.is_positive() {
        return;
    }
    let rtl = crate::direction::is_rtl();
    if peek.toolbar {
        let (pos, pivot) =
            if rtl { (Pos2::new(canvas.right(), canvas.top()), Align2::RIGHT_TOP) } else { (Pos2::new(canvas.left(), canvas.top()), Align2::LEFT_TOP) };
        let resp = egui::Area::new(Id::unique("toolbar-peek")).order(egui::Order::Foreground).fixed_pos(pos).pivot(pivot).show(ctx, |ui| {
            // The rail's natural height exceeds the canvas, and the Area grows to
            // fit it; a scroll area holds the rail to the canvas height instead,
            // the way the main dock's column does. The rail flows to fill the width
            // it is given, so it also gets the toolbar's column width to stay a
            // vertical rail.
            egui::ScrollArea::vertical().max_height(canvas.height()).show(ui, |ui| {
                ui.allocate_ui(Vec2::new(TOOLBAR_WIDTH, canvas.height()), |ui| {
                    super::toolbar(app, ui);
                });
            });
        });
        record_panel_strip(ctx, toolbar_strip_id(), resp.response.rect);
    }
    if peek.dock {
        let Some((layout, col_width)) = peek_layout(app) else {
            clear_panel_strip(ctx, dock_strip_id());
            return;
        };
        let range = dock_width_range(ctx.content_rect().width());
        let width = col_width.clamp(*range.start(), *range.end()).min(canvas.width());
        if width <= 0.0 {
            clear_panel_strip(ctx, dock_strip_id());
            return;
        }
        let (pos, pivot) =
            if rtl { (Pos2::new(canvas.left(), canvas.top()), Align2::LEFT_TOP) } else { (Pos2::new(canvas.right(), canvas.top()), Align2::RIGHT_TOP) };
        let resp = egui::Area::new(peek_area_id()).order(egui::Order::Foreground).fixed_pos(pos).pivot(pivot).show(ctx, |ui| {
            ui.allocate_ui(Vec2::new(width, canvas.height()), |ui| {
                show_column(app, ui, &layout);
            });
        });
        record_panel_strip(ctx, dock_strip_id(), resp.response.rect);
    }
}

#[cfg(test)]
mod tests;
