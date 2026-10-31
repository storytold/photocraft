//! Compact floating chrome. Workspace remains the only owner of panel placement.
use super::{
    Action, COMPACT_COLUMN_WIDTH, Layout, PANELS, PhotocraftApp, Placement, Side, Target, Value, Zone, area_id, body, commit, compact_grip_width, compat,
    floats, group_members, in_dock_column, json, normalize, paint_header_glyph, panel_icon, panel_menu, permissions, tree_groups, valid_workspace,
    workspace_of,
};
use craft_ui::docking::{DockArea, DropFeedback, DropRequest, Hit};
use egui::{Pos2, Rect, Sense, Vec2, pos2, vec2};

#[path = "part_resize.rs"]
mod part_resize;

const HEADER: f32 = super::contract::HEADER_CONTROL_SIZE;
const GRIP: f32 = 11.0;
const ROW: f32 = 35.0;
const INSET: f32 = 3.0;

#[derive(Clone)]
struct Strip {
    first: String,
    rows: Vec<Vec<String>>,
    rect: Rect,
    attached: bool,
    embedded: bool,
    left: bool,
    offset: f32,
}
impl Strip {
    fn panels(&self) -> impl Iterator<Item = &String> {
        self.rows.iter().flatten()
    }
    fn header_inset(&self) -> f32 {
        if self.attached { INSET } else { 0.0 }
    }
    fn cells(&self) -> Vec<(String, Rect)> {
        let mut y = self.rect.top() + self.header_inset() + HEADER + GRIP - self.offset;
        let mut out = Vec::new();
        for row in &self.rows {
            for id in row {
                out.push((id.clone(), Rect::from_min_size(pos2(self.rect.left(), y), vec2(self.rect.width(), ROW))));
                y += ROW;
            }
        }
        out
    }
}
#[derive(Clone, Copy)]
enum DragView {
    Palette,
    Editor,
}

#[derive(Clone)]
struct Drag {
    panel: String,
    whole: bool,
    view: DragView,
    press: Pos2,
    source: Rect,
    generation: u64,
}
#[derive(Clone)]
struct Resize {
    panel: String,
    rect: Rect,
    press: Pos2,
    left: bool,
    attached: bool,
    height: bool,
    minimum: f32,
    generation: u64,
}
#[derive(Clone)]
struct EditorResize {
    panel: String,
    rect: Rect,
    press: Pos2,
    left: bool,
    bottom: bool,
    generation: u64,
}
#[derive(Clone)]
struct Flight {
    panel: String,
    from: Rect,
    at: f64,
    generation: u64,
}
fn key(part: &str) -> egui::Id {
    area_id().with(("presentation", part))
}

const SPACER: &str = "__photocraft_column_spacer__";
pub(super) fn is_spacer(id: &str) -> bool {
    id.starts_with(SPACER)
}

pub(super) fn rendered_layout<'a>(
    app: &PhotocraftApp,
    canonical: &'a Layout<String>,
    available_height: f32,
    preview: Option<&(String, f32)>,
    pixel: f32,
) -> std::borrow::Cow<'a, Layout<String>> {
    let compact_float = canonical.floating.iter().any(|w| w.native.is_none() && w.panels().all(|p| app.ui.panel_presentation.compact.contains(p)));
    let size_columns = app.ui.panel_presentation.migrated && compat::columns(canonical).iter().any(|(_, c)| !c.iconic);
    if !compact_float && !size_columns {
        return std::borrow::Cow::Borrowed(canonical);
    }
    let mut layout = canonical.clone();
    layout.floating.retain(|w| w.native.is_some() || compat::independent_window(w) || !w.panels().all(|p| app.ui.panel_presentation.compact.contains(p)));
    fit_compact_windows(&mut layout, &app.ui.panel_presentation, pixel);
    if size_columns {
        let mut pending: Vec<_> = layout.root.iter_mut().collect();
        while let Some(node) = pending.pop() {
            match node {
                super::Node::Split { first, second, .. } => pending.extend([first.as_mut(), second.as_mut()]),
                super::Node::Dock { columns, .. } => {
                    for column in columns.iter_mut().filter(|c| !c.iconic) {
                        size_column(app, column, available_height, preview);
                    }
                }
                _ => {}
            }
        }
    }
    std::borrow::Cow::Owned(layout)
}
fn size_column(app: &PhotocraftApp, column: &mut super::Column<String>, available_height: f32, preview: Option<&(String, f32)>) {
    let Some(first) = column.groups.first().and_then(|g| g.panels.first()).cloned() else { return };
    let minimum: f32 = column
        .groups
        .iter()
        .filter(|g| !g.minimized)
        .map(|g| g.panels.get(g.active).and_then(|id| PANELS.iter().find(|p| p.0 == id)).map_or(80.0, |p| p.2.min_height()))
        .sum();
    let total = preview
        .filter(|(panel, _)| column.groups.iter().any(|g| g.panels.contains(panel)))
        .map(|(_, h)| *h)
        .or_else(|| app.ui.panel_presentation.heights.get(&first).copied())
        .unwrap_or(available_height)
        .clamp(minimum, available_height.max(minimum));
    let known: f32 = column.groups.iter().filter_map(|g| g.height).sum();
    let missing = column.groups.iter().filter(|g| g.height.is_none()).count();
    let default = if missing > 0 { (total - known).max(missing as f32) / (missing as f32) } else { 1.0 };
    let weights: f32 = column.groups.iter().map(|g| g.height.unwrap_or(default).max(1.0)).sum();
    for group in &mut column.groups {
        group.height = Some(total * group.height.unwrap_or(default).max(1.0) / weights.max(1.0));
    }
    if total <= available_height - 26.0 {
        column.groups.push(craft_ui::docking::Group::new(vec![format!("{SPACER}{first}")], 0));
    }
}
/// The last real row's divider is the column's bottom edge, above a view-only spacer.
pub(super) fn bottom_resize(layout: &Layout<String>, action: &Action<String>) -> Option<Value> {
    let Action::ResizeGroup { panel, height: Some(height) } = action else { return None };
    let (_, column) = compat::columns(layout).into_iter().find(|(_, c)| {
        c.groups.last().and_then(|g| g.panels.first()).is_some_and(|id| is_spacer(id)) && c.groups.iter().rev().nth(1).is_some_and(|g| g.panels.contains(panel))
    })?;
    let total: f32 = column.groups.iter().filter(|g| !g.panels.iter().any(|p| is_spacer(p)) && !g.panels.contains(panel)).filter_map(|g| g.height).sum();
    Some(json!({"panel":panel,"operation":"edge-size","height":(total+height).max(100.0)}))
}

fn resize_preview(ctx: &egui::Context, strip: &mut Strip) {
    let Some(start) = ctx.data(|d| d.get_temp::<Resize>(key("resize"))).filter(|r| r.panel == strip.first) else { return };
    if start.attached && !start.height {
        return;
    }
    if start.height {
        if let Some((_, height)) = height_preview(ctx) {
            strip.rect = Rect::from_min_size(start.rect.min, vec2(start.rect.width(), height));
        }
        return;
    }
    let dx = ctx.pointer_interact_pos().unwrap_or(start.press).x - start.press.x;
    let width = (start.rect.width() + if start.left { -dx } else { dx }).clamp(COMPACT_COLUMN_WIDTH, 240.0);
    strip.rect =
        Rect::from_min_size(pos2(if start.left { start.rect.right() - width } else { start.rect.left() }, start.rect.top()), vec2(width, start.rect.height()));
}

/// Apply transient widths before the workspace's fit pass. Nothing is persisted until release.
pub(super) fn preview_columns(ctx: &egui::Context, view: &mut std::borrow::Cow<'_, Layout<String>>) {
    let Some(start) = ctx.data(|d| d.get_temp::<Resize>(key("resize"))).filter(|r| r.attached && !r.height) else { return };
    let dx = ctx.pointer_interact_pos().unwrap_or(start.press).x - start.press.x;
    let width = (start.rect.width() + if start.left { -dx } else { dx }).clamp(COMPACT_COLUMN_WIDTH, 240.0);
    let mut pending: Vec<_> = view.to_mut().root.iter_mut().collect();
    while let Some(node) = pending.pop() {
        match node {
            super::Node::Split { first, second, .. } => pending.extend([first.as_mut(), second.as_mut()]),
            super::Node::Dock { columns, .. } => {
                for column in columns.iter_mut().filter(|c| c.iconic && c.groups.iter().any(|g| g.panels.contains(&start.panel))) {
                    column.iconic_width = Some(width);
                }
            }
            _ => {}
        }
    }
}

fn attached_strip(app: &PhotocraftApp, ctx: &egui::Context, column: &super::Column<String>, rect: Rect, side: Side) -> Strip {
    let first = column.groups.first().and_then(|g| g.panels.first()).cloned().unwrap_or_default();
    let height = app.ui.panel_presentation.heights.get(&first).copied().unwrap_or(rect.height()).min(rect.height()).max(0.0);
    Strip {
        first: first.clone(),
        rows: column.groups.iter().map(|g| g.panels.clone()).collect(),
        rect: Rect::from_min_size(rect.min, vec2(rect.width(), height)),
        attached: true,
        embedded: false,
        left: (side == Side::Start) != crate::direction::is_rtl(),
        offset: ctx.data(|d| d.get_temp::<f32>(key("scroll").with(&first))).unwrap_or(0.0),
    }
}

pub(super) fn observe_column(ctx: &egui::Context, column: &super::Column<String>, rect: Rect, side: Side) {
    let Some(first) = column.groups.first().and_then(|g| g.panels.first()) else { return };
    ctx.data_mut(|d| {
        let mut columns = d.get_temp::<Vec<(String, Rect, Side)>>(key("columns")).unwrap_or_default();
        columns.push((first.clone(), rect, side));
        d.insert_temp(key("columns"), columns);
    });
}

pub(super) fn attached_column(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    column: &super::Column<String>,
    rect: Rect,
    side: Side,
    commands: &mut Vec<(&'static str, Value)>,
) {
    let strip = attached_strip(app, ui.ctx(), column, rect, side);
    ui.ctx().data_mut(|d| {
        let mut drawn = d.get_temp::<Vec<Strip>>(key("strips")).unwrap_or_default();
        drawn.push(strip.clone());
        d.insert_temp(key("strips"), drawn);
    });
    paint_strip(app, ui, &strip, commands);
}

fn paint_strip(app: &mut PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    if super::modal_open(app, ui.ctx()) {
        ui.disable();
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.interact(strip.rect, key("background").with(&strip.first), Sense::click());
    ui.set_min_size(strip.rect.size());
    ui.set_max_size(strip.rect.size());
    ui.painter().rect_filled(strip.rect, if strip.attached { 0.0 } else { t.radius_sm }, t.card);
    let clip = ui.clip_rect();
    let pixel = ui.ctx().pixels_per_point().recip();
    ui.set_clip_rect(clip.intersect(strip.rect));
    header(app, ui, strip, commands);
    ui.set_clip_rect(clip.intersect(strip.rect.shrink(pixel)));
    icon_list(app, ui, strip, commands);
    ui.set_clip_rect(clip);
    resize(app, ui, strip, commands);
    if strip.attached {
        height_handle(app, ui, &strip.first, strip.rect, 100.0, commands);
    } else if !strip.embedded {
        // A standalone palette owns its outline; embedded parts share the parent's outside stroke.
        ui.painter().rect_stroke(strip.rect, t.radius_sm, egui::Stroke::new(pixel, t.card_border), egui::StrokeKind::Inside);
        for left in [true, false] {
            if keyboard_resize_focus(ui.ctx(), &strip.first, left) {
                let x = if left { strip.rect.left() + pixel * 0.5 } else { strip.rect.right() - pixel * 0.5 };
                ui.painter().vline(x, (strip.rect.top() + HEADER + GRIP)..=strip.rect.bottom(), egui::Stroke::new(pixel, t.accent));
            }
        }
    }
}

fn keyboard_resize_focus(ctx: &egui::Context, panel: &str, left: bool) -> bool {
    keyboard_handle_focus(ctx, key("resize").with((panel, left)))
}
fn keyboard_handle_focus(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.input(|i| i.focused && !i.pointer.any_down()) && ctx.memory(|m| m.has_focus(id))
}

pub(super) fn cancel_resize(app: &PhotocraftApp, ctx: &egui::Context) {
    if let Some(start) = ctx.data(|d| d.get_temp::<EditorResize>(key("editor-resize")))
        && (app.ui.docking_generation != start.generation
            || app.session.prefs().workspace_locked
            || super::modal_open(app, ctx)
            || ctx.input(|i| !i.focused || i.key_pressed(egui::Key::Escape) || (!i.pointer.primary_down() && !i.pointer.primary_released())))
    {
        ctx.data_mut(|d| d.remove::<EditorResize>(key("editor-resize")));
    }
    let Some(start) = ctx.data(|d| d.get_temp::<Resize>(key("resize"))) else { return };
    let cancel = ctx.input(|i| !i.focused || i.key_pressed(egui::Key::Escape) || (!i.pointer.primary_down() && !i.pointer.primary_released()));
    if cancel || super::modal_open(app, ctx) || app.session.prefs().workspace_locked || app.ui.docking_generation != start.generation {
        ctx.data_mut(|d| {
            d.remove::<Resize>(key("resize"));
            d.insert_temp(key("suppressed"), true);
        });
    }
}

fn strips(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>) -> Vec<Strip> {
    let mut out = Vec::new();
    let columns = ctx.data(|d| d.get_temp::<Vec<(String, Rect, Side)>>(key("columns"))).unwrap_or_default();
    for (side, column) in compat::columns(layout).into_iter().filter(|(_, c)| c.iconic) {
        let Some(first) = column.groups.first().and_then(|g| g.panels.first()) else { continue };
        let Some((_, rect, _)) = columns.iter().find(|(id, _, _)| id == first) else { continue };
        out.push(attached_strip(app, ctx, column, *rect, side));
    }
    for window in &layout.floating {
        if window.native.is_some() || compat::independent_window(window) || !window.panels().all(|p| app.ui.panel_presentation.compact.contains(p)) {
            continue;
        }
        let Some(first) = window.panels().next() else { continue };
        let rows: Vec<_> = tree_groups(&window.root).into_iter().map(|g| g.panels).collect();
        let width = app.ui.panel_presentation.widths.get(first).copied().unwrap_or(COMPACT_COLUMN_WIDTH).clamp(COMPACT_COLUMN_WIDTH, 240.0);
        let count = rows.iter().map(Vec::len).sum::<usize>();
        let height = HEADER + GRIP + ROW * count as f32 + 1.5;
        let screen = ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap_or_else(|| ctx.content_rect());
        let height = height.min(screen.height().max(HEADER + GRIP + ROW));
        let position = pos2(
            window.rect[0].clamp(screen.left(), (screen.right() - width).max(screen.left())),
            window.rect[1].clamp(screen.top(), (screen.bottom() - height).max(screen.top())),
        );
        let rect = Rect::from_min_size(position, vec2(width, height));
        let left = rect.right() + window.rect[2] <= ctx.content_rect().right();
        out.push(Strip {
            first: first.clone(),
            rows,
            rect,
            attached: false,
            embedded: false,
            left,
            offset: ctx.data(|d| d.get_temp::<f32>(key("scroll").with(first))).unwrap_or(0.0),
        });
    }
    out.extend(ctx.data(|d| d.get_temp::<Vec<Strip>>(key("embedded-strips"))).unwrap_or_default());
    for strip in &mut out {
        if strip.embedded {
            continue;
        }
        resize_preview(ctx, strip);
        if !strip.attached {
            let scale = ctx.pixels_per_point();
            let snap = |p: Pos2| pos2((p.x * scale).round() / scale, (p.y * scale).round() / scale);
            let size = vec2((strip.rect.width() * scale).ceil() / scale, (strip.rect.height() * scale).ceil() / scale);
            strip.rect = Rect::from_min_size(snap(strip.rect.min), size);
        }
    }
    out
}
pub(super) fn prepare(ctx: &egui::Context, app: &PhotocraftApp, layout: &Layout<String>, bounds: Rect) {
    cancel_resize(app, ctx);
    ctx.data_mut(|d| {
        d.remove::<Vec<Strip>>(key("embedded-strips"));
        d.remove::<Vec<(String, Rect)>>(key("group-rects"));
    });
    let drawn = strips(app, ctx, layout);
    let (pointer, keyboard) = ctx.input(|i| (i.pointer.any_pressed(), i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))));
    ctx.data_mut(|d| {
        let focus = if pointer {
            false
        } else if keyboard {
            true
        } else {
            d.get_temp::<bool>(key("keyboard-input")).unwrap_or(false)
        };
        d.insert_temp(key("keyboard-input"), focus);
        d.insert_temp(key("bounds"), bounds);
        d.insert_temp(key("strips"), drawn.into_iter().filter(|s| !s.attached).collect::<Vec<_>>());
        d.insert_temp(key("columns"), Vec::<(String, Rect, Side)>::new());
    });
}

pub(super) fn embedded_limits(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>) -> std::collections::HashMap<String, (f32, f32)> {
    let resize = ctx.data(|d| d.get_temp::<Resize>(key("resize")));
    let mut out = std::collections::HashMap::new();
    for window in layout.floating.iter().filter(|w| compat::independent_window(w)) {
        for group in tree_groups(&window.root).into_iter().filter(|g| g.panels.iter().all(|p| app.ui.panel_presentation.compact.contains(p))) {
            let Some(first) = group.panels.first() else { continue };
            let mut width = app.ui.panel_presentation.widths.get(first).copied().unwrap_or(COMPACT_COLUMN_WIDTH);
            if let Some(preview) = part_resize::width(ctx, first) {
                width = preview;
            }
            if let Some(start) = resize.as_ref().filter(|r| group.panels.contains(&r.panel) && !r.height) {
                let dx = ctx.pointer_interact_pos().unwrap_or(start.press).x - start.press.x;
                width = (start.rect.width() + if start.left { -dx } else { dx }).clamp(COMPACT_COLUMN_WIDTH, 240.0);
            }
            let height = HEADER + GRIP + ROW * group.panels.len() as f32 + 1.5 - super::contract::FULL_HEADER_HEIGHT;
            for id in group.panels {
                out.insert(id, (width, height));
            }
        }
    }
    out
}
pub(super) fn embedded_group(app: &mut PhotocraftApp, ui: &mut egui::Ui, panels: &[String], rect: Rect, commands: &mut Vec<(&'static str, Value)>) {
    let Some(first) = panels.first() else { return };
    let strip = Strip {
        first: first.clone(),
        rows: vec![panels.to_vec()],
        rect,
        attached: false,
        embedded: true,
        left: rect.center().x < ui.ctx().content_rect().center().x,
        offset: ui.data(|d| d.get_temp::<f32>(key("scroll").with(first))).unwrap_or(0.0),
    };
    ui.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Strip>>(key("embedded-strips")).push(strip.clone());
        d.get_temp_mut_or_default::<Vec<Strip>>(key("strips")).push(strip.clone());
    });
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("compact-group", first)).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    paint_strip(app, &mut child, &strip, commands);
    // Adjacent parts meet in a one-physical-pixel gap. Both strokes occupy that same
    // gap, and an outside edge coincides with the shared floating frame's outline.
    ui.painter().with_clip_rect(ui.clip_rect().expand(ui.ctx().pixels_per_point().recip()).intersect(ui.ctx().content_rect())).rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(ui.ctx().pixels_per_point().recip(), crate::theme::Tokens::get(ui.ctx()).card_border),
        egui::StrokeKind::Outside,
    );
}
/// Prefer the exterior of adjacent compact parts; fall back to the side with less overlap.
fn opens_right(ctx: &egui::Context, strip: &Strip, strips: &[Strip], width: f32, bounds: Rect) -> bool {
    // Dock side is authoritative: scoring clamped candidates can otherwise put the
    // editor over its own right-edge icon merely to avoid the Layers body behind it.
    if strip.attached {
        return strip.left;
    }
    let left_space = strip.rect.left() - bounds.left();
    let right_space = bounds.right() - strip.rect.right();
    if (left_space >= width) != (right_space >= width) {
        return right_space >= width;
    }
    let candidates = [
        Rect::from_min_size(pos2((strip.rect.left() - width).max(bounds.left()), strip.rect.top()), vec2(width, strip.rect.height())),
        Rect::from_min_size(pos2(strip.rect.right().min((bounds.right() - width).max(bounds.left())), strip.rect.top()), vec2(width, strip.rect.height())),
    ];
    let obstacles: Vec<_> = strips
        .iter()
        .filter(|s| s.first != strip.first)
        .map(|s| s.rect)
        .chain(super::last_bodies(ctx).into_iter().filter(|(id, _)| id != super::CANVAS && !is_spacer(id) && !strip.panels().any(|p| p == id)).map(|(_, r)| r))
        .collect();
    let overlap = |rect: Rect| {
        obstacles
            .iter()
            .map(|other| {
                let r = rect.intersect(*other);
                if r.is_positive() { r.area() } else { 0.0 }
            })
            .sum::<f32>()
    };
    let [left, right] = candidates.map(overlap);
    if (left - right).abs() > 0.1 { right < left } else { strip.left }
}
fn expansion(app: &PhotocraftApp, ctx: &egui::Context, strip: &Strip) -> Value {
    let mut params = json!({"panel":strip.first,"operation":"expand"});
    if !strip.attached
        && !strip.embedded
        && let Some(window) = app.ui.docking.as_ref().and_then(|w| w.layout.floating.iter().find(|w| w.contains(&strip.first)))
    {
        let all = strips(app, ctx, &app.ui.docking.as_ref().map_or_else(Layout::default, |w| w.layout.clone()));
        let bounds = ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap_or_else(|| ctx.content_rect());
        let width = window.rect[2];
        let x = if opens_right(ctx, strip, &all, (width - strip.rect.width()).max(0.0), bounds) { strip.rect.left() } else { strip.rect.right() - width };
        params["x"] = json!(x.clamp(bounds.left(), (bounds.right() - width).max(bounds.left())));
        params["y"] = json!(strip.rect.top());
    }
    params
}

pub(super) fn button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, glyph: &str, label: &str) -> egui::Response {
    rounded_button(ui, rect, id, glyph, label, egui::CornerRadius::same(2))
}
fn rounded_button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, glyph: &str, label: &str, corners: egui::CornerRadius) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, corners, t.card_border);
    }
    paint_header_glyph(ui.painter(), rect, glyph, t.text_dim);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label)
}
fn begin(app: &PhotocraftApp, ui: &egui::Ui, response: &egui::Response, panel: &str, whole: bool, view: DragView, source: Rect) {
    if app.session.prefs().workspace_locked || !response.drag_started() || ui.data(|d| d.get_temp::<bool>(key("suppressed"))).unwrap_or(false) {
        return;
    }
    let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(source.center());
    ui.data_mut(|d| d.insert_temp(key("drag"), Drag { panel: panel.into(), whole, view, press, source, generation: app.ui.docking_generation }));
}
fn header_corners(strip: &Strip, rect: Rect, radius: f32) -> egui::CornerRadius {
    let mut corners = egui::CornerRadius::same(2);
    if !strip.attached {
        let outer = egui::CornerRadius::from(radius);
        if rect.left() <= strip.rect.left() {
            corners.nw = outer.nw;
        }
        if rect.right() >= strip.rect.right() {
            corners.ne = outer.ne;
        }
    }
    corners
}

fn header(app: &PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let top = Rect::from_min_size(strip.rect.min + vec2(0.0, strip.header_inset()), vec2(strip.rect.width(), HEADER + GRIP));
    let scale = ui.ctx().pixels_per_point();
    let pair = (COMPACT_COLUMN_WIDTH * scale).ceil() / scale;
    let x = if strip.attached { strip.rect.center().x - 10.0 } else { strip.rect.right() - pair };
    let button_width = if strip.attached { 20.0 } else { (pair * scale * 0.5).floor() / scale };
    let arrow = Rect::from_min_size(pos2(x, top.top()), vec2(button_width, HEADER));
    let response = rounded_button(
        ui,
        arrow,
        key("expand").with(&strip.first),
        if if strip.embedded { horizontal_peer_left(app, &strip.first).unwrap_or(strip.left) } else { strip.left } {
            "chevrons-right"
        } else {
            "chevrons-left"
        },
        tl!("Expand Panel Group"),
        header_corners(strip, arrow, t.radius_sm),
    );
    if response.clicked() {
        commands.push(("window.panel.group", expansion(app, ui.ctx(), strip)));
    }
    if !strip.attached {
        let close = Rect::from_min_max(pos2(arrow.right(), top.top()), pos2(strip.rect.right(), top.top() + HEADER));
        let response = rounded_button(ui, close, key("close").with(&strip.first), "x", tl!("Close Tab Group"), header_corners(strip, close, t.radius_sm));
        if response.clicked() {
            commands.push(("window.panel.group", json!({"panel":strip.first,"operation":if strip.embedded {"close-row"} else {"close-window"}})));
        }
    }
    let grip = Rect::from_min_max(pos2(top.left(), top.top() + HEADER), top.max);
    let response = ui.interact(grip, key("grip").with(&strip.first), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !app.session.prefs().workspace_locked, tl!("Move Panel Group")));
    if !app.session.prefs().workspace_locked {
        response.clone().on_hover_cursor(egui::CursorIcon::Grab);
    }
    let half = compact_grip_width(strip.rect.width()) * 0.5;
    for y in [-1.5, 1.5] {
        ui.painter().line_segment([grip.center() + vec2(-half, y), grip.center() + vec2(half, y)], egui::Stroke::new(1.0, t.text_faint.gamma_multiply(0.3)));
    }
    begin(app, ui, &response, &strip.first, true, DragView::Palette, strip.rect);
    if response.double_clicked() {
        commands.push(("window.panel.group", expansion(app, ui.ctx(), strip)));
    }
}
fn resize(app: &PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    if app.session.prefs().workspace_locked || strip.embedded {
        return;
    }
    for left in [true, false] {
        if strip.attached && left == strip.left {
            continue;
        }
        let x = if left { strip.rect.left() } else { strip.rect.right() };
        let pixel = ui.ctx().pixels_per_point().recip();
        let center = (x / pixel).round() * pixel + if left { pixel * 0.5 } else { -pixel * 0.5 };
        let rect = Rect::from_min_max(pos2(center - pixel * 0.5, strip.rect.top() + HEADER + GRIP), pos2(center + pixel * 0.5, strip.rect.bottom()));
        let result = craft_ui::layout::ResizeHandle::new(
            key("resize").with((&strip.first, left)),
            craft_ui::layout::SplitAxis::Horizontal,
            rect,
            tl!("Resize panels"),
            craft_ui::layout::ResizeHandleStyle {
                hovered: egui::Color32::TRANSPARENT,
                dragged: egui::Color32::TRANSPARENT,
                focused: crate::theme::Tokens::get(ui.ctx()).accent,
            },
        )
        .hit_margin(2.5)
        .value(strip.rect.width(), COMPACT_COLUMN_WIDTH..=240.0)
        .reversed(left)
        .show(ui);
        let response = result.response;
        if result.delta != 0.0 && !response.dragged() {
            let width = (strip.rect.width() + if left { -result.delta } else { result.delta }).clamp(COMPACT_COLUMN_WIDTH, 240.0);
            commands.push(("window.panel.group", json!({"panel":strip.first,"operation":"strip-width","width":width})));
            if left
                && !strip.attached
                && !strip.embedded
                && let Some(w) = app.ui.docking.as_ref().and_then(|w| w.layout.floating.iter().find(|w| w.contains(&strip.first)))
            {
                let mut rect = w.rect;
                rect[0] = strip.rect.right() - width;
                commands.push(("window.panel.layout", json!({"action":Action::MoveFloating {panel:strip.first.clone(),rect}})));
            }
        }
        if response.drag_started() {
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
            ui.data_mut(|d| {
                d.insert_temp(
                    key("resize"),
                    Resize {
                        panel: strip.first.clone(),
                        rect: strip.rect,
                        press,
                        left,
                        attached: strip.attached,
                        height: false,
                        minimum: 100.0,
                        generation: app.ui.docking_generation,
                    },
                )
            });
        }
        if response.drag_stopped()
            && let Some(start) = ui.data(|d| d.get_temp::<Resize>(key("resize")))
        {
            let dx = ui.ctx().pointer_interact_pos().unwrap_or(start.press).x - start.press.x;
            let width =
                if start.attached { strip.rect.width() } else { (start.rect.width() + if start.left { -dx } else { dx }).clamp(COMPACT_COLUMN_WIDTH, 240.0) };
            commands.push(("window.panel.group", json!({"panel":start.panel,"operation":"strip-width","width":width})));
            if start.left
                && !start.attached
                && !strip.embedded
                && let Some(w) = app.ui.docking.as_ref().and_then(|w| w.layout.floating.iter().find(|w| w.contains(&start.panel)))
            {
                let mut rect = w.rect;
                rect[0] = start.rect.right() - width;
                commands.push(("window.panel.layout", json!({"action":Action::MoveFloating {panel:start.panel,rect}})));
            }
        }
        if response.dragged() {
            ui.ctx().request_repaint();
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<Resize>(key("resize")));
        }
    }
}

pub(super) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, layout: &Layout<String>, bounds: Rect, commands: &mut Vec<(&'static str, Value)>) {
    let ctx = ui.ctx().clone();
    let drawn = strips(app, &ctx, layout);
    ctx.data_mut(|d| d.insert_temp(key("strips"), drawn.clone()));
    let t = crate::theme::Tokens::get(&ctx);
    for strip in &drawn {
        if !strip.attached && !strip.embedded {
            egui::Area::new(key("float").with(&strip.first))
                .order(egui::Order::Middle)
                .fixed_pos(strip.rect.min)
                .movable(false)
                .fade_in(false)
                .show(&ctx, |ui| paint_strip(app, ui, strip, commands));
        }
    }
    column_height_handles(app, ui, layout, bounds, commands);
    let pixel = ctx.pixels_per_point().recip();
    if compat::columns(layout).iter().any(|(side, _)| *side == Side::Start) || crate::toolbar_docking::docked_this_frame(&ctx) {
        let start = if crate::direction::is_rtl() { bounds.right() } else { bounds.left() };
        let x = (start / pixel).round() * pixel + if crate::direction::is_rtl() { -pixel * 0.5 } else { pixel * 0.5 };
        ui.painter().vline(x, bounds.y_range(), egui::Stroke::new(pixel, t.card_border));
    }
    let columns = ctx.data(|d| d.get_temp::<Vec<(String, Rect, Side)>>(key("columns"))).unwrap_or_default();
    for (first, rect, side) in columns {
        let left = (side == Side::Start) != crate::direction::is_rtl();
        let x = if left { rect.right() } else { rect.left() };
        let x = (x / pixel).round() * pixel + if left { -pixel * 0.5 } else { pixel * 0.5 };
        let stock = DockArea::<String>::hit_id(area_id(), &Hit::Divider(craft_ui::docking::Divider::Column(first.clone())));
        let tint = if keyboard_resize_focus(&ctx, &first, !left) || keyboard_handle_focus(&ctx, stock) { t.accent } else { t.card_border };
        ui.painter().vline(x, rect.y_range(), egui::Stroke::new(pixel, tint));
    }
    for (hit, rect) in DockArea::<String>::hit_rects(&ctx, area_id()) {
        if let Hit::Divider(craft_ui::docking::Divider::Group(_)) = hit {
            let y = (rect.center().y / pixel).round() * pixel - pixel * 0.5;
            let tint = if keyboard_handle_focus(&ctx, DockArea::<String>::hit_id(area_id(), &hit)) { t.accent } else { t.card_border };
            ui.painter().hline(rect.x_range(), y, egui::Stroke::new(pixel, tint));
        }
    }
    part_resize::show(app, ui, layout);
    flyout(app, &ctx, layout, &drawn, commands);
    drag(app, &ctx, layout, bounds, commands);
    animate_landing(app, &ctx, &drawn);
    let mut areas: Vec<_> = drawn.iter().map(|s| s.rect).collect();
    areas.extend(super::last_bodies(&ctx).into_iter().filter(|(id, _)| id != super::CANVAS).map(|(_, rect)| rect));
    if app.ui.docking_flyout.is_some()
        && let Some(rect) = ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect")))
    {
        areas.push(rect);
    }
    areas.extend(DockArea::<String>::hit_rects(&ctx, area_id()).into_iter().filter_map(|(h, r)| matches!(h, Hit::Border(_) | Hit::ColumnBar(_)).then_some(r)));
    crate::panel_visibility::areas(&ctx, areas);
}
fn icon_list(app: &mut PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let view = Rect::from_min_max(pos2(strip.rect.left(), strip.rect.top() + strip.header_inset() + HEADER + GRIP), strip.rect.max - vec2(0.0, 1.5));
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt((&strip.first, "icon-list")).max_rect(view).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(view.intersect(ui.clip_rect()));
    child.style_mut().spacing.scroll = egui::style::ScrollStyle {
        bar_width: 3.0,
        floating_width: 2.0,
        foreground_color: !t.dark(),
        bar_inner_margin: 1.0,
        dormant_handle_opacity: 0.35,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        ..egui::style::ScrollStyle::floating()
    };
    let output =
        egui::ScrollArea::vertical().id_salt(key("scroll").with(&strip.first)).auto_shrink([false, false]).show_viewport(&mut child, |ui, viewport| {
            ui.set_min_size(vec2(strip.rect.width(), strip.panels().count() as f32 * ROW));
            let mut visible = strip.clone();
            visible.offset = viewport.top();
            for (id, cell) in visible.cells() {
                let r = ui.interact(cell.shrink2(vec2(INSET, 1.5)), key("icon").with(&id), Sense::click_and_drag());
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, normalize_label(&id)));
                let on = app.ui.docking_flyout.as_deref() == Some(id.as_str());
                if on || r.hovered() {
                    ui.painter().rect_filled(r.rect, t.radius_sm, t.card_border);
                }
                if on || r.has_focus() && ui.data(|d| d.get_temp::<bool>(key("keyboard-input"))).unwrap_or(false) {
                    ui.painter().rect_stroke(r.rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                }
                let icon = Rect::from_center_size(
                    pos2(cell.left() + (COMPACT_COLUMN_WIDTH * ui.ctx().pixels_per_point()).ceil() / ui.ctx().pixels_per_point() * 0.5, cell.center().y),
                    Vec2::splat(20.0),
                );
                crate::icons::paint(ui, icon, panel_icon(&id), 20.0, t.text_dim);
                let available = cell.right() - icon.right() - 12.0;
                let minimum = if available > 0.0 {
                    ui.painter()
                        .layout_no_wrap(format!("{}…", normalize_label(&id).chars().take(2).collect::<String>()), egui::FontId::proportional(12.0), t.text)
                        .size()
                        .x
                } else {
                    f32::INFINITY
                };
                if available >= minimum {
                    let galley = ui.painter().layout(normalize_label(&id), egui::FontId::proportional(12.0), t.text, available);
                    let mut job = (*galley.job).clone();
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job.wrap.max_width = available;
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    ui.painter().galley(pos2(icon.right() + 6.0, cell.center().y - galley.size().y * 0.5), galley, t.text);
                }
                begin(app, ui, &r, &id, false, DragView::Palette, cell);
                if r.clicked() {
                    if on {
                        app.ui.docking_flyout = None;
                    } else {
                        commands.push(("window.panel.group", json!({"panel":id,"operation":"peek"})));
                    }
                }
                r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(normalize_label(&id));
            }
        });
    ui.data_mut(|d| d.insert_temp(key("scroll").with(&strip.first), output.state.clamped_offset().y));
}

fn normalize_label(id: &str) -> String {
    PANELS.iter().find(|p| p.0 == id).map(|p| tl!(p.1).to_string()).unwrap_or_else(|| id.into())
}

#[derive(Clone)]
struct EditorPlacement {
    panel: String,
    right: bool,
    top: f32,
    anchor: Rect,
    bounds: Rect,
    generation: u64,
}

fn normal_editor_size(app: &PhotocraftApp, layout: &Layout<String>, panel: &str) -> [f32; 2] {
    app.ui
        .panel_presentation
        .flyouts
        .get(panel)
        .or_else(|| app.ui.panel_presentation.floating_sizes.get(panel))
        .copied()
        .or_else(|| {
            layout
                .floating
                .iter()
                .find(|w| w.contains(panel) && !compat::independent_window(w))
                .map(|w| [w.rect[2], w.rect[3]])
                .filter(|s| s[0] >= 220.0 && s[1] >= 120.0)
        })
        .unwrap_or([320.0, 400.0])
}
fn preferred_editor_size(app: &PhotocraftApp, layout: &Layout<String>, panel: &str) -> [f32; 2] {
    if app.ui.panel_presentation.flyouts.contains_key(panel) || app.ui.panel_presentation.floating_sizes.contains_key(panel) || floats(layout, panel) {
        normal_editor_size(app, layout, panel)
    } else {
        compat::columns(layout)
            .into_iter()
            .find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == panel)))
            .map_or([320.0, 400.0], |(_, c)| [c.width, 440.0])
    }
}
fn editor_bounds(ctx: &egui::Context) -> Rect {
    ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap_or_else(|| ctx.content_rect())
}
fn editor_geometry(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, strip: &Strip, strips: &[Strip], panel: &str) -> Option<(Rect, bool)> {
    let bounds = editor_bounds(ctx);
    if !bounds.is_finite() || bounds.width() < 80.0 || bounds.height() < 80.0 {
        return None;
    }
    let [width, height] = preferred_editor_size(app, layout, panel);
    let placement = ctx
        .data(|d| d.get_temp::<EditorPlacement>(key("editor-placement")))
        .filter(|p| p.panel == panel && p.anchor == strip.rect && p.bounds == bounds && p.generation == app.ui.docking_generation)
        .unwrap_or_else(|| {
            let top = strip.cells().into_iter().find(|(p, _)| p == panel).map_or(strip.rect.top(), |(_, r)| r.top());
            EditorPlacement {
                panel: panel.into(),
                right: opens_right(ctx, strip, strips, width, bounds),
                top: top.min(bounds.bottom() - height.min(bounds.height())).max(bounds.top()),
                anchor: strip.rect,
                bounds,
                generation: app.ui.docking_generation,
            }
        });
    let available = if placement.right { bounds.right() - strip.rect.right() } else { strip.rect.left() - bounds.left() };
    if available < 80.0 {
        return None;
    }
    let width = width.max(220.0).min(available);
    let height = height.max(120.0).min((bounds.bottom() - placement.top).max(0.0));
    // The owner and editor share one physical border band rather than two facing outlines.
    let anchor = if placement.right { strip.rect.right() - ctx.pixels_per_point().recip() } else { strip.rect.left() + ctx.pixels_per_point().recip() };
    let x = if placement.right { anchor } else { anchor - width };
    let mut rect = Rect::from_min_size(pos2(x, placement.top), vec2(width, height));
    if let Some(start) = ctx.data(|d| d.get_temp::<EditorResize>(key("editor-resize"))).filter(|r| r.panel == panel) {
        let delta = ctx.pointer_interact_pos().unwrap_or(start.press) - start.press;
        let width = (start.rect.width()
            + if start.bottom {
                0.0
            } else if start.left {
                -delta.x
            } else {
                delta.x
            })
        .max(220.0)
        .min(available);
        let height = (start.rect.height() + if start.bottom { delta.y } else { 0.0 }).max(120.0).min((bounds.bottom() - placement.top).max(0.0));
        rect = Rect::from_min_size(pos2(if placement.right { anchor } else { anchor - width }, placement.top), vec2(width, height));
    }
    let right = placement.right;
    ctx.data_mut(|d| d.insert_temp(key("editor-placement"), placement));
    Some((rect, right))
}

fn flyout(app: &mut PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, strips: &[Strip], commands: &mut Vec<(&'static str, Value)>) {
    let Some(id) = app.ui.docking_flyout.clone() else {
        ctx.data_mut(|d| {
            d.remove::<EditorPlacement>(key("editor-placement"));
            d.remove::<Rect>(key("flyout-rect"));
        });
        return;
    };
    let Some(strip) = strips.iter().find(|s| s.panels().any(|p| p == &id)) else {
        app.ui.docking_flyout = None;
        return;
    };
    let Some((rect, opens_right)) = editor_geometry(app, ctx, layout, strip, strips, &id) else {
        app.ui.docking_flyout = None;
        return;
    };
    ctx.data_mut(|d| d.insert_temp(key("flyout-rect"), rect));
    if !egui::Popup::is_any_open(ctx)
        && ctx.input(|i| {
            i.key_pressed(egui::Key::Escape)
                || (app.session.prefs().workspace.auto_collapse_icon_panels
                    && i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !rect.expand(4.0).contains(p) && !strip.rect.contains(p)))
        })
    {
        app.ui.docking_flyout = None;
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    egui::Area::new(key("flyout")).order(egui::Order::Middle).fixed_pos(rect.min).movable(false).fade_in(false).show(ctx, |ui| {
        if super::modal_open(app, ui.ctx()) {
            ui.disable();
        }
        ui.set_min_size(rect.size());
        ui.set_max_size(rect.size());
        ui.painter().rect_filled(rect, 0.0, t.card);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0));
        super::contract::single_header(ui, header, &normalize_label(&id));
        let arrow = Rect::from_min_size(pos2(header.right() - 42.0, header.top() + INSET), Vec2::splat(20.0));
        if button(ui, arrow, key("flyout-collapse"), if opens_right { "chevrons-left" } else { "chevrons-right" }, tl!("Collapse Panel Group")).clicked() {
            app.ui.docking_flyout = None;
        }
        let menu = button(ui, arrow.translate(vec2(20.0, 0.0)), key("flyout-menu"), "ellipsis", tl!("Panel menu"));
        egui::Popup::menu(&menu).show(|ui| panel_menu(app, ui, &id));
        let response = ui.interact(Rect::from_min_max(header.min, pos2(arrow.left(), header.bottom())), key("flyout-drag"), Sense::drag());
        begin(app, ui, &response, &id, false, DragView::Editor, rect);
        let content = Rect::from_min_max(pos2(rect.left() + 6.0, header.bottom()), rect.max - vec2(6.0, 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt((&id, "flyout-body")).max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(content);
        body(app, &mut child, &id);
        editor_resize(app, ui, &id, rect, !opens_right, commands);
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(ctx.pixels_per_point().recip(), t.card_border), egui::StrokeKind::Inside);
    });
}

fn editor_resize(app: &PhotocraftApp, ui: &mut egui::Ui, panel: &str, rect: Rect, left: bool, commands: &mut Vec<(&'static str, Value)>) {
    if app.session.prefs().workspace_locked || !ui.is_enabled() {
        return;
    }
    for bottom in [false, true] {
        let band = if bottom {
            Rect::from_min_max(pos2(rect.left() + 3.0, rect.bottom() - 3.0), rect.max)
        } else {
            let x = if left { rect.left() } else { rect.right() };
            Rect::from_min_max(pos2(x - 2.0, rect.top() + 26.0), pos2(x + 2.0, rect.bottom() - 3.0))
        };
        let response = ui.interact(band, key("editor-edge").with((panel, bottom)), Sense::drag()).on_hover_cursor(if bottom {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        });
        if response.drag_started() {
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(band.center());
            ui.data_mut(|d| {
                d.insert_temp(key("editor-resize"), EditorResize { panel: panel.into(), rect, press, left, bottom, generation: app.ui.docking_generation })
            });
        }
        if response.dragged() {
            ui.ctx().request_repaint();
        }
        if response.drag_stopped() && ui.data(|d| d.get_temp::<EditorResize>(key("editor-resize"))).is_some() {
            commands.push(("window.panel.group", json!({"panel":panel,"operation":"flyout-size","width":rect.width(),"height":rect.height()})));
            ui.data_mut(|d| d.remove::<EditorResize>(key("editor-resize")));
        }
    }
}

#[derive(Clone)]
struct Landing {
    target: Target<String>,
    marker: Rect,
    compact: bool,
}
fn horizontal_landing(ctx: &egui::Context, p: Pos2) -> Option<Landing> {
    let canvas = super::last_bodies(ctx).into_iter().find(|(id, _)| id == super::CANVAS)?.1;
    for bottom in [false, true] {
        let y = if bottom { canvas.bottom() } else { canvas.top() };
        if canvas.x_range().contains(p.x) && (p.y - y).abs() <= 12.0 {
            let pixel = ctx.pixels_per_point().recip();
            return Some(Landing {
                target: Target::At { anchor: super::CANVAS.into(), placement: Placement::Edge(if bottom { Zone::Bottom } else { Zone::Top }) },
                marker: Rect::from_min_max(pos2(canvas.left(), y - pixel * 0.5), pos2(canvas.right(), y + pixel * 0.5)),
                compact: false,
            });
        }
    }
    None
}

fn landing(ctx: &egui::Context, source: &str, whole: bool, p: Pos2) -> Option<Landing> {
    let strips = ctx.data(|d| d.get_temp::<Vec<Strip>>(key("strips"))).unwrap_or_default();
    let compact = strips.iter().any(|s| s.panels().any(|id| id == source));
    let covered_by_float = strips.iter().any(|s| !s.attached && !s.panels().any(|id| id == source) && s.rect.contains(p));
    // Both full/stock and compact sources resolve against these exact fitted boundaries.
    let columns = ctx.data(|d| d.get_temp::<Vec<(String, Rect, Side)>>(key("columns"))).unwrap_or_default();
    if !covered_by_float && let Some(target) = horizontal_landing(ctx, p) {
        return Some(target);
    }
    let mut edges = Vec::new();
    for (anchor, rect, side) in &columns {
        if covered_by_float || anchor == source || whole && strips.iter().any(|s| s.first == *anchor && s.panels().any(|id| id == source)) {
            continue;
        }
        if !rect.y_range().contains(p.y) {
            continue;
        }
        let left = (*side == Side::Start) != crate::direction::is_rtl();
        for after in [false, true] {
            let x = if left != after { rect.left() } else { rect.right() };
            let distance = (p.x - x).abs();
            if distance <= 18.0 {
                edges.push((
                    distance,
                    Landing {
                        target: Target::At { anchor: anchor.clone(), placement: Placement::Column { after } },
                        marker: Rect::from_min_max(pos2(x - 1.0, rect.top()), pos2(x + 1.0, rect.bottom())),
                        compact,
                    },
                ));
            }
        }
    }
    if let Some((_, edge)) = edges.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
        return Some(edge);
    }
    for strip in strips.iter().rev() {
        if whole && strip.panels().any(|id| id == source) {
            continue;
        }
        if !strip.attached && !strip.panels().any(|id| id == source) && (strip.rect.top() - 10.0..=strip.rect.bottom() + 10.0).contains(&p.y) {
            for left in [true, false] {
                let x = if left { strip.rect.left() } else { strip.rect.right() };
                if (p.x - x).abs() <= 8.0 {
                    let pixel = ctx.pixels_per_point().recip();
                    return Some(Landing {
                        target: Target::At { anchor: strip.first.clone(), placement: Placement::Split(if left { Zone::Left } else { Zone::Right }) },
                        marker: Rect::from_min_max(
                            pos2(if left { strip.rect.left() } else { strip.rect.right() } - pixel, strip.rect.top()),
                            pos2(if left { strip.rect.left() } else { strip.rect.right() } + pixel, strip.rect.bottom()),
                        ),
                        compact: false,
                    });
                }
            }
        }
        if p.y < strip.rect.top() + strip.header_inset() + HEADER + GRIP - 2.0 || !strip.rect.expand2(vec2(8.0, 0.0)).contains(p) {
            continue;
        }
        let cells = strip.cells();
        let (anchor, cell) =
            cells.iter().filter(|(id, _)| id != source).min_by(|a, b| (a.1.center().y - p.y).abs().total_cmp(&(b.1.center().y - p.y).abs()))?;
        let below = p.y >= cell.center().y;
        let y = if below { cell.bottom() } else { cell.top() };
        let placement = if strip.attached { Placement::Group { below, height: None } } else { Placement::Split(if below { Zone::Bottom } else { Zone::Top }) };
        return Some(Landing {
            target: Target::At { anchor: anchor.clone(), placement },
            marker: Rect::from_min_size(pos2(strip.rect.left(), y - 1.0), vec2(strip.rect.width(), 2.0)),
            compact: true,
        });
    }
    if covered_by_float {
        return None;
    }
    if let Some(bounds) = ctx.data(|d| d.get_temp::<Rect>(key("bounds"))) {
        for left in [true, false] {
            let x = if left { bounds.left() } else { bounds.right() };
            if (p.x - x).abs() <= 18.0 && bounds.y_range().contains(p.y) {
                let side = if left != crate::direction::is_rtl() { Side::Start } else { Side::End };
                // Existing docks are handled by their real boundaries above.
                if !columns.iter().any(|(_, _, s)| *s == side) {
                    return Some(Landing {
                        target: Target::NewDock { side },
                        marker: Rect::from_min_max(pos2(x - 1.0, bounds.top()), pos2(x + 1.0, bounds.bottom())),
                        compact,
                    });
                }
            }
        }
    }
    None
}
// Stock tabs retain the shared renderer's precise insertion/reorder resolver.
// Compact sources use this fallback only when no column/compact landing owns the pointer.
/// One full surface owns the pointer, even when it deliberately offers no target.
/// Tab insertion has a single physical marker per logical slot; body centers never merge tabs.
fn full_landing(ctx: &egui::Context, layout: &Layout<String>, source: &str, whole: bool, p: Pos2, compact: bool) -> (Option<Landing>, bool) {
    let hits = DockArea::<String>::hit_rects(ctx, area_id());
    let mut bodies = super::last_bodies(ctx);
    // A collapsed root tab bar has no body callback; its fitted header still owns insertion slots.
    for (id, frame) in ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap_or_default() {
        if id == super::CANVAS || floats(layout, &id) || in_dock_column(layout, &id) {
            continue;
        }
        let members = group_members(layout, &id);
        if !bodies.iter().any(|(p, _)| members.contains(p)) {
            bodies.push((id, Rect::from_min_max(pos2(frame.left(), frame.bottom()), frame.max)));
        }
    }
    bodies.reverse();
    bodies.sort_by_key(|(id, _)| !floats(layout, id));
    let rtl = crate::direction::is_rtl();
    let pixel = ctx.pixels_per_point().recip();
    for (id, body) in bodies.into_iter().filter(|(id, _)| id != super::CANVAS && id != super::document_header::ID && !is_spacer(id)) {
        let members = group_members(layout, &id);
        if whole && members.iter().any(|id| id == source) {
            continue;
        }
        let Some(anchor) = members.iter().find(|id| id.as_str() != source).cloned() else { continue };
        let tabs: Vec<_> = members.iter().filter_map(|id| hits.iter().find_map(|(h, r)| matches!(h,Hit::Tab(p) if p==id).then_some((id, r)))).collect();
        let top = tabs.first().map_or(body.top() - super::contract::FULL_HEADER_HEIGHT, |(_, r)| r.top());
        let frame = Rect::from_min_max(pos2(body.left(), top), body.max);
        if !frame.expand2(vec2(6.0, 8.0)).contains(p) {
            continue;
        }
        let floating = floats(layout, &anchor);
        let header = Rect::from_min_max(frame.min, pos2(frame.right(), body.top()));
        let bottom_edge = body.height() <= 0.0 && p.y >= frame.bottom() - 5.0;
        if header.expand2(vec2(6.0, 0.0)).contains(p) && p.y > frame.top() + 5.0 && !bottom_edge {
            let mut slots = Vec::new();
            for (index, (tab, rect)) in tabs.iter().enumerate() {
                if tab.as_str() == source {
                    continue;
                }
                let leading = if rtl { rect.right() } else { rect.left() };
                let trailing = if rtl { rect.left() } else { rect.right() };
                let next = tabs.iter().skip(index + 1).map(|(id, _)| (*id).clone()).find(|id| id != source);
                for (x, before) in [(leading, Some((*tab).clone())), (trailing, next)] {
                    if (p.x - x).abs() <= 6.0 {
                        slots.push(((p.x - x).abs(), x, before));
                    }
                }
            }
            if let Some((last, last_rect)) = tabs.last() {
                let trailing = if rtl { last_rect.left() } else { last_rect.right() };
                let controls = if floating { 66.0 } else { 46.0 };
                let in_empty = if rtl { p.x < trailing && p.x > frame.left() + controls } else { p.x > trailing && p.x < frame.right() - controls };
                if in_empty && last.as_str() != source {
                    slots.push(((p.x - trailing).abs(), trailing, None));
                }
            }
            if let Some((_, x, before)) = slots.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
                let x = (x / pixel).round() * pixel;
                return (
                    Some(Landing {
                        target: Target::At { anchor, placement: Placement::Tab { before } },
                        marker: Rect::from_min_max(pos2(x - pixel * 0.5, header.top()), pos2(x + pixel * 0.5, header.bottom())),
                        compact: false,
                    }),
                    true,
                );
            }
            return (None, true);
        }
        let zone = if p.y <= frame.top() + 5.0 {
            Some(Zone::Top)
        } else if p.y >= frame.bottom() - 14.0 {
            Some(Zone::Bottom)
        } else if p.y >= body.top() && p.x <= frame.left() + 14.0 {
            Some(Zone::Left)
        } else if p.y >= body.top() && p.x >= frame.right() - 14.0 {
            Some(Zone::Right)
        } else {
            None
        };
        let Some(zone) = zone else { return (None, floating) };
        let horizontal = matches!(zone, Zone::Left | Zone::Right);
        if compact && !horizontal {
            return (None, true);
        }
        let placement = if in_dock_column(layout, &anchor) && !horizontal {
            Placement::Group { below: zone == Zone::Bottom, height: None }
        } else {
            match Placement::for_zone(layout, &anchor, zone, rtl) {
                Ok(p) => p,
                Err(_) => return (None, true),
            }
        };
        let mut marker_frame = frame;
        if horizontal
            && in_dock_column(layout, &anchor)
            && let Some((_, column)) = compat::columns(layout).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.contains(&anchor)))
            && let Some(first) = column.groups.first().and_then(|g| g.panels.first())
            && let Some((_, rect, _)) =
                ctx.data(|d| d.get_temp::<Vec<(String, Rect, Side)>>(key("columns"))).unwrap_or_default().iter().find(|(id, _, _)| id == first)
        {
            marker_frame = *rect;
        }
        let marker = match zone {
            Zone::Left | Zone::Right => {
                let x = if zone == Zone::Left { marker_frame.left() } else { marker_frame.right() };
                Rect::from_min_max(pos2(x - pixel * 0.5, marker_frame.top()), pos2(x + pixel * 0.5, marker_frame.bottom()))
            }
            _ => {
                let y = if zone == Zone::Top { frame.top() } else { frame.bottom() };
                Rect::from_min_max(pos2(frame.left(), y - pixel * 0.5), pos2(frame.right(), y + pixel * 0.5))
            }
        };
        return (Some(Landing { target: Target::At { anchor, placement }, marker, compact }), floating);
    }
    (None, false)
}

/// Compact sources can join the same full body edges as stock tabs.
fn resolved_landing(ctx: &egui::Context, layout: &Layout<String>, source: &str, whole: bool, p: Pos2, compact: bool) -> Option<Landing> {
    let (full, exclusive) = full_landing(ctx, layout, source, whole, p, compact);
    if exclusive || full.is_some() {
        return full;
    }
    super::document_header::landing(ctx, p)
        .map(|(target, marker)| Landing { target, marker, compact: false })
        .or_else(|| landing(ctx, source, whole, p))
        .or(full)
}
fn detached_target(app: &PhotocraftApp, layout: &Layout<String>, panel: &str, rect: Rect, bounds: Rect, pointer: Pos2) -> Target<String> {
    let mut action = Action::MoveGroup { panel: panel.into(), to: Target::Float { rect: Some([rect.left(), rect.top(), rect.width(), rect.height()]) } };
    restore_detached_rect(app, layout, &mut action, bounds, Some(pointer));
    match action {
        Action::MoveGroup { to, .. } => to,
        _ => Target::Float { rect: None },
    }
}

/// A horizontal dock's drawn width is not a sensible new floating window size.
pub(super) fn restore_detached_rect(app: &PhotocraftApp, layout: &Layout<String>, action: &mut Action<String>, bounds: Rect, pointer: Option<Pos2>) {
    let (panel, rect) = match action {
        Action::Float { panel, rect } => (panel, rect),
        Action::MovePanel { panel, to: Target::Float { rect: Some(rect) } }
        | Action::MoveGroup { panel, to: Target::Float { rect: Some(rect) } }
        | Action::MoveColumn { panel, to: Target::Float { rect: Some(rect) } }
        | Action::MoveWindow { panel, to: Target::Float { rect: Some(rect) } } => (panel, rect),
        _ => return,
    };
    if floats(layout, panel) {
        return;
    }
    let [width, height] = if compact_source(app, layout, panel) {
        [
            app.ui.panel_presentation.widths.get(panel).copied().unwrap_or(COMPACT_COLUMN_WIDTH),
            HEADER + GRIP + ROW * group_members(layout, panel).len() as f32 + 1.5,
        ]
    } else {
        normal_editor_size(app, layout, panel)
    };
    if let Some(pointer) = pointer {
        // Keep the grab point in the restored window when the horizontal dock was wider.
        if rect[2] > width {
            rect[0] = pointer.x - ((pointer.x - rect[0]) / rect[2]).clamp(0.0, 1.0) * width;
        }
        if rect[3] > height {
            rect[1] = pointer.y - ((pointer.y - rect[1]) / rect[3]).clamp(0.0, 1.0) * height;
        }
    }
    rect[2] = width.min(bounds.width()).max(1.0);
    rect[3] = height.min(bounds.height()).max(1.0);
    rect[0] = rect[0].clamp(bounds.left(), (bounds.right() - rect[2]).max(bounds.left()));
    rect[1] = rect[1].clamp(bounds.top(), (bounds.bottom() - rect[3]).max(bounds.top()));
}

pub(super) fn resolve_drag(
    app: &PhotocraftApp,
    ctx: &egui::Context,
    request: &DropRequest<'_, String>,
    commands: &mut Vec<(&'static str, Value)>,
) -> Option<DropFeedback> {
    let layout = app.ui.docking.as_ref().map(|w| std::borrow::Cow::Borrowed(&w.layout)).unwrap_or_else(|| std::borrow::Cow::Owned(super::legacy(app)));
    let compact = compact_source(app, &layout, request.panel);
    let force_float = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
    if force_float && !compact {
        return None;
    }
    let floating = request.floating_rect.map(|rect| match detached_target(app, &layout, request.panel, rect, request.screen, request.pointer) {
        Target::Float { rect: Some([x, y, w, h]) } => Rect::from_min_size(pos2(x, y), vec2(w, h)),
        _ => rect,
    });
    let size = floating.map_or(vec2(320.0, 400.0), |r| r.size());
    let target = if force_float { None } else { resolved_landing(ctx, &layout, request.panel, request.whole_group, request.pointer, compact) };
    if request.released {
        let to = target
            .as_ref()
            .map(|t| t.target.clone())
            .or_else(|| request.floating_rect.map(|r| detached_target(app, &layout, request.panel, r, request.screen, request.pointer)));
        if let Some(to) = to {
            if let Some(from) = request.floating_rect.filter(|_| target.is_some() && !matches!(to, Target::Float { .. })) {
                let at = ctx.input(|i| i.time);
                ctx.data_mut(|d| d.insert_temp(key("flight"), Flight { panel: request.panel.clone(), from, at, generation: app.ui.docking_generation }));
            }
            commands.push(("window.panel.group",json!({"operation":"drop","panel":request.panel,"whole":request.whole_group,
                "unit":if request.whole_group && request.floating_rect.is_none() && floats(&layout,request.panel){"window"}else{"group"},"to":to,"compact":target.as_ref().map_or(compact,|t|t.compact),"sourceWidth":size.x,"sourceHeight":size.y,"gap":ctx.pixels_per_point().recip()})));
        }
    }
    Some(DropFeedback { preview: if target.is_none() { floating } else { None }, insertion: target.map(|t| t.marker) })
}

fn compact_source(app: &PhotocraftApp, layout: &Layout<String>, panel: &str) -> bool {
    compat::compact(layout, &app.ui.panel_presentation, panel)
        || !floats(layout, panel) && !in_dock_column(layout, panel) && super::collapsed_members(app, layout).contains(panel)
}

pub(super) fn preview_parts(app: &PhotocraftApp, ctx: &egui::Context, layout: &mut std::borrow::Cow<'_, Layout<String>>) {
    part_resize::preview(app, ctx, layout);
}

pub(super) fn compact_detach(app: &PhotocraftApp, layout: &Layout<String>, action: &Action<String>) -> Option<Value> {
    let (panel, rect, whole) = match action {
        Action::Float { panel, rect } | Action::MovePanel { panel, to: Target::Float { rect: Some(rect) } } => (panel, rect, false),
        Action::MoveGroup { panel, to: Target::Float { rect: Some(rect) } } => (panel, rect, true),
        _ => return None,
    };
    if floats(layout, panel) || in_dock_column(layout, panel) || !compact_source(app, layout, panel) {
        return None;
    }
    Some(json!({"panel":panel,"operation":"drop","whole":whole,"unit":"group","to":Target::<String>::Float{rect:Some(*rect)}}))
}

fn drag(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, _bounds: Rect, commands: &mut Vec<(&'static str, Value)>) {
    if !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| d.remove::<bool>(key("suppressed")));
    }
    let Some(drag) = ctx.data(|d| d.get_temp::<Drag>(key("drag"))) else { return };
    let (down, released, cancel, disabled) = ctx.input(|i| {
        (i.pointer.primary_down(), i.pointer.primary_released(), !i.focused || i.key_pressed(egui::Key::Escape), i.modifiers.ctrl || i.modifiers.command)
    });
    if cancel || super::modal_open(app, ctx) || app.session.prefs().workspace_locked || app.ui.docking_generation != drag.generation || (!down && !released) {
        ctx.data_mut(|d| {
            d.remove::<Drag>(key("drag"));
            if down {
                d.insert_temp(key("suppressed"), true);
            }
        });
        return;
    }
    let p = ctx.pointer_interact_pos().unwrap_or(drag.press);
    let rect = drag.source.translate(p - drag.press);
    let editor = matches!(drag.view, DragView::Editor);
    let compact = !editor && compat::compact(layout, &app.ui.panel_presentation, &drag.panel);
    let target = if disabled { None } else { resolved_landing(ctx, layout, &drag.panel, drag.whole, p, compact) };
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, key("preview")));
    painter.rect_filled(rect, t.radius_sm, t.card.gamma_multiply(0.75));
    painter.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    if let Some(target) = &target {
        painter.rect_filled(target.marker, 0.0, t.accent);
    }
    if released {
        let at = ctx.input(|i| i.time);
        ctx.data_mut(|d| {
            d.remove::<Flight>(key("flight"));
            if target.as_ref().is_some_and(|t| !matches!(t.target, Target::Float { .. })) {
                d.insert_temp(key("flight"), Flight { panel: drag.panel.clone(), from: rect, at, generation: app.ui.docking_generation });
            }
        });
        let target = target.map_or_else(
            || {
                let [width, height] = normal_editor_size(app, layout, &drag.panel);
                let size = if editor { drag.source.size() } else { vec2(width, height) };
                Target::Float { rect: Some([rect.left(), rect.top(), size.x, size.y]) }
            },
            |t| t.target,
        );
        commands.push(("window.panel.group", json!({"operation":"drop","panel":drag.panel,"whole":drag.whole,"editor":editor,"unit":if layout.floating.iter().any(|w|w.contains(&drag.panel) && compat::independent_window(w)){"group"}else{"window"},"to":target,"compact":compact,"sourceWidth":drag.source.width(),"sourceHeight":drag.source.height(),"gap":ctx.pixels_per_point().recip()})));
        ctx.data_mut(|d| d.remove::<Drag>(key("drag")));
    } else {
        ctx.request_repaint();
    }
}

/// Animate only the released ghost. Canonical coordinates are committed once, so the
/// receiving group never takes a detour through the old window's origin.
fn animate_landing(app: &PhotocraftApp, ctx: &egui::Context, strips: &[Strip]) {
    let Some(flight) = ctx.data(|d| d.get_temp::<Flight>(key("flight"))) else { return };
    let age = (ctx.input(|i| i.time) - flight.at).max(0.0);
    if age >= 0.15
        || app.ui.docking_generation != flight.generation
        || app.session.prefs().workspace_locked
        || super::modal_open(app, ctx)
        || !ctx.input(|i| i.focused)
    {
        ctx.data_mut(|d| d.remove::<Flight>(key("flight")));
        return;
    }
    let target =
        strips.iter().flat_map(Strip::cells).find(|(id, _)| id == &flight.panel).map(|(_, r)| r).or_else(|| {
            DockArea::<String>::hit_rects(ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::Tab(id) if id==&flight.panel)).map(|(_, r)| r)
        });
    let Some(target) = target else {
        ctx.request_repaint();
        return;
    };
    let amount = (age / 0.15) as f32;
    let eased = 1.0 - (1.0 - amount).powi(3);
    let rect = Rect::from_min_max(flight.from.min.lerp(target.min, eased), flight.from.max.lerp(target.max, eased));
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, key("flight")));
    painter.rect_filled(rect, t.radius_sm, t.card.gamma_multiply((1.0 - amount) * 0.8));
    painter.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent.gamma_multiply(1.0 - amount)), egui::StrokeKind::Inside);
    ctx.request_repaint();
}

pub(super) fn drop_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("unknown panel")?.to_owned();
    let whole = params.get("whole").and_then(Value::as_bool).unwrap_or(false);
    let target: Target<String> = serde_json::from_value(params.get("to").cloned().ok_or("drop target is required")?).map_err(|e| e.to_string())?;
    let mut workspace = workspace_of(app)?;
    if matches!(&target, Target::At { anchor, .. } if anchor == super::document_header::ID) {
        super::document_header::ensure(&mut workspace.layout)?;
    }
    let group_only = params.get("unit").and_then(Value::as_str) == Some("group");
    let ids = moving_ids(&workspace.layout, &panel, whole, group_only);
    let remainder = workspace.layout.floating.iter().find(|w| w.contains(&panel)).and_then(|w| w.panels().find(|id| !ids.contains(id)).cloned());
    let editor = params.get("editor").and_then(Value::as_bool).unwrap_or(false);
    let source_closed = super::collapsed_members(app, &workspace.layout).contains(&panel);
    let horizontal_edge = matches!(&target, Target::At { placement: Placement::Edge(Zone::Top | Zone::Bottom), .. })
        || matches!(&target, Target::At { anchor, placement: Placement::Split(Zone::Bottom) } if anchor == super::document_header::ID);
    let source_compact = !editor && compact_source(app, &workspace.layout, &panel);
    let compact = source_compact;
    let anchor = match &target {
        Target::At { anchor, .. } => Some(anchor.clone()),
        _ => None,
    };
    if anchor.as_ref().is_some_and(|p| ids.contains(p)) {
        return Ok(json!({"unchanged":true}));
    }
    let destination_compact = anchor.as_ref().is_some_and(|a| compat::compact(&workspace.layout, &app.ui.panel_presentation, a));
    let destination_closed = matches!(&target, Target::At { placement: Placement::Tab { .. }, .. })
        && anchor.as_ref().is_some_and(|a| super::collapsed_members(app, &workspace.layout).contains(a));
    // A neighbouring column is an independent owner; its presentation must not be
    // inherited from the expanded column used only as the placement anchor.
    let separate_column =
        matches!(&target, Target::NewDock { .. } | Target::At { placement: Placement::Column { .. } | Placement::Split(Zone::Left | Zone::Right), .. });
    let floating = matches!(target, Target::Float { .. });
    let compact = if !separate_column && !floating && anchor.is_some() { destination_compact } else { compact };
    let source_size = workspace
        .layout
        .floating
        .iter()
        .find(|w| w.contains(&panel) && !compat::independent_window(w))
        .map(|w| [w.rect[2], w.rect[3]])
        .filter(|size| (220.0..=100_000.0).contains(&size[0]) && (120.0..=100_000.0).contains(&size[1]));
    let receiver = anchor.as_ref().and_then(|a| workspace.layout.floating.iter().find(|w| w.contains(a) && !w.contains(&panel))).map(|w| {
        let visible = if !compat::independent_window(w) && w.panels().all(|p| app.ui.panel_presentation.compact.contains(p)) {
            w.panels().next().and_then(|p| app.ui.panel_presentation.widths.get(p)).copied().unwrap_or(COMPACT_COLUMN_WIDTH)
        } else {
            w.rect[2]
        };
        let source_width = if compact {
            app.ui.panel_presentation.widths.get(&panel).copied().unwrap_or(COMPACT_COLUMN_WIDTH)
        } else {
            params.get("sourceWidth").and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite() && (220.0..=100_000.0).contains(v)).unwrap_or(320.0)
        };
        let gap = params.get("gap").and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite() && (0.0..=4.0).contains(v)).unwrap_or(1.0);
        let left = matches!(&target, Target::At { placement: Placement::Split(Zone::Left), .. });
        let mut rect = w.rect;
        rect[2] = (visible + source_width + gap).min(1_000_000.0);
        if !compact {
            let height = params
                .get("sourceHeight")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .filter(|v| v.is_finite() && (120.0..=100_000.0).contains(v))
                .or_else(|| source_size.map(|s| s[1]))
                .unwrap_or(400.0);
            rect[3] = rect[3].max(height);
        }
        if left {
            rect[0] -= source_width + gap;
        }
        rect
    });
    let action = if !whole {
        Action::MovePanel { panel: panel.clone(), to: target }
    } else if group_only {
        Action::MoveGroup { panel: panel.clone(), to: target }
    } else if floats(&workspace.layout, &panel) {
        Action::MoveWindow { panel: panel.clone(), to: target }
    } else if in_dock_column(&workspace.layout, &panel) {
        Action::MoveColumn { panel: panel.clone(), to: target }
    } else {
        Action::MoveGroup { panel: panel.clone(), to: target }
    };
    workspace.apply(action, permissions).map_err(|e| e.to_string())?;
    resize_drop_remainder(&mut workspace, params, remainder)?;
    if separate_column
        && (compact || destination_compact)
        && let (Some(rect), Some(anchor)) = (receiver, anchor.as_ref())
        && workspace.layout.floating.iter().any(|w| w.contains(anchor))
    {
        workspace.apply(Action::MoveFloating { panel: anchor.clone(), rect }, permissions).map_err(|e| e.to_string())?;
    }
    if in_dock_column(&workspace.layout, &panel) {
        workspace.apply(Action::SetIconic { panel: panel.clone(), iconic: compact }, permissions).map_err(|e| e.to_string())?;
    }
    if editor && !compact {
        workspace.apply(Action::SetMinimized { panel: panel.clone(), minimized: false }, permissions).map_err(|e| e.to_string())?;
    }
    if !valid_workspace(&workspace) {
        return Err("invalid resulting workspace".into());
    }
    if editor && !compact {
        app.ui.docking_collapsed.retain(|id| !ids.contains(id));
    }
    let mut settings = app.ui.panel_presentation.clone();
    if !floating && let Some(size) = source_size {
        for id in &ids {
            settings.floating_sizes.insert(id.clone(), size);
        }
    }
    settings.compact.retain(|p| !ids.contains(p));
    if !in_dock_column(&workspace.layout, &panel) && compact {
        settings.compact.extend(ids);
    }
    settings.compact.sort();
    settings.compact.dedup();
    let gap = params.get("gap").and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite() && (0.0..=4.0).contains(v)).unwrap_or(1.0);
    fit_compact_windows(&mut workspace.layout, &settings, gap);
    app.ui.panel_presentation = settings;
    let returns = app.ui.docking_returns.clone();
    commit(app, workspace, returns);
    let root_group = app.ui.docking.as_ref().is_some_and(|w| !floats(&w.layout, &panel) && !in_dock_column(&w.layout, &panel));
    if root_group {
        let members = app.ui.docking.as_ref().map(|w| group_members(&w.layout, &panel)).unwrap_or_default();
        app.ui.docking_collapsed.retain(|id| !members.contains(id));
        if destination_closed || horizontal_edge && !editor && (source_compact || source_closed) {
            app.ui.docking_collapsed.extend(members);
        } else {
            super::reveal_group(app, &panel);
        }
    }
    app.ui.docking_flyout = None;
    Ok(json!({"layout":app.ui.docking.as_ref().map(|w|&w.layout)}))
}

pub(super) fn height_preview(ctx: &egui::Context) -> Option<(String, f32)> {
    let resize = ctx.data(|d| d.get_temp::<Resize>(key("resize"))).filter(|r| r.height)?;
    let delta = ctx.pointer_interact_pos().unwrap_or(resize.press).y - resize.press.y;
    let bottom = ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap_or_else(|| ctx.content_rect()).bottom();
    Some((resize.panel, (resize.rect.height() + delta).clamp(resize.minimum, (bottom - resize.rect.top()).max(resize.minimum))))
}
fn height_handle(app: &PhotocraftApp, ui: &mut egui::Ui, panel: &str, rect: Rect, minimum: f32, commands: &mut Vec<(&'static str, Value)>) {
    if app.session.prefs().workspace_locked || super::modal_open(app, ui.ctx()) || rect.height() < minimum {
        return;
    }
    let band = Rect::from_min_max(pos2(rect.left() + 2.0, rect.bottom() - 3.0), pos2(rect.right() - 2.0, rect.bottom() + 1.0));
    let response = ui.interact(band, key("column-bottom").with(panel), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if response.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(band.center());
        ui.data_mut(|d| {
            d.insert_temp(
                key("resize"),
                Resize { panel: panel.into(), rect, press, left: false, attached: true, height: true, minimum, generation: app.ui.docking_generation },
            )
        });
    }
    if response.dragged() {
        ui.ctx().request_repaint();
    }
    if response.drag_stopped()
        && let Some(start) = ui.data(|d| d.get_temp::<Resize>(key("resize"))).filter(|r| r.height && r.panel == panel)
    {
        if let Some((_, height)) = height_preview(ui.ctx()) {
            commands.push(("window.panel.group", json!({"panel":panel,"operation":"edge-size","height":height,"previousHeight":start.rect.height()})));
        }
        ui.data_mut(|d| d.remove::<Resize>(key("resize")));
    }
}
fn column_height_handles(app: &PhotocraftApp, ui: &mut egui::Ui, layout: &Layout<String>, bounds: Rect, commands: &mut Vec<(&'static str, Value)>) {
    if !app.ui.panel_presentation.migrated {
        return;
    }
    let bodies = super::last_bodies(ui.ctx());
    for (_, column) in compat::columns(layout).into_iter().filter(|(_, c)| !c.iconic) {
        let Some(first) = column.groups.first().and_then(|g| g.panels.first()) else { continue };
        let body = bodies.iter().filter(|(id, _)| column.groups.iter().any(|g| g.panels.contains(id))).map(|(_, r)| *r).reduce(|a, b| a.union(b));
        if let Some(body) = body {
            let minimum: f32 = column
                .groups
                .iter()
                .filter(|g| !g.panels.iter().any(|p| is_spacer(p)))
                .map(|g| g.panels.get(g.active).and_then(|id| PANELS.iter().find(|p| p.0 == id)).map_or(80.0, |p| p.2.min_height()))
                .sum();
            let rect = Rect::from_min_max(pos2(body.left(), bounds.top()), pos2(body.right(), body.bottom().min(bounds.bottom())));
            height_handle(app, ui, first, rect, minimum, commands);
        }
    }
}

#[cfg(test)]
#[path = "presentation_demo.rs"]
mod demo;

#[cfg(test)]
#[path = "repair_tests.rs"]
mod repair_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_docking::{Node, valid};
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::Harness;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    }
    pub(super) fn layout(app: &PhotocraftApp) -> &Layout<String> {
        &app.ui.docking.as_ref().unwrap().layout
    }
    pub(super) fn compact_pair() -> PhotocraftApp {
        let mut app = app();
        app.run("window.panel.float", json!({"panel":"properties","x":100,"y":100,"width":320,"height":400})).unwrap();
        app.run("window.panel.move", json!({"panel":"actions","anchor":"properties","zone":"bottom"})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        app
    }
    pub(super) fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        harness_size(app, vec2(1280.0, 800.0))
    }
    fn harness_size(app: PhotocraftApp, size: Vec2) -> Harness<'static, PhotocraftApp> {
        let mut ready = false;
        let mut h = Harness::builder().with_size(size).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                    ready = true;
                    return;
                }
                crate::panels::dock_area(app, ui);
            },
            app,
        );
        h.run_steps(4);
        h
    }
    pub(super) fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    }
    #[test]
    fn floating_compact_column_click_peeks_without_detaching_or_changing_shape() {
        let mut h = harness(compact_pair());
        let before = layout(h.state()).clone();
        let strip = strips(h.state(), &h.ctx, &before).into_iter().find(|s| !s.attached).unwrap();
        let cell = strip.cells().into_iter().find(|(id, _)| id == "actions").unwrap().1;
        click(&mut h, cell.center());
        assert_eq!(layout(h.state()), &before);
        assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("actions"));
        assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_none());
    }
    #[test]
    fn escape_dismisses_the_panel_menu_before_closing_its_flyout() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"actions","operation":"peek"})).unwrap();
        let mut h = harness(app);
        let rect = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        click(&mut h, pos2(rect.right() - 12.0, rect.top() + 10.0));
        assert!(egui::Popup::is_any_open(&h.ctx));
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
        assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("actions"));
        assert!(!egui::Popup::is_any_open(&h.ctx));
    }
    #[test]
    fn docked_navigation_click_keeps_the_owner_and_peeks_beside_it() {
        let mut app = compact_pair();
        app.run("window.panel.edge", json!({"panel":"properties","edge":"rightInner","collapsed":true})).unwrap();
        app.run("window.panel.move", json!({"panel":"navigator","anchor":"properties","zone":"bottom"})).unwrap();
        let mut h = harness(app);
        let before = layout(h.state()).clone();
        let strip = strips(h.state(), &h.ctx, &before).into_iter().find(|s| s.panels().any(|p| p == "navigator")).unwrap();
        let cell = strip.cells().into_iter().find(|(id, _)| id == "navigator").unwrap().1;
        click(&mut h, cell.center());
        assert_eq!(layout(h.state()), &before);
        assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("navigator"));
    }
    #[test]
    fn dragging_a_compact_window_beside_expanded_layers_keeps_its_own_presentation() {
        let mut h = harness(compact_pair());
        let source = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| !s.attached).unwrap();
        let from = pos2(source.rect.center().x, source.rect.top() + INSET + HEADER + GRIP * 0.5);
        let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
        let to = pos2(bounds.right() - 2.0, 300.0);
        press(&mut h, from, to);
        release(&mut h, to);
        let column =
            compat::columns(layout(h.state())).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
        assert!(column.iconic);
        assert_eq!(column.groups.len(), 2);
        assert!(strips(h.state(), &h.ctx, layout(h.state())).iter().any(|s| s.attached && s.panels().any(|p| p == "properties")));
    }
    #[test]
    fn chevron_expands_all_rows_and_collapse_keeps_their_structure() {
        let mut app = compact_pair();
        let before = layout(&app).clone();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        assert!(app.ui.panel_presentation.compact.is_empty());
        assert_eq!(tree_groups(&layout(&app).floating[0].root).len(), 2);
        app.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        assert_eq!(layout(&app), &before);
        assert!(app.ui.panel_presentation.compact.contains(&"actions".into()));
    }
    #[test]
    fn full_editors_stay_open_on_each_workspace_edge_and_relocate_directly() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        for edge in ["left", "rightInner", "rightOuter", "left"] {
            app.run("window.panel.edge", json!({"panel":"properties","edge":edge})).unwrap();
            let column =
                compat::columns(layout(&app)).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
            assert!(!column.iconic);
            assert_eq!(column.groups.len(), 2);
            assert!(valid(layout(&app)));
        }
    }
    #[test]
    fn compact_resize_rejects_bad_values_without_a_partial_commit() {
        let mut app = compact_pair();
        for width in [44.0, 94.0, 240.0, 44.0] {
            app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":width})).unwrap();
            assert_eq!(app.ui.panel_presentation.widths.get("actions"), Some(&width));
        }
        let saved = crate::dock::snapshot(&app);
        for value in [json!(41), json!(241), json!("wide"), Value::Null] {
            assert!(app.run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":value})).is_err());
            assert_eq!(crate::dock::snapshot(&app), saved);
        }
    }
    #[test]
    fn legacy_column_migration_keeps_tabs_active_row_weights_and_presentation() {
        let saved = json!({
            "docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[{"panels":["properties","actions","channels"],"active":2,"rect":[100,100,320,400]}]},
            "dockingColumns":[{"members":["properties","actions","channels"],"rows":[{"panels":["properties","channels"],"active":1,"weight":3},{"panels":["actions"],"active":0,"weight":1}],"height":600,"attachment":{"side":"rightInner","order":0,"width":320,"expanded":true}}],
            "dockingStripWidths":{"properties":120},"dockingCollapsed":["properties","actions","channels"]
        });
        let workspace = compat::migrate(&saved).unwrap();
        assert!(valid_workspace(&workspace));
        let c = compat::columns(&workspace.layout).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
        assert_eq!(c.groups[0].panels, ["properties", "channels"]);
        assert_eq!(c.groups[0].active, 1);
        assert_eq!(c.groups[0].height, Some(450.0));
        assert_eq!(c.groups[1].height, Some(150.0));
        let p = compat::presentation(&saved);
        assert_eq!(p.widths.get("properties"), Some(&120.0));
        assert_eq!(p.heights.get("actions"), Some(&600.0));
    }
    #[test]
    fn legacy_horizontal_root_keeps_its_split_geometry() {
        let root = Node::Split {
            axis: super::super::SplitAxis::Horizontal,
            size: super::super::SplitSize::Ratio(0.37),
            first: Box::new(Node::Tabs { panels: vec!["layers".into()], active: 0 }),
            second: Box::new(Node::Tabs { panels: vec!["color".into(), "swatches".into()], active: 1 }),
        };
        let saved = json!({"docking":{"root":root,"floating":[]},"dockingColumns":[]});
        let workspace = compat::migrate(&saved).unwrap();
        let Node::Split { second, .. } = workspace.layout.root.as_ref().unwrap() else { panic!("canvas split") };
        assert_eq!(second.as_ref(), &root);
        assert!(valid_workspace(&workspace));
    }
    #[test]
    fn floating_stack_accepts_fractional_row_weights_and_rejects_invalid_ones_atomically() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"edge-size","heights":{"properties":0.25,"actions":0.75}})).unwrap();
        let Node::Split { size, .. } = &layout(&app).floating[0].root else { panic!("stack") };
        assert_eq!(*size, super::super::SplitSize::Ratio(0.25));
        let before = crate::dock::snapshot(&app);
        assert!(app.run("window.panel.group", json!({"panel":"properties","operation":"edge-size","heights":{"properties":-1}})).is_err());
        assert_eq!(crate::dock::snapshot(&app), before);
    }
    #[test]
    fn native_operation_can_float_a_group_outside_a_dock_column() {
        let mut app = app();
        app.ui.docking = Some(craft_ui::docking::Workspace::new(Layout {
            root: Some(Node::Split {
                axis: super::super::SplitAxis::Horizontal,
                size: super::super::SplitSize::Ratio(0.7),
                first: Box::new(Node::Tabs { panels: vec![super::super::CANVAS.into()], active: 0 }),
                second: Box::new(Node::Tabs { panels: vec!["properties".into()], active: 0 }),
            }),
            floating: vec![],
        }));
        app.run("window.panel.group", json!({"panel":"properties","operation":"native","enabled":false})).unwrap();
        assert!(layout(&app).floating.iter().any(|w| w.contains("properties")));
        assert!(valid(layout(&app)));
    }
    #[test]
    fn old_launcher_becomes_one_real_column_without_duplicate_panel_ownership() {
        let mut app = app();
        compat::initialize(&mut app).unwrap();
        let c = compat::columns(layout(&app)).into_iter().find(|(_, c)| c.iconic).unwrap().1;
        assert_eq!(c.groups.iter().flat_map(|g| &g.panels).map(String::as_str).collect::<Vec<_>>(), ["properties", "navigator", "color", "history", "brushes"]);
        let saved = crate::dock::snapshot(&app);
        compat::initialize(&mut app).unwrap();
        assert_eq!(crate::dock::snapshot(&app), saved);
        assert!(valid(layout(&app)));
    }
    #[test]
    fn launcher_migration_keeps_the_active_unmoved_layers_tab() {
        for selected in 0..3 {
            let mut app = app();
            app.ui.dock_tabs.layers = selected;
            compat::initialize(&mut app).unwrap();
            let row = compat::columns(layout(&app)).into_iter().flat_map(|(_, c)| &c.groups).find(|g| g.panels.iter().any(|p| p == "layers")).unwrap();
            assert_eq!(row.active, selected);
            assert_eq!(app.ui.dock_tabs.layers, selected);
        }
    }
    #[test]
    fn migrating_multiple_hidden_panels_keeps_their_return_anchor_without_duplicates() {
        let at = json!({"floating":null,"anchor":"layers","placement":{"Tab":{"before":null}},"size":null,"stack":null});
        let saved = json!({"docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[]},"dockingColumns":[],"dockingHidden":{"properties":at,"actions":at}});
        let mut workspace = compat::migrate(&saved).unwrap();
        assert_eq!(workspace.hidden.len(), 2);
        for id in ["properties", "actions"] {
            let at = workspace.hidden.iter().find(|(p, _)| p == id).unwrap().1.clone();
            assert_eq!(at.anchor.as_deref(), Some("layers"));
            workspace.set_visible(id.into(), true, None, permissions).unwrap();
        }
        assert!(valid_workspace(&workspace));
        assert_eq!(group_members(&workspace.layout, "layers").len(), 3);
    }
    #[test]
    fn ctrl_drag_and_escape_do_not_commit_a_new_layout() {
        let mut h = harness(compact_pair());
        let before = layout(h.state()).clone();
        let strip = strips(h.state(), &h.ctx, &before).into_iter().find(|s| !s.attached).unwrap();
        h.ctx.data_mut(|d| {
            d.insert_temp(
                key("drag"),
                Drag {
                    panel: "properties".into(),
                    whole: true,
                    view: DragView::Palette,
                    press: strip.rect.min,
                    source: strip.rect,
                    generation: h.state().ui.docking_generation,
                },
            )
        });
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(layout(h.state()), &before);
        assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_none());
    }
    #[test]
    fn tools_float_moves_only_the_whole_palette_and_restores_from_workspace() {
        let mut app = app();
        let tool = app.ui.tool;
        app.run("window.tools.float", json!({"x":150,"y":180})).unwrap();
        assert_eq!(app.ui.toolbar_floating, Some([150.0, 180.0]));
        assert_eq!(app.ui.tool, tool);
        let saved = crate::dock::snapshot(&app);
        app.run("window.tools.dock", json!({})).unwrap();
        crate::dock::apply(&mut app, &saved);
        assert_eq!(app.ui.toolbar_floating, Some([150.0, 180.0]));
    }

    pub(super) fn press(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
        h.event(Event::PointerMoved(from));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=5 {
            h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 5.0)));
            h.run_steps(1);
        }
    }
    pub(super) fn release(h: &mut Harness<'static, PhotocraftApp>, to: Pos2) {
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    }
    #[test]
    fn resizing_a_compact_strip_previews_without_saving_and_escape_restores_it() {
        let mut h = harness(compact_pair());
        let before = crate::dock::snapshot(h.state());
        let strip = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| !s.attached).unwrap();
        let from = pos2(strip.rect.right() - 1.0, strip.rect.top() + HEADER + GRIP + 15.0);
        let to = from + vec2(90.0, 0.0);
        press(&mut h, from, to);
        assert!(h.ctx.data(|d| d.get_temp::<Resize>(key("resize"))).is_some());
        assert_eq!(crate::dock::snapshot(h.state()), before, "preview must not touch persisted layout");
        h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
        release(&mut h, to);
        assert_eq!(crate::dock::snapshot(h.state()), before);
    }
    #[test]
    fn flyout_bottom_resize_commits_on_release_and_persists_the_editor_size() {
        let mut h = harness(compact_pair());
        h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"peek"})).unwrap();
        h.run_steps(3);
        let rect = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        let before = crate::dock::snapshot(h.state());
        let from = pos2(rect.center().x, rect.bottom() - 1.0);
        let to = from + vec2(0.0, 50.0);
        press(&mut h, from, to);
        assert!(h.ctx.data(|d| d.get_temp::<EditorResize>(key("editor-resize"))).is_some());
        assert_eq!(crate::dock::snapshot(h.state()), before);
        release(&mut h, to);
        assert_eq!(h.state().ui.panel_presentation.flyouts.get("properties"), Some(&[320.0, 450.0]));
        let saved = crate::dock::snapshot(h.state());
        let mut restored = app();
        crate::dock::apply(&mut restored, &saved);
        assert_eq!(restored.ui.panel_presentation.flyouts, h.state().ui.panel_presentation.flyouts);
    }
    #[test]
    fn full_column_height_preserves_row_proportions_and_keeps_spacers_out_of_saved_state() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"edge-size","height":600})).unwrap();
        let view = rendered_layout(&app, layout(&app), 800.0, None, 1.0);
        let c = compat::columns(&view).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
        assert_eq!(c.groups[0].height, Some(300.0));
        assert_eq!(c.groups[1].height, Some(300.0));
        assert!(c.groups.last().unwrap().panels.iter().all(|p| is_spacer(p)));
        let bottom = bottom_resize(&view, &Action::ResizeGroup { panel: "actions".into(), height: Some(150.0) }).unwrap();
        assert_eq!(bottom.get("height"), Some(&json!(450.0)));
        drop(view);
        app.run("window.panel.layout", json!({"action":Action::<String>::ResizeGroup {panel:"properties".into(),height:Some(400.0)}})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"edge-size","height":450})).unwrap();
        let view = rendered_layout(&app, layout(&app), 800.0, None, 1.0);
        let c = compat::columns(&view).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
        assert_eq!(c.groups[0].height, Some(300.0));
        assert_eq!(c.groups[1].height, Some(150.0));
        assert!(!crate::dock::snapshot(&app).to_string().contains(SPACER));
    }
    #[test]
    fn full_column_bottom_drag_previews_then_commits_the_whole_height() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
        let mut h = harness(app);
        let before = crate::dock::snapshot(h.state());
        let bodies = super::super::last_bodies(&h.ctx);
        let rect = bodies.iter().find(|(id, _)| id == "actions").unwrap().1;
        let from = pos2(rect.center().x, rect.bottom() - 1.0);
        let to = from - vec2(0.0, 120.0);
        press(&mut h, from, to);
        assert!(h.ctx.data(|d| d.get_temp::<Resize>(key("resize"))).is_some_and(|r| r.height));
        assert_eq!(crate::dock::snapshot(h.state()), before);
        release(&mut h, to);
        assert!(h.state().ui.panel_presentation.heights.get("properties").is_some_and(|height| *height < 750.0 && *height >= 400.0));
        assert!(h.state().ui.docking.as_ref().is_some_and(valid_workspace));
    }
    #[test]
    fn an_overflowing_compact_column_scrolls_without_changing_its_layout() {
        let mut app = compact_pair();
        for &(id, _, _) in PANELS.iter().filter(|p| !matches!(p.0, "properties" | "actions")) {
            app.run("window.panel.move", json!({"panel":id,"anchor":"actions","zone":"bottom"})).unwrap();
        }
        app.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        let mut h = harness_size(app, vec2(1000.0, 400.0));
        let before = crate::dock::snapshot(h.state());
        let strip = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| !s.attached).unwrap();
        h.event(Event::PointerMoved(strip.rect.center()));
        h.run_steps(1);
        h.event(Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -900.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(30);
        assert!(h.ctx.data(|d| d.get_temp::<f32>(key("scroll").with(&strip.first))).unwrap_or(0.0) > 100.0);
        assert_eq!(crate::dock::snapshot(h.state()), before);
    }
    #[test]
    fn side_attachment_keeps_independent_parts_with_one_physical_pixel_between_them() {
        let mut h = harness(compact_pair());
        h.state_mut().run("window.panel.float", json!({"panel":"navigator","x":400,"y":100})).unwrap();
        h.state_mut().run("window.panel.group", json!({"panel":"navigator","operation":"collapse"})).unwrap();
        h.run_steps(3);
        let target = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.first == "properties").unwrap();
        let landing = landing(&h.ctx, "navigator", true, pos2(target.rect.right(), target.rect.center().y)).unwrap();
        h.state_mut().run("window.panel.group", json!({"panel":"navigator","operation":"drop","whole":true,"to":landing.target})).unwrap();
        let window = layout(h.state()).floating.iter().find(|w| w.contains("navigator")).unwrap();
        assert!(window.contains("properties"));
        assert_eq!(group_members(layout(h.state()), "navigator"), ["navigator"]);
        assert_eq!(layout(h.state()).floating.len(), 1);
        h.run_steps(4);
        let drawn = strips(h.state(), &h.ctx, layout(h.state()));
        let a = drawn.iter().find(|s| s.first == "properties").unwrap().rect;
        let b = drawn.iter().find(|s| s.first == "navigator").unwrap().rect;
        assert!((b.left() - a.right() - h.ctx.pixels_per_point().recip()).abs() < 0.1);
    }
    #[test]
    fn releasing_a_drag_joins_the_stack_without_reentrant_context_locks() {
        let mut app = compact_pair();
        app.run("window.panel.float", json!({"panel":"navigator","x":400,"y":100})).unwrap();
        app.run("window.panel.group", json!({"panel":"navigator","operation":"collapse"})).unwrap();
        let mut h = harness(app);
        let targets = strips(h.state(), &h.ctx, layout(h.state()));
        let source = targets.iter().find(|s| s.first == "navigator").unwrap();
        let dest = targets.iter().find(|s| s.first == "properties").unwrap();
        let from = pos2(source.rect.center().x, source.rect.top() + INSET + HEADER + GRIP * 0.5);
        let to = dest.cells().first().unwrap().1.center();
        press(&mut h, from, to);
        assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_some());
        release(&mut h, to);
        assert_eq!(layout(h.state()).floating.len(), 1);
        assert!(layout(h.state()).floating[0].contains("navigator"));
        assert!(h.ctx.data(|d| d.get_temp::<Drag>(key("drag"))).is_none());
    }
    #[test]
    fn ctrl_keeps_a_compact_group_floating_when_released_at_the_workspace_edge() {
        let mut h = harness(compact_pair());
        let before = layout(h.state()).root.clone();
        let source = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| !s.attached).unwrap();
        let from = pos2(source.rect.center().x, source.rect.top() + INSET + HEADER + GRIP * 0.5);
        let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
        let to = pos2(bounds.left() + 1.0, 400.0);
        press(&mut h, from, to);
        h.event(Event::ModifiersChanged(Modifiers::CTRL));
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::CTRL });
        h.run_steps(3);
        assert_eq!(layout(h.state()).root, before);
        assert_eq!(layout(h.state()).floating.len(), 1);
    }
    #[test]
    fn a_future_workspace_and_its_presentation_survive_launcher_initialization_and_persistence() {
        let mut app = app();
        let future = json!({"docking":{"version":999,"layout":{"future":true}},"panelPresentation":{"futurePresentation":true}});
        crate::dock::apply(&mut app, &future);
        compat::initialize(&mut app).unwrap();
        assert!(app.ui.docking.is_none());
        assert!(!app.ui.panel_presentation.migrated);
        app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
        crate::dock::persist(&mut app, &egui::Context::default());
        assert_eq!(app.session.prefs().panel_layout.get("docking"), future.get("docking"));
        assert_eq!(app.session.prefs().panel_layout.get("panelPresentation"), future.get("panelPresentation"));
    }
    #[test]
    fn compact_native_column_returns_to_a_visible_in_app_strip_when_collapsed() {
        let mut app = compact_pair();
        app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"native","enabled":true,"x":-500,"y":100})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        assert!(layout(&app).floating.iter().find(|w| w.contains("properties")).unwrap().native.is_none());
        let h = harness(app);
        let s = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.first == "properties").unwrap();
        assert!(h.ctx.content_rect().contains_rect(s.rect));
    }
    /// Local microbenchmark, deliberately separate from frame-rate or release budgets.
    #[test]
    #[ignore = "explicit panel-state microbenchmark; run with --ignored --nocapture"]
    fn benchmark_idle_layout_persistence() {
        let mut app = compact_pair();
        for &(panel, _, _) in PANELS {
            app.run("window.panel.activate", json!({"panel":panel})).unwrap();
        }
        app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
        let ctx = egui::Context::default();
        crate::dock::persist(&mut app, &ctx);
        fn measure(mut action: impl FnMut()) -> (f64, f64) {
            let mut samples = Vec::new();
            for _ in 0..200 {
                let start = std::time::Instant::now();
                for _ in 0..32 {
                    action();
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / 32.0);
            }
            samples.sort_by(f64::total_cmp);
            (samples[100], samples[190])
        }
        let cached = measure(|| crate::dock::persist(&mut app, &ctx));
        let snapshot = measure(|| {
            std::hint::black_box(crate::dock::snapshot(&app));
        });
        assert_eq!(app.session.prefs().panel_layout, crate::dock::snapshot(&app));
        println!(
            "panel state, test profile, 18 registered panels: cached p50={:.3} us p95={:.3} us; serialize-every-frame reference p50={:.3} us p95={:.3} us",
            cached.0, cached.1, snapshot.0, snapshot.1
        );
    }

    /// Explicit visual evidence: no application EXE or desktop windows are built or opened.
    #[test]
    fn capture_port_chrome_at_multiple_scales() {
        let Some(path) = std::env::var_os("PHOTOCRAFT_PANEL_PORT_FIXTURES").map(std::path::PathBuf::from) else { return };
        let _gpu = photocraft_testkit::gpu_lock();
        std::fs::create_dir_all(&path).unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).with_max_steps(64).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
                let mut app = compact_pair();
                app.ui.theme = crate::theme::ThemeKind::ProMedium;
                app.run("window.panel.edge", json!({"panel":"properties","edge":"rightInner","collapsed":true})).unwrap();
                app.run("window.panel.group", json!({"panel":"properties","operation":"peek"})).unwrap();
                app
            });
            h.run_steps(6);
            h.render().unwrap().save(path.join(format!("port-attached-{scale}.png"))).unwrap();
            h.state_mut().run("window.panel.edge", json!({"panel":"properties","edge":"floating","x":100,"y":100,"collapsed":true})).unwrap();
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"strip-width","width":150})).unwrap();
            h.run_steps(5);
            h.render().unwrap().save(path.join(format!("port-floating-labels-{scale}.png"))).unwrap();
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
            h.state_mut().run("window.panel.edge", json!({"panel":"properties","edge":"left"})).unwrap();
            h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"edge-size","height":560})).unwrap();
            h.run_steps(5);
            h.render().unwrap().save(path.join(format!("port-full-stack-{scale}.png"))).unwrap();
            h.state_mut().run("window.tools.float", json!({"x":700,"y":150})).unwrap();
            h.run_steps(5);
            let tools = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("hidden-panels-hover").with("tools"))).unwrap();
            assert!((tools.width() - COMPACT_COLUMN_WIDTH).abs() < 1.0);
            assert!(h.ctx.content_rect().contains_rect(tools));
            h.render().unwrap().save(path.join(format!("port-floating-tools-{scale}.png"))).unwrap();
        }
    }
}

#[cfg(test)]
#[path = "followup_tests.rs"]
mod followup_tests;

#[cfg(test)]
#[path = "gesture_followup_tests.rs"]
mod gesture_followup_tests;

/// Observe fitted groups before their header is drawn, without registering stock input twice.
pub(super) fn observe_group(ctx: &egui::Context, panels: &[String], rect: Rect) {
    ctx.data_mut(|d| {
        let groups = d.get_temp_mut_or_default::<Vec<(String, Rect)>>(key("group-rects"));
        groups.extend(panels.iter().cloned().map(|p| (p, rect)));
    });
}
/// Toggle a horizontal part around its neighbour, keeping the other parts in place.
pub(super) fn command_geometry(app: &PhotocraftApp, ctx: &egui::Context, params: &mut Value) {
    if params.get("operation").and_then(Value::as_str) == Some("drop") {
        if let Some(rect) = drop_remainder_rect(app, ctx, params) {
            params["originRect"] = json!(rect);
        }
        return;
    }
    let Some(operation) = params.get("operation").and_then(Value::as_str).filter(|op| matches!(*op, "expand" | "collapse")) else { return };
    let expanding = operation == "expand";
    let Some(panel) = params.get("panel").and_then(Value::as_str).map(str::to_owned) else { return };
    let Some(window) = app.ui.docking.as_ref().and_then(|w| w.layout.floating.iter().find(|w| w.contains(&panel))) else { return };
    params["gap"] = json!(ctx.pixels_per_point().recip());
    let groups = ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap_or_default();
    let Some(source) = groups.iter().find(|(p, _)| p == &panel).map(|(_, r)| *r) else { return };
    let width = app.ui.panel_presentation.widths.get(&panel).copied().unwrap_or(COMPACT_COLUMN_WIDTH);
    if compat::independent_window(window) {
        let Some(mut frame) = groups.iter().filter(|(p, _)| window.contains(p)).map(|(_, r)| *r).reduce(Rect::union) else { return };
        let next = if expanding { app.ui.panel_presentation.floating_sizes.get(&panel).map_or(320.0, |size| size[0]) } else { width };
        let delta = next - source.width();
        if groups.iter().any(|(p, r)| p != &panel && window.contains(p) && r.left() >= source.right() - 1.0 && r.y_range().intersects(source.y_range())) {
            frame = frame.translate(vec2(-delta, 0.0));
        }
        frame.max.x += delta;
        if expanding {
            let height = app.ui.panel_presentation.floating_sizes.get(&panel).map_or(400.0, |size| size[1]);
            frame.max.y = frame.min.y + frame.height().max(height);
        } else {
            let mut compact: std::collections::HashSet<_> = app.ui.panel_presentation.compact.iter().cloned().collect();
            if let Some(workspace) = &app.ui.docking {
                compact.extend(group_members(&workspace.layout, &panel));
            }
            if let Some(height) = compact_tree_height(&window.root, &compact, ctx.pixels_per_point().recip()) {
                frame.max.y = frame.min.y + height;
            }
        }
        if !expanding {
            params["normalSize"] = json!([source.width(), source.height().max(120.0)]);
        }
        params["windowRect"] = json!([frame.left(), frame.top(), frame.width(), frame.height()]);
    } else if !expanding {
        let normal_sizes: serde_json::Map<String, Value> = groups
            .iter()
            .filter(|(id, rect)| window.contains(id) && rect.width() >= 220.0 && rect.height() >= 120.0)
            .map(|(id, rect)| (id.clone(), json!([rect.width(), rect.height()])))
            .collect();
        params["normalSizes"] = Value::Object(normal_sizes);
        if let Some(frame) = groups.iter().filter(|(id, _)| window.contains(id)).map(|(_, rect)| *rect).reduce(Rect::union) {
            params["windowRect"] = json!([frame.left(), frame.top(), frame.width(), frame.height()]);
        }
        let neighbours = ctx.data(|d| d.get_temp::<Vec<Strip>>(key("strips"))).unwrap_or_default();
        if neighbours.iter().any(|s| s.first != panel && (s.rect.left() - source.right()).abs() <= 8.0 && s.rect.y_range().intersects(source.y_range())) {
            let mut rect = params.get("windowRect").and_then(|v| serde_json::from_value::<[f32; 4]>(v.clone()).ok()).unwrap_or(window.rect);
            rect[0] = source.right() - width;
            params["windowRect"] = json!(rect);
        }
    }
}

fn compact_tree_height(node: &super::Node<String>, compact: &std::collections::HashSet<String>, gap: f32) -> Option<f32> {
    match node {
        super::Node::Tabs { panels, .. } if panels.iter().all(|p| compact.contains(p)) => Some(HEADER + GRIP + ROW * panels.len() as f32 + 1.5),
        super::Node::Split { axis, first, second, .. } => {
            let a = compact_tree_height(first, compact, gap)?;
            let b = compact_tree_height(second, compact, gap)?;
            Some(if *axis == super::SplitAxis::Horizontal { a.max(b) } else { a + b + gap })
        }
        _ => None,
    }
}

fn moving_ids(layout: &Layout<String>, panel: &str, whole: bool, group_only: bool) -> Vec<String> {
    if !whole {
        vec![panel.into()]
    } else if group_only {
        group_members(layout, panel)
    } else {
        layout.floating.iter().find(|w| w.contains(panel)).map_or_else(|| compat::members(layout, panel), |w| w.panels().cloned().collect())
    }
}
/// Remove the departed horizontal extent, not the neighbouring panes' visible widths.
/// Current fitted group intervals also handle tabs sharing a frame and vertically stacked rows.
fn drop_remainder_rect(app: &PhotocraftApp, ctx: &egui::Context, params: &Value) -> Option<[f32; 4]> {
    let panel = params.get("panel")?.as_str()?;
    let layout = &app.ui.docking.as_ref()?.layout;
    let window = layout.floating.iter().find(|w| w.contains(panel) && compat::independent_window(w))?;
    let target: Target<String> = serde_json::from_value(params.get("to")?.clone()).ok()?;
    if matches!(&target,Target::At{anchor,..} if window.contains(anchor)) {
        return None;
    }
    let ids =
        moving_ids(layout, panel, params.get("whole").and_then(Value::as_bool).unwrap_or(false), params.get("unit").and_then(Value::as_str) == Some("group"));
    let mut intervals: Vec<_> = ctx
        .data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects")))
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, r)| window.contains(id) && !ids.contains(id) && r.is_finite() && r.is_positive())
        .map(|(_, r)| (r.left(), r.right()))
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (left, mut start) = intervals.first().copied().map(|(a, _)| (a, a))?;
    let mut end = start;
    let mut width = 0.0;
    let pixel = ctx.pixels_per_point().recip();
    for (a, b) in intervals {
        if a <= end + pixel + 0.01 {
            end = end.max(b);
        } else {
            width += end - start + pixel;
            start = a;
            end = b;
        }
    }
    width += end - start;
    let mut rect = window.rect;
    rect[0] = left;
    rect[2] = width;
    Some(rect)
}
fn resize_drop_remainder(workspace: &mut super::Workspace, params: &Value, panel: Option<String>) -> Result<(), String> {
    if let (Some(panel), Some(raw)) = (panel, params.get("originRect")) {
        let rect: [f32; 4] = serde_json::from_value(raw.clone()).map_err(|e| format!("invalid origin rect: {e}"))?;
        workspace.apply(Action::MoveFloating { panel, rect }, permissions).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Compact-only compound frames have a natural height, including when joined or restored.
fn fit_compact_windows(layout: &mut Layout<String>, settings: &crate::state::PanelPresentation, gap: f32) {
    let compact = settings.compact.iter().cloned().collect();
    for window in layout.floating.iter_mut().filter(|w| w.native.is_none() && compat::independent_window(w)) {
        if let Some(height) = compact_tree_height(&window.root, &compact, gap) {
            window.rect[3] = height;
        }
    }
}
/// Closest horizontal sibling in the same shared tree; independent of viewport/header position.
fn horizontal_peer_left(app: &PhotocraftApp, panel: &str) -> Option<bool> {
    let window = app.ui.docking.as_ref()?.layout.floating.iter().find(|w| w.contains(panel))?;
    let mut node = &window.root;
    let mut side = None;
    for _ in 0..64 {
        let super::Node::Split { axis, first, second, .. } = node else { break };
        let in_first = tree_groups(first).iter().any(|g| g.panels.iter().any(|p| p == panel));
        if *axis == super::SplitAxis::Horizontal {
            side = Some(!in_first);
        }
        node = if in_first { first } else { second };
    }
    side
}
pub(super) fn collapse_left(app: &PhotocraftApp, ctx: &egui::Context, panel: &str, fallback: Rect) -> bool {
    if let Some((side, _)) =
        app.ui.docking.as_ref().and_then(|w| compat::columns(&w.layout).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == panel))))
    {
        return (side == Side::Start) != crate::direction::is_rtl();
    }
    horizontal_peer_left(app, panel).unwrap_or_else(|| {
        let frame = ctx
            .data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects")))
            .unwrap_or_default()
            .into_iter()
            .find(|(id, _)| id == panel)
            .map_or(fallback, |(_, r)| r);
        frame.center().x < ctx.content_rect().center().x
    })
}
