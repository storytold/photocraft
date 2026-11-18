//! Keyboard shortcuts: the notation and lookup of a command's keys, the clipboard key fix, and the
//! per-pass entry that runs the commands the runtime's keys fired.

use craft::keymap::{Chord, Platform};
use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Parse `Cmd+Shift+N` style strings. `Cmd` maps to ⌘ on macOS and Ctrl elsewhere.
pub fn parse(s: &str) -> Option<KeyboardShortcut> {
    Chord::parse(s).ok().map(|c| c.to_egui_shortcut())
}

/// Human-readable form for menus, tooltips and inline hints (`Cmd+Shift+N` → `⌘⇧N` on
/// macOS, `Ctrl+Shift+N` elsewhere). Works for lone keys too: `pretty("Alt")` is `⌥`/`Alt`,
/// `pretty("Enter")` is `↩`/`Enter`.
pub fn pretty(s: &str) -> String {
    match Chord::parse(s) {
        Ok(chord) => chord.display(Platform::current()),
        Err(_) => lone_key(s),
    }
}

/// A key text that is no chord (`Alt`, `Enter` as the options bar writes them), in the platform's symbols.
fn lone_key(s: &str) -> String {
    let mac = cfg!(target_os = "macos");
    s.split('+')
        .map(|p| match (p, mac) {
            ("Cmd", true) => "⌘".to_string(),
            ("Cmd", false) => "Ctrl".to_string(),
            ("Shift", true) => "⇧".to_string(),
            ("Alt", true) => "⌥".to_string(),
            ("Enter", true) => "↩".to_string(),
            ("Delete", true) => "⌫".to_string(),
            (other, _) => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(if mac { "" } else { "+" })
}

/// The command's effective shortcut in the platform's notation (`Ctrl+N`), or `None` when
/// the command has no binding (the user removed it in Edit › Keyboard Shortcuts).
pub fn shortcut_label(app: &PhotocraftApp, command: &str) -> Option<String> {
    effective_shortcut(app, command, default_shortcut(command).as_deref()).map(|s| pretty(&s))
}

/// The command's effective shortcut as a chord, for framework components that show or bind it.
pub fn chord(app: &PhotocraftApp, command: &str) -> Option<craft::keymap::Chord> {
    effective_shortcut(app, command, default_shortcut(command).as_deref()).and_then(|s| craft::keymap::Chord::parse(&s).ok())
}

/// `label` (English, shown in the UI language) followed by the command's effective shortcut
/// (`New document…     Ctrl+N`); bare `label` when the binding was removed.
pub fn command_label(app: &PhotocraftApp, label: &str, command: &str) -> String {
    let label = tl!(label);
    shortcut_label(app, command).map_or_else(|| label.to_string(), |sc| format!("{label}     {sc}"))
}

/// `label` (English, shown in the UI language) with the command's effective shortcut in
/// parentheses (`Rulers  (Ctrl+R)`); bare `label` when the binding was removed.
pub fn tip_label(app: &PhotocraftApp, label: &str, command: &str) -> String {
    let label = tl!(label);
    shortcut_label(app, command).map_or_else(|| label.to_string(), |sc| format!("{label}  ({sc})"))
}

/// The shortcut in effect for a command: the user's override, else `default`.
pub fn effective_shortcut(app: &PhotocraftApp, id: &str, default: Option<&str>) -> Option<String> {
    app.session.prefs().shortcut(id, default).map(str::to_string)
}

/// A command's further keys in effect besides its main one (#3665): the user's, else
/// Photoshop's (F3 for Copy, Shift+F7 for Inverse…). Menus show only the main key.
pub fn effective_alternates(app: &PhotocraftApp, id: &str) -> Vec<String> {
    let main = effective_shortcut(app, id, default_shortcut(id).as_deref());
    app.session.prefs().alternates(id, main.as_deref())
}

/// Default (unmodified) shortcut of a command id.
pub fn default_shortcut(id: &str) -> Option<String> {
    crate::menus::UI_COMMANDS
        .iter()
        .find(|c| c.0 == id)
        .and_then(|c| c.3)
        .or_else(|| photocraft_engine::commands::find(id).and_then(|c| c.shortcut))
        .or_else(|| crate::menu_catalog::CATALOG.iter().find(|c| c.3 == id).and_then(|c| c.2))
        .or_else(|| photocraft_engine::prefs::TEMPORARY_TOOLS.iter().find(|t| t.0 == id).map(|t| t.2))
        .map(str::to_string)
        .or_else(|| crate::tool_shortcuts::tool(id).and_then(crate::tool_shortcuts::default))
}

/// View navigation that Photoshop keeps live while a dialog is open (so the image can be judged
/// at any zoom): Zoom In / Out, Fit on Screen, 100%.
pub const NAV_COMMANDS: [&str; 4] = ["view.zoomIn", "view.zoomOut", "view.fitOnScreen", "view.actualPixels"];

/// The key press a clipboard event stands for outside text fields, with `mods` held: Copy, Cut
/// and Paste are ⌘C, ⌘X and ⌘V (with ⇧/⌥ as held). On Windows egui-winit also sends Cut for
/// Shift+Delete, which is Edit › Fill here (#530): a Cut with ⇧ and without Ctrl is that key.
fn clipboard_press(e: &egui::Event, mods: Modifiers, is_windows: bool) -> Option<(Key, Modifiers)> {
    let held = if mods.command { mods } else { Modifiers::COMMAND };
    match e {
        egui::Event::Copy => Some((Key::C, held)),
        egui::Event::Cut if is_windows && mods.shift && !mods.command => Some((Key::Delete, mods)),
        egui::Event::Cut => Some((Key::X, held)),
        egui::Event::Paste(_) => Some((Key::V, held)),
        // egui 0.37 emits this when the clipboard holds an image and no text. Treat it as the
        // paste shortcut; the command reads the OS clipboard once through the existing import
        // path, never a second time from this event.
        egui::Event::PasteImage(_) => Some((Key::V, held)),
        _ => None,
    }
}

/// Text is selected in a label (an error notice's message, say): dragged across, double- or
/// triple-clicked or shift-clicked, as egui selects it. ⌘C then copies that text, as in a text
/// field, rather than running Edit › Copy on the layer (#7807). A plain click on a label leaves an
/// empty selection behind, which must not take ⌘C from the document. Read once a frame, from the
/// frame before.
fn label_text_selected(ctx: &egui::Context) -> bool {
    let id = egui::Id::unique("pc-label-text-selected");
    let any = ctx.plugin_opt::<egui::text_selection::LabelSelectionState>().is_some_and(|p| p.lock().has_selection());
    let (selects, pressed) = ctx.input(|i| {
        let p = &i.pointer;
        let selects = (p.primary_down() && p.is_decidedly_dragging())
            || p.button_double_clicked(egui::PointerButton::Primary)
            || p.button_triple_clicked(egui::PointerButton::Primary)
            || (p.primary_pressed() && i.modifiers.shift);
        (selects, p.primary_pressed())
    });
    let selected = any && (selects || (!pressed && ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)));
    ctx.data_mut(|d| d.insert_temp(id, selected));
    selected
}

/// ⌘C, as egui-winit's `Event::Copy` or as a key press (the web, the Mac menu's key equivalent),
/// without ⇧ (Copy Merged) or ⌥.
fn plain_copy(e: &egui::Event, mods: Modifiers) -> bool {
    match e {
        egui::Event::Copy => !mods.shift && !mods.alt,
        egui::Event::Key { key: Key::C, pressed: true, modifiers: m, .. } => m.command && !m.shift && !m.alt,
        _ => false,
    }
}

/// egui-winit turns ⌘C / ⌘X / ⌘V into `Event::Copy` / `Cut` / `Paste` and drops the key press, and
/// drops ⌘V entirely when the clipboard holds no text (an image). Outside text fields, give the
/// pixel commands their key presses back: Copy/Cut/Paste become ⌘C/⌘X/⌘V presses (with ⇧/⌥ as
/// held, so ⇧⌘C is Copy Merged), and a ⌘V release without a ⌘V press (a paste event, or the Mac
/// menu's key equivalent) becomes a ⌘V press.
///
/// The web build gets both: eframe forwards the ⌘V key-down, then the browser's `paste` event
/// (the key-down's default action) arrives as `Event::Paste` in the same frame. A clipboard event
/// right after its own ⌘ key press is that press, not a second one (#2638).
///
/// With an image on the clipboard the V release is all that reaches us, and ⌘ is often let go
/// first: a V release whose press never arrived, after ⌘ was held and before any other key, is
/// that swallowed ⌘V too (#6329). Releasing in either order then pastes.
///
/// Ctrl+Insert copies and Shift+Insert pastes: egui-winit maps them on Windows (a Shift+Insert
/// release without a paste event pastes the image, like ⌘V), and we map them here on Linux.
///
/// With text selected in a label ([`label_text_selected`]) ⌘C copies the text; ⌘X and ⌘V stay
/// the document's.
pub fn clipboard_keys(ctx: &egui::Context, typing: bool, raw: &mut egui::RawInput) {
    let text_selected = label_text_selected(ctx);
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    if cfg!(target_os = "linux") {
        raw.events.retain_mut(|e| {
            let egui::Event::Key { key: Key::Insert, pressed: true, modifiers: m, .. } = *e else { return true };
            match (m.ctrl, m.shift) {
                (true, false) => *e = egui::Event::Copy,
                // Text fields need the clipboard text: egui-winit reads it and sends a Paste event.
                (false, true) if typing => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    return false;
                }
                (false, true) => *e = press(Key::V, Modifiers::COMMAND),
                _ => {}
            }
            true
        });
    }
    if typing {
        // A text field copies and pastes on egui's Copy / Cut / Paste events, which egui-winit
        // makes from the window's key and so never sends as key presses. A ⌘C / ⌘X / ⌘V press
        // here is the Mac menu's key equivalent (AppKit takes the key while Edit › Copy, Cut or
        // Paste is enabled), which a text field ignores: pasting text worked only while
        // PhotoCraft's own clipboard was empty. Give those presses back as the events; Paste asks
        // the window for the clipboard text, which arrives as a Paste event next frame.
        raw.events.retain_mut(|e| {
            let egui::Event::Key { key, pressed: true, modifiers: m, .. } = *e else { return true };
            if !m.command || m.alt || m.shift {
                return true;
            }
            match key {
                Key::C => *e = egui::Event::Copy,
                Key::X => *e = egui::Event::Cut,
                Key::V => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    return false;
                }
                _ => {}
            }
            true
        });
        return;
    }
    let seen = egui::Id::unique("pc-paste-key-seen");
    // A V press without ⌘ that reached us (the Move tool's key): its release is not a paste.
    let plain_v = egui::Id::unique("pc-plain-v-down");
    // ⌘ was held since the last key press without it: a swallowed ⌘V can be under way.
    let command_lately = egui::Id::unique("pc-paste-command-lately");
    // The modifiers held at each event: last frame's, updated as the events report changes.
    let mut mods = ctx.input(|i| i.modifiers);
    if mods.command {
        ctx.data_mut(|d| d.insert_temp(command_lately, true));
    }
    let mut out = Vec::with_capacity(raw.events.len());
    // The last ⌘C/⌘X/⌘V press this frame whose clipboard event hasn't arrived (yet).
    let mut unpaired: Option<Key> = None;
    for e in raw.events.drain(..) {
        if let egui::Event::ModifiersChanged(m) | egui::Event::Key { modifiers: m, .. } = &e {
            mods = *m;
            if mods.command {
                ctx.data_mut(|d| d.insert_temp(command_lately, true));
            }
        }
        if text_selected && plain_copy(&e, mods) {
            out.push(egui::Event::Copy);
            continue;
        }
        if let Some((key, held)) = clipboard_press(&e, mods, cfg!(target_os = "windows")) {
            if key == Key::V {
                ctx.data_mut(|d| {
                    d.insert_temp(seen, true);
                    d.remove_temp::<bool>(plain_v);
                });
            }
            if unpaired.take_if(|k| *k == key).is_none() {
                out.push(press(key, held));
            }
            continue;
        }
        match &e {
            // A ⌘C/⌘X/⌘V press egui-winit didn't swallow (the web, the Mac menu's key equivalent,
            // Shift+Insert on Linux): its clipboard event and, for ⌘V, its release must not run
            // the command a second time (#2638, #1638).
            egui::Event::Key { key: key @ (Key::C | Key::X | Key::V), pressed: true, modifiers, .. } if modifiers.command => {
                unpaired = Some(*key);
                if *key == Key::V {
                    ctx.data_mut(|d| {
                        d.insert_temp(seen, true);
                        d.remove_temp::<bool>(plain_v);
                    });
                }
            }
            egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command => {
                ctx.data_mut(|d| {
                    d.remove_temp::<bool>(command_lately);
                    if *key == Key::V {
                        d.insert_temp(plain_v, true);
                    }
                });
            }
            // A paste key's release ends that press, with or without ⌘ still held (else a stale
            // mark would swallow the next image paste); without a press it is the paste itself.
            egui::Event::Key { key: key @ (Key::V | Key::Insert), pressed: false, modifiers, .. } => {
                let had_press = ctx.data_mut(|d| d.remove_temp::<bool>(seen)).unwrap_or(false);
                let had_plain_press = *key == Key::V && ctx.data_mut(|d| d.remove_temp::<bool>(plain_v)).unwrap_or(false);
                // Taken: one swallowed ⌘V is one paste.
                let lately = *key == Key::V && ctx.data_mut(|d| d.remove_temp::<bool>(command_lately)).unwrap_or(false);
                let held = match key {
                    Key::V if modifiers.command => Some(*modifiers),
                    // ⌘ let go before V: the press egui-winit swallowed was ⌘V (#6329). The web
                    // always forwards the key-down, so there a lone release is no paste.
                    Key::V if lately && !had_plain_press && !cfg!(target_arch = "wasm32") => Some(Modifiers::COMMAND),
                    Key::Insert if cfg!(target_os = "windows") && modifiers.shift && !modifiers.ctrl => Some(Modifiers::COMMAND),
                    _ => None,
                };
                if let Some(held) = held
                    && !had_press
                {
                    out.push(press(Key::V, held));
                }
            }
            _ => {}
        }
        out.push(e);
    }
    raw.events = out;
}

/// The frame's key call, once per pass: runs the commands the runtime's keys fired in the pass before
/// ([`crate::shortcut_dispatch::run_fired`]), then the held-key work of the app; Escape is [`escape_keys`]'s. The fired list is taken,
/// so a second pass in one frame runs only what its own input fired. The main window runs no shortcuts while the
/// screen color picker is open.
pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let fired = std::mem::take(&mut app.framework.fired);
    if crate::screen_picker::busy(app) {
        return;
    }
    crate::shortcut_dispatch::run_fired(app, ctx, &fired);
    // A held tool letter released: a temporary tool gives the previous one back (#2672).
    crate::spring_tools::tick(app, ctx);
    // However the tool was picked (toolbar, flyout, shortcut, automation), its group's key and its
    // toolbar slot bring it back later (#2608). A temporary tool is no choice, so it isn't noted.
    app.ui.remember_group_tool(crate::tool_memory::selected(app));
}

/// Esc and the view keys. The UI pass runs it last (`ui_body`), so the canvas's
/// sessions and tools, the dialogs and every other UI that takes Esc have had it first, and only what they left
/// reaches the window's intents. The main window takes none while the screen color picker is open, and a foreground
/// save (Save in Background off, #6207) blocks the app until the file is written: only Esc reaches it, to cancel
/// while it still encodes (see `jobs_ui::tick`).
pub fn escape_keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if crate::screen_picker::busy(app) || crate::jobs_ui::foreground_save(app).is_some() {
        return;
    }
    handle_escape_and_view_keys(app, ctx);
}

/// Esc (reset a rotated view, else leave full screen, else deselect), the Delete Anchor Point and Path Selection Esc
/// stages, and the `+` / `−` path operations.
/// Runs last, so every state that owns Esc has already had its chance.
fn handle_escape_and_view_keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A Properties Levels/Curves eyedropper: Esc disarms it.
    if !app.dialog_open() && crate::layer_picker::armed(app).is_some() && crate::window_keys::take_escape(ctx) {
        crate::layer_picker::cancel(app);
        return;
    }
    // Esc with no active operation leaves a full screen mode, else deselects (#2674), like ⌘D.
    // The key is never taken from a state that handles Esc itself: a gesture on the canvas (a
    // marquee being drawn, a Direct Selection drag on an anchor, whose release is still to come,
    // #13072), inline type, the palette, the canvas menu, the brush picker, Rotate View, any dialog
    // and any open popup (flyouts and menus included).
    let esc_taken = crate::canvas::tools::in_gesture(app)
        || crate::direct_select::editing(app)
        || app.ui.text_edit.is_some()
        || app.ui.palette.open
        || app.ui.canvas_tool_menu.is_some()
        || app.ui.brush_picker.is_some()
        || app.ui.tool == Tool::RotateView
        || app.dialog_open()
        || ctx.any_popup_open();
    // A rotated view takes the first idle Esc (#13607), before leaving full screen or deselecting.
    // Rotate View handles its own Escape (its canvas tool), where it also ends a rotation drag.
    if !esc_taken
        && let Some(view) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i))
        && view.rotation != 0.0
        && crate::window_keys::take_escape(ctx)
    {
        view.rotation = 0.0;
        return;
    }
    // Both full screen modes (with and without the menu bar) return to Standard, as in Photoshop
    // (#4116). The shell brings the window to the mode it binds, and the Screen Mode check follows it.
    if !esc_taken && app.ui.view.screen_mode != craft::ui::shell::ScreenMode::Standard && crate::window_keys::take_escape(ctx) {
        app.ui.view.screen_mode = craft::ui::shell::ScreenMode::Standard;
        return;
    }
    if !esc_taken && app.session.active().is_some_and(|st| st.doc.selection.is_some()) && crate::window_keys::take_escape(ctx) {
        if let Err(e) = app.run("select.deselect", json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // Delete Anchor Point (#12773): Esc first hides the anchors of the saved path targeted in the
    // Paths panel, then deselects that path, so clicks edit nothing. The first Esc leaves the path
    // targeted whether or not an anchor was selected (#13072), and clears only anchors selected on
    // that path, never another path's.
    if !esc_taken
        && app.ui.tool == Tool::DeleteAnchorPoint
        && let Some((index, _)) = crate::vector_ui::selected_saved_path_index(app)
    {
        let ds = &app.ui.direct_selection;
        let on_this_path = ds.target == Some(crate::direct_select::PathRef::Saved(index));
        let selected = on_this_path && (!ds.anchors.is_empty() || ds.segment.is_some());
        if crate::window_keys::take_escape(ctx) {
            // The stage belongs to this path: choosing another one starts at the first Esc again.
            if selected || app.ui.direct_selection.escape_hidden != Some(index) {
                if selected {
                    app.ui.direct_selection.anchors.clear();
                    app.ui.direct_selection.segment = None;
                }
                app.ui.direct_selection.escape_hidden = Some(index);
            } else {
                app.ui.selected_path = None;
                app.ui.direct_selection.escape_hidden = None;
            }
            return;
        }
    }
    // Path Selection (#13287): with components of the saved path targeted in the Paths panel
    // selected, a bare Esc deselects the components and keeps the path targeted and drawn, with no
    // History step (the tool's bare Enter also untargets the path). A pixel selection takes the Esc
    // first, above; ⇧Esc and ⌥Esc pass on.
    if !esc_taken
        && app.ui.tool == Tool::PathSelection
        && ctx.input(|i| i.modifiers.is_none())
        && crate::vector_ui::saved_path_components_selected(app)
        && crate::window_keys::take_escape(ctx)
    {
        crate::path_components::deselect_components(app);
        return;
    }
    // + / − pick a Path Operation with the Pen tools and Path Selection (`path_components`).
    crate::path_components::handle_keys(app, ctx);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tool_key_default_tests.rs"]
mod tool_key_default_tests;
