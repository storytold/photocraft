//! Panel identities stay attached to their original content while tabs move between containers.
//! Positions and membership are workspace state; an unfinished drag is temporary context state.
use std::collections::{BTreeMap, BTreeSet};

use egui::{Pos2, Rect, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::{PhotocraftApp, dock::Group, theme::Tokens, widgets::CardResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PanelTab {
    pub group: Group,
    /// Canonical index (Pro order), independent of the container or current theme.
    pub tab: usize,
}

impl PanelTab {
    pub fn label(self) -> Option<&'static str> {
        self.group.tabs(true).get(self.tab).copied()
    }

    pub fn source_index(self, pro: bool) -> usize {
        self.label().and_then(|name| self.group.tabs(pro).iter().position(|n| *n == name)).unwrap_or(0)
    }

    pub fn from_source(group: Group, tab: usize, pro: bool) -> Option<Self> {
        let name = group.tabs(pro).get(tab)?;
        group.tabs(true).iter().position(|n| n == name).map(|tab| Self { group, tab })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatingGroup {
    pub id: u64,
    pub tabs: Vec<PanelTab>,
    pub selected: usize,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub open: bool,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Arrangement {
    /// None preserves the original theme's tab order and old saved workspaces.
    pub groups: Option<BTreeMap<Group, Vec<PanelTab>>>,
    pub selected: BTreeMap<Group, PanelTab>,
    pub floating: Vec<FloatingGroup>,
    /// Keep newly detached windows from inheriting a removed window's input state.
    next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Dock(Group),
    Floating(u64),
}

impl Arrangement {
    pub(crate) fn preserve_ids(&mut self, previous: &Self) {
        self.next_id = self.next_id.max(previous.next_id).max(previous.floating.iter().map(|g| g.id).max().unwrap_or(0));
    }
    pub fn tabs(&self, group: Group, pro: bool) -> Vec<PanelTab> {
        match &self.groups {
            Some(groups) => groups.get(&group).cloned().unwrap_or_default(),
            None => (0..group.tabs(pro).len()).filter_map(|tab| PanelTab::from_source(group, tab, pro)).collect(),
        }
    }

    fn ensure(&mut self, pro: bool) {
        if self.groups.is_none() {
            self.groups = Some(Group::ALL.into_iter().map(|g| (g, self.tabs(g, pro))).collect());
        }
    }

    pub fn normalize(&mut self, pro: bool) {
        // Older saved workspaces have floating IDs but no allocation counter.
        self.next_id = self.next_id.max(self.floating.iter().map(|g| g.id).max().unwrap_or(0));
        if self.groups.is_none() && self.floating.is_empty() {
            return;
        }
        self.ensure(pro);
        let mut seen = BTreeSet::new();
        if let Some(groups) = &mut self.groups {
            for tabs in groups.values_mut() {
                tabs.retain(|p| p.label().is_some() && seen.insert(*p));
            }
        }
        let mut ids = BTreeSet::new();
        for (i, group) in self.floating.iter_mut().enumerate() {
            group.tabs.retain(|p| p.label().is_some() && seen.insert(*p));
            group.selected = group.selected.min(group.tabs.len().saturating_sub(1));
            if !ids.insert(group.id) {
                group.id = (i as u64).saturating_add(1);
                while !ids.insert(group.id) {
                    group.id = group.id.saturating_add(1);
                }
            }
            for v in &mut group.position {
                *v = if v.is_finite() { v.clamp(-4000.0, 4000.0) } else { 100.0 };
            }
            for v in &mut group.size {
                *v = if v.is_finite() { v.clamp(120.0, 2000.0) } else { 300.0 };
            }
        }
        self.floating.retain(|g| !g.tabs.is_empty());
        // Malformed/older layouts must not lose a panel permanently.
        if let Some(groups) = &mut self.groups {
            for group in Group::ALL {
                for tab in 0..group.tabs(true).len() {
                    let panel = PanelTab { group, tab };
                    if seen.insert(panel) {
                        groups.entry(group).or_default().push(panel);
                    }
                }
            }
        }
    }

    pub fn place(&mut self, panels: &[PanelTab], target: Option<Location>, before: Option<PanelTab>, at: Pos2, pro: bool) {
        let mut seen = BTreeSet::new();
        let panels: Vec<_> = panels.iter().copied().filter(|p| p.label().is_some() && seen.insert(*p)).collect();
        if panels.is_empty() {
            return;
        }
        self.ensure(pro);
        if let Some(groups) = &mut self.groups {
            for tabs in groups.values_mut() {
                tabs.retain(|p| !panels.contains(p));
            }
        }
        for group in &mut self.floating {
            let selected = group.tabs.get(group.selected).copied();
            group.tabs.retain(|p| !panels.contains(p));
            group.selected = selected.and_then(|p| group.tabs.iter().position(|t| *t == p)).unwrap_or(0);
        }
        match target {
            Some(Location::Dock(g)) => {
                if let Some(groups) = &mut self.groups {
                    let tabs = groups.entry(g).or_default();
                    let index = before.and_then(|p| tabs.iter().position(|t| *t == p)).unwrap_or(tabs.len());
                    for (offset, panel) in panels.iter().enumerate() {
                        tabs.insert(index + offset, *panel);
                    }
                    if let Some(panel) = panels.first() {
                        self.selected.insert(g, *panel);
                    }
                }
            }
            Some(Location::Floating(id)) => {
                if let Some(group) = self.floating.iter_mut().find(|g| g.id == id) {
                    let index = before.and_then(|p| group.tabs.iter().position(|t| *t == p)).unwrap_or(group.tabs.len());
                    for (offset, panel) in panels.iter().enumerate() {
                        group.tabs.insert(index + offset, *panel);
                    }
                    group.selected = index;
                    group.open = true;
                } else {
                    // A stale drop target must not discard its panels.
                    self.make_float(panels, at);
                }
            }
            None => self.make_float(panels, at),
        }
        self.floating.retain(|g| !g.tabs.is_empty());
    }

    fn make_float(&mut self, tabs: Vec<PanelTab>, at: Pos2) {
        let mut id = self.next_id.max(self.floating.iter().map(|g| g.id).max().unwrap_or(0)).checked_add(1).unwrap_or(1);
        while self.floating.iter().any(|g| g.id == id) {
            id = id.checked_add(1).unwrap_or(1);
        }
        self.next_id = id;
        self.floating.push(FloatingGroup { id, tabs, selected: 0, position: [at.x, at.y], size: [300.0, 340.0], open: true, collapsed: false });
    }
}

#[derive(Clone)]
struct Drag {
    panels: Vec<PanelTab>,
    source: Location,
    whole: bool,
    moving_window: bool,
    preview: Option<PanelPreview>,
}

/// Reuse the panel's paint output; drawing its widgets twice would steal input and
/// could change document state. This snapshot is transient and never saved.
#[derive(Clone)]
struct PanelPreview {
    rect: Rect,
    anchor: egui::Vec2,
    shapes: Vec<egui::epaint::ClippedShape>,
}

#[derive(Clone)]
pub struct Target {
    pub location: Location,
    pub layer: egui::LayerId,
    pub rect: Rect,
    pub header: Rect,
    pub tabs: Vec<(PanelTab, Rect)>,
}

fn drag_id() -> egui::Id {
    egui::Id::new("panel-tab-drag")
}
fn canceled_id() -> egui::Id {
    egui::Id::new("panel-tab-drag-canceled")
}
fn targets_id() -> egui::Id {
    egui::Id::new("panel-drop-targets")
}
pub fn targets(ctx: &egui::Context) -> Vec<Target> {
    ctx.data(|d| d.get_temp(targets_id())).unwrap_or_default()
}

pub fn visible_tabs(app: &PhotocraftApp, group: Group, pro: bool) -> Vec<PanelTab> {
    app.ui
        .dock
        .arrangement
        .tabs(group, pro)
        .into_iter()
        .filter(|panel| !app.ui.dock.hidden_tabs.get(&panel.group).is_some_and(|hidden| panel.label().is_some_and(|name| hidden.iter().any(|h| h == name))))
        .collect()
}

pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context) {
    #[cfg(test)]
    ctx.data_mut(|d| d.remove::<Vec<egui::epaint::ClippedShape>>(drag_id().with("paint")));
    app.ui.dock.arrangement.normalize(Tokens::get(ctx).pro);
    ctx.data_mut(|d| d.insert_temp(targets_id(), Vec::<Target>::new()));
    if app.session.prefs().workspace_locked || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) && i.pointer.any_down()) {
        ctx.data_mut(|d| d.insert_temp(canceled_id(), true));
    } else if ctx.input(|i| !i.pointer.any_down()) {
        ctx.data_mut(|d| d.remove::<bool>(canceled_id()));
    }
}

pub fn register(ctx: &egui::Context, location: Location, rect: Rect, panels: &[PanelTab], card: &CardResponse) {
    let tabs = card.tabs.iter().filter_map(|(i, r)| panels.get(*i).map(|p| (*p, *r))).collect();
    ctx.data_mut(|d| {
        let mut all: Vec<Target> = d.get_temp(targets_id()).unwrap_or_default();
        all.push(Target { location, layer: card.strip.layer_id, rect, header: card.strip.rect, tabs });
        d.insert_temp(targets_id(), all);
    });
}

pub fn track(app: &PhotocraftApp, ctx: &egui::Context, location: Location, panels: &[PanelTab], card: &CardResponse) {
    if app.session.prefs().workspace_locked || ctx.data(|d| d.get_temp::<bool>(canceled_id()).unwrap_or(false)) {
        return;
    }
    let payload = if let Some((i, response)) = &card.tab_drag {
        panels.get(*i).filter(|_| response.dragged()).map(|p| Drag { panels: vec![*p], source: location, whole: false, moving_window: false, preview: None })
    } else if card.strip.dragged() {
        Some(Drag { panels: panels.to_vec(), source: location, whole: true, moving_window: false, preview: None })
    } else {
        None
    };
    if let Some(payload) = payload {
        ctx.data_mut(|d| {
            if d.get_temp::<Drag>(drag_id()).is_none() {
                d.insert_temp(drag_id(), payload);
            }
        });
    }
}

pub fn finish(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut drag) = ctx.data(|d| d.get_temp::<Drag>(drag_id())) else { return };
    let Some(pointer) = ctx.pointer_interact_pos() else {
        ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
        return;
    };
    let all = targets(ctx);
    // A window moved by its title bar covers the destination. Ignore only that
    // source window when finding the panel underneath; preserve other z-order.
    let layer = ctx.layer_id_at(pointer);
    let source_layer = all.iter().find(|t| t.location == drag.source).map(|t| t.layer);
    let through_source = drag.moving_window && layer == source_layer;
    let order = ctx.memory(|m| m.layer_ids().collect::<Vec<_>>());
    let hit = all
        .iter()
        .filter(|t| {
            t.header.expand2(vec2(0.0, 3.0)).contains(pointer)
                && (!drag.moving_window || t.location != drag.source)
                && (Some(t.layer) == layer || through_source)
        })
        .max_by_key(|t| order.iter().position(|layer| *layer == t.layer));
    // The preview and the release must use the same insertion position.
    let before = hit.and_then(|h| h.tabs.iter().find(|(_, r)| pointer.x < r.center().x).map(|(p, _)| *p));
    let t = Tokens::get(ctx);
    if ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
        // Existing whole-group column reorder is handled by dock::show.
        if drag.whole && matches!(drag.source, Location::Dock(_)) && hit.is_some_and(|h| matches!(h.location, Location::Dock(_))) {
            return;
        }
        if hit.is_some_and(|h| h.location == drag.source) && before.is_some_and(|p| drag.panels.contains(&p)) {
            return;
        }
        let location = hit.map(|h| h.location);
        if drag.moving_window && location.is_none() {
            // Ordinary movement keeps the existing window, size and identity.
            return;
        }
        app.ui.dock.arrangement.place(&drag.panels, location, before, pointer - vec2(20.0, 12.0), t.pro);
        if let Some(Location::Dock(g)) = location {
            *g.shown_mut(&mut app.ui.panels) = true;
            app.ui.dock.set_collapsed(g, false);
        }
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id()));
    if !drag.moving_window
        && drag.preview.is_none()
        && let Some(source) = all.iter().find(|target| target.location == drag.source && target.rect.is_positive())
    {
        // Floating window frames are complete only after Window::show returns.
        let shapes = ctx.graphics(|graphics| {
            graphics
                .get(source.layer)
                .map(|list| {
                    list.all_entries()
                        .filter(|entry| entry.clip_rect.intersect(source.rect).is_positive() && entry.shape.visual_bounding_rect().intersects(source.rect))
                        .cloned()
                        .map(|mut entry| {
                            entry.clip_rect = entry.clip_rect.intersect(source.rect);
                            entry
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        let anchor = ctx.input(|i| i.pointer.press_origin()).unwrap_or(source.rect.min + vec2(20.0, 12.0)) - source.rect.min;
        drag.preview = Some(PanelPreview { rect: source.rect, anchor, shapes });
        ctx.data_mut(|d| d.insert_temp(drag_id(), drag.clone()));
    }
    if let Some(preview) = &drag.preview {
        // Keep tall panels manageable without changing the cursor's grab point.
        let scale = (320.0 / preview.rect.width()).min(360.0 / preview.rect.height()).min(1.0);
        let transform = egui::emath::TSTransform::new(pointer.to_vec2() - preview.anchor * scale - preview.rect.min.to_vec2() * scale, scale);
        let rect = transform * preview.rect;
        let mut ghost = painter.clone();
        ghost.set_opacity(0.72);
        ghost.rect_filled(rect, t.radius_sm, t.card);
        for entry in &preview.shapes {
            let mut shape = entry.shape.clone();
            shape.transform(transform);
            ghost.with_clip_rect(transform * entry.clip_rect).add(shape);
        }
        painter.rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent.gamma_multiply(0.8)), egui::StrokeKind::Inside);
    }
    if let Some(hit) = hit.filter(|h| !(drag.whole && matches!(drag.source, Location::Dock(_)) && matches!(h.location, Location::Dock(_)))) {
        // A highlighted strip identifies the receiving group; the blue caret marks
        // exactly where the tab will be inserted, including after its last tab.
        painter.rect_filled(hit.header, 0.0, t.accent.gamma_multiply(0.18));
        painter.line_segment([hit.header.left_bottom(), hit.header.right_bottom()], egui::Stroke::new(3.0, t.accent));
        let x = before
            .and_then(|panel| hit.tabs.iter().find(|(p, _)| *p == panel).map(|(_, r)| r.left()))
            .or_else(|| hit.tabs.last().map(|(_, r)| r.right()))
            .unwrap_or(hit.header.left());
        painter.line_segment([pos2(x, hit.header.top()), pos2(x, hit.header.bottom())], egui::Stroke::new(3.0, t.accent));
    }
    // Graphics are drained at frame end; retain paint output for interaction tests.
    #[cfg(test)]
    {
        let shapes = ctx.graphics(|g| g.get(painter.layer_id()).map(|list| list.all_entries().cloned().collect::<Vec<_>>()).unwrap_or_default());
        ctx.data_mut(|d| d.insert_temp(drag_id().with("paint"), shapes));
    }
}

pub fn selected(app: &PhotocraftApp, g: Group, panels: &[PanelTab], pro: bool) -> usize {
    let panel = app.ui.dock.arrangement.selected.get(&g).copied().or_else(|| {
        let mut tabs = app.ui.dock_tabs;
        PanelTab::from_source(g, *g.tab_mut(&mut tabs), pro)
    });
    panel.and_then(|p| panels.iter().position(|x| *x == p)).unwrap_or(0)
}

pub fn select(app: &mut PhotocraftApp, g: Group, panel: PanelTab, pro: bool) {
    *panel.group.tab_mut(&mut app.ui.dock_tabs) = panel.source_index(pro);
    if app.ui.dock.arrangement.groups.is_some() {
        app.ui.dock.arrangement.selected.insert(g, panel);
    }
}

pub fn visible(app: &PhotocraftApp, panel: PanelTab) -> Option<bool> {
    if app.ui.dock.arrangement.groups.is_some()
        && app.ui.dock.hidden_tabs.get(&panel.group).is_some_and(|hidden| panel.label().is_some_and(|name| hidden.iter().any(|h| h == name)))
    {
        return Some(false);
    }
    let groups = app.ui.dock.arrangement.groups.as_ref()?;
    for group in &app.ui.dock.arrangement.floating {
        if let Some(index) = group.tabs.iter().position(|p| *p == panel) {
            return Some(group.open && !group.collapsed && group.selected == index);
        }
    }
    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
    groups.iter().find_map(|(g, tabs)| {
        tabs.contains(&panel).then(|| g.shown(&app.ui.panels) && !app.ui.dock.is_collapsed(*g) && tabs.get(selected(app, *g, tabs, pro)) == Some(&panel))
    })
}

pub fn toggle(app: &mut PhotocraftApp, panel: PanelTab, force_show: bool) -> Option<bool> {
    let on = force_show || !visible(app, panel)?;
    set_visible(app, panel, on)
}

pub fn set_visible(app: &mut PhotocraftApp, panel: PanelTab, on: bool) -> Option<bool> {
    if !on && !visible(app, panel)? {
        return Some(false);
    }
    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
    if on {
        app.ui.dock.show_tab(panel.group, panel.source_index(pro), pro);
    }
    *panel.group.tab_mut(&mut app.ui.dock_tabs) = panel.source_index(pro);
    for group in &mut app.ui.dock.arrangement.floating {
        if let Some(index) = group.tabs.iter().position(|p| *p == panel) {
            group.open = on;
            if on {
                group.collapsed = false;
            }
            group.selected = index;
            return Some(on);
        }
    }
    let group = app.ui.dock.arrangement.groups.as_ref()?.iter().find_map(|(g, tabs)| tabs.contains(&panel).then_some(*g))?;
    *group.shown_mut(&mut app.ui.panels) = on;
    app.ui.dock.arrangement.selected.insert(group, panel);
    if on {
        app.ui.dock.set_collapsed(group, false);
    }
    Some(on)
}

pub fn reveal(app: &mut PhotocraftApp, source: Group) -> bool {
    if app.ui.dock.arrangement.groups.is_none() {
        return false;
    }
    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
    let Some(panel) = PanelTab::from_source(source, *source.tab_mut(&mut app.ui.dock_tabs), pro) else { return false };
    app.ui.dock.show_tab(source, panel.source_index(pro), pro);
    for floating in &mut app.ui.dock.arrangement.floating {
        if let Some(index) = floating.tabs.iter().position(|p| *p == panel) {
            floating.open = true;
            floating.collapsed = false;
            floating.selected = index;
            return true;
        }
    }
    if let Some(groups) = &app.ui.dock.arrangement.groups
        && let Some(group) = groups.iter().find_map(|(g, tabs)| tabs.contains(&panel).then_some(*g))
    {
        app.ui.dock.arrangement.selected.insert(group, panel);
        *group.shown_mut(&mut app.ui.panels) = true;
        app.ui.dock.set_collapsed(group, false);
        return true;
    }
    false
}

/// The rail follows a panel into its current container, including detached groups.
pub fn rail_click(app: &mut PhotocraftApp, source: Group) -> bool {
    if app.ui.dock.arrangement.groups.is_none() {
        return false;
    }
    let pro = matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
    let Some(panel) = PanelTab::from_source(source, *source.tab_mut(&mut app.ui.dock_tabs), pro) else {
        return false;
    };
    let showing = visible(app, panel).unwrap_or(false);
    if let Some(group) = app.ui.dock.arrangement.floating.iter_mut().find(|g| g.tabs.contains(&panel)) {
        if showing {
            group.collapsed = true;
        } else {
            reveal(app, source);
        }
        return true;
    }
    if let Some(groups) = &app.ui.dock.arrangement.groups
        && let Some(group) = groups.iter().find_map(|(g, tabs)| tabs.contains(&panel).then_some(*g))
    {
        if showing {
            app.ui.dock.set_collapsed(group, true);
        } else {
            reveal(app, source);
        }
        return true;
    }
    false
}

pub fn floating(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let pro = Tokens::get(ctx).pro;
    let locked = app.session.prefs().workspace_locked;
    let ids: Vec<_> = app.ui.dock.arrangement.floating.iter().map(|g| g.id).collect();
    for id in ids {
        let Some(mut group) = app.ui.dock.arrangement.floating.iter().find(|g| g.id == id).cloned() else { continue };
        if !group.open || group.tabs.is_empty() {
            continue;
        }
        let panels: Vec<_> = group
            .tabs
            .iter()
            .copied()
            .filter(|panel| !app.ui.dock.hidden_tabs.get(&panel.group).is_some_and(|hidden| panel.label().is_some_and(|name| hidden.iter().any(|h| h == name))))
            .collect();
        if panels.is_empty() {
            continue;
        }
        let mut selected = group.tabs.get(group.selected).and_then(|p| panels.iter().position(|v| v == p)).unwrap_or(0);
        let labels: Vec<_> = panels.iter().filter_map(|p| p.label()).collect();
        let title = labels.get(selected).copied().unwrap_or("Panel");
        let screen = ctx.content_rect();
        let position = pos2(
            group.position[0].clamp(screen.left(), (screen.right() - 100.0).max(screen.left())),
            group.position[1].clamp(screen.top(), (screen.bottom() - 50.0).max(screen.top())),
        );
        let mut dock_home = false;
        let mut requested = None;
        let expanded_id = egui::Id::new(("floating-panel", group.id));
        let was_collapsed = group.collapsed;
        // Collapsing must not replace egui's remembered expanded resize state.
        let window_id = if was_collapsed { expanded_id.with("collapsed") } else { expanded_id };
        let mut move_delta = egui::Vec2::ZERO;
        let frame = egui::Frame::window(&ctx.global_style()).inner_margin(0);
        let mut window = egui::Window::new(tl!(title))
            .id(window_id)
            .frame(frame)
            .title_bar(false)
            .movable(!locked)
            .resizable(!locked && !was_collapsed)
            .collapsible(false)
            .current_pos(position)
            .default_size(group.size)
            .min_size([180.0, if was_collapsed { 26.0 } else { 120.0 }]);
        if was_collapsed {
            window = window.fixed_size([group.size[0], 28.0]);
        }
        let shown = window.show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let header = crate::widgets::floating_header(ui, &format!("floating-panel-{}", group.id), &labels, &mut selected, was_collapsed);
            if header.close {
                group.open = false;
            }
            if header.toggle_collapse || (was_collapsed && header.card.tab_clicked) {
                group.collapsed = !group.collapsed;
                ctx.request_repaint();
            }
            group.selected = panels.get(selected).and_then(|p| group.tabs.iter().position(|v| v == p)).unwrap_or(0);
            let response = header.card;
            if let Some(action) = response.tab_context {
                match action {
                    crate::tab_strip::TabContextAction::Close(i) => {
                        if let Some(panel) = panels.get(i) {
                            app.ui.dock.hide_tab(panel.group, panel.source_index(pro), pro);
                        }
                    }
                    crate::tab_strip::TabContextAction::CloseGroup => group.open = false,
                }
            }
            if !was_collapsed {
                // Allocate the viewport once. A child's natural size cannot expand the
                // parent window; self-scrolling panels get the actual bounded height.
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ui.available_height().max(0.0)), egui::Sense::hover());
                let inner = rect.shrink2(vec2(8.0, 6.0));
                let mut body = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                body.set_clip_rect(ui.clip_rect().intersect(inner));
                body.spacing_mut().item_spacing.y = ctx.global_style().spacing.item_spacing.y;
                if let Some(panel) = group.tabs.get(group.selected).copied() {
                    *panel.group.tab_mut(&mut app.ui.dock_tabs) = panel.source_index(pro);
                    if panel.group.scrolls_itself(panel.source_index(pro)) {
                        crate::panels::dock_body(app, &mut body, panel.group, panel.source_index(pro));
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt(("floating-panel-scroll", group.id, panel))
                            .max_height(inner.height().max(0.0))
                            .auto_shrink([false, false])
                            .show(&mut body, |ui| crate::panels::dock_body(app, ui, panel.group, panel.source_index(pro)));
                    }
                    let current = *panel.group.tab_mut(&mut app.ui.dock_tabs);
                    if current != panel.source_index(pro) {
                        requested = Some(panel.group);
                    }
                }
            }
            register(ctx, Location::Floating(group.id), ui.min_rect(), &panels, &response);
            if response.tab_drag.is_some() {
                track(app, ctx, Location::Floating(group.id), &panels, &response);
            } else if !locked && response.strip.dragged() {
                move_delta = ctx.input(|i| i.pointer.delta());
                if !ctx.data(|d| d.get_temp::<bool>(canceled_id()).unwrap_or(false)) {
                    ctx.data_mut(|d| {
                        if d.get_temp::<Drag>(drag_id()).is_none() {
                            d.insert_temp(
                                drag_id(),
                                Drag { panels: group.tabs.clone(), source: Location::Floating(group.id), whole: true, moving_window: true, preview: None },
                            );
                        }
                    });
                }
                ctx.request_repaint();
            }
            egui::Popup::menu(&response.menu).show(|ui| {
                if ui.button(if group.collapsed { tl!("Expand Panel Group") } else { tl!("Collapse Panel Group") }).clicked() {
                    group.collapsed = !group.collapsed;
                    ui.close();
                }
                if ui.add_enabled(!locked, egui::Button::new(tl!("Dock Panel Group"))).clicked() {
                    dock_home = true;
                    ui.close();
                }
            });
        });
        if let Some(shown) = shown {
            group.position = [shown.response.rect.left() + move_delta.x, shown.response.rect.top() + move_delta.y];
            if !was_collapsed {
                group.size = [shown.response.rect.width(), shown.response.rect.height()];
            }
            ctx.data_mut(|d| {
                let mut all: Vec<Target> = d.get_temp(targets_id()).unwrap_or_default();
                if let Some(target) = all.iter_mut().find(|t| t.location == Location::Floating(group.id)) {
                    target.rect = shown.response.rect;
                }
                d.insert_temp(targets_id(), all);
            });
        }
        if let Some(saved) = app.ui.dock.arrangement.floating.iter_mut().find(|g| g.id == group.id) {
            *saved = group.clone();
        }
        if let Some(source) = requested {
            reveal(app, source);
        }
        if dock_home {
            for panel in group.tabs {
                app.ui.dock.arrangement.place(&[panel], Some(Location::Dock(panel.group)), None, position, pro);
                *panel.group.shown_mut(&mut app.ui.panels) = true;
                app.ui.dock.set_collapsed(panel.group, false);
            }
        }
    }
}
