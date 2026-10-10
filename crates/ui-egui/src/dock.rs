//! Right-dock layout: a column of panes, each a tab strip of [modules](crate::modules).
//!
//! Any module can sit in any pane. Drag a tab onto another pane's strip to group it there,
//! between two panes (or past the last) to give it a pane of its own, and drag a strip's
//! empty part to move the whole pane. A pane's height never follows its content: content
//! taller than the pane scrolls inside it, and the last expanded pane (Layers by default)
//! fills what the others leave, so the column itself never scrolls. Drag the gap between two
//! panes to resize them; collapse a pane to its tab strip with a double-click (Pro) or its
//! chevron (Studio). Window › Workspace › Lock Workspace freezes all of it.
//!
//! In the Pro themes the dock looks and behaves like Photoshop's (tab strips, the icon rail at its
//! side). In the others it is an inspector of rounded cards with a header that collapses it to an
//! icon rail (each icon opens its module as a flyout) and a + that lists every module.
//!
//! The layout is [`DockLayout`] in `UiState::dock` (serialisable, drivable with `ui.set`), saved
//! with Window › Workspace › New Workspace…, reset by Reset Workspace, and remembered across
//! launches in the preferences (`panelLayout`) while Remember Workspace Changes is on. Layouts
//! saved before modules (fixed groups: `panels` flags, `dockTabs`, `dock.order`) are migrated.

use std::collections::BTreeMap;

use egui::{Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::modules::{self, Module};
use crate::theme::Tokens;
use crate::widgets;

/// One pane of the dock: a tab strip of modules.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pane {
    /// Module ids, in tab order.
    pub tabs: Vec<String>,
    /// The front tab's module id.
    pub active: String,
    /// The height the user dragged it to (points, tab strip included). Unset = its default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// Collapsed to its tab strip.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

impl Pane {
    pub fn new(tabs: &[&str]) -> Self {
        Self { tabs: tabs.iter().map(|s| (*s).to_owned()).collect(), active: tabs.first().map(|s| (*s).to_owned()).unwrap_or_default(), ..Self::default() }
    }

    /// The front tab (the stored one when it is still a tab, else the first).
    pub fn front(&self) -> &str {
        if self.tabs.contains(&self.active) { &self.active } else { self.tabs.first().map_or("", String::as_str) }
    }

    /// The module that sizes the pane: its first tab.
    fn lead(&self) -> Option<&'static Module> {
        self.tabs.iter().find_map(|t| modules::get(t))
    }

    /// The smallest height any of its modules accepts.
    fn min_height(&self) -> f32 {
        self.tabs.iter().filter_map(|t| modules::get(t)).map(|m| m.size.min).fold(80.0, f32::max)
    }

    fn default_height(&self) -> f32 {
        self.lead().map_or(200.0, |m| m.size.default).max(self.min_height())
    }

    fn compact_height(&self) -> f32 {
        self.lead().map_or(130.0, |m| m.size.compact).max(self.min_height())
    }

    fn preferred_fill(&self) -> f32 {
        self.lead().map_or(0.0, |m| m.size.fill).max(self.min_height())
    }

    /// The stored (or default) height, sanitised.
    fn height(&self) -> f32 {
        match self.height {
            Some(h) if h.is_finite() => h.clamp(self.min_height(), MAX_HEIGHT),
            _ => self.default_height(),
        }
    }

    /// A stable key for egui ids (its first tab).
    fn key(&self) -> &str {
        self.tabs.first().map_or("", String::as_str)
    }
}

/// The dock's panes, top to bottom, and (Studio) whether it is collapsed to its icon rail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockLayout {
    #[serde(alias = "sections")]
    pub panes: Vec<Pane>,
    /// Studio themes: the inspector is collapsed to an icon rail.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub rail: bool,
    /// The module shown as a flyout from the collapsed rail (not saved).
    #[serde(skip)]
    pub flyout: Option<String>,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self::essentials(true)
    }
}

/// Gap between panes; it is also the splitter's grab area.
pub const GAP: f32 = 6.0;
/// Upper bound on a stored height (guards against absurd values from `ui.set`).
const MAX_HEIGHT: f32 = 4000.0;
/// Most panes a layout keeps (guards against absurd `ui.set` input).
const MAX_PANES: usize = 64;

/// The preset groups: the fixed groups of layouts saved before modules and of the workspace
/// presets, in Photoshop's Essentials order. Pro puts Color before Swatches, like Photoshop;
/// Studio leads with Swatches.
pub const GROUPS: [&str; 6] = ["color", "properties", "character", "navigator", "history", "layers"];

/// A preset group's modules, in tab order.
pub fn group_tabs(key: &str, pro: bool) -> &'static [&'static str] {
    match key {
        "color" if pro => &["color", "swatches", "gradients", "patterns"],
        "color" => &["swatches", "color", "gradients", "patterns"],
        "properties" => &["properties", "adjustments"],
        "character" => &["character", "paragraph"],
        "navigator" => &["navigator", "histogram", "info"],
        "history" => &["history", "actions", "layerComps"],
        "layers" => &["layers", "channels", "paths"],
        _ => &[],
    }
}

/// Where a dragged tab lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    /// Join pane `i` as its front tab, after its other tabs.
    Join(usize),
    /// Join pane `i` as its front tab at slot `k` of its tabs (`k` counted before the dragged
    /// tab leaves; `tabs.len()` = last). The same pane reorders its tabs.
    Insert(usize, usize),
    /// A pane of its own, inserted at index `i` (`panes.len()` = last).
    NewAt(usize),
}

impl DockLayout {
    /// A layout of preset groups (`GROUPS` keys), in the order given.
    pub fn preset(groups: &[&str], pro: bool) -> Self {
        Self { panes: groups.iter().map(|g| Pane::new(group_tabs(g, pro))).filter(|s| !s.tabs.is_empty()).collect(), rail: false, flyout: None }
    }

    /// Photoshop's Essentials: Color, Properties, Layers.
    pub fn essentials(pro: bool) -> Self {
        Self::preset(&["color", "properties", "layers"], pro)
    }

    /// Switching between Pro and Studio: an untouched Color preset group takes the theme's tab
    /// order (Photoshop leads with Color, Studio with Swatches), keeping the front tab the
    /// order's first when it was.
    pub fn follow_theme(&mut self, pro: bool) {
        let (from, to) = (group_tabs("color", !pro), group_tabs("color", pro));
        for s in &mut self.panes {
            if s.tabs.iter().map(String::as_str).eq(from.iter().copied()) {
                if from.first().is_some_and(|f| s.active == *f) {
                    s.active = to.first().map(|t| (*t).to_owned()).unwrap_or_default();
                }
                s.tabs = to.iter().map(|t| (*t).to_owned()).collect();
            }
        }
    }

    /// The pane holding module `id`.
    pub fn pane_of(&self, id: &str) -> Option<usize> {
        self.panes.iter().position(|s| s.tabs.iter().any(|t| t == id))
    }

    /// Module `id` is in the dock (collapsed or not).
    pub fn visible(&self, id: &str) -> bool {
        self.pane_of(id).is_some()
    }

    /// Module `id` is its pane's front tab and the pane is expanded.
    pub fn is_front(&self, id: &str) -> bool {
        self.pane_of(id).and_then(|i| self.panes.get(i)).is_some_and(|s| !s.collapsed && s.front() == id)
    }

    /// Bring module `id` forward: added if it isn't in the dock (to a pane holding one of its
    /// siblings, else a new pane above the filling one), made its pane's front tab, and the
    /// pane expanded. A panel asked for always comes back, whatever state it was left in (#129).
    pub fn show(&mut self, id: &str) {
        let Some(m) = modules::get(id) else { return };
        let i = match self.pane_of(id) {
            Some(i) => i,
            None => match m.siblings.iter().find_map(|s| self.pane_of(s)) {
                Some(i) => {
                    if let Some(s) = self.panes.get_mut(i) {
                        s.tabs.push(id.to_owned());
                    }
                    i
                }
                None => match GROUPS.iter().find(|g| group_tabs(g, true).contains(&id)) {
                    // A preset group's module brings its whole group back, in its Essentials place.
                    Some(g) => {
                        self.show_group(g, true);
                        let Some(i) = self.pane_of(id) else { return };
                        i
                    }
                    None => {
                        // Above the filler, so Layers keeps the rest of the column.
                        let at = self.panes.iter().rposition(|s| !s.collapsed).unwrap_or(self.panes.len());
                        self.panes.insert(at, Pane::new(&[id]));
                        at
                    }
                },
            },
        };
        if let Some(s) = self.panes.get_mut(i) {
            s.active = id.to_owned();
            s.collapsed = false;
        }
    }

    /// The + picker: module `id` as a pane of its own, above the filler, whatever else is
    /// docked (`show` would join a sibling's pane as a tab, or bring a whole group back). A
    /// module already docked is just brought forward.
    pub fn add_pane(&mut self, id: &str) {
        if modules::get(id).is_none() {
            return;
        }
        if self.visible(id) {
            self.show(id);
            return;
        }
        let at = self.panes.iter().rposition(|s| !s.collapsed).unwrap_or(self.panes.len());
        self.panes.insert(at, Pane::new(&[id]));
    }

    /// Take module `id` out of the dock; a pane left empty goes too.
    pub fn close(&mut self, id: &str) {
        for s in &mut self.panes {
            s.tabs.retain(|t| t != id);
        }
        self.panes.retain(|s| !s.tabs.is_empty());
        if self.flyout.as_deref() == Some(id) {
            self.flyout = None;
        }
    }

    /// Hide the pane holding module `id` (Window › <panel> on a showing panel hides its panel
    /// group, like Photoshop).
    pub fn hide(&mut self, id: &str) {
        if let Some(i) = self.pane_of(id) {
            self.panes.remove(i);
        }
        if self.flyout.as_deref().is_some_and(|f| self.pane_of(f).is_none()) {
            self.flyout = None;
        }
    }

    /// Window › <panel>: like Photoshop, choosing a panel that is showing hides it; otherwise it
    /// is brought forward (a collapsed or background tab counts as not showing, #129).
    /// Returns whether it is visible afterwards.
    pub fn toggle(&mut self, id: &str) -> bool {
        if self.is_front(id) {
            self.hide(id);
            false
        } else {
            self.show(id);
            true
        }
    }

    /// A preset group is showing: any of its modules is in the dock.
    pub fn group_shown(&self, key: &str) -> bool {
        group_tabs(key, true).iter().any(|m| self.visible(m))
    }

    /// Show a preset group: its front module is brought forward, or (none of it showing) the
    /// whole group comes back as a pane, in its Essentials place.
    pub fn show_group(&mut self, key: &str, pro: bool) {
        let tabs = group_tabs(key, pro);
        if let Some(id) = tabs.iter().find(|m| self.visible(m)) {
            self.show(id);
            return;
        }
        if tabs.is_empty() {
            return;
        }
        // Below the last pane that holds a group preceding it in GROUPS.
        let rank = |s: &Pane| GROUPS.iter().position(|g| group_tabs(g, pro).iter().any(|m| s.tabs.iter().any(|t| t == m)));
        let mine = GROUPS.iter().position(|g| *g == key);
        let at = self.panes.iter().rposition(|s| rank(s).zip(mine).is_some_and(|(r, me)| r < me)).map_or(0, |i| i + 1);
        self.panes.insert(at.min(self.panes.len()), Pane::new(tabs));
    }

    /// Hide every module of a preset group.
    pub fn hide_group(&mut self, key: &str) {
        for m in group_tabs(key, true) {
            self.close(m);
        }
    }

    pub fn set_collapsed(&mut self, i: usize, on: bool) {
        if let Some(s) = self.panes.get_mut(i) {
            s.collapsed = on;
        }
    }

    /// Move pane `i` so it is drawn just before pane `before` (or last when `None`).
    pub fn move_pane(&mut self, i: usize, before: Option<usize>) {
        if before == Some(i) || i >= self.panes.len() {
            return;
        }
        let s = self.panes.remove(i);
        let at = match before {
            Some(b) if b > i => b - 1,
            Some(b) => b,
            None => self.panes.len(),
        };
        self.panes.insert(at.min(self.panes.len()), s);
    }

    /// Put module `id` in pane `i` as its front tab, from wherever it is (or isn't).
    pub fn drop_tab_or_add(&mut self, id: &str, i: usize) {
        if modules::get(id).is_none() || i >= self.panes.len() {
            return;
        }
        if self.pane_of(id).is_some() {
            self.drop_tab(id, Drop::Join(i));
        } else if let Some(s) = self.panes.get_mut(i) {
            s.tabs.push(id.to_owned());
            s.active = id.to_owned();
            s.collapsed = false;
        }
    }

    /// Move module `id` to where a tab drag dropped it.
    pub fn drop_tab(&mut self, id: &str, to: Drop) {
        let Some(from) = self.pane_of(id) else { return };
        let alone = self.panes.get(from).is_some_and(|s| s.tabs.len() == 1);
        let at = self.panes.get(from).and_then(|s| s.tabs.iter().position(|t| t == id)).unwrap_or(0);
        match to {
            Drop::Join(t) if t == from || t >= self.panes.len() => return,
            Drop::Insert(t, _) if t >= self.panes.len() => return,
            Drop::Insert(t, k) if t == from && (k == at || k == at + 1) => {
                // Next to itself: nothing moves, but the tab comes forward.
                if let Some(s) = self.panes.get_mut(t) {
                    s.active = id.to_owned();
                }
                return;
            }
            Drop::NewAt(k) if alone && (k == from || k == from + 1) => return,
            _ => {}
        }
        if let Some(s) = self.panes.get_mut(from) {
            s.tabs.retain(|t| t != id);
        }
        match to {
            Drop::Insert(t, k) => {
                let k = if t == from && k > at { k - 1 } else { k };
                if let Some(s) = self.panes.get_mut(t) {
                    s.tabs.insert(k.min(s.tabs.len()), id.to_owned());
                    s.active = id.to_owned();
                    s.collapsed = false;
                }
            }
            Drop::Join(t) => {
                if let Some(s) = self.panes.get_mut(t) {
                    s.tabs.push(id.to_owned());
                    s.active = id.to_owned();
                    s.collapsed = false;
                }
            }
            Drop::NewAt(k) => self.panes.insert(k.min(self.panes.len()), Pane::new(&[id])),
        }
        self.panes.retain(|s| !s.tabs.is_empty());
    }

    /// Drop unknown and repeated modules, empty panes and absurd heights (input from files,
    /// `ui.set` or a plug-in that is gone).
    pub fn sanitize(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        for s in &mut self.panes {
            s.tabs.retain(|t| {
                let keep = modules::get(t).is_some() && !seen.contains(t);
                if keep {
                    seen.push(t.clone());
                }
                keep
            });
            if !s.tabs.contains(&s.active) {
                s.active = s.tabs.first().cloned().unwrap_or_default();
            }
            s.height = s.height.filter(|h| h.is_finite() && *h > 0.0).map(|h| h.min(MAX_HEIGHT));
        }
        self.panes.retain(|s| !s.tabs.is_empty());
        self.panes.truncate(MAX_PANES);
        if self.flyout.as_deref().is_some_and(|f| !self.visible(f)) {
            self.flyout = None;
        }
    }

    /// Lay out the panes in a column `avail` points tall with `strip`-high tab strips.
    /// Returns each pane's height. The last expanded pane fills the rest. Panes the user
    /// never resized give way first, down to their compact heights, so the filler gets its
    /// preferred height (Layers: ~10 rows, #147); when the column is still too short every
    /// pane gives way down to its minimum height.
    pub fn heights_for(&self, avail: f32, strip: f32) -> Vec<f32> {
        let avail = if avail.is_finite() { avail.max(0.0) } else { 0.0 };
        let panes = &self.panes;
        let filler = panes.iter().rposition(|s| !s.collapsed);
        let mut hs: Vec<f32> = panes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if s.collapsed {
                    strip
                } else if Some(i) == filler {
                    0.0
                } else {
                    s.height()
                }
            })
            .collect();
        if let Some(f) = filler {
            let gaps = GAP * panes.len().saturating_sub(1) as f32;
            let min_fill = panes.get(f).map_or(0.0, Pane::min_height);
            let pref_fill = panes.get(f).map_or(0.0, Pane::preferred_fill);
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + pref_fill - avail).max(0.0);
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(s) = panes.get(i) else { continue };
                if s.collapsed || s.height.is_some() {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - s.compact_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let used: f32 = hs.iter().sum::<f32>() + gaps;
            let mut deficit = (used + min_fill - avail).max(0.0);
            // Squeeze the expanded panes nearest the filler first.
            for i in (0..f).rev() {
                if deficit <= 0.0 {
                    break;
                }
                let Some(s) = panes.get(i) else { continue };
                if s.collapsed {
                    continue;
                }
                if let Some(h) = hs.get_mut(i) {
                    let give = (*h - s.min_height()).max(0.0).min(deficit);
                    *h -= give;
                    deficit -= give;
                }
            }
            let rest = avail - hs.iter().sum::<f32>() - gaps;
            if let Some(h) = hs.get_mut(f) {
                *h = rest.max(min_fill);
            }
        }
        hs
    }

    /// Splitters push expanded panes on the side they move into, nearest first, down to
    /// each pane's minimum. The opposite pane grows; the filler absorbs its share implicitly.
    fn resize(&mut self, heights: &[f32], i: usize, dy: f32) {
        if !dy.is_finite() || dy == 0.0 {
            return;
        }
        let expanded: Vec<usize> = self.panes.iter().zip(heights).enumerate().filter_map(|(k, (pane, _))| (!pane.collapsed).then_some(k)).collect();
        if !expanded.contains(&i) {
            return;
        }
        let filler = expanded.last().copied();
        let Some(j) = expanded.iter().copied().find(|k| *k > i) else { return };
        let (grow, shrink): (usize, Vec<usize>) = if dy > 0.0 {
            (i, expanded.iter().copied().filter(|k| *k >= j).collect())
        } else {
            (j, expanded.iter().rev().copied().filter(|k| *k <= i).collect())
        };
        // Pin other expanded panes at their displayed heights on the first drag.
        for (k, (pane, height)) in self.panes.iter_mut().zip(heights).enumerate() {
            if Some(k) != filler && !pane.collapsed && pane.height.is_none() {
                pane.height = Some(*height);
            }
        }
        let mut left = dy.abs();
        for k in shrink {
            if left <= 0.0 {
                break;
            }
            let (Some(pane), Some(&height)) = (self.panes.get_mut(k), heights.get(k)) else { continue };
            let give = (height - pane.min_height()).max(0.0).min(left);
            left -= give;
            if Some(k) != filler {
                pane.height = Some(height - give);
            }
        }
        if Some(grow) != filler
            && let (Some(pane), Some(&height)) = (self.panes.get_mut(grow), heights.get(grow))
        {
            pane.height = Some(height + (dy.abs() - left));
        }
    }
}

/// The current theme is a Pro (Photoshop) one.
pub fn is_pro(app: &PhotocraftApp) -> bool {
    matches!(app.ui.theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium)
}

/// Show module `id` (Window › <panel>, the icon rail, the picker).
pub fn reveal(app: &mut PhotocraftApp, id: &str) {
    app.ui.dock.show(id);
    app.ui.panels.dock = true;
}

/// Pro icon rail click: a hidden module is shown, a background tab brought forward, a collapsed
/// pane expanded and the expanded front module's pane collapsed to its tab strip. A docked panel
/// is never hidden from the rail (it used to toggle visibility, so one stray click made a panel
/// vanish: #129).
pub fn rail_click(app: &mut PhotocraftApp, id: &str) {
    match app.ui.dock.pane_of(id) {
        Some(i) if app.ui.dock.is_front(id) => app.ui.dock.set_collapsed(i, true),
        _ => reveal(app, id),
    }
}

/// Per-pane interactions collected while drawing, applied afterwards.
enum Action {
    Select(usize, String),
    ToggleCollapse(usize),
    ClosePane(usize),
    CloseTab(String),
    MovePane(usize, Option<usize>),
    DropTab(String, Drop),
}

/// Rects of the panes drawn last frame (screen points), keyed by their front module, for
/// tests and automation.
pub fn last_rects(ctx: &egui::Context) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(rects_id())).unwrap_or_default()
}

/// The rect of the pane holding module `id`, as drawn last frame.
pub fn module_rect(app: &PhotocraftApp, ctx: &egui::Context, id: &str) -> Option<Rect> {
    let i = app.ui.dock.pane_of(id)?;
    let s = app.ui.dock.panes.get(i)?;
    last_rects(ctx).into_iter().find(|(f, _)| f == s.front()).map(|(_, r)| r)
}

fn rects_id() -> egui::Id {
    egui::Id::new("dock-pane-rects")
}

/// A pane's tab strip as drawn last frame (screen points), for tests and automation.
#[derive(Clone, Debug, PartialEq)]
pub struct StripRects {
    /// The pane's index, top to bottom.
    pub pane: usize,
    /// `(module id, rect)` of the tabs on the strip (the others are in the chevron menu).
    pub tabs: Vec<(String, Rect)>,
    /// The panel menu button.
    pub menu: Rect,
    /// The » overflow button, when some tabs didn't fit.
    pub chevron: Option<Rect>,
    /// Studio: the collapse and close buttons.
    pub collapse: Option<Rect>,
    pub close: Option<Rect>,
}

/// The tab strips drawn last frame.
pub fn last_strips(ctx: &egui::Context) -> Vec<StripRects> {
    ctx.data(|d| d.get_temp::<Vec<StripRects>>(strips_id())).unwrap_or_default()
}

fn strips_id() -> egui::Id {
    egui::Id::new("dock-strip-rects")
}

/// Height of a pane's tab strip (a collapsed pane is just this).
pub fn strip_height(pro: bool) -> f32 {
    if pro { 28.0 } else { 40.0 }
}

/// Draw one module's body in `ui`, bounded to the height it has (scrolling unless it fills).
fn module_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, m: &Module, salt: &str) {
    let inner = ui.available_height().max(0.0);
    if Tokens::get(ui.ctx()).pro && m.id == "color" {
        ui.data_mut(|d| d.insert_temp(crate::panels::color_field_fill_id(), inner));
    }
    if m.fills {
        ui.set_min_height(inner);
        (m.body)(app, ui);
    } else {
        egui::ScrollArea::vertical()
            .id_salt(("dock-scroll", salt, m.id))
            .max_height(inner)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show(ui, |ui| (m.body)(app, ui));
    }
}

/// Draw the dock's panes filling `ui`.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let strip = strip_height(t.pro);
    let area = ui.available_rect_before_wrap();
    app.ui.dock.sanitize();
    let heights = app.ui.dock.heights_for(area.height(), strip);
    let locked = app.session.prefs().workspace_locked;
    let rects = rects_after_layout(&heights, area);
    let mut actions: Vec<Action> = Vec::new();
    let mut moving: Option<usize> = None;
    let mut tab_drag: Option<String> = None;
    let mut tab_dropped: Option<String> = None;
    let mut strips: Vec<StripRects> = Vec::with_capacity(rects.len());
    let panes = app.ui.dock.panes.clone();
    let count = panes.len();
    for (i, (pane, rect)) in panes.iter().zip(rects.iter().copied()).enumerate() {
        let collapsed = pane.collapsed;
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("dock-pane", pane.key())).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing.y = if t.pro { 0.0 } else { 6.0 };
        let mods: Vec<&'static Module> = pane.tabs.iter().filter_map(|id| modules::get(id)).collect();
        if mods.is_empty() {
            continue;
        }
        let titles: Vec<&str> = mods.iter().map(|m| m.title).collect();
        let front = pane.front().to_owned();
        let mut sel = mods.iter().position(|m| m.id == front).unwrap_or(0);
        let before = sel;
        let resp = widgets::dock_card(&mut child, pane.key(), &titles, &mut sel, collapsed, |ui, shown| {
            if let Some(m) = mods.get(shown) {
                module_body(app, ui, m, pane.key());
            }
        });
        strips.push(StripRects {
            pane: i,
            tabs: resp.tabs.iter().filter_map(|(k, r)| mods.get(*k).map(|m| (m.id.to_owned(), *r))).collect(),
            menu: resp.menu.rect,
            chevron: resp.chevron,
            collapse: resp.collapse.as_ref().map(|r| r.rect),
            close: resp.close.as_ref().map(|r| r.rect),
        });
        if sel != before
            && let Some(m) = mods.get(sel)
        {
            actions.push(Action::Select(i, m.id.to_owned()));
        }
        if let Some(context) = resp.tab_context {
            match context {
                crate::tab_strip::TabContextAction::Close(k) => {
                    if let Some(m) = mods.get(k) {
                        actions.push(Action::CloseTab(m.id.to_owned()));
                    }
                }
                crate::tab_strip::TabContextAction::CloseGroup => actions.push(Action::ClosePane(i)),
            }
        }
        if resp.strip.double_clicked() || resp.tab_double_clicked || (collapsed && resp.tab_clicked) || resp.collapse.as_ref().is_some_and(|r| r.clicked()) {
            actions.push(Action::ToggleCollapse(i));
        }
        if resp.close.as_ref().is_some_and(|r| r.clicked()) {
            actions.push(Action::ClosePane(i));
        }
        if !locked {
            if resp.strip.dragged() {
                moving = Some(i);
            }
            if resp.strip.drag_stopped()
                && let Some(p) = ui.ctx().pointer_interact_pos()
            {
                actions.push(Action::MovePane(i, pane_drop_before(&rects, i, p.y)));
            }
            if let Some(m) = resp.tab_drag.and_then(|k| mods.get(k)) {
                tab_drag = Some(m.id.to_owned());
            }
            if let Some(m) = resp.tab_drag_stopped.and_then(|k| mods.get(k)) {
                tab_dropped = Some(m.id.to_owned());
            }
        }
        if let Some(add) = &resp.add {
            egui::Popup::menu(add)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| picker_list(app, ui, add.id.with("search"), Some(i)));
        }
        let front_module = mods.get(sel).copied();
        egui::Popup::menu(&resp.menu).show(|ui| {
            crate::widgets::style_spectrum_popup_menu(ui);
            ui.set_min_width(170.0);
            if let Some(extra) = front_module.and_then(|m| m.menu) {
                extra(app, ui);
                ui.separator();
            }
            ui.menu_button(tl!("Add Tab"), |ui| picker_list(app, ui, egui::Id::new(("dock-add-tab", i)), Some(i)));
            ui.separator();
            if ui.button(if collapsed { tl!("Expand Panel Group") } else { tl!("Collapse Panel Group") }).clicked() {
                actions.push(Action::ToggleCollapse(i));
                ui.close();
            }
            if ui.add_enabled(!locked && i > 0, egui::Button::new(tl!("Move Panel Group Up"))).clicked() {
                actions.push(Action::MovePane(i, Some(i.saturating_sub(1))));
                ui.close();
            }
            if ui.add_enabled(!locked && i + 1 < count, egui::Button::new(tl!("Move Panel Group Down"))).clicked() {
                actions.push(Action::MovePane(i, if i + 2 < count { Some(i + 2) } else { None }));
                ui.close();
            }
            ui.separator();
            if let Some(m) = front_module
                && ui.button(tl!("Close")).clicked()
            {
                actions.push(Action::CloseTab(m.id.to_owned()));
                ui.close();
            }
            if ui.button(tl!("Close Tab Group")).clicked() {
                actions.push(Action::ClosePane(i));
                ui.close();
            }
        });
        // Splitter in the gap below this pane: resizes it against the next expanded one.
        if i + 1 < count && !collapsed && panes.iter().skip(i + 1).any(|s| !s.collapsed) {
            let gap = Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(rect.width(), GAP)).expand2(vec2(0.0, 2.0));
            let sresp = ui.interact(gap, ui.id().with(("dock-splitter", i)), Sense::drag());
            if sresp.hovered() || sresp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                ui.painter().line_segment([gap.left_center(), gap.right_center()], Stroke::new(2.0, t.accent.gamma_multiply(0.7)));
            }
            if sresp.dragged() {
                app.ui.dock.resize(&heights, i, sresp.drag_delta().y);
            }
        }
    }
    let pointer = ui.ctx().pointer_interact_pos();
    // A pane dragged by its strip: an insertion line where it would land.
    if let (Some(i), Some(p)) = (moving, pointer) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let before = pane_drop_before(&rects, i, p.y);
        let line_y = match before.and_then(|b| rects.get(b)) {
            Some(r) => r.top() - GAP / 2.0,
            None => rects.last().map_or(area.top(), |r| r.bottom() + GAP / 2.0),
        };
        ui.painter().line_segment([pos2(area.left(), line_y), pos2(area.right(), line_y)], Stroke::new(3.0, t.accent));
    }
    // A tab dragged: outline the pane it would join, or a line where its new pane would go.
    if let (Some(id), Some(p)) = (tab_drag.as_ref().or(tab_dropped.as_ref()), pointer) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let target = match tab_drop_target(&rects, strip, p) {
            Drop::Join(k) => tab_slot(&strips, &panes, k, p).map_or(Drop::Join(k), |at| Drop::Insert(k, at)),
            other => other,
        };
        match target {
            Drop::Insert(k, at) => {
                // An insertion line between the tabs where it would land.
                let shown = strips.iter().find(|s| s.pane == k).map(|s| s.tabs.as_slice()).unwrap_or_default();
                let tabs = panes.get(k).map(|s| s.tabs.as_slice()).unwrap_or_default();
                let before = tabs.get(at).and_then(|id| shown.iter().find(|(t, _)| t == id));
                let after = at.checked_sub(1).and_then(|i| tabs.get(i)).and_then(|id| shown.iter().find(|(t, _)| t == id));
                match (before, after) {
                    (Some((_, r)), _) => widgets::drop_line(ui, *r, false, true, &t),
                    (None, Some((_, r))) => widgets::drop_line(ui, *r, true, true, &t),
                    _ => {}
                }
            }
            Drop::Join(k) => {
                if let Some(r) = rects.get(k)
                    && app.ui.dock.pane_of(id) != Some(k)
                {
                    ui.painter().rect_stroke(r.shrink(1.0), t.radius, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
            }
            Drop::NewAt(k) => {
                let y = match rects.get(k) {
                    Some(r) => r.top() - GAP / 2.0,
                    None => rects.last().map_or(area.top(), |r| r.bottom() + GAP / 2.0),
                };
                ui.painter().line_segment([pos2(area.left(), y), pos2(area.right(), y)], Stroke::new(3.0, t.accent));
            }
        }
        if let Some(id) = tab_dropped {
            actions.push(Action::DropTab(id, target));
        }
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(rects_id(), panes.iter().zip(&rects).map(|(s, r)| (s.front().to_owned(), *r)).collect::<Vec<_>>());
        d.insert_temp(strips_id(), strips);
    });
    ui.advance_cursor_after_rect(area);
    // Applied after drawing, most index-sensitive last.
    for a in actions {
        let dock = &mut app.ui.dock;
        match a {
            Action::Select(i, id) => {
                if let Some(s) = dock.panes.get_mut(i) {
                    s.active = id;
                }
            }
            Action::ToggleCollapse(i) => {
                let on = dock.panes.get(i).is_some_and(|s| !s.collapsed);
                dock.set_collapsed(i, on);
            }
            Action::CloseTab(id) => dock.close(&id),
            Action::ClosePane(i) => {
                if i < dock.panes.len() {
                    dock.panes.remove(i);
                }
            }
            Action::MovePane(i, before) => dock.move_pane(i, before),
            Action::DropTab(id, to) => dock.drop_tab(&id, to),
        }
        // Indices shift after the first structural change; one per frame is all a user makes.
        if dock.panes.len() != count {
            break;
        }
    }
}

fn rects_after_layout(heights: &[f32], area: Rect) -> Vec<Rect> {
    let mut y = area.top();
    heights
        .iter()
        .map(|h| {
            let r = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), h.max(0.0)));
            y = r.bottom() + GAP;
            r
        })
        .collect()
}

/// The pane a pane dragged from index `dragged` lands before when released at `y`
/// (`None` = last). Dropping onto itself, or just below itself, keeps its place.
fn pane_drop_before(rects: &[Rect], dragged: usize, y: f32) -> Option<usize> {
    match rects.iter().position(|r| y < r.center().y) {
        Some(k) if k == dragged || k == dragged + 1 => Some(dragged),
        other => other,
    }
}

/// The slot of pane `k`'s tabs a tab dragged to `x` on its strip lands at: before the first shown
/// tab whose middle is right of `p`, else after the last shown one. `None` off the strip's tab
/// row (the pane has none shown, or the pointer is over its body).
fn tab_slot(strips: &[StripRects], panes: &[Pane], k: usize, p: egui::Pos2) -> Option<usize> {
    let shown = &strips.iter().find(|s| s.pane == k)?.tabs;
    let row = shown.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b))?;
    if !(row.top() - 4.0..=row.bottom() + 4.0).contains(&p.y) {
        return None;
    }
    let tabs = &panes.get(k)?.tabs;
    let index = |id: &str| tabs.iter().position(|t| t == id);
    match shown.iter().find(|(_, r)| r.center().x >= p.x) {
        Some((id, _)) => index(id),
        None => shown.last().and_then(|(id, _)| index(id)).map(|i| i + 1),
    }
}

/// Where a tab dragged to `p` lands: a pane's tab strip, or its upper body, joins it; the
/// lower edge of a pane, a gap, or below the last pane makes a new pane there.
fn tab_drop_target(rects: &[Rect], strip: f32, p: egui::Pos2) -> Drop {
    let edge = (strip * 0.6).max(12.0);
    for (k, r) in rects.iter().enumerate() {
        if p.y < r.top() {
            return Drop::NewAt(k);
        }
        if p.y <= r.bottom() {
            if p.y > r.bottom() - edge && r.height() > strip + edge {
                return Drop::NewAt(k + 1);
            }
            return Drop::Join(k);
        }
    }
    Drop::NewAt(rects.len())
}

// ----------------------------------------------------------------------------- Studio chrome

/// Give an icon button its name for screen readers and automation (the tooltip alone isn't one).
fn named(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, resp.enabled(), label));
    resp
}

/// Studio inspector header: collapse to the rail (») and the module picker (+).
pub fn inspector_header(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if named(crate::icons::button(ui, "chevrons-right", 26.0, false, tl!("Collapse to Icons")), tl!("Collapse to Icons")).clicked() {
            app.ui.dock.rail = true;
            app.ui.dock.flyout = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let plus = named(crate::icons::button(ui, "plus", 26.0, false, tl!("Add Panel")), tl!("Add Panel"));
            picker(app, &plus);
        });
    });
}

/// The + picker: every module, searchable, ticked when it is in the dock; a click toggles it.
pub fn picker(app: &mut PhotocraftApp, anchor: &egui::Response) {
    let search_id = anchor.id.with("search");
    egui::Popup::menu(anchor).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| picker_list(app, ui, search_id, None));
}

/// The picker's search field and module list, in a popup or a pane's ≡ › Add Tab (which
/// adds the module to that pane, `into`).
pub fn picker_list(app: &mut PhotocraftApp, ui: &mut egui::Ui, search_id: egui::Id, into: Option<usize>) {
    const WIDTH: f32 = 200.0;
    ui.set_width(WIDTH);
    let t = Tokens::get(ui.ctx());
    let mut query: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    let field = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tl!("Search panels")).desired_width(WIDTH));
    if !field.has_focus() && ui.memory(|m| m.focused().is_none()) {
        field.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(search_id, query.clone()));
    ui.add_space(2.0);
    let q = query.trim().to_lowercase();
    let mut list: Vec<(&str, &'static Module)> = modules::ALL
        .iter()
        .map(|m| (tl!(m.title), *m))
        .filter(|(t, m)| q.is_empty() || t.to_lowercase().contains(&q) || m.title.to_lowercase().contains(&q))
        .collect();
    list.sort_by_cached_key(|(t, _)| t.to_lowercase());
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if list.is_empty() {
        ui.label(egui::RichText::new(tl!("No panels match")).color(t.text_faint));
    }
    egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (k, (title, m)) in list.iter().enumerate() {
            let mut on = app.ui.dock.visible(m.id);
            let row = ui.horizontal(|ui| {
                ui.add(crate::icons::image(m.icon, 14.0, t.text_dim));
                ui.checkbox(&mut on, *title)
            });
            if row.inner.changed() || (enter && k == 0) {
                if app.ui.dock.visible(m.id) && !(enter && k == 0) {
                    app.ui.dock.close(m.id);
                } else if let Some(i) = into.filter(|i| *i < app.ui.dock.panes.len()) {
                    app.ui.dock.drop_tab_or_add(m.id, i);
                } else {
                    app.ui.dock.add_pane(m.id);
                    app.ui.panels.dock = true;
                }
            }
        }
    });
}

/// Studio's collapsed inspector: one icon per docked module (a line between panes), the picker
/// at the bottom. An icon opens its module as a flyout beside the rail; a second click closes it.
pub fn studio_rail(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if crate::icons::rail_button(ui, "chevrons-left", 32.0, false, tl!("Expand Panels")).clicked() {
        app.ui.dock.rail = false;
        app.ui.dock.flyout = None;
    }
    ui.add_space(6.0);
    let panes = app.ui.dock.panes.clone();
    for (i, s) in panes.iter().enumerate() {
        if i > 0 {
            let y = ui.cursor().top() + 2.0;
            let r = ui.max_rect();
            ui.painter().line_segment([pos2(r.left() + 6.0, y), pos2(r.right() - 6.0, y)], Stroke::new(1.0, t.separator));
            ui.add_space(5.0);
        }
        for id in &s.tabs {
            let Some(m) = modules::get(id) else { continue };
            let on = app.ui.dock.flyout.as_deref() == Some(m.id);
            let resp = crate::icons::rail_button(ui, m.icon, 32.0, on, tl!(m.title));
            if resp.clicked() {
                app.ui.dock.flyout = if on { None } else { Some(m.id.to_owned()) };
            }
            if resp.middle_clicked() {
                app.ui.dock.close(m.id);
            }
            resp.context_menu(|ui| {
                if ui.button(tl!("Open")).clicked() {
                    app.ui.dock.flyout = Some(m.id.to_owned());
                    ui.close();
                }
                if ui.button(tl!("Close")).clicked() {
                    app.ui.dock.close(m.id);
                    ui.close();
                }
                if ui.button(tl!("Close Tab Group")).clicked() {
                    app.ui.dock.hide(m.id);
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Expand Panels")).clicked() {
                    app.ui.dock.rail = false;
                    app.ui.dock.flyout = None;
                    ui.close();
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("rail-icon", m.id)), resp.rect));
        }
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        let plus = crate::icons::rail_button(ui, "plus", 32.0, false, tl!("Add Panel"));
        picker(app, &plus);
    });
}

/// The flyout of the module picked on Studio's collapsed rail, to the rail's left.
pub fn flyout(app: &mut PhotocraftApp, ctx: &egui::Context, rail: Rect) {
    let Some(id) = app.ui.dock.flyout.clone() else { return };
    let Some(m) = modules::get(&id) else {
        app.ui.dock.flyout = None;
        return;
    };
    let icon = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("rail-icon", m.id)))).unwrap_or(rail);
    let width = 300.0;
    let height = m.size.default.max(m.size.fill).clamp(160.0, (rail.height() - 16.0).max(160.0));
    let top = icon.top().min(rail.bottom() - height).max(rail.top());
    let rect = Rect::from_min_size(pos2(rail.left() - width - 8.0, top), vec2(width, height));
    let area = egui::Area::new(egui::Id::new("dock-flyout")).order(egui::Order::Middle).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_max_size(rect.size());
        let mut sel = 0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect);
        let resp = widgets::card_ex(&mut child, m.id, &[m.title], &mut sel, false, |ui, _| module_body(app, ui, m, "flyout"));
        if resp.close.as_ref().is_some_and(|r| r.clicked()) || resp.collapse.as_ref().is_some_and(|r| r.clicked()) {
            app.ui.dock.flyout = None;
        }
        egui::Popup::menu(&resp.menu).show(|ui| {
            if let Some(extra) = m.menu {
                extra(app, ui);
            }
        });
    });
    // A click outside the flyout and the rail closes it, as a popover does.
    let outside = ctx.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !rail.contains(p)));
    let menu_open = egui::Popup::is_any_open(ctx);
    if outside && !menu_open && !area.response.hovered() {
        app.ui.dock.flyout = None;
    }
}

// ----------------------------------------------------------------------------- persistence

/// What `prefs.panelLayout` holds: the live layout and open panels.
fn snapshot(app: &PhotocraftApp) -> Value {
    json!({"workspace": app.ui.workspace, "panels": app.ui.panels, "dock": app.ui.dock, "timelineOpen": app.ui.timeline.open})
}

/// Remember the layout in the preferences once the user lets go of the mouse (Workspace ›
/// Remember Workspace Changes). Cheap: a small JSON compare per frame.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.session.prefs().workspace.remember_workspace_changes || ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = snapshot(app);
    if app.session.prefs().panel_layout != now {
        app.session.prefs.edit(|p| p.panel_layout = now);
    }
}

/// Restore the remembered layout at launch. Unreadable parts keep their defaults.
pub fn restore(app: &mut PhotocraftApp) {
    if !app.session.prefs().workspace.remember_workspace_changes {
        return;
    }
    let saved = app.session.prefs().panel_layout.clone();
    if saved.as_object().is_none_or(|o| o.is_empty()) {
        return;
    }
    apply(app, &saved);
    if let Some(ws) = saved.get("workspace").and_then(Value::as_str) {
        app.ui.workspace = ws.to_string();
    }
}

/// Apply the `panels`, `dock` and `timelineOpen` parts of a saved layout (a workspace or
/// `panelLayout`). A layout saved before modules (`dock.order`, `dockTabs` and per-group `panels`
/// flags) is migrated. Missing or invalid dock parts are left alone; old layouts without Timeline
/// visibility restore it closed.
pub fn apply(app: &mut PhotocraftApp, v: &Value) {
    if let Some(p) = v.get("panels").and_then(|p| serde_json::from_value(p.clone()).ok()) {
        app.ui.panels = p;
    }
    let pro = is_pro(app);
    match v.get("dock") {
        Some(d) if d.get("panes").is_some() || d.get("sections").is_some() => {
            if let Ok(mut d) = serde_json::from_value::<DockLayout>(d.clone()) {
                d.sanitize();
                app.ui.dock = d;
            }
        }
        _ if v.get("dock").is_some() || v.get("dockTabs").is_some() || v.get("panels").is_some_and(has_legacy_flags) => {
            app.ui.dock = from_legacy(v, pro);
        }
        _ => {}
    }
    app.ui.timeline.open = v.get("timelineOpen").and_then(Value::as_bool).unwrap_or(false);
    if !app.ui.timeline.open {
        app.ui.timeline.playing = false;
    }
}

/// `ui.set`'s dock keys: `dock` as a whole layout (`{"panes": [...]}`) or in the pre-module
/// form (`order` / `heights` / `collapsed` / `hiddenTabs`), `panels.<group>` flags that show or
/// hide a preset group, and `dockTabs.<group>` (front tab index within the group). Returns the
/// new layout (`None` when no key concerns the dock); nothing is applied, so a rejected call
/// changes nothing.
pub fn layout_from_control(app: &PhotocraftApp, panels: Option<&Value>, dock_tabs: Option<&Value>, dock: Option<&Value>) -> Result<Option<DockLayout>, String> {
    let pro = is_pro(app);
    let flags: Vec<(&str, bool)> = match panels.and_then(Value::as_object) {
        Some(p) => GROUPS
            .iter()
            .filter_map(|g| p.get(*g).map(|v| v.as_bool().map(|b| (*g, b)).ok_or_else(|| format!("panels.{g} must be true or false"))))
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let fronts: Vec<(&str, &'static str)> = match dock_tabs {
        Some(t) => t
            .as_object()
            .ok_or("dockTabs must be an object")?
            .iter()
            .map(|(g, i)| {
                let key = GROUPS.iter().copied().find(|k| k == g).ok_or_else(|| format!("unknown dockTabs field `{g}`"))?;
                let tabs = group_tabs(key, pro);
                let id = i
                    .as_u64()
                    .and_then(|i| tabs.get(usize::try_from(i).ok()?))
                    .ok_or_else(|| format!("dockTabs.{g} must be a tab index below {}", tabs.len()))?;
                Ok((key, *id))
            })
            .collect::<Result<_, String>>()?,
        None => Vec::new(),
    };
    if flags.is_empty() && fronts.is_empty() && dock.is_none() {
        return Ok(None);
    }
    let mut out = app.ui.dock.clone();
    if let Some(d) = dock {
        let o = d.as_object().ok_or("dock must be an object")?;
        if o.contains_key("panes") || o.contains_key("sections") {
            out = serde_json::from_value(d.clone()).map_err(|e| format!("dock: {e}"))?;
            out.sanitize();
        } else {
            if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "order" | "heights" | "collapsed" | "hiddenTabs")) {
                return Err(format!("unknown dock field `{k}`"));
            }
            // A pre-module layout keeps what shows and which tabs are in front.
            let mut shown = serde_json::Map::new();
            let mut front = serde_json::Map::new();
            for g in GROUPS {
                shown.insert(g.to_owned(), json!(out.group_shown(g)));
                let tabs = group_tabs(g, pro);
                if let Some(i) = tabs.iter().find_map(|m| out.pane_of(m)).and_then(|i| out.panes.get(i)).and_then(|s| tabs.iter().position(|m| *m == s.front()))
                {
                    front.insert(g.to_owned(), json!(i));
                }
            }
            out = from_legacy(&json!({"panels": shown, "dockTabs": front, "dock": d}), pro);
        }
    }
    for (g, on) in flags {
        if !on {
            out.hide_group(g);
        } else if !out.group_shown(g) {
            out.show_group(g, pro);
        }
    }
    for (_, id) in fronts {
        if let Some(s) = out.pane_of(id).and_then(|i| out.panes.get_mut(i)) {
            s.active = id.to_owned();
        }
    }
    Ok(Some(out))
}

/// `panels` (from a layout saved before modules) carries per-preset-group visibility flags.
fn has_legacy_flags(panels: &Value) -> bool {
    GROUPS.iter().any(|g| panels.get(g).is_some_and(Value::is_boolean))
}

/// The pre-module default visibility of each preset group.
fn legacy_default(group: &str) -> bool {
    matches!(group, "color" | "properties" | "layers")
}

/// Rebuild a layout saved as fixed groups: `panels.<group>` visibility, `dockTabs.<group>` front
/// tab index, `dock.order` / `heights` / `collapsed` / `hiddenTabs` (tab names).
pub fn from_legacy(v: &Value, pro: bool) -> DockLayout {
    let panels = v.get("panels");
    let tabs = v.get("dockTabs");
    let dock = v.get("dock");
    let shown = |g: &str| panels.and_then(|p| p.get(g)).and_then(Value::as_bool).unwrap_or(legacy_default(g));
    let mut order: Vec<&str> = Vec::new();
    for g in dock.and_then(|d| d.get("order")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if let Some(k) = GROUPS.iter().find(|k| **k == g)
            && !order.contains(k)
        {
            order.push(k);
        }
    }
    // Groups missing from the saved order slot in below their default predecessor.
    for (i, g) in GROUPS.iter().enumerate() {
        if order.contains(g) {
            continue;
        }
        match GROUPS.iter().take(i).rev().find_map(|p| order.iter().position(|x| x == p)) {
            Some(at) => order.insert(at + 1, g),
            None => order.push(g),
        }
    }
    let heights: BTreeMap<String, f32> = dock.and_then(|d| d.get("heights")).and_then(|h| serde_json::from_value(h.clone()).ok()).unwrap_or_default();
    let collapsed: Vec<String> = dock.and_then(|d| d.get("collapsed")).and_then(|c| serde_json::from_value(c.clone()).ok()).unwrap_or_default();
    let hidden: BTreeMap<String, Vec<String>> = dock.and_then(|d| d.get("hiddenTabs")).and_then(|h| serde_json::from_value(h.clone()).ok()).unwrap_or_default();
    let mut out = DockLayout { panes: Vec::new(), rail: false, flyout: None };
    for g in order.into_iter().filter(|g| shown(g)) {
        let all = group_tabs(g, pro);
        let gone = hidden.get(g);
        let kept: Vec<&str> =
            all.iter().copied().filter(|id| !gone.is_some_and(|names| names.iter().any(|n| modules::get(id).is_some_and(|m| m.title == n)))).collect();
        if kept.is_empty() {
            continue;
        }
        let mut s = Pane::new(&kept);
        if let Some(front) = tabs.and_then(|t| t.get(g)).and_then(Value::as_u64).and_then(|k| all.get(usize::try_from(k).ok()?)).filter(|id| kept.contains(id))
        {
            s.active = (*front).to_owned();
        }
        s.height = heights.get(g).copied();
        s.collapsed = collapsed.iter().any(|c| c == g);
        out.panes.push(s);
    }
    out.sanitize();
    out
}

#[cfg(test)]
#[path = "dock_tests.rs"]
mod tests;
