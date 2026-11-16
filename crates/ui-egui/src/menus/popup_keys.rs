//! The keyboard in egui's popup menus: the right-click menus (a layer's in the Layers panel, the
//! canvas's, a tab's…) and the menus that panel and options-bar buttons open. egui moves the
//! keyboard focus along a menu's entries with ↑ / ↓ (`MenuState::handle_keyboard`) and wraps at
//! its ends: ↓ on the last entry went to the first, ↑ on the first to the last. Here ↑ / ↓ stop
//! at the ends instead (#15227), and Home / End, which egui leaves alone, go to the first / last
//! entry (#15219). Greyed entries take no focus, so the ends are the first and last enabled
//! entries, the ones egui's arrows reach.
//!
//! The menu bar's menus are driven by [`crate::menu_nav`], not by egui's focus, and keep their
//! keys there.

use egui::containers::menu::MenuState;
use egui::{Context, Event, FocusDirection, Id, Key, LayerId};

/// The menu state egui keeps for the menu popup on `layer`: every menu popup (a context menu, a
/// menu button's, a submenu) has one under its layer's id while egui handles its keys. `None`
/// for any other layer.
fn menu_state(ctx: &Context, layer: LayerId) -> Option<MenuState> {
    ctx.data(|d| d.get_temp::<MenuState>(layer.id.with(MenuState::ID)))
}

/// The entries of `layer` that take the keyboard focus, in the order they were drawn in the last
/// pass: the ones egui's menu keys move between.
fn entries(ctx: &Context, layer: LayerId) -> Vec<Id> {
    ctx.viewport(|v| v.prev_pass.widgets.get_layer(layer).filter(|w| w.enabled && w.sense.is_focusable()).map(|w| w.id).collect())
}

/// The menu the keys act in: the one the focused entry is in, else the one open menu that has no
/// open submenu (a menu just opened with the pointer, nothing focused in it yet). `None` when no
/// menu was drawn in the last pass, or several were with none of them focused. egui keeps the
/// layer of a menu closed since, with no widgets in it, and its menu state: only layers that
/// drew widgets in the last pass count.
fn current_menu(ctx: &Context, focused: Option<Id>) -> Option<LayerId> {
    let layer = focused.and_then(|id| ctx.viewport(|v| v.prev_pass.widgets.get(id).map(|w| w.layer_id)));
    if let Some(layer) = layer.filter(|l| menu_state(ctx, *l).is_some()) {
        return Some(layer);
    }
    let menus: Vec<(LayerId, MenuState)> = ctx
        .viewport(|v| v.prev_pass.widgets.layer_ids().filter(|l| v.prev_pass.widgets.get_layer(*l).next().is_some()).collect::<Vec<_>>())
        .into_iter()
        .filter_map(|l| Some((l, menu_state(ctx, l)?)))
        .collect();
    let mut deepest = menus.iter().filter(|(_, state)| state.open_item.is_none_or(|sub| !menus.iter().any(|(l, _)| l.id == sub))).map(|(l, _)| *l);
    let menu = deepest.next()?;
    deepest.next().is_none().then_some(menu)
}

/// The key that set this pass's focus move, read from the raw events as egui's
/// `Memory::begin_pass` reads them: the last press of an arrow or Esc without modifiers, or of
/// Tab with ⇧ or none. Keys taken by the app earlier in the frame still moved egui's focus.
fn focus_key(ctx: &Context) -> Option<Key> {
    ctx.input(|i| {
        i.raw.events.iter().rev().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => match key {
                Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::Escape if !modifiers.any() => Some(*key),
                Key::Tab if !modifiers.any() || modifiers.shift_only() => Some(*key),
                _ => None,
            },
            _ => None,
        })
    })
}

/// Take this frame's presses of `key` without modifiers; true if there was one. (`consume_key`
/// would take it with ⇧ or ⌥ held too.)
fn take_bare(ctx: &Context, key: Key) -> bool {
    ctx.input_mut(|i| {
        let n = i.events.len();
        i.events.retain(|e| !matches!(e, Event::Key { key: k, pressed: true, modifiers, .. } if *k == key && modifiers.is_none()));
        i.events.len() != n
    })
}

/// Handle this frame's ↑ / ↓ at a menu's ends and its Home / End. Runs before the frame's menus
/// are drawn (egui moves the focus while drawing them) and before the page keys, which would take
/// Home and End to scroll the canvas.
pub fn handle(ctx: &Context) {
    // A text field in a menu keeps Home and End for its caret. An open dropdown list keeps its
    // own keys: there the arrows step the dropdown's value and letters pick one, and its focus
    // isn't its value, so Home and End moving the focus would only part the two.
    if crate::menu_nav::is_open(ctx) || ctx.text_edit_focused() || crate::widgets::dropdown_list_open(ctx) {
        return;
    }
    let focused = ctx.memory(|m| m.focused());
    let Some(layer) = current_menu(ctx, focused) else { return };
    let entries = entries(ctx, layer);
    let (Some(&first), Some(&last)) = (entries.first(), entries.last()) else { return };
    let jump = if take_bare(ctx, Key::Home) {
        Some(first)
    } else if take_bare(ctx, Key::End) {
        Some(last)
    } else {
        None
    };
    if let Some(to) = jump {
        if focused != Some(to) {
            // As when the arrows move off a row, a submenu open in this menu closes.
            MenuState::from_id(ctx, layer.id, |s| s.open_item = None);
            ctx.memory_mut(|m| {
                m.request_focus(to);
                m.move_focus(FocusDirection::None);
            });
            ctx.request_repaint();
        }
        return;
    }
    // ↓ on the last entry and ↑ on the first: egui would wrap around to the other end. The focus
    // stays where it is.
    let at_end = match focus_key(ctx) {
        Some(Key::ArrowDown) => focused == Some(last),
        Some(Key::ArrowUp) => focused == Some(first),
        _ => false,
    };
    if at_end {
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
    }
}

#[cfg(test)]
pub(crate) mod tests;
