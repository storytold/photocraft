//! Native-menu behaviour for the menu bar's menus (#138): a menu or submenu taller than the
//! window scrolls (mouse wheel / trackpad, and the ▲ / ▼ arrows at its top and bottom that scroll
//! while the pointer rests on them, like macOS and Windows menus on a small display), and the
//! keyboard drives an open menu: ↑ / ↓ move the highlight (and keep it in view), → opens a
//! submenu or the next menu, ← closes the submenu or opens the previous menu, ↩ / Space run the
//! highlighted item, Esc closes.
//!
//! The highlight is view state only (egui memory), per top-level menu; it follows the pointer too,
//! so the mouse and the keyboard share one highlighted row like a native menu.
//!
//! An open submenu stays open while the pointer heads for it (#1218), as in macOS and Photoshop
//! menus: crossing the rows between its row and the submenu, on a diagonal, a curved path or with
//! a pause on the way, the highlight follows the pointer, but no row crossed opens its submenu.
//! The pointer resting on another row, or moving away from the submenu, switches as before.

use egui::containers::menu::{MenuState, SubMenu, find_menu_root};
use egui::{Id, Key, Modifiers, Pos2, Rect, Sense, Ui, pos2, vec2};

/// Height of a scroll-arrow strip.
pub const ARROW: f32 = 16.0;
/// Gap kept between a menu and the window edge.
const EDGE: f32 = 6.0;
/// How fast resting on a scroll arrow scrolls, in points per second.
const SPEED: f32 = 600.0;
/// How far above and below an open submenu the pointer may aim and still be heading for it.
const AIM_SLACK: f32 = 24.0;
/// How long the pointer may rest on another row, heading for an open submenu, before that row
/// takes over (macOS waits about as long).
const AIM_REST: f32 = 0.3;

/// The pointer heading for the open submenu of a row (#1218).
#[derive(Clone, Debug)]
struct Aim {
    /// The row whose submenu is open, and its index in its level.
    row: Id,
    index: usize,
    /// Where the pointer last was on that row.
    from: Pos2,
    /// The row's rect, from when the pointer was last on it.
    rect: Rect,
    /// The pointer was off the row with its submenu still open in the last frame.
    held: bool,
    /// The pointer reached the submenu: the trip is over, and leaving the submenu for another
    /// row of this menu doesn't hold it (`from` is where the pointer left the row on the way in).
    arrived: bool,
    /// When the pointer last moved (`InputState::time`). Resting is timed from here, not from
    /// egui's smoothed velocity, which reads "moving" for about 0.1 s after the pointer stops.
    moved_at: f64,
}

/// One actionable row of a menu level: a command or a submenu.
#[derive(Clone, Debug)]
pub struct Row {
    /// The row's widget id (a submenu's popup id derives from it).
    pub widget: Id,
    pub enabled: bool,
    /// The command it runs; `None` for a submenu.
    pub command: Option<String>,
    /// Where it was drawn (scrolled out of view when outside its level's `views` rect).
    pub rect: Rect,
    /// The letter that runs it from the keyboard ([`crate::menu_keys`]), upper case.
    pub mnemonic: Option<char>,
}

/// Keyboard / highlight state of the open menu.
#[derive(Clone, Debug, Default)]
pub struct Nav {
    /// The open top-level menu (index into the menu bar).
    pub(crate) top: Option<usize>,
    /// Underline the rows' mnemonics this frame ([`crate::menu_keys`]).
    pub underline: bool,
    /// Popup ids of the top-level menus, from the last frame.
    tops: Vec<Id>,
    /// The level the arrow keys move in (0 = the top-level menu).
    pub(crate) level: usize,
    /// Highlighted row per level.
    hi: Vec<Option<usize>>,
    /// This frame's rows per level (deeper levels are dropped when their parent redraws).
    pub(crate) rows: Vec<Vec<Row>>,
    /// Menu-root id per level (the owner of egui's `MenuState` for that level).
    roots: Vec<Id>,
    /// Bottom of the menu bar: where top-level menus hang from.
    pub bar_bottom: Option<f32>,
    /// The visible (scrolled) part of each level.
    pub(crate) views: Vec<Rect>,
    /// [`Self::views`] and each level's row rects as the frame before left them: a level's own
    /// view is only known once its rows are drawn (and its submenu's once that is), and a
    /// submenu's rows are dropped when its parent starts drawing, so the tests made while drawing
    /// them use these.
    last_views: Vec<Rect>,
    last_rows: Vec<Vec<Rect>>,
    /// The full height of each level's rows (taller than its view when it scrolls).
    pub(crate) contents: Vec<f32>,
    /// Per level: the row its open submenu hangs from (level 0 has none). From the frame
    /// before, since a submenu is drawn while its row is.
    anchors: Vec<Option<Rect>>,
    /// Scroll the highlighted row of `level` into view when it is next drawn.
    reveal: bool,
    /// Highlight the first enabled row of this level once it has been drawn (a keyboard-opened
    /// submenu).
    first_at: Option<usize>,
    /// Per level: the pointer heading for the open submenu of one of its rows.
    aims: Vec<Option<Aim>>,
    /// A held submenu of this level let go: the row under the pointer takes the highlight even
    /// if the pointer rests.
    follow: Option<usize>,
    /// The keys moved the highlight, and the pointer hasn't moved since.
    keyboard: bool,
    /// Help's search field has text this frame.
    pub(crate) search_text: bool,
}

fn nav_id() -> Id {
    Id::unique("pc-menu-nav")
}

fn open_id() -> Id {
    Id::unique("pc-menu-open")
}

/// Is one of the menu bar's menus open? (Shortcuts leave the keys to it meanwhile.)
pub fn is_open(ctx: &egui::Context) -> bool {
    // Only while the menu bar is still drawn (Full Screen Mode hides it with a menu open).
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(open_id())).is_some_and(|seen| seen + 2 >= pass)
}

impl Nav {
    /// Last frame's state, reset when another top-level menu (or none) is open now.
    pub fn load(ctx: &egui::Context) -> Nav {
        let mut nav: Nav = ctx.data(|d| d.get_temp(nav_id())).unwrap_or_default();
        let top = nav.tops.iter().position(|id| egui::Popup::is_id_open(ctx, *id));
        if top != nav.top {
            nav = Nav { top, tops: std::mem::take(&mut nav.tops), keyboard: nav.keyboard && top.is_some(), ..Default::default() };
        }
        // egui moves the focus with the arrow keys too, along the menu titles and into a submenu
        // beside the menu: gray rows, and a gray title one step behind the open menu.
        if let Some(focus) = ctx.memory(|m| m.focused()).filter(|_| nav.keyboard && !ctx.text_edit_focused()) {
            ctx.memory_mut(|m| m.surrender_focus(focus));
        }
        nav
    }

    /// The state as the last frame left it (tests, inspection).
    pub fn current(ctx: &egui::Context) -> Nav {
        ctx.data(|d| d.get_temp(nav_id())).unwrap_or_default()
    }

    /// The highlighted row of `level`, if any.
    pub fn highlighted(&self, level: usize) -> Option<usize> {
        self.hi.get(level).copied().flatten()
    }

    fn set_hi(&mut self, level: usize, row: Option<usize>) {
        if self.hi.len() <= level {
            self.hi.resize(level + 1, None);
        }
        self.hi[level] = row;
    }

    /// Start drawing `level`: forget its rows from before, and every deeper level's.
    fn begin_level(&mut self, ui: &Ui, level: usize, root: Id) {
        if level == 0 {
            self.last_views = self.views.clone();
            self.last_rows = self.rows.iter().map(|rows| rows.iter().map(|r| r.rect).collect()).collect();
            self.keyboard &= !pointer_acted(ui.ctx());
        }
        self.follow_pointer(ui, level);
        self.rows.truncate(level);
        self.rows.resize(level + 1, Vec::new());
        self.roots.truncate(level);
        self.roots.resize(level + 1, root);
        self.views.truncate(level);
        self.views.resize(level + 1, Rect::NOTHING);
        self.contents.truncate(level);
        self.contents.resize(level + 1, 0.0);
    }

    /// Before `level`'s rows are drawn: the pointer having moved onto one of them (where they were
    /// last frame) highlights it now, not once that row is drawn, so a row drawn before it (the
    /// one above, stepping down) isn't drawn highlighted a frame longer. Where rows touch, the
    /// lower one wins, as in egui's hit test. Also while the pointer heads for an open submenu:
    /// only the submenu stays. When that submenu is let go (the pointer rested), the row under the
    /// pointer takes the highlight now too, moving or not: both rows were lit for a frame. In the
    /// submenu, the highlight goes back to the row it hangs from.
    fn follow_pointer(&mut self, ui: &Ui, level: usize) {
        let held = self.held_open(ui, level);
        let moved = pointer_acted(ui.ctx());
        let index = self.aims.get(level).and_then(Option::as_ref).map(|a| a.index);
        let sub = self.last_views.get(level + 1).copied();
        let in_sub = ui.ctx().pointer_hover_pos().is_some_and(|p| sub.is_some_and(|v| v.contains(p)));
        if let Some(index) = index.filter(|_| in_sub && !self.keyboard) {
            self.set_hi(level, Some(index));
            return;
        }
        if !(moved || held == Some(false)) || !self.last_views.get(level).is_some_and(|v| pointer_on(ui, *v)) {
            return;
        }
        let Some(i) = self.last_rows.get(level).and_then(|rects| rects.iter().rposition(|r| pointer_on(ui, *r))) else { return };
        // On a row of this level, nothing in a deeper one is highlighted: back on the row a
        // submenu hangs from, the submenu stays open with nothing in it lit, as in native menus
        // (its last row stayed lit beside this one).
        for deeper in self.hi.iter_mut().skip(level + 1) {
            *deeper = None;
        }
        if self.highlighted(level) != Some(i) {
            self.set_hi(level, Some(i));
        }
        self.level = level;
    }

    /// Draw a row of `level` with `add`, highlighted like a hovered row when the keyboard (or the
    /// pointer) is on it, and register it for the keys. `add` returns the row's response. A row
    /// without a `command` opens a submenu.
    pub fn row<R>(
        &mut self,
        ui: &mut Ui,
        level: usize,
        enabled: bool,
        command: Option<&str>,
        mnemonic: Option<char>,
        add: impl FnOnce(&mut Ui, &mut Nav) -> (egui::Response, R),
    ) -> R {
        let i = self.rows.get(level).map_or(usize::MAX, Vec::len);
        let lit = enabled && self.highlighted(level) == Some(i);
        let saved = (ui.visuals().widgets.inactive, ui.visuals().widgets.active, ui.visuals().widgets.hovered);
        if self.keyboard && !lit {
            // The keys have the highlight: a pointer left resting on this row doesn't light it too.
            ui.visuals_mut().widgets.hovered = ui.visuals().widgets.inactive;
        }
        if lit {
            // Active too: egui draws a focused widget so, and the arrow keys move the focus.
            let hovered = ui.visuals().widgets.hovered;
            (ui.visuals_mut().widgets.inactive, ui.visuals_mut().widgets.active) = (hovered, hovered);
        }
        let root = self.roots.get(level).copied().filter(|_| command.is_none());
        let open = root.and_then(|root| MenuState::from_id(ui.ctx(), root, |s| s.open_item));
        let (r, out) = add(ui, self);
        (ui.visuals_mut().widgets.inactive, ui.visuals_mut().widgets.active, ui.visuals_mut().widgets.hovered) = saved;
        if let Some(root) = root {
            self.aim(level, i, root, open, &r);
        }
        if let Some(rows) = self.rows.get_mut(level) {
            rows.push(Row { widget: r.id, enabled, command: command.map(str::to_string), rect: r.rect, mnemonic });
            self.after_row(level, i, lit, &r);
        }
        out
    }

    /// After a submenu row is drawn: if egui just closed its submenu because the pointer is over
    /// another row of this menu, keep it open while the pointer heads for it (#1218). `open` was
    /// the menu's open submenu before the row was drawn.
    fn aim(&mut self, level: usize, index: usize, root: Id, open: Option<Id>, r: &egui::Response) {
        if self.aims.len() <= level {
            self.aims.resize(level + 1, None);
        }
        let Some(p) = r.ctx.pointer_hover_pos() else { return };
        let sub = SubMenu::id_from_widget_id(r.id);
        // egui closes an open submenu with the pointer on another row: the keys opened this one,
        // and a pointer parked elsewhere doesn't undo that.
        if self.keyboard && open == Some(sub) {
            MenuState::from_id(&r.ctx, root, |s| s.open_item = Some(sub));
            return;
        }
        let was_held = self.aims.get(level).and_then(Option::as_ref).is_some_and(|a| a.row == r.id && a.held);
        let held = self.hold(level, index, root, open, p, r);
        if open == Some(sub) && MenuState::from_id(&r.ctx, root, |s| s.open_item) != Some(sub) {
            // Closed this frame (by egui or the hold), after it was drawn: draw the next frame
            // without it, also with the pointer resting, when nothing else asks for one.
            r.ctx.request_repaint();
        }
        if was_held && !held {
            // Let go: the row under the pointer takes the highlight, moving or not.
            self.follow = Some(level);
        }
    }

    /// [`Self::aim`]: is this row's submenu open with the pointer off the row (held) now?
    fn hold(&mut self, level: usize, index: usize, root: Id, open: Option<Id>, p: Pos2, r: &egui::Response) -> bool {
        let ctx = &r.ctx;
        let sub = SubMenu::id_from_widget_id(r.id);
        let target = self.views.get(level + 1).copied();
        // On this menu's rows or separators, not its border (egui's own test for "on the menu").
        let on_rows = ctx.layer_id_at(p) == Some(r.layer_id) && self.last_views.get(level).is_some_and(|v| v.contains(p));
        let Some(slot) = self.aims.get_mut(level) else { return false };
        if open != Some(sub) {
            if slot.as_ref().is_some_and(|a| a.row == r.id) {
                *slot = None;
            }
            return false;
        }
        let (now, moved) = ctx.input(|i| (i.time, i.pointer.delta() != egui::Vec2::ZERO));
        if on_row(r.interact_rect, p) {
            *slot = Some(Aim { row: r.id, index, from: p, rect: r.interact_rect, held: false, arrived: false, moved_at: now });
            return false;
        }
        let Some(aim) = slot.as_mut().filter(|a| a.row == r.id) else { return false };
        if moved {
            aim.moved_at = now;
        }
        let kept = MenuState::from_id(ctx, root, |s| s.open_item) == Some(sub);
        // Off this menu's rows (on the submenu, the menu's border, or anywhere else): egui
        // decides, and the row stays lit while the submenu is open.
        if kept && !on_rows {
            aim.arrived |= target.is_some_and(|t| t.contains(p));
            aim.held = true;
            return true;
        }
        aim.held = false;
        // On another row of this menu, the submenu stays only while the pointer heads for it,
        // also when egui kept it: egui's own check counts a pointer moving straight up or down
        // beside the submenu as moving toward it (its ray test skips the axis the pointer doesn't
        // move along), and judges the direction over the last 0.1 s, so a slide toward the arrow
        // and then a step down still reads as heading for it. A click or a key closes menus as
        // usual; so does resting on the other row.
        let rest = (now - aim.moved_at) as f32;
        if aim.arrived || !keeps(ctx, aim.from, p, target, rest) {
            if kept {
                MenuState::from_id(ctx, root, |s| s.open_item = None);
            }
            return false;
        }
        if !kept {
            MenuState::from_id(ctx, root, |s| s.open_item = Some(sub));
        }
        aim.held = true;
        ctx.request_repaint_after_secs(AIM_REST - rest);
        true
    }

    /// Have the keys moved the highlight, with the pointer not moved since? A pointer left resting
    /// on a row then neither lights it nor opens its submenu.
    pub(crate) fn keyboard(&self) -> bool {
        self.keyboard
    }

    /// Is the row drawn next in `level` the highlighted one? Then it's lit also with its submenu
    /// open, which shows where the submenu hangs from.
    pub(crate) fn next_row_lit(&self, level: usize) -> bool {
        self.highlighted(level).is_some_and(|i| self.rows.get(level).is_some_and(|rows| rows.len() == i))
    }

    /// Is the pointer off the row `ui` draws next in `level`, whose submenu is open, on another
    /// row or a separator of this menu? The row is then drawn plain, not in the open-submenu
    /// style, which is for the pointer (or the keyboard) in the submenu: the row flashed gray for a
    /// frame when the pointer
    /// stepped onto a separator or the next row, or came back out of the submenu onto another row.
    pub(crate) fn elsewhere_in_menu(&self, ui: &Ui, level: usize) -> bool {
        self.aims.get(level).and_then(Option::as_ref).is_some_and(|a| a.row == ui.next_auto_id()) && self.held_open(ui, level).is_some()
    }

    /// With the pointer off the row of `level` whose submenu is open, onto this menu's other rows
    /// or separators (its view, on `ui`'s layer): does that submenu stay open this frame
    /// ([`keeps`], as [`Self::hold`] will find once the row is drawn)? `None` while the pointer is
    /// on that row, off this menu's rows (its border included), or no submenu of this level is
    /// open.
    fn held_open(&self, ui: &Ui, level: usize) -> Option<bool> {
        let ctx = ui.ctx();
        let aim = self.aims.get(level).and_then(Option::as_ref)?;
        let p = ctx.pointer_hover_pos()?;
        let on_rows = ctx.layer_id_at(p) == Some(ui.layer_id()) && self.last_views.get(level).is_some_and(|v| v.contains(p));
        if on_row(aim.rect, p) || !on_rows {
            return None;
        }
        let (now, moved) = ctx.input(|i| (i.time, i.pointer.delta() != egui::Vec2::ZERO));
        let rest = if moved { 0.0 } else { (now - aim.moved_at) as f32 };
        Some(!aim.arrived && keeps(ctx, aim.from, p, self.last_views.get(level + 1).copied(), rest))
    }

    /// After a row is drawn: the pointer moving onto it highlights it; a keyboard move scrolls
    /// the highlighted row into view.
    fn after_row(&mut self, level: usize, i: usize, lit: bool, r: &egui::Response) {
        let moved = r.ctx.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO);
        let follow = self.follow == Some(level);
        // `hovered` is false while the primary button is held (egui attributes the pointer to
        // the pressed widget), so a press-drag through the items never highlighted anything
        // (#8887). `contains_pointer` keeps answering while a button is down.
        let over = r.contains_pointer();
        if (moved || follow) && over && !lit {
            if follow {
                self.follow = None;
            }
            self.set_hi(level, Some(i));
            self.level = level;
            r.ctx.request_repaint();
        }
        if lit && self.reveal && level == self.level {
            r.scroll_to_me(None);
            self.reveal = false;
        }
    }

    /// Call with the response of a separator drawn in `level`. A separator is not a row, so nothing
    /// else takes the highlight when the pointer moves onto it, and the last row would stay lit (a
    /// native menu clears it). Once a held submenu is let go (`follow`), the separator takes the
    /// highlight even if the pointer has stopped.
    pub fn separator(&mut self, level: usize, r: &egui::Response) {
        if !r.contains_pointer() {
            return;
        }
        let moved = r.ctx.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO);
        let follow = self.follow == Some(level);
        if !(moved || follow) {
            return;
        }
        if follow {
            self.follow = None;
        }
        self.level = level;
        for deeper in self.hi.iter_mut().skip(level + 1) {
            *deeper = None;
        }
        if self.highlighted(level).is_some() {
            self.set_hi(level, None);
            r.ctx.request_repaint();
        }
    }

    fn enabled(&self, level: usize) -> Vec<usize> {
        self.rows.get(level).map(|rows| rows.iter().enumerate().filter(|(_, r)| r.enabled).map(|(i, _)| i).collect()).unwrap_or_default()
    }

    /// ↑ / ↓: the next enabled row (wrapping), or the first / last one.
    fn step(&mut self, level: usize, down: bool) {
        let on = self.enabled(level);
        let (Some(&first), Some(&last)) = (on.first(), on.last()) else { return };
        let next = match self.highlighted(level) {
            None => {
                if down {
                    first
                } else {
                    last
                }
            }
            Some(cur) if down => on.iter().copied().find(|&i| i > cur).unwrap_or(first),
            Some(cur) => on.iter().rev().copied().find(|&i| i < cur).unwrap_or(last),
        };
        self.set_hi(level, Some(next));
        self.hi.truncate(level + 1);
        self.level = level;
        self.reveal = true;
        // The keys move the highlight now, not the pointer heading for a submenu.
        if let Some(aim) = self.aims.get_mut(level) {
            *aim = None;
        }
    }

    /// Open the highlighted submenu of `level` and move the keyboard into it.
    /// No row is highlighted any more: the rows have changed, and a highlight is a position among
    /// them.
    pub(crate) fn clear_highlight(&mut self) {
        self.hi.clear();
    }

    /// Light the first enabled row of `level` once it has been drawn (a menu opened from the
    /// keyboard, as on Windows).
    pub(crate) fn highlight_first(&mut self, level: usize) {
        self.first_at = Some(level);
    }

    /// The enabled row of the level the arrows are in whose mnemonic is `c`.
    pub(crate) fn row_with_mnemonic(&self, c: char) -> Option<Row> {
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.rows.get(level)?.iter().find(|r| r.enabled && r.mnemonic == Some(c)).cloned()
    }

    /// Open the submenu of row `widget` of the level the arrows are in.
    pub(crate) fn open_sub_row(&mut self, ctx: &egui::Context, widget: Id) {
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.open_sub(ctx, level, widget);
    }

    fn open_sub(&mut self, ctx: &egui::Context, level: usize, widget: Id) {
        let Some(&root) = self.roots.get(level) else { return };
        let sub = SubMenu::id_from_widget_id(widget);
        // egui drops an open item it has never drawn: mark it shown so it survives to the next frame.
        MenuState::mark_shown(ctx, sub);
        MenuState::from_id(ctx, root, |s| s.open_item = Some(sub));
        self.level = level + 1;
        self.hi.truncate(level + 1);
        self.first_at = Some(level + 1);
    }

    /// Remember the row the submenu of `level` (1 = a top-level menu's submenu) hangs from.
    pub fn set_anchor(&mut self, level: usize, row: Rect) {
        if self.anchors.len() <= level {
            self.anchors.resize(level + 1, None);
        }
        if let Some(a) = self.anchors.get_mut(level) {
            *a = Some(row);
        }
    }

    /// Handle this frame's navigation keys (after the menus were drawn). `tops` are the menu
    /// bar's popup ids; an activated command lands in `clicked`.
    pub fn keys(&mut self, ctx: &egui::Context, tops: &[Id], clicked: &mut Option<String>) {
        self.tops = tops.to_vec();
        let Some(top) = self.top.filter(|_| !tops.is_empty()) else { return };
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.level = level;
        // A text field in the menu (Help › Search) keeps the keys that edit it: Space types a
        // space, and ← / → move its cursor while it has text and no row is highlighted. Any other
        // text field keeps them all (F10 or Alt leaves it the focus). A zoom field keeps the
        // keyboard while a menu title is clicked (zoom_field.rs), but is not in the menu: the menu
        // takes the keys, unless it has a text field of its own (Help › Search, which the zoom
        // field keeps out).
        let search = super::search::shown(ctx);
        let typing = ctx.text_edit_focused() && !(crate::zoom_field::is_focused(ctx) && !search);
        let cursor = typing && (!search || self.search_text && self.hi.iter().all(Option::is_none));
        let held = |k: Key| match k {
            Key::ArrowLeft | Key::ArrowRight => cursor,
            Key::Space => typing,
            _ => false,
        };
        let press = |k: Key| !held(k) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let current = self.highlighted(level).and_then(|i| self.rows.get(level)?.get(i)).cloned();
        let switch = |by: usize| {
            let n = tops.len();
            if let Some(&id) = tops.get((top + by) % n) {
                egui::Popup::open_id(ctx, id);
            }
        };
        if press(Key::Escape) {
            egui::Popup::close_all(ctx);
        } else if press(Key::ArrowDown) {
            self.step_away(ctx, level, true);
        } else if press(Key::ArrowUp) {
            self.step_away(ctx, level, false);
        } else if press(Key::ArrowRight) {
            match current {
                Some(Row { widget, enabled: true, command: None, .. }) => self.open_sub(ctx, level, widget),
                _ => switch(1),
            }
        } else if press(Key::ArrowLeft) {
            if level > 0 {
                if let Some(&root) = self.roots.get(level - 1) {
                    MenuState::from_id(ctx, root, |s| s.open_item = None);
                }
                self.hi.truncate(level);
                self.level = level - 1;
            } else {
                switch(tops.len() - 1);
            }
        } else if press(Key::Enter) || press(Key::Space) {
            crate::zoom_field::drop_text(ctx, ' ');
            match current {
                Some(Row { enabled: true, command: Some(c), .. }) => {
                    *clicked = Some(c);
                    egui::Popup::close_all(ctx);
                }
                Some(Row { widget, enabled: true, command: None, .. }) => self.open_sub(ctx, level, widget),
                _ => {}
            }
        } else {
            return;
        }
        self.keyboard = true;
        ctx.request_repaint();
    }

    /// ↑ / ↓ in `level`: leaving a row closes its open submenu, as in native menus.
    fn step_away(&mut self, ctx: &egui::Context, level: usize, down: bool) {
        let from = self.highlighted(level);
        self.step(level, down);
        if let Some(&root) = self.roots.get(level).filter(|_| self.highlighted(level) != from) {
            MenuState::from_id(ctx, root, |s| s.open_item = None);
        }
    }

    pub fn store(self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            if self.top.is_some() {
                d.insert_temp(open_id(), pass);
            } else {
                d.remove::<u64>(open_id());
            }
            d.insert_temp(nav_id(), self);
        });
    }
}

/// Did the pointer move or a button go down this frame? Either hands the menu back from the keys.
fn pointer_acted(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_pressed())
}

/// Is the pointer at `p` on a row at `rect`? Its bottom edge is the next row's (rows touch), as
/// in egui's hit test, where the row drawn later, the lower one, wins: otherwise the pointer on
/// that shared edge is on both rows, and one row is highlighted while the other's submenu stays.
pub(crate) fn on_row(rect: Rect, p: Pos2) -> bool {
    rect.x_range().contains(p.x) && p.y >= rect.top() && p.y < rect.bottom()
}

/// Does a submenu at `target` stay open with the pointer at `p` on another part of its parent
/// menu, having left the submenu's row at `from` and rested `rest` seconds? Only while it heads
/// for the submenu and hasn't rested [`AIM_REST`]; a click or a key closes menus as usual.
fn keeps(ctx: &egui::Context, from: Pos2, p: Pos2, target: Option<Rect>, rest: f32) -> bool {
    let acted = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_click() || !i.keys_down.is_empty());
    !acted && rest < AIM_REST && target.is_some_and(|target| heading(from, p, target))
}

/// Is the pointer at `p`, having left a submenu's row at `from`, heading for the submenu at
/// `target`? It is while it stays inside the triangle from `from` to the submenu's near edge
/// (with [`AIM_SLACK`] above and below), as macOS menus judge it.
fn heading(from: Pos2, p: Pos2, target: Rect) -> bool {
    if !target.is_positive() {
        return false;
    }
    let x = if target.center().x >= from.x { target.left() } else { target.right() };
    let (a, b) = (pos2(x, target.top() - AIM_SLACK), pos2(x, target.bottom() + AIM_SLACK));
    let side = |u: Pos2, v: Pos2| (v.x - u.x) * (p.y - u.y) - (v.y - u.y) * (p.x - u.x);
    let (d1, d2, d3) = (side(from, a), side(a, b), side(b, from));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Is the pointer over `rect` on this ui's layer (not under a submenu drawn on top)?
fn pointer_on(ui: &Ui, rect: Rect) -> bool {
    ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()))
}

/// The height a menu level's rows may take. Every level stays below the menu bar (#319): a
/// top-level menu hangs from it, and a submenu that is too tall for the window between its row
/// and the bottom, or (opening upward, as egui does when that fits) between the bar and its row,
/// scrolls instead of egui sliding it up over the menu bar, where it hid the menu titles.
/// `anchor` is the submenu's row (unknown on its first frame); `frame` the popup's margins.
pub fn level_room(screen: Rect, bar_bottom: Option<f32>, depth: usize, anchor: Option<Rect>, frame: f32) -> f32 {
    level_room_in(screen, screen, bar_bottom, depth, anchor, None, frame)
}

/// [`level_room`] when only `visible`, a part of the window's content rect `screen`, can be seen
/// (#315: a window taller than its display runs under the taskbar). egui still places popups
/// within the whole window, so a submenu opens upward only when downward doesn't fit the window.
/// `rows` is the level's rows' height when known (the popup shrinks to it).
pub fn level_room_in(screen: Rect, visible: Rect, bar_bottom: Option<f32>, depth: usize, anchor: Option<Rect>, rows: Option<f32>, frame: f32) -> f32 {
    let top = bar_bottom.unwrap_or(visible.top()).max(visible.top()) + EDGE;
    let bottom = visible.bottom().min(screen.bottom()) - EDGE;
    let below_bar = bottom - top;
    let room = match anchor.filter(|_| depth > 1) {
        Some(row) => {
            // Downward from the row, or upward from it, whichever leaves more room. egui opens a
            // submenu upward only when the downward one doesn't fit the window (it hangs from the
            // row's top less half the frame margin).
            let (down, up) = (bottom - row.top(), row.bottom() - top);
            let opens_up = |h: f32| row.top() - (frame - 2.0) / 2.0 + h + frame > screen.bottom();
            let want = rows.unwrap_or(f32::INFINITY);
            if want > down && up > down && opens_up(want.min(up)) { up } else { down }.min(below_bar)
        }
        // A window running under the taskbar: egui opens a submenu downward whenever it fits the
        // window, so a top-level menu ends a scrolling submenu's height above the taskbar, leaving
        // room for the submenu of its last row.
        None if visible.bottom() < screen.bottom() - 0.5 => below_bar - (4.0 * ARROW + frame),
        None => below_bar,
    } - frame;
    if room.is_finite() { room.max(4.0 * ARROW) } else { 4.0 * ARROW }
}

/// Draw one level of a menu, bounded by the window: when its rows don't fit they scroll, with
/// scroll arrows at the top and bottom. `depth` is 1 for a top-level menu.
pub fn level(ui: &mut Ui, depth: usize, nav: &mut Nav, rows: impl FnOnce(&mut Ui, &mut Nav)) {
    let ctx = ui.ctx().clone();
    let level = depth.saturating_sub(1);
    nav.begin_level(ui, level, find_menu_root(ui).scope_id);
    let screen = ctx.content_rect();
    // Only the part of the window on its monitor, clear of the taskbar, can be seen (#315).
    let visible = crate::work_area::visible_rect(&ctx);
    // The menu frame's margin and stroke around the rows.
    let frame = ui.spacing().menu_margin.sum().y + 2.0;
    let anchor = nav.anchors.get(level).copied().flatten();
    let key = ui.scope_id().with(("pc-menu-level", depth));
    // Rows' height from the last frame: does this level overflow?
    let content: Option<f32> = ctx.data(|d| d.get_temp(key));
    let room = level_room_in(screen, visible, nav.bar_bottom, depth, anchor, content, frame);
    let over = content.is_some_and(|h| h > room + 0.5);
    let up = over.then(|| ui.allocate_exact_size(vec2(0.0, ARROW), Sense::hover()).0);
    let height = if over { room - 2.0 * ARROW } else { room };
    // Wheel, scroll bar and dragging the rows all scroll (#160).
    // The popup's `Ui` is only as tall as the popup was last frame (egui's `default_area_size`, 400
    // pt, on the first): a scroll area never grows past that, so every menu stuck at that height
    // and scrolled even on a tall window (#235). Ask for the room the window has; auto-shrink then
    // fits the area to its rows, so a menu scrolls only when they don't fit.
    // egui 0.37 made `ScrollAreaState::offset` private: a scroll target requested by last
    // frame's arrows is applied through the builder instead of writing the stored state.
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("menu-level", depth))
        .max_height(height)
        .min_scrolled_height(height)
        .scroll_source(egui::containers::scroll_area::ScrollSource::ALL);
    if let Some(target) = ctx.data(|d| d.get_temp::<f32>(scroll_request_id(key))) {
        area = area.vertical_scroll_offset(target);
    }
    let out = area.show(ui, |ui| rows(ui, nav));
    ctx.data_mut(|d| d.remove_temp::<f32>(scroll_request_id(key)));
    let down = over.then(|| ui.allocate_exact_size(vec2(0.0, ARROW), Sense::hover()).0);
    ctx.data_mut(|d| d.insert_temp(key, out.content_size.y));
    if let Some(v) = nav.views.get_mut(level) {
        // `inner_rect` is the size asked for, before auto-shrink fits it to the rows.
        *v = Rect::from_min_size(out.inner_rect.min, vec2(out.inner_rect.width(), out.inner_rect.height().min(out.content_size.y)));
    }
    if let Some(c) = nav.contents.get_mut(level) {
        *c = out.content_size.y;
    }
    if nav.first_at == Some(level) {
        nav.first_at = None;
        nav.step(level, true);
        ctx.request_repaint();
    }
    let (Some(up), Some(down)) = (up, down) else { return };
    let t = crate::theme::Tokens::get(&ctx);
    let x = ui.min_rect().x_range();
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let offset = out.state.clamped_offset().y;
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
    let mut target = offset;
    for (strip, dir, live) in [(up, -1.0, offset > 0.5), (down, 1.0, offset < max - 0.5)] {
        let strip = Rect::from_x_y_ranges(x, strip.y_range());
        let c = strip.center();
        let (h, w) = (4.5, 6.0);
        let tri = if dir < 0.0 {
            vec![pos2(c.x - w, c.y + h * 0.5), pos2(c.x + w, c.y + h * 0.5), pos2(c.x, c.y - h * 0.5)]
        } else {
            vec![pos2(c.x - w, c.y - h * 0.5), pos2(c.x + w, c.y - h * 0.5), pos2(c.x, c.y + h * 0.5)]
        };
        ui.painter().add(egui::Shape::convex_polygon(tri, if live { t.text } else { t.text_faint.gamma_multiply(0.5) }, egui::Stroke::NONE));
        let id = key.with(dir < 0.0);
        let r = ui.interact(strip, id, Sense::hover());
        let label = if dir < 0.0 { "Scroll menu up" } else { "Scroll menu down" };
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, live, tl!(label)));
        if live && pointer_on(ui, strip) {
            target = (offset + dir * SPEED * dt.max(1.0 / 120.0)).clamp(0.0, max);
            ctx.request_repaint();
        }
        // The wheel scrolls over the arrows too.
        if pointer_on(ui, strip) {
            let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                target = (target - wheel).clamp(0.0, max);
            }
        }
    }
    if target != offset {
        // Applied to the scroll area on its next frame (egui 0.37 keeps the offset private).
        ctx.data_mut(|d| d.insert_temp(scroll_request_id(key), target));
        ctx.request_repaint();
    }
}

/// The temp key that carries an arrow/wheel scroll target to the next `ScrollArea` build.
fn scroll_request_id(key: egui::Id) -> egui::Id {
    key.with("scroll-request")
}

#[cfg(test)]
mod tests;
