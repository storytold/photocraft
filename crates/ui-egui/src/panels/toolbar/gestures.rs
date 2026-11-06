//! Completed toolbar gestures, independent of egui's modifier-blind click count.
use std::cell::Cell;

use super::{PhotocraftApp, Pos2, Rect, Tool, json};

/// Contact associated with the emulated primary pointer, including its device identity.
type TouchKey = (egui::TouchDeviceId, egui::TouchId);

/// Transient press state also remembers modifiers released before the button and drag movement.
#[derive(Clone, Copy)]
struct Press {
    origin: Pos2,
    modified: bool,
    moved: bool,
    time: f64,
    touch: Option<TouchKey>,
}

/// Keep the completed release position when a later event changes or hides the pointer.
#[derive(Clone, Copy)]
struct Release {
    origin: Pos2,
    pos: Pos2,
    modifiers: egui::Modifiers,
    ordinary: bool,
    clicked: bool,
    from_initial: bool,
}

/// Only consecutive ordinary releases on the same slot can form a view-action pair.
#[derive(Clone, Copy)]
struct PlainClick {
    slot: egui::Id,
    pos: Pos2,
    time: f64,
    count: u8,
}

/// Report interruptions in event order, so they clear future history without undoing prior clicks.
enum Change {
    Release(Release),
    Interrupted,
    Activate(Option<egui::accesskit::NodeId>),
}

/// A completed release and a later pending press are independent, including within one frame.
#[derive(Default)]
struct Frame {
    release: Option<Release>,
    press: Option<Press>,
    started_press: bool,
    releases: usize,
    interrupted_before_release: bool,
    interrupted_after_release: bool,
}

/// Associate native-before-press and web-after-press metadata with the emulated pointer.
/// End retains that association through its release; another contact's Cancel cannot abort it.
fn touch_event(event: &egui::Event, press: &mut Option<Press>, pending: &mut Option<(TouchKey, Pos2)>, released: Option<TouchKey>, max_distance: f32) -> bool {
    match event {
        egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Start, pos, .. } => {
            let key = (*device_id, *id);
            *pending = Some((key, *pos));
            if let Some(p) = press
                && p.touch.is_none()
                && p.origin.distance_sq(*pos) <= max_distance.powi(2)
            {
                p.touch = Some(key);
            }
            false
        }
        egui::Event::Touch { device_id, id, phase: egui::TouchPhase::End, .. } => {
            let key = (*device_id, *id);
            *pending = pending.filter(|(contact, _)| *contact != key);
            false
        }
        egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Cancel, .. } => {
            let key = (*device_id, *id);
            let owns_press = press.is_some_and(|p| p.touch == Some(key));
            if owns_press {
                *press = None;
            }
            *pending = pending.filter(|(contact, _)| *contact != key);
            owns_press || released == Some(key)
        }
        _ => false,
    }
}

/// Copy only fixed-size input metadata; text, files and accessibility payloads stay in egui.
enum Replay {
    Input(egui::Event),
    Activate(Option<egui::accesskit::NodeId>),
    Other,
}

/// Replay one frame without input-sized allocation, including several completed clicks.
/// Each input borrow ends before callbacks query widget ownership or dispatch commands.
/// The immediate PointerGone suffix of touch-end is not an interruption; cancellation is.
fn scan(ui: &egui::Ui, initial: Option<Press>, mut apply: impl FnMut(Change)) -> Frame {
    let options = ui.ctx().options(|o| o.input_options);
    let mut frame = Frame { press: initial, ..Default::default() };
    let mut interrupted = false;
    let mut just_released = false;
    let mut from_initial = initial.is_some();
    let mut pending_touch: Option<(TouchKey, Pos2)> = None;
    let mut released_touch = None;
    let (time, events) = ui.input(|i| (i.time, i.events.len()));
    for index in 0..events {
        let event = ui.input(|i| match i.events.get(index) {
            Some(
                event @ (egui::Event::PointerButton { .. }
                | egui::Event::PointerGone
                | egui::Event::Touch { .. }
                | egui::Event::WindowFocused(false)
                | egui::Event::ModifiersChanged(_)
                | egui::Event::PointerMoved(_)),
            ) => Replay::Input(event.clone()),
            Some(egui::Event::Key { key: egui::Key::Space | egui::Key::Enter, pressed: true, .. }) => Replay::Activate(None),
            Some(egui::Event::AccessKitActionRequest(request))
                if request.action == egui::accesskit::Action::Click && request.target_tree == egui::accesskit::TreeId::ROOT =>
            {
                Replay::Activate(Some(request.target_node))
            }
            _ => Replay::Other,
        });
        let event = match event {
            Replay::Input(event) => event,
            Replay::Activate(target) => {
                just_released = false;
                apply(Change::Activate(target));
                continue;
            }
            Replay::Other => {
                just_released = false;
                continue;
            }
        };
        match &event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers } => {
                if *pressed {
                    // The web bridge repeats primary-down when another touch starts.
                    // Its existing contact still owns the release and cancellation.
                    if let Some(p) = frame.press.as_mut().filter(|p| p.touch.is_some()) {
                        p.modified |= !modifiers.is_none();
                        pending_touch = None;
                    } else {
                        frame.started_press = true;
                        from_initial = false;
                        let touch = pending_touch.filter(|(_, at)| at.distance_sq(*pos) <= options.max_click_dist.powi(2)).map(|(key, _)| key);
                        pending_touch = None;
                        frame.press = Some(Press { origin: *pos, modified: !modifiers.is_none(), moved: false, time, touch });
                    }
                } else if let Some(p) = frame.press.take() {
                    frame.releases = frame.releases.saturating_add(1);
                    released_touch = p.touch;
                    let release = Release {
                        origin: p.origin,
                        from_initial,
                        pos: *pos,
                        modifiers: *modifiers,
                        ordinary: !p.modified && modifiers.is_none(),
                        clicked: !p.moved && p.origin.distance_sq(*pos) <= options.max_click_dist.powi(2) && time - p.time <= options.max_click_duration,
                    };
                    // A previously held flyout owns the first completed press, not a later
                    // canvas click. Future click history still follows the latest release.
                    if frame.release.is_none() {
                        frame.release = Some(release);
                        frame.interrupted_before_release = interrupted;
                    }
                    frame.interrupted_after_release = false;
                    from_initial = false;
                    apply(Change::Release(release));
                } else {
                    apply(Change::Interrupted);
                }
                just_released = !pressed;
            }
            egui::Event::PointerGone if just_released && released_touch.is_some() => just_released = false,
            egui::Event::PointerGone => {
                // A mouse may leave the window during a held flyout drag and return.
                // Break click history, but retain that drag's release ownership.
                if let Some(p) = &mut frame.press {
                    p.moved = true;
                }
                interrupted = true;
                frame.interrupted_after_release = true;
                just_released = false;
                apply(Change::Interrupted);
            }
            egui::Event::Touch { .. } => {
                if touch_event(&event, &mut frame.press, &mut pending_touch, released_touch, options.max_click_dist) {
                    if frame.press.is_none() {
                        from_initial = false;
                    }
                    interrupted = true;
                    frame.interrupted_after_release = true;
                    apply(Change::Interrupted);
                }
                just_released = false;
            }
            egui::Event::WindowFocused(false) | egui::Event::PointerButton { .. } => {
                frame.press = None;
                from_initial = false;
                pending_touch = None;
                released_touch = None;
                interrupted = true;
                frame.interrupted_after_release = true;
                just_released = false;
                apply(Change::Interrupted);
            }
            egui::Event::ModifiersChanged(modifiers) => {
                if let Some(p) = &mut frame.press {
                    p.modified |= !modifiers.is_none();
                }
                just_released = false;
            }
            egui::Event::PointerMoved(pos) => {
                if let Some(p) = &mut frame.press {
                    p.moved |= p.origin.distance_sq(*pos) > options.max_click_dist.powi(2);
                }
                just_released = false;
            }
            _ => just_released = false,
        }
    }
    if frame.release.is_none() {
        frame.interrupted_before_release = interrupted;
    }
    frame
}

/// A current clipped response and its toolkit identity, independent of the final frame click.
#[derive(Clone, Copy)]
pub(super) struct Target {
    id: egui::Id,
    layer: egui::LayerId,
    rect: Rect,
    full_rect: Rect,
    primary_clicked: bool,
    enabled: bool,
    focused: bool,
}

impl Target {
    /// Retain the response's actual interaction bounds, including scroll-area clipping.
    pub(super) fn new(response: &egui::Response) -> Self {
        Self {
            id: response.id,
            layer: response.layer_id,
            rect: response.interact_rect,
            full_rect: response.rect,
            primary_clicked: response.clicked_by(egui::PointerButton::Primary),
            enabled: response.enabled(),
            focused: response.has_focus(),
        }
    }

    /// A single acknowledged click retains egui's ownership, including its interaction radius
    /// beside a clipped edge. That frame-level acknowledgement cannot authorize earlier releases.
    /// Batched replay instead checks clipped bounds, top/modal layers and later click/drag blockers.
    fn owns(&self, ctx: &egui::Context, pos: Pos2, single_release: bool) -> bool {
        if !self.enabled {
            return false;
        }
        let local = ctx.layer_transform_from_global(self.layer).map_or(pos, |transform| transform * pos);
        if single_release && self.primary_clicked {
            return self.full_rect.contains(local);
        }
        if ctx.layer_id_at(pos) != Some(self.layer) || !ctx.memory(|m| m.allows_interaction(self.layer)) {
            return false;
        }
        if !self.rect.contains(local) {
            return false;
        }
        ctx.viewport(|v| {
            let widgets = &v.prev_pass.widgets;
            let Some((layer, order)) = widgets.order(self.id) else { return false };
            let Some(previous) = widgets.get(self.id) else { return false };
            layer == self.layer
                && previous.enabled
                && previous.sense.senses_click()
                && previous.interact_rect.contains(local)
                && !widgets
                    .get_layer(layer)
                    .skip(order.saturating_add(1))
                    .any(|w| w.enabled && (w.sense.senses_click() || w.sense.senses_drag()) && w.interact_rect.contains(local))
        })
    }

    /// Keyboard uses the focused response; accessibility activation names its own widget.
    fn activated(&self, target: Option<egui::accesskit::NodeId>) -> bool {
        self.enabled && target.map_or(self.focused, |id| id == self.id.accesskit_id())
    }
}

/// Geometry is bounded by the visible static tool slots and rows, never by incoming events.
#[derive(Clone, Copy)]
pub(super) enum Control<'a> {
    Slot { target: Target, tools: &'a [Tool], key: egui::Id, displayed: Tool, held: bool },
    Row { target: Target, key: egui::Id, displayed: Tool, tool: Tool, held: bool },
}

impl Control<'_> {
    /// Both endpoints belong to an ordinary clicked control; a held flyout owns only its release.
    fn accepts(&self, ctx: &egui::Context, r: Release, single_release: bool) -> bool {
        match *self {
            Self::Slot { target, held, .. } => {
                r.clicked && !(held && r.from_initial) && target.owns(ctx, r.origin, single_release) && target.owns(ctx, r.pos, single_release)
            }
            Self::Row { target, held, .. } => {
                let pos = ctx.layer_transform_from_global(target.layer).map_or(r.pos, |transform| transform * r.pos);
                target.owns(ctx, r.pos, single_release)
                    && (super::tool_flyout_release(target.rect, Some(pos), true, held && r.from_initial)
                        || (r.clicked && target.owns(ctx, r.origin, single_release)))
            }
        }
    }

    /// Identify the response used by keyboard and accessibility events during ordered replay.
    fn target(&self) -> Target {
        match *self {
            Self::Slot { target, .. } | Self::Row { target, .. } => target,
        }
    }

    /// Share a slot's displayed tool between button clicks and flyout choices in the same batch.
    fn memory(&self) -> (egui::Id, Tool) {
        match *self {
            Self::Slot { key, displayed, .. } | Self::Row { key, displayed, .. } => (key, displayed),
        }
    }
}

/// Mirror the existing transient slot memory while actions run; commit it outside the input lock.
struct SlotMemory {
    key: egui::Id,
    tool: Tool,
    changed: bool,
}

/// Preserve completed release ownership separately from cancellation and the next pending press.
pub(super) struct Input {
    pub(super) active_press: bool,
    pub(super) press_started_at: Option<f64>,
    /// A duplicate emulated down must not clear the original held flyout.
    pub(super) started_press: bool,
    pub(super) interrupted_before_release: bool,
    interrupted_after_release: bool,
    initial: Option<Press>,
    single_release: bool,
    previous: Cell<Option<PlainClick>>,
    click_id: egui::Id,
    popup_id: egui::Id,
    open_key: Option<egui::Id>,
}

impl Input {
    /// Apply eligible tool and flyout releases in input order, independently of row drawing order.
    /// Geometry storage is bounded by TOOL_SECTIONS, not by the number of incoming events.
    /// Input borrows end before hit tests and dispatch; transient slot data commits afterward.
    pub(super) fn apply(&self, ui: &egui::Ui, app: &mut PhotocraftApp, controls: &[Control<'_>]) {
        if controls.is_empty() {
            return;
        }
        let options = ui.ctx().options(|o| o.input_options);
        let time = ui.input(|i| i.time);
        let mut memories = Vec::<SlotMemory>::new();
        for control in controls {
            let (key, tool) = control.memory();
            if !memories.iter().any(|m| m.key == key) {
                memories.push(SlotMemory { key, tool, changed: false });
            }
        }
        let mut previous = self.previous.get();
        let mut close_key = None;
        scan(ui, self.initial, |change| {
            let (control, release) = match change {
                Change::Interrupted => {
                    previous = None;
                    return;
                }
                Change::Activate(target) => {
                    previous = None;
                    let Some(control) = controls.iter().find(|c| c.target().activated(target)) else { return };
                    (control, None)
                }
                Change::Release(r) => {
                    let Some(control) = controls.iter().find(|c| c.accepts(ui.ctx(), r, self.single_release)) else {
                        previous = None;
                        return;
                    };
                    (control, Some(r))
                }
            };
            let (key, pick) = match *control {
                Control::Slot { tools, key, displayed, .. } => {
                    let count = if let Some(r) = release.filter(|r| r.ordinary) {
                        previous
                            .filter(|p| {
                                p.slot == key && time - p.time < options.max_double_click_delay && p.pos.distance_sq(r.pos) < options.max_click_dist.powi(2)
                            })
                            .map_or(1, |p| p.count.saturating_add(1).min(3))
                    } else {
                        0
                    };
                    previous = release.filter(|_| count > 0).map(|r| PlainClick { slot: key, pos: r.pos, time, count });
                    let shown = if tools.contains(&app.ui.tool) { app.ui.tool } else { memories.iter().find(|m| m.key == key).map_or(displayed, |m| m.tool) };
                    let pick = if tools.len() > 1 && release.is_some_and(|r| r.modifiers.alt) { super::next_slot_tool(tools, shown) } else { shown };
                    crate::tool_memory::select(app, pick);
                    if count == 2 && shown == Tool::Hand {
                        let _ = app.run("view.fitOnScreen", json!({}));
                    }
                    if count == 2 && shown == Tool::Zoom {
                        let _ = app.run("view.actualPixels", json!({}));
                    }
                    if self.open_key == Some(key) {
                        close_key = Some(key);
                    }
                    (key, pick)
                }
                Control::Row { key, tool, .. } => {
                    previous = None;
                    crate::tool_memory::select(app, tool);
                    close_key = Some(key);
                    (key, tool)
                }
            };
            if let Some(memory) = memories.iter_mut().find(|m| m.key == key) {
                memory.tool = pick;
                memory.changed = true;
            }
        });
        self.previous.set(previous);
        ui.data_mut(|d| {
            for memory in memories.iter().filter(|m| m.changed) {
                d.insert_temp(memory.key, memory.tool);
            }
            if let Some(key) = close_key
                && d.get_temp::<(egui::Id, Rect)>(self.popup_id).is_some_and(|(open, _)| open == key)
            {
                d.remove::<(egui::Id, Rect)>(self.popup_id);
            }
            if let Some(previous) = previous {
                d.insert_temp(self.click_id, previous);
            } else {
                d.remove::<PlainClick>(self.click_id);
            }
        });
    }

    /// Keyboard activation cannot seed or complete the next ordinary pointer pair.
    pub(super) fn clear_clicks(&self, ui: &egui::Ui) {
        self.previous.set(None);
        ui.data_mut(|d| d.remove::<PlainClick>(self.click_id));
    }

    /// A later interruption clears held ownership after the completed release is handled.
    pub(super) fn interrupted_after_release(&self) -> bool {
        self.interrupted_after_release
    }

    /// Clear future history only after controls have consumed earlier completed releases.
    pub(super) fn finish(&self, ui: &egui::Ui) {
        if self.interrupted_after_release {
            self.clear_clicks(ui);
        }
    }
}

/// Snapshot pointer transitions once per frame, retaining transient press state across frames.
/// Tool cycling continues to use main's release-time Alt modifiers, including the Pen group.
pub(super) fn read(ui: &egui::Ui, id: egui::Id) -> Input {
    let press_id = id.with("primary-press");
    let click_id = id.with("plain-click");
    let initial = ui.data(|d| d.get_temp::<Press>(press_id));
    let frame = scan(ui, initial, |_| {});
    let result = Input {
        active_press: frame.press.is_some(),
        press_started_at: frame.press.map(|p| p.time),
        started_press: frame.started_press,
        interrupted_before_release: frame.interrupted_before_release,
        interrupted_after_release: frame.interrupted_after_release,
        initial,
        single_release: frame.releases == 1,
        previous: Cell::new(ui.data(|d| d.get_temp::<PlainClick>(click_id))),
        click_id,
        popup_id: id,
        open_key: ui.data(|d| d.get_temp::<(egui::Id, Rect)>(id)).map(|(key, _)| key),
    };
    if ui.input(|i| i.pointer.primary_released()) || result.interrupted_before_release {
        ui.data_mut(|d| d.remove::<PlainClick>(click_id));
    }
    ui.data_mut(|d| {
        if let Some(press) = frame.press {
            d.insert_temp(press_id, press);
        } else {
            d.remove::<Press>(press_id);
        }
    });
    result
}
