//! Right-to-left layout (Arabic, Hebrew, Persian, Urdu).
//!
//! The direction follows the UI language ([`crate::i18n::Lang::dir`]). In a right-to-left
//! language the UI mirrors, as in right-to-left macOS and Windows apps: the menu bar starts at
//! the right, the toolbox docks on the right, the dock and its rail on the left, and the title
//! and status bars read from the right. Dialogs, floating windows, docked panel interiors,
//! menus, popups and document tabs mirror too; the canvas itself stays left-to-right
//! (Photoshop-like), as does numeric text inside mirrored rows.
//!
//! Two mechanisms work together:
//! - the egui fork's `Style::right_to_left` flag ([`crate::theme::set_direction`], called from
//!   [`crate::theme::apply`] and on language changes from `i18n::sync_context`) mirrors every
//!   egui-owned widget: submenus and popups open reading-end first, vertical scrollbars sit on
//!   the left, collapsing-header triangles point left, checkbox/radio icons swap sides, and
//!   window title bars put the close button on the left; and
//! - these helpers mirror what egui does not: row and column order for explicit layouts, side
//!   panels, and hand-painted geometry. A nested `ui.horizontal` under a [`body_layout`] or
//!   [`row`] parent follows the row's direction on its own.
//!
//! Mirror each surface exactly once: the flag, a [`body_layout`] scope and an explicit
//! [`row`]/[`trailing`] pin each flip a row — never combine two on the same row. Pointer
//! semantics (scrub drags, slider value→x mapping) stay physical.
//!
//! `PHOTOCRAFT_UI_DIRECTION=rtl` (or `ltr`) forces a direction whatever the language, so the
//! mirrored layout can be checked before a right-to-left catalog is installed. It is read once
//! per process; the web build has no environment and always follows the language.

use std::cell::Cell;
use std::sync::OnceLock;

use egui::{Align, Align2, Layout};

pub use crate::i18n::Dir;

thread_local! {
    // Like the drawing language, per thread: test harnesses run side by side.
    static FORCED: Cell<Option<Dir>> = const { Cell::new(None) };
}

/// The direction the UI is drawn in on this thread.
pub fn current() -> Dir {
    FORCED.get().or_else(env_override).unwrap_or_else(|| crate::i18n::current().dir())
}

/// Is the UI drawn right-to-left?
pub fn is_rtl() -> bool {
    current() == Dir::Rtl
}

/// Draw in `dir` whatever the language until the guard drops on this thread (tests).
#[must_use]
pub fn direction_scope(dir: Dir) -> impl Drop {
    struct Restore {
        previous: Option<Dir>,
        _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            FORCED.set(self.previous);
        }
    }
    let restore = Restore { previous: FORCED.get(), _thread: std::marker::PhantomData };
    FORCED.set(Some(dir));
    restore
}

fn env_override() -> Option<Dir> {
    static ENV: OnceLock<Option<Dir>> = OnceLock::new();
    *ENV.get_or_init(|| parse_dir(&std::env::var("PHOTOCRAFT_UI_DIRECTION").ok()?))
}

/// `rtl` / `ltr`, any case; anything else (including empty) follows the language.
fn parse_dir(value: &str) -> Option<Dir> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rtl" => Some(Dir::Rtl),
        "ltr" => Some(Dir::Ltr),
        _ => None,
    }
}

/// A side panel on the reading-start side: left, or right in a right-to-left UI (the toolbox).
pub fn start_panel(id: impl egui::AsIdSalt) -> egui::Panel {
    if is_rtl() { egui::Panel::right(id) } else { egui::Panel::left(id) }
}

/// A side panel on the reading-end side: right, or left in a right-to-left UI (the dock).
pub fn end_panel(id: impl egui::AsIdSalt) -> egui::Panel {
    if is_rtl() { egui::Panel::left(id) } else { egui::Panel::right(id) }
}

/// A vertically centred row in reading order.
pub fn row() -> Layout {
    if is_rtl() { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) }
}

/// A vertically centred row that starts from the reading end: for groups pinned to the far end
/// of a bar, such as the title bar's controls or the status bar's Fit button.
pub fn trailing() -> Layout {
    if is_rtl() { Layout::left_to_right(Align::Center) } else { Layout::right_to_left(Align::Center) }
}

/// [`egui::Ui::horizontal_centered`] in reading order: a row filling the space left, with its
/// items centred vertically, starting from the right in a right-to-left UI.
pub fn horizontal_centered<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let size = ui.available_size_before_wrap();
    ui.allocate_ui_with_layout(size, row(), add)
}

/// The popup alignment for a menu opening below its button: under its reading-start edge.
pub fn below() -> egui::RectAlign {
    if is_rtl() { egui::RectAlign::BOTTOM_END } else { egui::RectAlign::BOTTOM_START }
}

/// The body layout for dialogs and window bodies: in a right-to-left UI blocks right-align and
/// their nested `ui.horizontal` rows mirror; in a left-to-right UI it is egui's default.
///
/// egui mirrors a nested row only when the parent layout runs right-to-left, or is vertical with
/// `Align::Max` cross alignment; a `top_down(Align::Min)` body keeps every nested row
/// left-to-right even with the direction flag set, so mirrored bodies scope themselves with this.
pub fn body_layout() -> Layout {
    if is_rtl() { Layout::top_down(Align::Max) } else { Layout::top_down(Align::Min) }
}

/// A row hugging the reading end with top-aligned items: the footer shape for window and dialog
/// bodies ([`trailing`] centres its items).
pub fn trailing_min() -> Layout {
    if is_rtl() { Layout::left_to_right(Align::Min) } else { Layout::right_to_left(Align::Min) }
}

/// The body of an open menu popup. egui pins popup bodies left-to-right (`Popup::menu` and
/// `SubMenu::show` both lay them out `top_down_justified(Align::Min)` whatever the direction
/// flag), so in a right-to-left UI every popup body scopes itself through this helper.
/// The cross align stays `Min`: rows fill the width (justified), and a `Max` align would seed
/// the scope's `min_rect` at the far edge of the pinned `max_rect`, so the sizing pass would
/// measure the popup at the full default area width instead of the rows' width. The sizing
/// pass itself runs in an LTR scope: it measures against the wide default area, and rows
/// anchored to the reading end would measure — and lock in — that full width. Left-to-right
/// the popup's own layout is used unchanged, sizing pass included.
pub fn popup_body<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if is_rtl() {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(Dir::Ltr);
            add_contents(ui)
        } else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(ui.max_rect()).layout(Layout::top_down_justified(Align::Min)), add_contents).inner
        }
    } else {
        add_contents(ui)
    }
}

/// A full-width row in reading order for bodies that stay `top_down(Align::Min)`:
/// [`horizontal_centered`] without the vertical centring. Rows inside a [`body_layout`] parent
/// already mirror; opt in here only where that scope does not reach.
///
/// The row allocates one interact height, like `ui.horizontal`: a full-remaining-height frame
/// would let a nested center-aligned scope fill the window's whole remaining height (the fork
/// fills horizontal `Align::Center` frames), and a non-resizable window then sizes itself to
/// the screen height. Taller content still grows the row's `min_rect` past the frame.
pub fn dialog_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let size = egui::vec2(row_width(ui), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row_min(), add)
}

/// [`row`] with top-aligned items, for rows inside a vertical body.
fn row_min() -> Layout {
    if is_rtl() { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) }
}

/// The horizontal keys meaning "next on screen" and "previous on screen":
/// `(ArrowRight, ArrowLeft)` left-to-right, swapped in a right-to-left UI. Menu-bar navigation
/// uses one tuple so the open-menu and bar-focus key machines cannot diverge.
pub fn horizontal_nav_keys() -> (egui::Key, egui::Key) {
    if is_rtl() { (egui::Key::ArrowLeft, egui::Key::ArrowRight) } else { (egui::Key::ArrowRight, egui::Key::ArrowLeft) }
}

/// Wrap a floating window's (or dialog's) content: in a right-to-left UI the body's blocks
/// right-align and its nested rows mirror; in a left-to-right UI the content draws unchanged.
/// Call once at the top of each `egui::Window` body closure.
pub fn window_body<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    if is_rtl() {
        ui.with_layout(body_layout(), add)
    } else {
        let inner = add(ui);
        egui::InnerResponse::new(inner, ui.response())
    }
}

/// A label inside a mirrored body ([`body_layout`]/[`window_body`]): in a right-to-left UI the
/// paragraph base direction is pinned right-to-left and the galley aligns to the reading-start
/// (right) edge, so mixed Arabic/Latin lines keep their phrase order instead of scrambling and
/// hug the right margin. In a left-to-right UI this is egui's plain label, unchanged.
pub fn body_label(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let text = text.into();
    if !is_rtl() {
        return ui.label(text);
    }
    let mut job = std::sync::Arc::unwrap_or_clone(text.into_layout_job(ui.style(), egui::FontSelection::Default, ui.text_valign()));
    job.direction = text_direction();
    job.halign = Align::Max;
    ui.label(egui::WidgetText::LayoutJob(std::sync::Arc::new(job)))
}

/// Run a block inside a left-to-right scope even in a right-to-left UI. `egui::Grid` places its
/// first cell at the raw cursor, which is `x = -inf` at the start of a right-to-left row, so a
/// grid must never sit directly in an RTL row; wrap it in this and mirror the cell order with
/// [`grid_cells`].
pub fn ltr<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.with_layout(Layout::left_to_right(Align::Min), add)
}

/// [`ltr`] for a block in a vertical flow: keeps the top-down direction (and so the parent's
/// cursor advance) but keeps nested rows left-to-right, which a mirrored body
/// (`top_down(Align::Max)`) would not. Wrap direction-blind widgets such as `egui::Grid` in this.
pub fn ltr_vertical<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.with_layout(Layout::top_down(Align::Min), add)
}

/// Two `egui::Grid` cells in reading order — label then editor left-to-right, editor then label
/// right-to-left. `egui::Grid` ignores the direction flag, so the column order is emitted
/// manually. Call once per row; end the row with `ui.end_row()` as usual.
pub fn grid_cells<L, E>(ui: &mut egui::Ui, label: L, editor: E)
where
    L: FnOnce(&mut egui::Ui),
    E: FnOnce(&mut egui::Ui),
{
    if is_rtl() {
        editor(ui);
        label(ui);
    } else {
        label(ui);
        editor(ui);
    }
}

/// One settings row without a `Grid`: the label occupies `label_w` at the reading start and the
/// editor follows toward the reading end — the mirrored shape of a two-column settings grid.
/// `label_w = 0.0` lets a bare checkbox hug the reading start.
pub fn settings_row<L, E>(ui: &mut egui::Ui, label_w: f32, label: L, editor: E)
where
    L: FnOnce(&mut egui::Ui),
    E: FnOnce(&mut egui::Ui),
{
    let width = row_width(ui);
    ui.allocate_ui_with_layout(egui::vec2(width, 0.0), row(), |ui| {
        ui.allocate_ui_with_layout(egui::vec2(label_w, 0.0), row(), |ui| {
            label(ui);
            // A scope only advances the parent cursor past the child's used min_rect; claim the
            // whole label column so the editor keeps the gap `label_w - label_width + spacing.x`.
            let cell = ui.max_rect();
            ui.expand_to_include_rect(cell);
        });
        editor(ui);
        ui.add_space(7.0);
    });
}

/// +1 left-to-right, −1 right-to-left: the signed x-step for hand-painted row geometry (eye
/// column, thumbnails, indicators), which the egui flag never mirrors.
pub fn step() -> f32 {
    if is_rtl() { -1.0 } else { 1.0 }
}

/// The reading-start x for content offset `off` from a rect's start edge: `rect.left() + off`
/// left-to-right, `rect.right() - off` right-to-left.
pub fn start_x(rect: egui::Rect, off: f32) -> f32 {
    if is_rtl() { rect.right() - off } else { rect.left() + off }
}

/// The min-x of a `w`-wide element hugging `rect`'s reading-start edge with margin `off` (an eye
/// icon or thumbnail column built with `Rect::from_min_size`): `rect.left() + off` left-to-right,
/// `rect.right() - off - w` right-to-left, so the element grows inward from the start edge.
pub fn start_x_w(rect: egui::Rect, off: f32, w: f32) -> f32 {
    if is_rtl() { rect.right() - off - w } else { rect.left() + off }
}

/// The reading-end x for content offset `off` from a rect's end edge: `rect.right() - off`
/// left-to-right, `rect.left() + off` right-to-left (the scrollbar side of a mirrored row).
pub fn end_x(rect: egui::Rect, off: f32) -> f32 {
    if is_rtl() { rect.left() + off } else { rect.right() - off }
}

/// The x for a `w`-wide painted element placed `off` beyond `rect` in reading order (a label
/// after a thumbnail): `rect.right() + off` left-to-right, `rect.left() - off - w` right-to-left.
/// With an alignment anchor instead of a galley, pass `w = 0.0` and [`start_align`].
pub fn after_x(rect: egui::Rect, off: f32, w: f32) -> f32 {
    if is_rtl() { rect.left() - off - w } else { rect.right() + off }
}

/// A `size` rect whose reading-start-top corner sits `off` from `rect`'s reading-start-top
/// corner: LTR min = rect.left_top() + off; RTL min = (rect.right() - off.x - size.x, rect.top() + off.y).
pub fn start_rect(rect: egui::Rect, off: egui::Vec2, size: egui::Vec2) -> egui::Rect {
    if is_rtl() {
        egui::Rect::from_min_size(egui::pos2(rect.right() - off.x - size.x, rect.top() + off.y), size)
    } else {
        egui::Rect::from_min_size(rect.left_top() + off, size)
    }
}

/// A full-height, `w`-wide cell at `rect`'s reading-start edge, clamped to `rect`'s width.
pub fn start_cell(rect: egui::Rect, w: f32) -> egui::Rect {
    let w = w.min(rect.width());
    egui::Rect::from_min_size(egui::pos2(start_x_w(rect, 0.0, w), rect.top()), egui::vec2(w, rect.height()))
}

/// The walking-cursor step of a hand-painted row: `x + step() * w`.
pub fn advance(x: f32, w: f32) -> f32 {
    x + step() * w
}

/// A rect of width `w` whose reading-start edge sits at `x` (it grows against `step()`).
pub fn start_span(x: f32, w: f32, top: f32, bottom: f32) -> egui::Rect {
    if is_rtl() {
        egui::Rect::from_min_max(egui::pos2(x - w, top), egui::pos2(x, bottom))
    } else {
        egui::Rect::from_min_max(egui::pos2(x, top), egui::pos2(x + w, bottom))
    }
}

/// The (anchor, limit) span a name is painted in, after `prev` inside `container`:
/// LTR (prev.right() + pad, container.right() - edge_pad); RTL (prev.left() - pad, container.left() + edge_pad).
pub fn name_span(prev: egui::Rect, container: egui::Rect, pad: f32, edge_pad: f32) -> (f32, f32) {
    if is_rtl() {
        (prev.left() - pad, container.left() + edge_pad)
    } else {
        (prev.right() + pad, container.right() - edge_pad)
    }
}

/// The reading-start edge of `rect` (its right edge in a right-to-left UI, else its left).
pub fn start_edge(rect: egui::Rect) -> f32 {
    if is_rtl() { rect.right() } else { rect.left() }
}

/// Fold `rects` into the running reading-end edge of a row: rtl folds the min of the lefts,
/// ltr the max of the rights, starting from `start`.
pub fn row_end_edge(rects: impl IntoIterator<Item = egui::Rect>, start: f32) -> f32 {
    if is_rtl() {
        rects.into_iter().map(|r| r.left()).fold(start, f32::min)
    } else {
        rects.into_iter().map(|r| r.right()).fold(start, f32::max)
    }
}

/// `Align2` for `Painter::text` hugging the reading-start edge of a row: `LEFT_CENTER`
/// left-to-right, `RIGHT_CENTER` right-to-left.
pub fn start_align() -> Align2 {
    if is_rtl() { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }
}

/// `Align2` for `Painter::text` hugging the reading-end edge of a row (a shortcut hint):
/// `RIGHT_CENTER` left-to-right, `LEFT_CENTER` right-to-left.
pub fn end_align() -> Align2 {
    if is_rtl() { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER }
}

/// Is `p` on the reading-start side of the vertical edge `edge` (a mirrored hit test)?
pub fn start_side(p: f32, edge: f32) -> bool {
    if is_rtl() { p > edge } else { p < edge }
}

/// A directional glyph's x mirrored around `cx` in a right-to-left UI (disclosure triangles),
/// unchanged left-to-right.
pub fn glyph_x(x: f32, cx: f32) -> f32 {
    if is_rtl() { 2.0 * cx - x } else { x }
}

/// Single-line galley elided with "…" to `max_width` (nothing when there's no room).
pub fn elided_galley(painter: &egui::Painter, text: &str, font: egui::FontId, color: egui::Color32, italics: bool, max_width: f32) -> Option<std::sync::Arc<egui::Galley>> {
    if max_width < 8.0 {
        return None;
    }
    let format = egui::text::TextFormat { font_id: font, color, italics, ..Default::default() };
    let layout = |s: &str, wrap: bool| {
        let mut job = egui::text::LayoutJob::default();
        job.append(s, 0.0, format.clone());
        if wrap {
            job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        }
        painter.layout_job(job)
    };
    // A right-to-left name (Arabic, Hebrew) too long for the row is cut at its logical end, so its
    // start stays and "…" is at the left (`bidi_text::elide`).
    if let Some(shown) = crate::bidi_text::elide(text, |s| layout(s, false).size().x <= max_width) {
        return Some(layout(&shown, false));
    }
    // egui puts a right-to-left name into display order itself; left-to-right names are cut at
    // their right end by egui.
    Some(layout(text, true))
}

/// Paint `text` in the span anchor→limit at height `cy`, elided to fit, aligned into the
/// reading direction (RIGHT_CENTER at the anchor in RTL, LEFT_CENTER in LTR). A span whose
/// anchor sits past its limit in LTR (an unconverted caller) keeps the left-to-right contract.
/// Returns the painted rect.
pub fn paint_label_span(painter: &egui::Painter, anchor: f32, cy: f32, limit: f32, text: &str, font: egui::FontId, color: egui::Color32) -> Option<egui::Rect> {
    if is_rtl() && anchor > limit {
        let g = elided_galley(painter, text, font, color, false, anchor - limit)?;
        let r = Align2::RIGHT_CENTER.anchor_size(egui::pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    } else {
        let g = elided_galley(painter, text, font, color, false, limit - anchor)?;
        let r = Align2::LEFT_CENTER.anchor_size(egui::pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    }
}

/// Paint a single-line elided label at the x-anchor `x` (a [`start_x`] reading-start edge),
/// vertically centred on `cy`; returns (painted rect, elided). RTL paints right-aligned from
/// the reading-start edge.
pub fn paint_elided(painter: &egui::Painter, x: f32, cy: f32, max_w: f32, text: &str, font: egui::FontId, color: egui::Color32) -> Option<(egui::Rect, bool)> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::text::TextFormat { font_id: font.clone(), color, ..Default::default() });
    let elided = painter.layout_job(job).size().x > max_w;
    let g = elided_galley(painter, text, font, color, false, max_w)?;
    let align = if is_rtl() { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    let r = align.anchor_size(egui::pos2(x, cy), g.size());
    painter.galley(r.min, g, color);
    Some((r, elided))
}

/// (pos, align) for `Painter::text` hugging `rect`'s reading-start vertical centre, inset `off`.
pub fn start_text_anchor(rect: egui::Rect, off: f32) -> (egui::Pos2, egui::Align2) {
    if is_rtl() {
        (egui::pos2(rect.right() - off, rect.center().y), Align2::RIGHT_CENTER)
    } else {
        (egui::pos2(rect.left() + off, rect.center().y), Align2::LEFT_CENTER)
    }
}

/// Same at the reading-end edge.
pub fn end_text_anchor(rect: egui::Rect, off: f32) -> (egui::Pos2, egui::Align2) {
    if is_rtl() {
        (egui::pos2(rect.left() + off, rect.center().y), Align2::LEFT_CENTER)
    } else {
        (egui::pos2(rect.right() - off, rect.center().y), Align2::RIGHT_CENTER)
    }
}

/// First-show x of a floating panel window `width` wide, `inset` px inside the canvas's
/// reading-end edge, shifted `cascade` px further toward the reading end (cascading panels;
/// negative cascades run back toward the canvas centre). Clamped to keep 8 px of the window
/// on the canvas.
pub fn float_window_x(canvas: egui::Rect, width: f32, inset: f32, cascade: f32) -> f32 {
    if is_rtl() {
        (canvas.left() + inset - cascade).min(canvas.right() - 8.0 - width).max(canvas.left() + 8.0)
    } else {
        (canvas.right() - width - inset + cascade).max(canvas.left() + 8.0)
    }
}

/// Clamp a resizable window's rect to exactly `min`, keeping the edge the window is anchored on
/// fixed: under RTL the fork anchors the window rect at its right edge (resize grip on the
/// left), so the clamped rect grows leftward from a fixed right edge; in LTR it keeps the fixed
/// left edge. The top edge is fixed in both directions. The returned rect's size is exactly
/// `min` — it never preserves the input rect's current size, so shrink drags are honoured.
pub fn clamp_window_min_size(rect: egui::Rect, min: egui::Vec2) -> egui::Rect {
    let size = min;
    if is_rtl() {
        egui::Rect::from_min_size(egui::pos2(rect.right() - size.x, rect.top()), size)
    } else {
        egui::Rect::from_min_size(rect.min, size)
    }
}

/// An Area hanging off `anchor`'s reading-end edge, `side_offset` px toward the canvas:
/// rtl pivot RIGHT_TOP at anchor.left_top() - (side_offset, 0); ltr LEFT_TOP at
/// anchor.right_top() + (side_offset, 0).
pub fn popup_area_near(ui: &egui::Ui, id: impl egui::AsIdSalt, anchor: egui::Rect, side_offset: f32) -> egui::Area {
    let id = ui.scope_id().with(id);
    if is_rtl() {
        egui::Area::new(id).pivot(Align2::RIGHT_TOP).fixed_pos(anchor.left_top() - egui::vec2(side_offset, 0.0))
    } else {
        egui::Area::new(id).pivot(Align2::LEFT_TOP).fixed_pos(anchor.right_top() + egui::vec2(side_offset, 0.0))
    }
}

/// The [`egui::PopupAnchor`] of `resp` translated `offset` px past its reading-end edge
/// (`ParentRect` moved by (step() * offset, 0); the other variants unchanged).
pub fn popup_anchor_reading_end(resp: &egui::Response, offset: f32) -> egui::PopupAnchor {
    match egui::PopupAnchor::from(resp) {
        egui::PopupAnchor::ParentRect(rect) => egui::PopupAnchor::ParentRect(rect.translate(egui::vec2(step() * offset, 0.0))),
        other => other,
    }
}

/// A two-column caption/editor form. LTR: `egui::Grid::new(id).num_columns(2).spacing(spacing)`.
/// RTL: hand rows via [`settings_row`] under [`body_layout`] (the fork's `Grid` cannot run
/// under a mirrored body). `rows` calls `row(caption, editor)` once per row; the editor
/// closure receives the caption label's Id for accessibility labeling. An empty caption
/// draws a bare editor hugging the reading start.
pub fn form_grid(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    spacing: [f32; 2],
    caption_w: f32,
    rows: impl FnOnce(&mut dyn FnMut(&str, &mut dyn FnMut(&mut egui::Ui, egui::Id))),
) {
    two_col_grid(ui, id, spacing, caption_w, None, |grid_rows| {
        let mut adapt = |caption: &str, editor: &mut dyn FnMut(&mut egui::Ui, egui::Id)| grid_rows(egui::WidgetText::from(caption), editor);
        rows(&mut adapt);
    });
}

/// The shared body of [`form_grid`] and [`settings_grid`]; see [`settings_grid`] for the RTL
/// geometry. `rows` calls `row(caption, editor)` once per row.
fn two_col_grid(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    spacing: [f32; 2],
    caption_w: f32,
    min_col_width: Option<f32>,
    rows: impl FnOnce(&mut dyn FnMut(egui::WidgetText, &mut dyn FnMut(&mut egui::Ui, egui::Id))),
) {
    if is_rtl() {
        ui.with_layout(body_layout(), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(spacing[0], spacing[1]);
            let label_w = caption_w.max(min_col_width.unwrap_or(0.0));
            let mut row = |caption: egui::WidgetText, editor: &mut dyn FnMut(&mut egui::Ui, egui::Id)| {
                if caption.text().is_empty() {
                    dialog_row(ui, |ui| editor(ui, egui::Id::NULL));
                } else {
                    let caption_id = std::cell::Cell::new(egui::Id::NULL);
                    settings_row(ui, label_w, |ui| caption_id.set(ui.label(caption).id), |ui| editor(ui, caption_id.get()));
                }
            };
            rows(&mut row);
        });
    } else {
        let mut grid = egui::Grid::new(id).num_columns(2).spacing(spacing);
        if let Some(w) = min_col_width {
            grid = grid.min_col_width(w);
        }
        grid.show(ui, |ui| {
            let mut row = |caption: egui::WidgetText, editor: &mut dyn FnMut(&mut egui::Ui, egui::Id)| {
                let caption_id = ui.label(caption).id;
                editor(ui, caption_id);
                ui.end_row();
            };
            rows(&mut row);
        });
    }
}

/// A two-column settings grid of label/editor rows, direction-aware. LTR: a plain
/// `egui::Grid` (byte-identical layout). RTL: hand rows via [`settings_row`] under
/// [`body_layout`]: the fork's `Grid` gives its last column the remaining width, which
/// detaches a right-aligned label from its editor in a mirrored grid, while the hand rows
/// keep the label column fixed at `label_w` hugging the reading start, so the
/// label<->editor gap equals the LTR formula (`label_w - label_width + spacing.x`) in both
/// directions. Pass `label_w` as the LTR label column width (the widest label, or
/// `min_col_width` where the grid sets one). `rows` calls `row(label, editor)` once per
/// row; the editor closure receives the label's Id for accessibility labeling. An empty
/// label draws a bare editor hugging the reading start.
pub fn settings_grid(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    spacing: [f32; 2],
    label_w: f32,
    min_col_width: Option<f32>,
    rows: impl FnOnce(&mut dyn FnMut(egui::WidgetText, &mut dyn FnMut(&mut egui::Ui, egui::Id))),
) {
    two_col_grid(ui, id, spacing, label_w, min_col_width, rows)
}

/// An `egui::Grid` in an LTR scope ([`ltr_vertical`]) so the fork's placer never sees an RTL
/// cursor; cell order inside a row stays the caller's (mirror with [`grid_cells`] or an
/// explicit reversal).
pub fn ltr_grid<R>(ui: &mut egui::Ui, id: impl egui::AsIdSalt, cols: usize, spacing: egui::Vec2, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ltr_vertical(ui, |ui| egui::Grid::new(id).num_columns(cols).spacing(spacing).show(ui, add)).inner
}

/// The usable row width: in RTL the fork's available-width read balloons, so read
/// `ui.max_rect().width()`; LTR keeps `ui.available_size_before_wrap().x`.
pub fn row_width(ui: &egui::Ui) -> f32 {
    if is_rtl() { ui.max_rect().width() } else { ui.available_size_before_wrap().x }
}

/// A full-width centred row inside a horizontal ScrollArea. RTL: the fork anchors content at
/// the viewport's left edge and clamps offset >= 0, which would leave a mirrored row's
/// trailing end cropped off-screen and unreachable, so the row's region is mirrored about the
/// viewport's left edge (offset = (clip.min.x - max.min.x).max(0), region translated by
/// 2*offset) and the strip then anchors at the reading-start (right) edge; LTR: plain
/// [`horizontal_centered`].
pub fn hscroll_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    if is_rtl() {
        let offset = (ui.clip_rect().min.x - ui.max_rect().min.x).max(0.0);
        let rect = ui.max_rect().translate(egui::vec2(2.0 * offset, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| horizontal_centered(ui, add)).inner
    } else {
        horizontal_centered(ui, add)
    }
}

/// The base direction of a text line in the UI's direction: right-to-left in a right-to-left
/// language, left-to-right otherwise.
pub fn text_direction() -> egui::epaint::text::TextDirection {
    if is_rtl() { egui::epaint::text::TextDirection::RightToLeft } else { egui::epaint::text::TextDirection::LeftToRight }
}

/// The (anchor, offset) pair for an `egui::Area` pinned to the screen's reading-end corner:
/// bottom-right left-to-right, bottom-left right-to-left (`bottom = false` picks the top pair);
/// a positive `x_off` moves inward from that edge.
pub fn end_corner_anchor(bottom: bool, x_off: f32, y_off: f32) -> (Align2, egui::Vec2) {
    if is_rtl() {
        let anchor = if bottom { Align2::LEFT_BOTTOM } else { Align2::LEFT_TOP };
        (anchor, egui::vec2(x_off, y_off))
    } else {
        let anchor = if bottom { Align2::RIGHT_BOTTOM } else { Align2::RIGHT_TOP };
        (anchor, egui::vec2(-x_off, y_off))
    }
}

/// Split `bar` into `(rest, end_strip)`: a `w`-wide strip at its reading-end edge (right
/// left-to-right, left right-to-left), clamped into `bar`.
pub fn split_end(bar: egui::Rect, w: f32) -> (egui::Rect, egui::Rect) {
    if is_rtl() {
        let split = (bar.left() + w).min(bar.right());
        (egui::Rect::from_min_max(egui::pos2(split, bar.top()), bar.max), egui::Rect::from_min_max(bar.min, egui::pos2(split, bar.bottom())))
    } else {
        let split = (bar.right() - w).max(bar.left());
        (egui::Rect::from_min_max(bar.min, egui::pos2(split, bar.bottom())), egui::Rect::from_min_max(egui::pos2(split, bar.top()), bar.max))
    }
}

/// A rect of width `w` whose reading-end edge sits at `x` (it grows against `step()`):
/// `[x, x + w]` left-to-right, `[x - w, x]` right-to-left.
pub fn end_span(x: f32, w: f32, top: f32, bottom: f32) -> egui::Rect {
    if is_rtl() {
        egui::Rect::from_min_max(egui::pos2(x - w, top), egui::pos2(x, bottom))
    } else {
        egui::Rect::from_min_max(egui::pos2(x, top), egui::pos2(x + w, bottom))
    }
}

/// A full-width, interact-height row in reading order ([`row`]): the pinned-height reading-row
/// shape.
pub fn full_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row(), add)
}

/// [`full_row`] starting from the reading end ([`trailing`]).
pub fn full_trailing<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let size = egui::vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, trailing(), add)
}

/// A row pinned to the reading end only in a right-to-left UI; left-to-right the caller's own
/// layout runs unchanged.
pub fn trailing_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if is_rtl() { ui.with_layout(trailing(), add).inner } else { add(ui) }
}

#[cfg(test)]
mod tests;
