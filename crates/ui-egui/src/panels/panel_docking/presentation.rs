//! Compact floating chrome. Workspace remains the only owner of panel placement.
use super::{
    Action, COMPACT_COLUMN_WIDTH, Layout, PANELS, PhotocraftApp, Placement, Side, Target, Value, Zone, area_id, body, commit, compact_grip_width, compat,
    floats, group_members, in_dock_column, json, normalize, paint_header_glyph, panel_icon, panel_menu, permissions, tree_groups, valid_workspace,
    workspace_of,
};
use craft_ui::docking::{DockArea, DropFeedback, DropRequest, Hit};
use egui::{Pos2, Rect, Sense, Vec2, pos2, vec2};

const HEADER: f32 = 20.0;
const GRIP: f32 = 11.0;
const ROW: f32 = 35.0;
const INSET: f32 = 3.0;

#[derive(Clone)]
struct Strip {
    first: String,
    rows: Vec<Vec<String>>,
    rect: Rect,
    attached: bool,
    left: bool,
    offset: f32,
}
impl Strip {
    fn panels(&self) -> impl Iterator<Item = &String> {
        self.rows.iter().flatten()
    }
    fn cells(&self) -> Vec<(String, Rect)> {
        let mut y = self.rect.top() + INSET + HEADER + GRIP - self.offset;
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
#[derive(Clone)]
struct Drag {
    panel: String,
    whole: bool,
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
) -> std::borrow::Cow<'a, Layout<String>> {
    let compact_float = canonical.floating.iter().any(|w| w.native.is_none() && w.panels().all(|p| app.ui.panel_presentation.compact.contains(p)));
    let size_columns = app.ui.panel_presentation.migrated && compat::columns(canonical).iter().any(|(_, c)| !c.iconic);
    if !compact_float && !size_columns {
        return std::borrow::Cow::Borrowed(canonical);
    }
    let mut layout = canonical.clone();
    layout.floating.retain(|w| w.native.is_some() || !w.panels().all(|p| app.ui.panel_presentation.compact.contains(p)));
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
    if cancel || app.session.prefs().workspace_locked || app.ui.docking_generation != start.generation {
        ctx.data_mut(|d| {
            d.remove::<Resize>(key("resize"));
            d.insert_temp(key("suppressed"), true);
        });
    }
}

fn strips(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>) -> Vec<Strip> {
    let hits = DockArea::<String>::hit_rects(ctx, area_id());
    let mut out = Vec::new();
    for (side, column) in compat::columns(layout).into_iter().filter(|(_, c)| c.iconic) {
        let Some(first) = column.groups.first().and_then(|g| g.panels.first()) else { continue };
        let Some((_, bar)) = hits.iter().find(|(h, _)| matches!(h, Hit::Icon(id) if id == first)) else { continue };
        let left = (side == Side::Start) != crate::direction::is_rtl();
        let height = app.ui.panel_presentation.heights.get(first).copied().unwrap_or(ctx.content_rect().bottom() - bar.top());
        out.push(Strip {
            first: first.clone(),
            rows: column.groups.iter().map(|g| g.panels.clone()).collect(),
            rect: Rect::from_min_size(bar.min, vec2(bar.width(), height)),
            attached: true,
            left,
            offset: ctx.data(|d| d.get_temp::<f32>(key("scroll").with(first))).unwrap_or(0.0),
        });
    }
    for window in &layout.floating {
        if window.native.is_some() || !window.panels().all(|p| app.ui.panel_presentation.compact.contains(p)) {
            continue;
        }
        let Some(first) = window.panels().next() else { continue };
        let rows: Vec<_> = tree_groups(&window.root).into_iter().map(|g| g.panels).collect();
        let width = app.ui.panel_presentation.widths.get(first).copied().unwrap_or(COMPACT_COLUMN_WIDTH).clamp(COMPACT_COLUMN_WIDTH, 240.0);
        let count = rows.iter().map(Vec::len).sum::<usize>();
        let height = HEADER + GRIP + ROW * count as f32 + 2.0 * INSET;
        let screen = ctx.content_rect();
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
            left,
            offset: ctx.data(|d| d.get_temp::<f32>(key("scroll").with(first))).unwrap_or(0.0),
        });
    }
    for strip in &mut out {
        resize_preview(ctx, strip);
    }
    out
}
pub(super) fn prepare(ctx: &egui::Context, app: &PhotocraftApp, layout: &Layout<String>, bounds: Rect) {
    cancel_resize(app, ctx);
    let drawn = strips(app, ctx, layout);
    ctx.data_mut(|d| {
        d.insert_temp(key("bounds"), bounds);
        d.insert_temp(key("strips"), drawn);
    });
}

pub(super) fn button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, glyph: &str, label: &str) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let response = ui.interact(rect, id, Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, t.card_border);
    }
    paint_header_glyph(ui.painter(), rect, glyph, t.text_dim);
    response.on_hover_cursor(egui::CursorIcon::Default).on_hover_text(label)
}
pub(super) fn begin(app: &PhotocraftApp, ui: &egui::Ui, response: &egui::Response, panel: &str, whole: bool, source: Rect) {
    if app.session.prefs().workspace_locked || !response.drag_started() || ui.data(|d| d.get_temp::<bool>(key("suppressed"))).unwrap_or(false) {
        return;
    }
    let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(source.center());
    ui.data_mut(|d| d.insert_temp(key("drag"), Drag { panel: panel.into(), whole, press, source, generation: app.ui.docking_generation }));
}
fn header(app: &PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let top = Rect::from_min_size(strip.rect.min + vec2(0.0, INSET), vec2(strip.rect.width(), HEADER + GRIP));
    ui.painter().rect_filled(top, 0.0, t.card);
    let x = if strip.attached { strip.rect.center().x - 10.0 } else { strip.rect.right() - 40.0 - INSET };
    let arrow = Rect::from_min_size(pos2(x, top.top()), Vec2::splat(20.0));
    let response = button(ui, arrow, key("expand").with(&strip.first), if strip.left { "chevrons-right" } else { "chevrons-left" }, tl!("Expand Panel Group"));
    if response.clicked() {
        commands.push(("window.panel.group", json!({"panel":strip.first,"operation":"expand"})));
    }
    begin(app, ui, &response, &strip.first, true, strip.rect);
    if !strip.attached {
        let close = arrow.translate(vec2(20.0, 0.0));
        let response = button(ui, close, key("close").with(&strip.first), "x", tl!("Close Tab Group"));
        if response.clicked() {
            commands.push(("window.panel.group", json!({"panel":strip.first,"operation":"close-window"})));
        }
        begin(app, ui, &response, &strip.first, true, strip.rect);
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
    begin(app, ui, &response, &strip.first, true, strip.rect);
    if response.double_clicked() {
        commands.push(("window.panel.group", json!({"panel":strip.first,"operation":"expand"})));
    }
}
fn resize(app: &PhotocraftApp, ui: &mut egui::Ui, strip: &Strip, commands: &mut Vec<(&'static str, Value)>) {
    if app.session.prefs().workspace_locked {
        return;
    }
    for left in [true, false] {
        if strip.attached && left == strip.left {
            continue;
        }
        let x = if left { strip.rect.left() } else { strip.rect.right() };
        let rect = Rect::from_min_max(pos2(x - 2.0, strip.rect.top() + HEADER + GRIP), pos2(x + 2.0, strip.rect.bottom()));
        let response = ui.interact(rect, key("resize").with((&strip.first, left)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
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
            let width = (start.rect.width() + if start.left { -dx } else { dx }).clamp(COMPACT_COLUMN_WIDTH, 240.0);
            commands.push(("window.panel.group", json!({"panel":start.panel,"operation":"strip-width","width":width})));
            if start.left
                && !start.attached
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
        let paint = |ui: &mut egui::Ui, app: &mut PhotocraftApp, commands: &mut Vec<(&'static str, Value)>| {
            if super::modal_open(app, ui.ctx()) {
                ui.disable();
            }
            ui.interact(strip.rect, key("background").with(&strip.first), Sense::click());
            ui.set_min_size(strip.rect.size());
            ui.set_max_size(strip.rect.size());
            ui.painter().rect_filled(strip.rect, t.radius_sm, t.card);
            if strip.attached {
                ui.painter().vline(
                    strip.rect.right() - ctx.pixels_per_point().recip() * 0.5,
                    strip.rect.y_range(),
                    egui::Stroke::new(ctx.pixels_per_point().recip(), t.card_border),
                );
            } else {
                ui.painter().rect_stroke(strip.rect, t.radius_sm, egui::Stroke::new(ctx.pixels_per_point().recip(), t.card_border), egui::StrokeKind::Inside);
            }
            header(app, ui, strip, commands);
            icon_list(app, ui, strip, commands);
            resize(app, ui, strip, commands);
            if strip.attached {
                height_handle(app, ui, &strip.first, strip.rect, 100.0, commands);
            }
        };
        if strip.attached {
            let mut child =
                ui.new_child(egui::UiBuilder::new().id_salt((&strip.first, "compact")).max_rect(strip.rect).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(strip.rect);
            paint(&mut child, app, commands);
        } else {
            egui::Area::new(key("float").with(&strip.first))
                .order(egui::Order::Middle)
                .fixed_pos(strip.rect.min)
                .movable(false)
                .fade_in(false)
                .show(&ctx, |ui| paint(ui, app, commands));
        }
    }
    column_height_handles(app, ui, layout, bounds, commands);
    let pixel = ctx.pixels_per_point().recip();
    if compat::columns(layout).iter().any(|(side, _)| *side == Side::Start) {
        ui.painter().vline(bounds.left() + pixel * 0.5, bounds.y_range(), egui::Stroke::new(pixel, t.card_border));
    }
    for (hit, rect) in DockArea::<String>::hit_rects(&ctx, area_id()) {
        match hit {
            Hit::Divider(craft_ui::docking::Divider::Column(_)) => {
                let x = (rect.center().x / pixel).round() * pixel - pixel * 0.5;
                ui.painter().vline(x, rect.y_range(), egui::Stroke::new(pixel, t.card_border));
            }
            Hit::Divider(craft_ui::docking::Divider::Group(_)) => {
                let y = (rect.center().y / pixel).round() * pixel - pixel * 0.5;
                ui.painter().hline(rect.x_range(), y, egui::Stroke::new(pixel, t.card_border));
            }
            _ => {}
        }
    }
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
    let view = Rect::from_min_max(pos2(strip.rect.left(), strip.rect.top() + INSET + HEADER + GRIP), strip.rect.max - vec2(0.0, INSET));
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
                if on {
                    ui.painter().rect_stroke(r.rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                }
                let icon = Rect::from_center_size(pos2(cell.left() + COMPACT_COLUMN_WIDTH * 0.5, cell.center().y), Vec2::splat(20.0));
                crate::icons::paint(ui, icon, panel_icon(&id), 20.0, t.text_dim);
                let available = cell.right() - icon.right() - 12.0;
                let minimum = ui
                    .painter()
                    .layout_no_wrap(format!("{}…", normalize_label(&id).chars().take(2).collect::<String>()), egui::FontId::proportional(12.0), t.text)
                    .size()
                    .x;
                if available >= minimum {
                    let galley = ui.painter().layout(normalize_label(&id), egui::FontId::proportional(12.0), t.text, available);
                    let mut job = (*galley.job).clone();
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job.wrap.max_width = available;
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    ui.painter().galley(pos2(icon.right() + 6.0, cell.center().y - galley.size().y * 0.5), galley, t.text);
                }
                begin(app, ui, &r, &id, false, cell);
                if r.clicked() {
                    if on {
                        app.ui.docking_flyout = None;
                    } else {
                        commands.push(("window.panel.group", json!({"panel":id,"operation":"peek"})));
                    }
                }
                r.on_hover_text(normalize_label(&id));
            }
        });
    ui.data_mut(|d| d.insert_temp(key("scroll").with(&strip.first), output.state.clamped_offset().y));
}

fn normalize_label(id: &str) -> String {
    PANELS.iter().find(|p| p.0 == id).map(|p| tl!(p.1).to_string()).unwrap_or_else(|| id.into())
}

fn flyout(app: &mut PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, strips: &[Strip], commands: &mut Vec<(&'static str, Value)>) {
    let Some(id) = app.ui.docking_flyout.clone() else { return };
    let Some(strip) = strips.iter().find(|s| s.panels().any(|p| p == &id)) else {
        app.ui.docking_flyout = None;
        return;
    };
    let (width, height) = if let Some(w) = layout.floating.iter().find(|w| w.contains(&id)) {
        (w.rect[2], w.rect[3])
    } else {
        compat::columns(layout)
            .into_iter()
            .find(|(_, c)| c.groups.iter().any(|g| g.panels.contains(&id)))
            .map(|(_, c)| (c.width, 440.0))
            .unwrap_or((320.0, 440.0))
    };
    let top = strip.cells().into_iter().find(|(p, _)| p == &id).map_or(strip.rect.top(), |(_, r)| r.top());
    let width = width.clamp(220.0, 520.0);
    let height = app.ui.panel_presentation.flyouts.get(&id).map_or(height, |size| size[1]).max(120.0).min(ctx.content_rect().height());
    let y = top.min(ctx.content_rect().bottom() - height).max(ctx.content_rect().top());
    let x = if strip.left { strip.rect.right() } else { strip.rect.left() - width };
    let mut rect = Rect::from_min_size(pos2(x, y), vec2(width, height));
    if let Some(start) = ctx.data(|d| d.get_temp::<EditorResize>(key("editor-resize"))).filter(|r| r.panel == id) {
        let delta = ctx.pointer_interact_pos().unwrap_or(start.press) - start.press;
        let width = (start.rect.width()
            + if start.bottom {
                0.0
            } else if start.left {
                -delta.x
            } else {
                delta.x
            })
        .clamp(220.0, 520.0);
        let height = (start.rect.height() + if start.bottom { delta.y } else { 0.0 }).clamp(120.0, ctx.content_rect().height().max(120.0));
        rect = Rect::from_min_size(
            pos2(if start.left && !start.bottom { start.rect.right() - width } else { start.rect.left() }, start.rect.top()),
            vec2(width, height),
        );
    }
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
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(ctx.pixels_per_point().recip(), t.card_border), egui::StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0));
        ui.painter().text(header.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, normalize_label(&id), egui::FontId::proportional(12.0), t.text);
        let arrow = Rect::from_min_size(pos2(header.right() - 42.0, header.top()), Vec2::splat(20.0));
        if button(ui, arrow, key("flyout-collapse"), if strip.left { "chevrons-left" } else { "chevrons-right" }, tl!("Collapse Panel Group")).clicked() {
            app.ui.docking_flyout = None;
        }
        let menu = button(ui, arrow.translate(vec2(20.0, 0.0)), key("flyout-menu"), "ellipsis", tl!("Panel menu"));
        egui::Popup::menu(&menu).show(|ui| panel_menu(app, ui, &id));
        let response = ui.interact(Rect::from_min_max(header.min, pos2(arrow.left(), header.bottom())), key("flyout-drag"), Sense::drag());
        begin(app, ui, &response, &id, false, rect);
        let content = Rect::from_min_max(pos2(rect.left() + 6.0, header.bottom()), rect.max - vec2(6.0, 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt((&id, "flyout-body")).max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(content);
        body(app, &mut child, &id);
        editor_resize(app, ui, &id, rect, !strip.left, commands);
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
fn landing(ctx: &egui::Context, source: &str, whole: bool, p: Pos2, size: Vec2, visual_width: f32) -> Option<Landing> {
    let strips = ctx.data(|d| d.get_temp::<Vec<Strip>>(key("strips"))).unwrap_or_default();
    for strip in strips.iter().rev() {
        if whole && strip.panels().any(|id| id == source) {
            continue;
        }
        if !strip.attached && !strip.panels().any(|id| id == source) && (strip.rect.top() - 10.0..=strip.rect.bottom() + 10.0).contains(&p.y) {
            for left in [true, false] {
                let x = if left { strip.rect.left() } else { strip.rect.right() };
                if (p.x - x).abs() <= 8.0 {
                    let pixel = ctx.pixels_per_point().recip();
                    let x = if left { x - visual_width + pixel } else { x - pixel };
                    return Some(Landing {
                        target: Target::Float { rect: Some([x, strip.rect.top(), size.x, size.y]) },
                        marker: Rect::from_min_max(
                            pos2(if left { strip.rect.left() } else { strip.rect.right() } - pixel, strip.rect.top()),
                            pos2(if left { strip.rect.left() } else { strip.rect.right() } + pixel, strip.rect.bottom()),
                        ),
                        compact: false,
                    });
                }
            }
        }
        if p.y < strip.rect.top() + INSET + HEADER + GRIP - 2.0 || !strip.rect.expand2(vec2(8.0, 0.0)).contains(p) {
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
    None
}
pub(super) fn resolve_drag(
    _app: &PhotocraftApp,
    ctx: &egui::Context,
    request: &DropRequest<'_, String>,
    commands: &mut Vec<(&'static str, Value)>,
) -> Option<DropFeedback> {
    let size = request.floating_rect.map_or(vec2(320.0, 400.0), |r| r.size());
    let width = if _app.ui.panel_presentation.compact.contains(request.panel) {
        _app.ui.panel_presentation.widths.get(request.panel).copied().unwrap_or(COMPACT_COLUMN_WIDTH)
    } else {
        size.x
    };
    let target = landing(ctx, request.panel, request.whole_group, request.pointer, size, width)?;
    if request.released {
        if let Some(from) = request.floating_rect {
            let at = ctx.input(|i| i.time);
            ctx.data_mut(|d| d.insert_temp(key("flight"), Flight { panel: request.panel.clone(), from, at, generation: _app.ui.docking_generation }));
        }
        commands.push((
            "window.panel.group",
            json!({"operation":"drop","panel":request.panel,"whole":request.whole_group,"unit":"group","to":target.target,"compact":target.compact}),
        ));
    }
    Some(DropFeedback { preview: None, insertion: Some(target.marker) })
}

fn drag(app: &PhotocraftApp, ctx: &egui::Context, layout: &Layout<String>, bounds: Rect, commands: &mut Vec<(&'static str, Value)>) {
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
    let compact = compat::compact(layout, &app.ui.panel_presentation, &drag.panel);
    let size = layout.floating.iter().find(|w| w.contains(&drag.panel)).map_or(vec2(320.0, 400.0), |w| vec2(w.rect[2], w.rect[3]));
    let mut target = if disabled { None } else { landing(ctx, &drag.panel, drag.whole, p, size, drag.source.width()) };
    if target.is_none() && !disabled {
        for left in [true, false] {
            let x = if left { bounds.left() } else { bounds.right() };
            if (p.x - x).abs() <= 18.0 && bounds.y_range().contains(p.y) {
                let side = if left != crate::direction::is_rtl() { Side::Start } else { Side::End };
                let anchor = compat::columns(layout)
                    .into_iter()
                    .filter(|(s, _)| *s == side)
                    .flat_map(|(_, c)| &c.groups)
                    .flat_map(|g| &g.panels)
                    .find(|id| *id != &drag.panel);
                let to = anchor.map_or(Target::NewDock { side }, |anchor| Target::At { anchor: anchor.clone(), placement: Placement::Column { after: false } });
                target = Some(Landing { target: to, marker: Rect::from_min_max(pos2(x - 1.0, bounds.top()), pos2(x + 1.0, bounds.bottom())), compact });
                break;
            }
        }
        if target.is_none() {
            let hits = DockArea::<String>::hit_rects(ctx, area_id());
            if let Some((Hit::Tab(anchor), r)) =
                hits.iter().find(|(hit, r)| matches!(hit,Hit::Tab(id) if id!=&drag.panel) && r.expand2(vec2(0.0, 18.0)).contains(p))
            {
                target =
                    Some(Landing { target: Target::At { anchor: anchor.clone(), placement: Placement::Tab { before: None } }, marker: *r, compact: false });
            }
        }
    }
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, key("preview")));
    painter.rect_filled(rect, t.radius_sm, t.card.gamma_multiply(0.75));
    painter.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    if let Some(target) = &target {
        painter.rect_filled(target.marker, 0.0, t.accent);
    }
    if released {
        let at = ctx.input(|i| i.time);
        ctx.data_mut(|d| d.insert_temp(key("flight"), Flight { panel: drag.panel.clone(), from: rect, at, generation: app.ui.docking_generation }));
        let target = target.map_or_else(
            || {
                let size = layout.floating.iter().find(|w| w.contains(&drag.panel)).map_or(vec2(320.0, 400.0), |w| vec2(w.rect[2], w.rect[3]));
                Target::Float { rect: Some([rect.left(), rect.top(), size.x, size.y]) }
            },
            |t| t.target,
        );
        commands.push(("window.panel.group", json!({"operation":"drop","panel":drag.panel,"whole":drag.whole,"to":target,"compact":compact})));
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
    if age >= 0.15 || app.ui.docking_generation != flight.generation {
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
    let group_only = params.get("unit").and_then(Value::as_str) == Some("group");
    let ids = if whole && group_only {
        group_members(&workspace.layout, &panel)
    } else if whole {
        compat::members(&workspace.layout, &panel)
    } else {
        vec![panel.clone()]
    };
    let compact = compat::compact(&workspace.layout, &app.ui.panel_presentation, &panel);
    let anchor = match &target {
        Target::At { anchor, .. } => Some(anchor.clone()),
        _ => None,
    };
    if anchor.as_ref().is_some_and(|p| ids.contains(p)) {
        return Ok(json!({"unchanged":true}));
    }
    let destination_compact = anchor.as_ref().is_some_and(|a| compat::compact(&workspace.layout, &app.ui.panel_presentation, a));
    // A neighbouring column is an independent owner; its presentation must not be
    // inherited from the expanded column used only as the placement anchor.
    let separate_column = matches!(&target, Target::NewDock { .. } | Target::At { placement: Placement::Column { .. }, .. });
    let floating = matches!(target, Target::Float { .. });
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
    if in_dock_column(&workspace.layout, &panel) {
        workspace
            .apply(Action::SetIconic { panel: panel.clone(), iconic: compact && (separate_column || anchor.is_none() || destination_compact) }, permissions)
            .map_err(|e| e.to_string())?;
    }
    if !valid_workspace(&workspace) {
        return Err("invalid resulting workspace".into());
    }
    let mut settings = app.ui.panel_presentation.clone();
    settings.compact.retain(|p| !ids.contains(p));
    if floating && compact || !in_dock_column(&workspace.layout, &panel) && destination_compact {
        settings.compact.extend(ids);
    }
    settings.compact.sort();
    settings.compact.dedup();
    app.ui.panel_presentation = settings;
    let returns = app.ui.docking_returns.clone();
    commit(app, workspace, returns);
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
mod tests {
    use super::*;
    use crate::panel_docking::{Node, valid};
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::Harness;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    }
    fn layout(app: &PhotocraftApp) -> &Layout<String> {
        &app.ui.docking.as_ref().unwrap().layout
    }
    fn compact_pair() -> PhotocraftApp {
        let mut app = app();
        app.run("window.panel.float", json!({"panel":"properties","x":100,"y":100,"width":320,"height":400})).unwrap();
        app.run("window.panel.move", json!({"panel":"actions","anchor":"properties","zone":"bottom"})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"collapse"})).unwrap();
        app
    }
    fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
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
    fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
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
        for value in [json!(43), json!(241), json!("wide"), Value::Null] {
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
                Drag { panel: "properties".into(), whole: true, press: strip.rect.min, source: strip.rect, generation: h.state().ui.docking_generation },
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

    fn press(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
        h.event(Event::PointerMoved(from));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for step in 1..=5 {
            h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 5.0)));
            h.run_steps(1);
        }
    }
    fn release(h: &mut Harness<'static, PhotocraftApp>, to: Pos2) {
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
        let view = rendered_layout(&app, layout(&app), 800.0, None);
        let c = compat::columns(&view).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
        assert_eq!(c.groups[0].height, Some(300.0));
        assert_eq!(c.groups[1].height, Some(300.0));
        assert!(c.groups.last().unwrap().panels.iter().all(|p| is_spacer(p)));
        let bottom = bottom_resize(&view, &Action::ResizeGroup { panel: "actions".into(), height: Some(150.0) }).unwrap();
        assert_eq!(bottom.get("height"), Some(&json!(450.0)));
        drop(view);
        app.run("window.panel.layout", json!({"action":Action::<String>::ResizeGroup {panel:"properties".into(),height:Some(400.0)}})).unwrap();
        app.run("window.panel.group", json!({"panel":"properties","operation":"edge-size","height":450})).unwrap();
        let view = rendered_layout(&app, layout(&app), 800.0, None);
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
    fn side_snap_uses_one_physical_pixel_and_does_not_merge_independent_columns() {
        let mut h = harness(compact_pair());
        h.state_mut().run("window.panel.float", json!({"panel":"navigator","x":400,"y":100})).unwrap();
        h.state_mut().run("window.panel.group", json!({"panel":"navigator","operation":"collapse"})).unwrap();
        h.run_steps(3);
        let target = strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.first == "properties").unwrap();
        let landing = landing(&h.ctx, "navigator", true, pos2(target.rect.right(), target.rect.center().y), vec2(320.0, 400.0), 44.0).unwrap();
        h.state_mut().run("window.panel.group", json!({"panel":"navigator","operation":"drop","whole":true,"to":landing.target})).unwrap();
        let window = layout(h.state()).floating.iter().find(|w| w.contains("navigator")).unwrap();
        assert_eq!(window.rect[0], target.rect.right() - h.ctx.pixels_per_point().recip());
        assert_eq!(layout(h.state()).floating.len(), 2);
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
