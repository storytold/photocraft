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
}

/// A completed release and a later pending press are independent, including within one frame.
#[derive(Default)]
struct Frame {
    release: Option<Release>,
    press: Option<Press>,
    started_press: bool,
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

/// Replay one frame without allocating, including multiple completed clicks in the same frame.
/// The immediate PointerGone suffix of touch-end is not an interruption; cancellation is.
fn scan(ui: &egui::Ui, initial: Option<Press>, mut apply: impl FnMut(Change)) -> Frame {
    let options = ui.ctx().options(|o| o.input_options);
    let mut frame = Frame { press: initial, ..Default::default() };
    let mut interrupted = false;
    let mut just_released = false;
    let mut from_initial = initial.is_some();
    let mut pending_touch: Option<(TouchKey, Pos2)> = None;
    let mut released_touch = None;
    ui.input(|input| {
        for event in &input.events {
            match event {
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
                            frame.press = Some(Press { origin: *pos, modified: !modifiers.is_none(), moved: false, time: input.time, touch });
                        }
                    } else if let Some(p) = frame.press.take() {
                        released_touch = p.touch;
                        let release = Release {
                            origin: p.origin,
                            from_initial,
                            pos: *pos,
                            modifiers: *modifiers,
                            ordinary: !p.modified && modifiers.is_none(),
                            clicked: !p.moved && input.time - p.time <= options.max_click_duration,
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
                    if touch_event(event, &mut frame.press, &mut pending_touch, released_touch, options.max_click_dist) {
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
    });
    if frame.release.is_none() {
        frame.interrupted_before_release = interrupted;
    }
    frame
}

/// Eligible widgets from this frame's layout, bounded by the visible static tool slots and rows.
/// Regular clicks retain egui's ownership; a held flyout also owns its initiating release.
#[derive(Clone, Copy)]
pub(super) enum Control<'a> {
    Slot { rect: Rect, tools: &'a [Tool], key: egui::Id, displayed: Tool, held: bool },
    Row { rect: Rect, key: egui::Id, displayed: Tool, tool: Tool, primary: bool, held: bool },
}

impl Control<'_> {
    /// Match the toolkit's clicked widget, or a flyout row reached by its original held press.
    fn accepts(&self, r: Release) -> bool {
        match *self {
            Self::Slot { rect, held, .. } => r.clicked && rect.contains(r.origin) && rect.contains(r.pos) && !(held && r.from_initial),
            Self::Row { rect, primary, held, .. } => {
                super::tool_flyout_release(rect, Some(r.pos), true, held && r.from_initial)
                    || (primary && r.clicked && rect.contains(r.origin) && rect.contains(r.pos))
            }
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
    /// A duplicate emulated down must not clear the original held flyout.
    pub(super) started_press: bool,
    pub(super) interrupted_before_release: bool,
    interrupted_after_release: bool,
    initial: Option<Press>,
    previous: Cell<Option<PlainClick>>,
    click_id: egui::Id,
    popup_id: egui::Id,
    open_key: Option<egui::Id>,
}

impl Input {
    /// Apply eligible tool and flyout releases in input order, independently of row drawing order.
    /// Geometry storage is bounded by TOOL_SECTIONS, not by the number of incoming events.
    /// Context data is committed after scanning, outside egui's input read lock.
    pub(super) fn apply(&self, ui: &egui::Ui, app: &mut PhotocraftApp, controls: &[Control<'_>]) {
        if controls.is_empty() || !ui.input(|i| i.pointer.primary_released()) {
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
            let Change::Release(r) = change else {
                previous = None;
                return;
            };
            let Some(control) = controls.iter().find(|c| c.accepts(r)) else {
                previous = None;
                return;
            };
            let (key, pick) = match *control {
                Control::Slot { tools, key, displayed, .. } => {
                    let count = if r.ordinary {
                        previous
                            .filter(|p| {
                                p.slot == key && time - p.time < options.max_double_click_delay && p.pos.distance_sq(r.pos) < options.max_click_dist.powi(2)
                            })
                            .map_or(1, |p| p.count.saturating_add(1).min(3))
                    } else {
                        0
                    };
                    previous = (count > 0).then_some(PlainClick { slot: key, pos: r.pos, time, count });
                    let shown = if tools.contains(&app.ui.tool) { app.ui.tool } else { memories.iter().find(|m| m.key == key).map_or(displayed, |m| m.tool) };
                    let pick = if tools.len() > 1 && r.modifiers.alt { super::next_slot_tool(tools, shown) } else { shown };
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
        started_press: frame.started_press,
        interrupted_before_release: frame.interrupted_before_release,
        interrupted_after_release: frame.interrupted_after_release,
        initial,
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
