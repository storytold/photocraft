//! Keyboard shortcuts: Photoshop defaults from the command registry, plus single-key tools.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Parse `Cmd+Shift+N` style strings. `Cmd` maps to ⌘ on macOS and Ctrl elsewhere.
pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part {
            "Cmd" => mods |= Modifiers::COMMAND,
            "Shift" => mods |= Modifiers::SHIFT,
            "Alt" => mods |= Modifiers::ALT,
            "Ctrl" => mods |= Modifiers::CTRL,
            "" => key = Some(Key::Plus),
            k => {
                key = Key::from_name(k).or(match k {
                    "=" => Some(Key::Equals),
                    "-" => Some(Key::Minus),
                    "[" => Some(Key::OpenBracket),
                    ";" => Some(Key::Semicolon),
                    "'" => Some(Key::Quote),
                    "]" => Some(Key::CloseBracket),
                    _ => None,
                })
            }
        }
    }
    Some(KeyboardShortcut::new(mods, key?))
}

/// Human-readable form for menus.
pub fn pretty(s: &str) -> String {
    let mac = cfg!(target_os = "macos");
    s.split('+')
        .map(|p| match (p, mac) {
            ("Cmd", true) => "⌘".to_string(),
            ("Cmd", false) => "Ctrl".to_string(),
            ("Shift", true) => "⇧".to_string(),
            ("Alt", true) => "⌥".to_string(),
            (other, _) => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(if mac { "" } else { "+" })
}

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || !app.ui.dialogs.is_empty() || app.discard.is_some() {
        return;
    }
    // Liquify / Puppet Warp / Perspective Warp: ↩ commits, Esc cancels.
    if crate::distort_ui::keys(app, ctx) {
        return;
    }
    // Free Transform: ↩ commits, Esc cancels.
    if app.ui.transform.is_some() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            crate::transform_tool::commit(app);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            crate::transform_tool::cancel(app);
            return;
        }
    }
    // Pen path in progress: ↩ finishes (open path), Esc cancels.
    if app.ui.pen.is_some() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            crate::vector_ui::pen_commit(app, false);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            app.ui.pen = None;
            return;
        }
    }
    // Inline type editing eats text and navigation keys; ⌘-shortcuts still reach the menus.
    let editing = crate::type_tool::handle_keys(app, ctx);
    // Registry + UI command shortcuts, most-modifiers first so ⇧⌘Z wins over ⌘Z.
    // Edit › Keyboard Shortcuts overrides replace the defaults (and can bind any menu item).
    let prefs = app.session.prefs();
    let mut all: Vec<(String, KeyboardShortcut)> = crate::menus::UI_COMMANDS
        .iter()
        .filter_map(|(id, _, _, sc)| Some((id.to_string(), parse(prefs.shortcut(id, *sc)?)?)))
        .chain(photocraft_engine::command_specs().iter().filter_map(|c| Some((c.id.to_string(), parse(prefs.shortcut(c.id, c.shortcut)?)?))))
        .chain(
            prefs
                .shortcuts
                .iter()
                .filter(|(id, sc)| {
                    !sc.is_empty() && photocraft_engine::commands::find(id).is_none() && !crate::menus::UI_COMMANDS.iter().any(|c| c.0 == id.as_str())
                })
                .filter_map(|(id, sc)| Some((id.clone(), parse(sc)?))),
        )
        .filter(|(_, sc)| sc.modifiers != Modifiers::NONE || !matches!(sc.logical_key, Key::X | Key::D))
        .collect();
    all.sort_by_key(|(_, sc)| std::cmp::Reverse(sc.modifiers.shift as u8 + sc.modifiers.alt as u8 + sc.modifiers.command as u8));
    for (id, sc) in all {
        // While typing, clipboard/select-all shortcuts belong to the text, not the pixels.
        if editing && matches!(id.as_str(), "edit.copy" | "edit.cut" | "edit.paste" | "edit.copyMerged" | "select.all" | "edit.pasteSpecial.pasteInPlace") {
            continue;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
            if crate::menus::is_enabled(app, &id) {
                let r = if id.starts_with("image.adjustments.") && !crate::panels::adjustment_sliders(id.rsplit('.').next().unwrap_or("")).is_empty() {
                    let label = photocraft_engine::commands::find(&id).map(|c| c.label).unwrap_or("");
                    crate::dialogs::open_command_dialog(app, &id, label);
                    Ok(serde_json::Value::Null)
                } else {
                    crate::menus::invoke(app, ctx, &id, json!({}))
                };
                if let Err(e) = r {
                    app.ui.status = e;
                }
            }
            return;
        }
    }
    if editing {
        return;
    }
    // Single-key tools and colours (no modifiers).
    let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    // Enter / Escape commit or cancel in-progress tool state (polygonal lasso, crop).
    if !app.ui.polygon.is_empty() || app.ui.crop_rect.is_some() {
        if pressed(Key::Enter) {
            if !app.ui.polygon.is_empty() {
                crate::canvas::commit_polygon(app, Modifiers::NONE);
            } else {
                crate::canvas::commit_crop(app);
            }
            return;
        }
        if pressed(Key::Escape) {
            app.ui.polygon.clear();
            app.ui.crop_rect = None;
            return;
        }
    }
    // Tool keys; pressing the key of the current group cycles within it. With Preferences ›
    // Tools › Use Shift Key for Tool Switch, only ⇧+key cycles and the plain key keeps the
    // group's current tool.
    let shift_switch = app.session.prefs().tools.use_shift_key_for_tool_switch;
    for t in Tool::ALL {
        let Some(k) = Key::from_name(&t.key().to_string()) else { continue };
        let cycle_shift = shift_switch && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, k));
        if cycle_shift || pressed(k) {
            let group: Vec<Tool> = Tool::ALL.iter().copied().filter(|x| x.key() == t.key()).collect();
            app.ui.tool = match group.iter().position(|x| *x == app.ui.tool) {
                Some(i) if shift_switch && !cycle_shift => group[i],
                Some(i) => group[(i + 1) % group.len()],
                None => group[0],
            };
            return;
        }
    }
    if pressed(Key::X) {
        let _ = app.run("tools.swapColors", json!({}));
    }
    if pressed(Key::D) {
        let _ = app.run("tools.defaultColors", json!({}));
    }
    let b = &mut app.session.tools.brush;
    if pressed(Key::OpenBracket) {
        b.size = (b.size / 1.25).max(1.0).round();
    }
    if pressed(Key::CloseBracket) {
        b.size = (b.size * 1.25).min(2500.0).round().max(b.size + 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_shortcuts() {
        let sc = parse("Cmd+Shift+N").unwrap();
        assert_eq!(sc.logical_key, Key::N);
        assert!(sc.modifiers.command && sc.modifiers.shift);
        assert_eq!(parse("Cmd+]").unwrap().logical_key, Key::CloseBracket);
        assert_eq!(parse("Cmd+=").unwrap().logical_key, Key::Equals);
        assert_eq!(parse("F7").unwrap().logical_key, Key::F7);
        assert!(parse("Cmd+Nonsense").is_none());
    }

    #[test]
    fn every_registered_shortcut_parses() {
        for c in photocraft_engine::command_specs() {
            if let Some(sc) = c.shortcut {
                assert!(parse(sc).is_some(), "{}: {sc}", c.id);
            }
        }
        for (id, _, _, sc) in crate::menus::UI_COMMANDS {
            if let Some(sc) = sc {
                assert!(parse(sc).is_some(), "{id}: {sc}");
            }
        }
    }

    #[test]
    fn no_duplicate_shortcuts() {
        let mut seen = std::collections::HashMap::new();
        let all = photocraft_engine::command_specs()
            .iter()
            .filter_map(|c| c.shortcut.map(|s| (c.id, s)))
            .chain(crate::menus::UI_COMMANDS.iter().filter_map(|(id, _, _, s)| s.map(|s| (*id, s))));
        for (id, s) in all {
            if let Some(prev) = seen.insert(s, id) {
                panic!("shortcut {s} bound to both {prev} and {id}");
            }
        }
    }
}
