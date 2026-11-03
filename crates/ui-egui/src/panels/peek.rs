//! Auto-show hidden panels (#4096 rework): the pointer at the canvas edge brings back
//! panels Tab or Shift+Tab hid, as an overlay over the canvas.
//!
//! The old implementation drew the native dock in an `egui::Area`; the new main has no
//! `right_dock` anymore, so the dock peek is a second `DockArea` that renders only the
//! dock column (no canvas) inside a foreground `Area` anchored at the canvas rect's end
//! edge. The canvas itself never moves, so the image does not jump under the pointer.
//!
//! Trigger and hold, per frame, at the frame's dock branch:
//! * Trigger: the preference is on, chrome is shown, the pointer is in the 18-point band
//!   at the canvas edge, no button is down, and the point is not over a drawn scrollbar
//!   bar or the ruler band — except the outermost 3 points, which always trigger, so the
//!   edge itself is never dead.
//! * Hold: once active, the peek stays while the pointer is over the strip it drew
//!   (2-point tolerance), a drag that started in the strip is still down, a popup opened
//!   while it was active is still open, or keyboard focus is on a widget it drew.
//! * Qualification: only panels Tab hid (`hidden_by_tab`) or the dock Shift+Tab hid
//!   (`dock_hidden_by_shift_tab`, a separate hide source) peek back. A panel hidden any
//!   other way stays hidden.

use std::collections::HashSet;

use craft_ui::docking::{Layout, Node, Side};
use egui::{Align2, Id, Pos2, Rect, Vec2};

use crate::PhotocraftApp;
use crate::canvas::rulers::RULER;
use crate::canvas::scrollbars::{bar_rects, last_drawn};
use crate::panel_docking::show_column;

/// What Auto-Show Hidden Panels is bringing back this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Peek {
    pub toolbar: bool,
    pub dock: bool,
}

/// The second `DockArea`'s id: distinct from the main layout's, so its hit rects and drag
/// state never collide with the main layout's.
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

fn note_focus(
    ctx: &egui::Context,
    strip_id: Id,
    focus_id: Id,
    p: Pos2,
    button_down: bool,
    press_origin: Option<Pos2>,
    focused: Option<Id>,
) {
    // Only focus gained through the peek itself joins the hold set: the pointer over its
    // strip, or a press that started in it. Focus gained elsewhere (a click on the canvas)
    // must never hold the peek open after the pointer leaves.
    let interacting =
        over_strip(ctx, strip_id, p) || (button_down && press_origin.is_some_and(|o| strip(ctx, strip_id).is_some_and(|r| r.expand(2.0).contains(o))));
    if interacting
        && let Some(f) = focused {
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

/// The end-side dock column of `layout`, if any: a dock node is the root or follows only
/// same-side children of horizontal splits to its window edge.
fn end_dock_column(layout: &Layout<String>) -> Option<Node<String>> {
    fn find(node: &Node<String>) -> Option<Node<String>> {
        match node {
            Node::Dock { side, .. } if *side == Side::End => Some(node.clone()),
            Node::Split { first, second, .. } => find(first).or_else(|| find(second)),
            _ => None,
        }
    }
    layout.root.as_ref().and_then(find)
}

/// The peek layout: only the end-side dock column, no canvas, no floating windows, plus
/// its width in points (the columns' widths, edge inwards). A custom workspace without an
/// end-side dock falls back to the native dock's column; a layout with no dock column at
/// all gives nothing to peek.
pub(crate) fn peek_layout(app: &PhotocraftApp) -> Option<(Layout<String>, f32)> {
    let base = match &app.ui.docking {
        Some(workspace) => workspace.layout.clone(),
        None => crate::panel_docking::legacy(app),
    };
    let dock = end_dock_column(&base).or_else(|| end_dock_column(&crate::panel_docking::legacy(app)))?;
    let width = match &dock {
        Node::Dock { columns, .. } => columns.iter().map(|c| if c.iconic { c.iconic_width.unwrap_or(c.width) } else { c.width }).sum(),
        _ => return None,
    };
    Some((Layout { root: Some(dock), floating: Vec::new() }, width))
}

/// The edge trigger and hold state for this frame. Call after the dock branch drew, so
/// `last_canvas_rect` and the scrollbar flags are this frame's. A strip recorded last
/// frame is what keeps an active peek alive: it only exists while the peek drew, so a
/// stale strip can never reopen a panel on its own.
pub(crate) fn compute_peek(app: &PhotocraftApp, ctx: &egui::Context, chrome: bool, tabbed_in: bool) -> Peek {
    let mut peek = Peek::default();
    let clear = |ctx: &egui::Context| {
        clear_panel_strip(ctx, toolbar_strip_id());
        clear_panel_strip(ctx, dock_strip_id());
        clear_focus_set(ctx, toolbar_focus_id());
        clear_focus_set(ctx, dock_focus_id());
    };
    if !app.session.prefs().workspace.auto_show_hidden_panels || !chrome || tabbed_in {
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
    // Only bars actually drawn this frame block the edge: the scrollbar flags come from the
    // canvas's own draw, never from assuming both bars are shown.
    let (h_drawn, v_drawn) = last_drawn(ctx);
    let (_, v_bar) = bar_rects(canvas, h_drawn, v_drawn);
    let over_vbar = v_bar.is_some_and(|r| r.contains(p));
    let over_ruler = app.ui.extras.rulers && p.x < canvas.left() + RULER;
    // Only panels Tab hid come back on hover, plus the dock Shift+Tab hid (its own hide
    // source): a panel hidden any other way stays closed, as in Photoshop.
    let tab_hidden = app.ui.panels.hidden_by_tab;
    let toolbar_tab_hidden = tab_hidden.is_some_and(|[toolbar, _, _]| toolbar);
    let dock_tab_hidden = tab_hidden.is_some_and(|[_, _, dock]| dock);
    let dock_shift_hidden = app.ui.panels.dock_hidden_by_shift_tab;
    let popup_open = ctx.any_popup_open();
    let focused = ctx.memory(|m| m.focused());
    // The hold needs an active peek behind it: the strip only exists while the peek drew.
    let hold = |ctx: &egui::Context, strip_id: Id, focus_id: Id| {
        strip(ctx, strip_id).is_some()
            && (over_strip(ctx, strip_id, p)
                || (button_down && press_origin.is_some_and(|o| strip(ctx, strip_id).is_some_and(|r| r.expand(2.0).contains(o))))
                || popup_open
                || focused.is_some_and(|f| focus_set(ctx, focus_id).contains(&f)))
    };
    const ZONE: f32 = 18.0;
    const EDGE_ALWAYS: f32 = 3.0;
    if !app.ui.panels.toolbar && toolbar_tab_hidden {
        let edge = if rtl { canvas.right() } else { canvas.left() };
        let blocked = if rtl { over_vbar } else { over_ruler };
        let dist = (p.x - edge).abs();
        let trigger = !button_down && dist <= ZONE && (dist <= EDGE_ALWAYS || !blocked);
        peek.toolbar = trigger || hold(ctx, toolbar_strip_id(), toolbar_focus_id());
    }
    if !app.ui.panels.dock && (dock_tab_hidden || dock_shift_hidden) {
        let edge = if rtl { canvas.left() } else { canvas.right() };
        let blocked = if rtl { over_ruler } else { over_vbar };
        let dist = (p.x - edge).abs();
        let trigger = !button_down && dist <= ZONE && (dist <= EDGE_ALWAYS || !blocked);
        // A dock column must exist to draw: without one the peek is not active, so its
        // strip clears below instead of lingering.
        peek.dock = (trigger || hold(ctx, dock_strip_id(), dock_focus_id()))
            && peek_layout(app).is_some_and(|(_, width)| width > 0.0);
    }
    if peek.toolbar {
        note_focus(ctx, toolbar_strip_id(), toolbar_focus_id(), p, button_down, press_origin, focused);
    } else {
        clear_panel_strip(ctx, toolbar_strip_id());
        clear_focus_set(ctx, toolbar_focus_id());
    }
    if peek.dock {
        note_focus(ctx, dock_strip_id(), dock_focus_id(), p, button_down, press_origin, focused);
    } else {
        clear_panel_strip(ctx, dock_strip_id());
        clear_focus_set(ctx, dock_focus_id());
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
        let (pos, pivot) = if rtl {
            (Pos2::new(canvas.right(), canvas.top()), Align2::RIGHT_TOP)
        } else {
            (Pos2::new(canvas.left(), canvas.top()), Align2::LEFT_TOP)
        };
        let resp = egui::Area::new(Id::unique("toolbar-peek"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .pivot(pivot)
            .show(ctx, |ui| {
                super::toolbar(app, ui);
            });
        record_panel_strip(ctx, toolbar_strip_id(), resp.response.rect);
    }
    if peek.dock {
        let Some((layout, col_width)) = peek_layout(app) else {
            clear_panel_strip(ctx, dock_strip_id());
            return;
        };
        let range = super::dock::dock_width_range(ctx.content_rect().width());
        let width = col_width.clamp(*range.start(), *range.end()).min(canvas.width());
        if width <= 0.0 {
            clear_panel_strip(ctx, dock_strip_id());
            return;
        }
        let (pos, pivot) = if rtl {
            (Pos2::new(canvas.left(), canvas.top()), Align2::LEFT_TOP)
        } else {
            (Pos2::new(canvas.right(), canvas.top()), Align2::RIGHT_TOP)
        };
        let resp = egui::Area::new(peek_area_id())
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .pivot(pivot)
            .show(ctx, |ui| {
                ui.allocate_ui(Vec2::new(width, canvas.height()), |ui| {
                    show_column(app, ui, &layout);
                });
            });
        record_panel_strip(ctx, dock_strip_id(), resp.response.rect);
    }
}

#[cfg(test)]
mod tests;
