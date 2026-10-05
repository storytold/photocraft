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

/// With ⇧ held the OS reports the shifted character as the logical key (US layout): ⇧; is `:`.
fn shifted(k: Key) -> Option<Key> {
    Some(match k {
        Key::Equals => Key::Plus,
        Key::Semicolon => Key::Colon,
        Key::OpenBracket => Key::OpenCurlyBracket,
        Key::CloseBracket => Key::CloseCurlyBracket,
        Key::Slash => Key::Questionmark,
        Key::Backslash => Key::Pipe,
        Key::Num1 => Key::Exclamationmark,
        _ => return None,
    })
}

/// Does a key press match a shortcut, the way Photoshop matches them?
///
/// - Modifiers match exactly (⌘⌥I is not ⌘I), with ⌘ = Ctrl off the Mac.
/// - `=` also matches `+` (⌘+ with ⇧ on a US layout, the numpad `+`, the `+` key of Nordic and
///   German layouts): Photoshop zooms in on both.
/// - Delete also matches Backspace (the Mac's "delete" key).
/// - A ⇧ shortcut on a punctuation key matches the shifted character (⌘⇧; arrives as ⌘`:`).
pub fn key_matches(sc: &KeyboardShortcut, key: Key, mods: Modifiers) -> bool {
    let k = sc.logical_key;
    let direct = key == k || (k == Key::Delete && key == Key::Backspace);
    let plus = k == Key::Equals && key == Key::Plus;
    let via_shift = sc.modifiers.shift && shifted(k) == Some(key);
    if !(direct || plus || via_shift) {
        return false;
    }
    // Typing `+` (or another shifted symbol) may need ⇧ on this layout: don't require it absent.
    let shifted_symbol =
        matches!(key, Key::Plus | Key::Colon | Key::OpenCurlyBracket | Key::CloseCurlyBracket | Key::Questionmark | Key::Pipe | Key::Exclamationmark);
    let shift_ok = mods.shift == sc.modifiers.shift || (shifted_symbol && !sc.modifiers.shift);
    mods.alt == sc.modifiers.alt && shift_ok && mods.cmd_ctrl_matches(sc.modifiers)
}

/// Consume a press of `sc` (see [`key_matches`]); true when one was found.
pub fn consume(ctx: &egui::Context, sc: &KeyboardShortcut) -> bool {
    ctx.input_mut(|i| {
        let before = i.events.len();
        i.events.retain(|e| !matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if key_matches(sc, *key, *modifiers)));
        i.events.len() != before
    })
}

/// View navigation that Photoshop keeps live while a dialog is open (so the image can be judged
/// at any zoom): Zoom In / Out, Fit on Screen, 100%.
pub const NAV_COMMANDS: [&str; 4] = ["view.zoomIn", "view.zoomOut", "view.fitOnScreen", "view.actualPixels"];

/// ⌘+ / ⌘- / ⌘0 / ⌘1 (or their Edit › Keyboard Shortcuts overrides). Returns true when one ran.
fn nav_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    for id in NAV_COMMANDS {
        let sc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).and_then(|c| parse(app.session.prefs().shortcut(id, c.3)?));
        if let Some(sc) = sc
            && consume(ctx, &sc)
        {
            if crate::menus::is_enabled(app, id) {
                let _ = crate::menus::invoke(app, ctx, id, json!({}));
            }
            return true;
        }
    }
    false
}

/// egui-winit turns ⌘C / ⌘X / ⌘V into `Event::Copy` / `Cut` / `Paste` and drops the key press, and
/// drops ⌘V entirely when the clipboard holds no text (an image). Outside text fields, give the
/// pixel commands their key presses back: Copy/Cut/Paste become ⌘C/⌘X/⌘V presses (with ⇧/⌥ as
/// held, so ⇧⌘C is Copy Merged), and a ⌘V release without a paste event becomes a ⌘V press.
pub fn clipboard_keys(ctx: &egui::Context, typing: bool, raw: &mut egui::RawInput) {
    if typing {
        return;
    }
    let seen = egui::Id::new("pc-paste-key-seen");
    // The modifiers held at each event: last frame's, updated as the events report changes.
    let mut mods = ctx.input(|i| i.modifiers);
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let mut out = Vec::with_capacity(raw.events.len());
    for e in raw.events.drain(..) {
        if let egui::Event::ModifiersChanged(m) | egui::Event::Key { modifiers: m, .. } = &e {
            mods = *m;
        }
        let held = if mods.command { mods } else { Modifiers::COMMAND };
        let key = match &e {
            egui::Event::Copy => Some(Key::C),
            egui::Event::Cut => Some(Key::X),
            egui::Event::Paste(_) => Some(Key::V),
            _ => None,
        };
        if let Some(key) = key {
            if key == Key::V {
                ctx.data_mut(|d| d.insert_temp(seen, true));
            }
            out.push(press(key, held));
            continue;
        }
        if let egui::Event::Key { key: Key::V, pressed: false, modifiers, .. } = &e
            && modifiers.command
            && !ctx.data_mut(|d| d.remove_temp::<bool>(seen)).unwrap_or(false)
        {
            out.push(press(Key::V, *modifiers));
        }
        out.push(e);
    }
    raw.events = out;
}

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || !app.ui.dialogs.is_empty() || app.discard.is_some() {
        // Dialogs and focused sliders keep canvas zoom; a focused text field keeps its keys.
        if !ctx.text_edit_focused() {
            nav_keys(app, ctx);
        }
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
        if consume(ctx, &sc) {
            // ⌘V can arrive with an external image still on the OS clipboard (the periodic import
            // hasn't run yet): pull it in before deciding whether Paste is enabled.
            if matches!(id.as_str(), "edit.paste" | "edit.pasteSpecial.pasteInPlace") {
                app.import_os_clipboard();
            }
            if crate::menus::is_enabled(app, &id) {
                let r = if crate::adjust_dialog::has_dialog(&id) {
                    crate::adjust_dialog::open(app, &id);
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
    fn keys_match_like_photoshop() {
        let cmd = Modifiers::COMMAND;
        let zoom_in = parse("Cmd+=").unwrap();
        // ⌘= , ⌘+ typed with ⇧ (US), ⌘+ from the numpad or a Nordic/German `+` key.
        assert!(key_matches(&zoom_in, Key::Equals, cmd));
        assert!(key_matches(&zoom_in, Key::Plus, cmd | Modifiers::SHIFT));
        assert!(key_matches(&zoom_in, Key::Plus, cmd));
        assert!(!key_matches(&zoom_in, Key::Plus, Modifiers::NONE));
        assert!(!key_matches(&zoom_in, Key::Equals, cmd | Modifiers::ALT));
        assert!(key_matches(&parse("Cmd+-").unwrap(), Key::Minus, cmd));
        assert!(key_matches(&parse("Cmd+0").unwrap(), Key::Num0, cmd));
        // Ctrl (Windows) is ⌘.
        assert!(key_matches(&zoom_in, Key::Plus, Modifiers::CTRL | Modifiers { command: true, ..Default::default() }));
        // Exact modifiers: ⌘⌥I is Image Size, never Invert (⌘I); ⇧⌘Z is not ⌘Z.
        let invert = parse("Cmd+I").unwrap();
        assert!(key_matches(&invert, Key::I, cmd));
        assert!(!key_matches(&invert, Key::I, cmd | Modifiers::ALT));
        assert!(!key_matches(&parse("Cmd+Z").unwrap(), Key::Z, cmd | Modifiers::SHIFT));
        // ⇧ shortcuts on punctuation arrive as the shifted character.
        assert!(key_matches(&parse("Cmd+Shift+;").unwrap(), Key::Colon, cmd | Modifiers::SHIFT));
        assert!(key_matches(&parse("Cmd+Shift+]").unwrap(), Key::CloseCurlyBracket, cmd | Modifiers::SHIFT));
        assert!(!key_matches(&parse("Cmd+;").unwrap(), Key::Colon, cmd | Modifiers::SHIFT));
        // Delete and the Mac's delete (Backspace) both clear.
        let clear = parse("Delete").unwrap();
        assert!(key_matches(&clear, Key::Delete, Modifiers::NONE) && key_matches(&clear, Key::Backspace, Modifiers::NONE));
        assert!(!key_matches(&clear, Key::Backspace, cmd));
    }

    #[test]
    fn clipboard_events_become_pixel_shortcuts_outside_text() {
        let ctx = egui::Context::default();
        let raw = |mut events: Vec<egui::Event>, modifiers| {
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            egui::RawInput { events, ..Default::default() }
        };
        let keys = |r: &egui::RawInput| -> Vec<(Key, bool, bool)> {
            r.events
                .iter()
                .filter_map(|e| if let egui::Event::Key { key, pressed, modifiers, .. } = e { Some((*key, *pressed, modifiers.shift)) } else { None })
                .collect()
        };
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let mut r = raw(vec![egui::Event::Copy, egui::Event::Cut], cmd_shift);
        clipboard_keys(&ctx, false, &mut r);
        assert_eq!(keys(&r), vec![(Key::C, true, true), (Key::X, true, true)]);
        // A text paste becomes ⌘V; its key release then adds nothing.
        let up = egui::Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
        let mut r = raw(vec![egui::Event::Paste("x".into()), up.clone()], Modifiers::COMMAND);
        clipboard_keys(&ctx, false, &mut r);
        assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
        // An image on the clipboard: egui-winit sends only the release. It still pastes.
        let mut r = raw(vec![up.clone()], Modifiers::COMMAND);
        clipboard_keys(&ctx, false, &mut r);
        assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
        // In a text field the events stay text clipboard events.
        let mut r = raw(vec![egui::Event::Paste("x".into())], Modifiers::COMMAND);
        clipboard_keys(&ctx, true, &mut r);
        assert!(matches!(r.events.as_slice(), [_, egui::Event::Paste(_)]));
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
