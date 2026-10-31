//! Width gestures for independent parts of one shared floating split tree.
//! Only the gesture snapshot is transient; committed widths remain shared split sizes.
use super::{
    COMPACT_COLUMN_WIDTH, GRIP, HEADER, Layout, PhotocraftApp, Pos2, Rect, area_id, commit, compat, key, pos2, tree_groups, valid_workspace, workspace_of,
};
use craft_ui::layout::{ResizeHandle, ResizeHandleStyle, SplitAxis, SplitSize};

#[derive(Clone)]
struct Part {
    panels: Vec<String>,
    rect: Rect,
}

#[derive(Clone)]
struct Resize {
    panel: String,
    parts: Vec<Part>,
    source: Rect,
    frame: Rect,
    press: Pos2,
    left: bool,
    compact: bool,
    generation: u64,
}

fn state_key() -> egui::Id {
    key("part-width-resize")
}

fn next_width(start: &Resize, ctx: &egui::Context) -> f32 {
    let pointer = ctx.pointer_interact_pos().unwrap_or(start.press);
    let delta = if start.left { start.press.x - pointer.x } else { pointer.x - start.press.x };
    bounded_width(start, ctx, start.source.width() + delta)
}

fn bounded_width(start: &Resize, ctx: &egui::Context, requested: f32) -> f32 {
    let bounds = ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap_or_else(|| ctx.content_rect());
    let growth = if start.left { start.frame.left() - bounds.left() } else { bounds.right() - start.frame.right() };
    let minimum = if start.compact { COMPACT_COLUMN_WIDTH } else { 220.0 };
    let maximum = (start.source.width() + growth.max(0.0)).min(if start.compact { 240.0 } else { 100_000.0 }).max(minimum);
    requested.clamp(minimum, maximum)
}

pub(super) fn width(ctx: &egui::Context, panel: &str) -> Option<f32> {
    let start = ctx.data(|d| d.get_temp::<Resize>(state_key()))?;
    start.parts.iter().any(|p| p.panels.iter().any(|p| p == panel) && same_column(p.rect, start.source)).then(|| next_width(&start, ctx))
}

fn same_column(a: Rect, b: Rect) -> bool {
    (a.left() - b.left()).abs() < 0.1 && (a.right() - b.right()).abs() < 0.1
}

fn fit(node: &mut super::super::Node<String>, start: &Resize, width: f32, gap: f32, depth: usize) -> Option<f32> {
    if depth > 64 {
        return None;
    }
    if let super::super::Node::Split { axis, size, first, second } = node {
        let a = fit(first, start, width, gap, depth + 1)?;
        let b = fit(second, start, width, gap, depth + 1)?;
        if *axis == SplitAxis::Horizontal {
            *size = SplitSize::FixedFirst(a);
            return Some(a + b + gap);
        }
        return Some(a.max(b));
    }
    let groups = tree_groups(node);
    let rect = start.parts.iter().filter(|p| groups.iter().any(|g| g.panels.iter().any(|id| p.panels.contains(id)))).map(|p| p.rect).reduce(Rect::union)?;
    Some(if same_column(rect, start.source) { width } else { rect.width() })
}

fn resize_window(layout: &mut Layout<String>, start: &Resize, width: f32, gap: f32) -> bool {
    let Some(window) = layout.floating.iter_mut().find(|w| w.contains(&start.panel)) else { return false };
    let Some(total) = fit(&mut window.root, start, width, gap, 0) else { return false };
    window.rect = [start.frame.left() - if start.left { width - start.source.width() } else { 0.0 }, start.frame.top(), total, start.frame.height()];
    true
}

pub(super) fn preview(app: &PhotocraftApp, ctx: &egui::Context, layout: &mut std::borrow::Cow<'_, Layout<String>>) {
    let Some(start) = ctx.data(|d| d.get_temp::<Resize>(state_key())) else { return };
    let cancelled = start.generation != app.ui.docking_generation
        || app.session.prefs().workspace_locked
        || super::super::modal_open(app, ctx)
        || ctx.input(|i| !i.focused || i.key_pressed(egui::Key::Escape) || !i.pointer.primary_down() && !i.pointer.primary_released());
    if cancelled {
        ctx.data_mut(|d| d.remove::<Resize>(state_key()));
        return;
    }
    resize_window(layout.to_mut(), &start, next_width(&start, ctx), ctx.pixels_per_point().recip());
}

fn commit_width(app: &mut PhotocraftApp, ctx: &egui::Context, start: &Resize, width: f32) -> Result<(), String> {
    let mut workspace = workspace_of(app)?;
    if !resize_window(&mut workspace.layout, start, width, ctx.pixels_per_point().recip()) || !valid_workspace(&workspace) {
        return Err("invalid floating part resize".into());
    }
    let mut settings = app.ui.panel_presentation.clone();
    for part in start.parts.iter().filter(|p| same_column(p.rect, start.source)) {
        for id in &part.panels {
            if settings.compact.contains(id) {
                settings.widths.insert(id.clone(), width);
            } else {
                settings.floating_sizes.insert(id.clone(), [width, part.rect.height().max(120.0)]);
                if let Some(size) = settings.flyouts.get_mut(id) {
                    size[0] = width;
                }
            }
        }
    }
    app.ui.panel_presentation = settings;
    let returns = app.ui.docking_returns.clone();
    commit(app, workspace, returns);
    Ok(())
}

pub(super) fn show(app: &mut PhotocraftApp, ui: &egui::Ui, layout: &Layout<String>) {
    let ctx = ui.ctx().clone();
    if app.session.prefs().workspace_locked || super::super::modal_open(app, &ctx) {
        return;
    }
    let observed = ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap_or_default();
    for window in layout.floating.iter().filter(|w| w.native.is_none() && compat::independent_window(w)) {
        let Some(first) = window.panels().next() else { continue };
        let parts: Vec<_> = tree_groups(&window.root)
            .into_iter()
            .filter_map(|g| {
                let rect = observed.iter().find(|(id, _)| g.panels.contains(id))?.1;
                Some(Part { panels: g.panels, rect })
            })
            .collect();
        let Some(frame) = parts.iter().map(|p| p.rect).reduce(Rect::union) else { continue };
        let layer = craft_ui::docking::chrome::FloatingFrame::layer(area_id().with(("floating", first)));
        let mut controls = egui::Ui::new(ctx.clone(), key("part-width-surface").with(window.id), egui::UiBuilder::new().layer_id(layer).max_rect(frame));
        controls.set_clip_rect(ctx.content_rect());
        for part in &parts {
            let Some(panel) = part.panels.first() else { continue };
            for left in [true, false] {
                let x = if left { part.rect.left() } else { part.rect.right() - 3.0 };
                // The right handle owns the intervening seam; the next left handle starts inside its own part.
                let end = if left { x + 3.0 } else { part.rect.right() + ctx.pixels_per_point().recip() };
                // Reserve the bottom edge, and its larger outer corners, for vertical input.
                let bottom = if (part.rect.bottom() - frame.bottom()).abs() < 0.1 {
                    let outer = if left { (part.rect.left() - frame.left()).abs() < 0.1 } else { (part.rect.right() - frame.right()).abs() < 0.1 };
                    if outer { 12.0 } else { 4.0 }
                } else {
                    1.0
                };
                let band = Rect::from_min_max(pos2(x, part.rect.top() + HEADER + GRIP + 1.0), pos2(end, part.rect.bottom() - bottom));
                if !band.is_positive() {
                    continue;
                }
                let compact = part.panels.iter().all(|p| app.ui.panel_presentation.compact.contains(p));
                let response = ResizeHandle::new(
                    key("part-width").with((panel, left)),
                    SplitAxis::Horizontal,
                    band,
                    tl!("Resize panels"),
                    ResizeHandleStyle {
                        hovered: egui::Color32::TRANSPARENT,
                        dragged: egui::Color32::TRANSPARENT,
                        focused: crate::theme::Tokens::get(&ctx).accent,
                    },
                )
                .hit_margin(0.0)
                .value(part.rect.width(), if compact { COMPACT_COLUMN_WIDTH..=240.0 } else { 220.0..=100_000.0 })
                .reversed(left)
                .show(&mut controls);
                if response.response.drag_started() {
                    let press = ctx.input(|i| i.pointer.press_origin()).unwrap_or(band.center());
                    ctx.data_mut(|d| {
                        d.insert_temp(
                            state_key(),
                            Resize {
                                panel: panel.clone(),
                                parts: parts.clone(),
                                source: part.rect,
                                frame,
                                press,
                                left,
                                compact,
                                generation: app.ui.docking_generation,
                            },
                        )
                    });
                }
                if response.response.dragged() {
                    ctx.request_repaint();
                }
                if response.response.drag_stopped() {
                    if let Some(start) = ctx.data(|d| d.get_temp::<Resize>(state_key()))
                        && let Err(error) = commit_width(app, &ctx, &start, next_width(&start, &ctx))
                    {
                        app.ui.status = error;
                    }
                    ctx.data_mut(|d| d.remove::<Resize>(state_key()));
                } else if response.delta != 0.0 && !response.response.dragged() {
                    let start = Resize {
                        panel: panel.clone(),
                        parts: parts.clone(),
                        source: part.rect,
                        frame,
                        press: band.center(),
                        left,
                        compact,
                        generation: app.ui.docking_generation,
                    };
                    let width = bounded_width(&start, &ctx, part.rect.width() + if left { -response.delta } else { response.delta });
                    if let Err(error) = commit_width(app, &ctx, &start, width) {
                        app.ui.status = error;
                    }
                }
            }
        }
    }
}
