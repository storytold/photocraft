use super::dialogs::toolbar_tab;
use super::{BTreeMap, Map, PhotocraftApp, RichText, Tokens, Value, default_shortcut, json, prefs, vec2};

/// Every shortcut-bearing command (engine registry plus shell commands): (id, label, menu path,
/// default shortcut), menu items first in Photoshop order.
pub fn shortcut_items(app: &PhotocraftApp) -> Vec<(String, String, Vec<String>, Option<String>)> {
    let mut out: Vec<(String, String, Vec<String>, Option<String>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for it in crate::menus::menu_items(app) {
        if it.label == "---" || !seen.insert(it.id.clone()) {
            continue;
        }
        let default = default_shortcut(&it.id).or(it.shortcut.clone().filter(|_| !app.session.prefs().shortcuts.contains_key(&it.id)));
        out.push((it.id, it.label, it.path, default));
    }
    // Commands without a menu item: the colour and fill keys (D, X, ⌥⌫…) sit under Tools, as
    // Photoshop lists its colour keys in the Tools shortcut set, after the rest; then the held
    // temporary tools (#249).
    let tools_key = |id: &str| id.starts_with("tools.") || photocraft_engine::fill_key_cmds::IDS.contains(&id);
    let mut tools = Vec::new();
    let mut layer = Vec::new();
    // The Mixer Brush's Load, Clean and switches have no default key but are listed together under
    // Tools, as in the reference (#11959).
    let mut mixer = Vec::new();
    for c in photocraft_engine::command_specs() {
        if prefs::MIXER_BRUSH_COMMANDS.contains(&c.id) && seen.insert(c.id.to_string()) {
            mixer.push((c.id.to_string(), c.label.to_string(), vec!["Tools".to_string()], c.shortcut.map(Into::into)));
            continue;
        }
        if seen.insert(c.id.to_string()) && c.shortcut.is_some() {
            let item = (c.id.to_string(), c.label.to_string(), c.menu.iter().map(|s| s.to_string()).collect::<Vec<_>>(), c.shortcut.map(Into::into));
            if c.menu.is_empty() && tools_key(c.id) {
                tools.push((item.0, item.1, vec!["Tools".to_string()], item.3));
            } else if c.menu.is_empty() && c.id.starts_with("layer.") {
                // Stamp Visible / Stamp Down (#217): no menu item in Photoshop; listed at the end
                // of the Layer section.
                layer.push((item.0, item.1, vec!["Layer".to_string()], item.3));
            } else {
                out.push(item);
            }
        }
    }
    let at = out.iter().rposition(|i| i.2.first().map(String::as_str) == Some("Layer")).map_or(out.len(), |i| i + 1);
    out.splice(at..at, layer);
    out.extend(tools);
    mixer.sort_by_key(|item| prefs::MIXER_BRUSH_COMMANDS.iter().position(|id| *id == item.0));
    out.extend(mixer);
    out.extend(
        crate::state::Tool::ALL
            .into_iter()
            .map(|tool| (crate::tool_shortcuts::id(tool), tool.label().to_string(), vec!["Tools".into()], crate::tool_shortcuts::default(tool))),
    );
    for (id, label, def) in prefs::TEMPORARY_TOOLS {
        if seen.insert(id.to_string()) {
            out.push((id.to_string(), label.to_string(), vec!["Tools".into(), "Temporary".into()], Some(def.to_string())));
        }
    }
    out
}

/// A fingerprint of the bindings a shortcut conflict describes.
fn conflict_state(overrides: &BTreeMap<String, String>, alternates: &BTreeMap<String, Vec<String>>, selected: &str, set: Option<&Value>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (overrides, alternates, selected, set.map(Value::to_string)).hash(&mut h);
    h.finish()
}

/// Keys the system takes before the app sees them (switching apps, closing the window, force
/// quit): Edit › Keyboard Shortcuts won't assign them, as Photoshop won't. `sc` is in our
/// notation, where `Cmd` is Ctrl off the Mac.
pub fn reserved_shortcut(sc: &str) -> bool {
    prefs::reserved_shortcut(sc)
}

/// Is a shortcut field in the front dialog (Edit › Keyboard Shortcuts) waiting for keys? Then
/// the key pressed is the shortcut to record, so nothing else may take it first (#8965): not the
/// dialog's own ⌘Z (#9487), the view keys that work under dialogs (⌘0, ⌘+…), or the macOS menu
/// bar, which swallows the key equivalent of a disabled item.
pub(crate) fn capturing_shortcut(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| d.kind == crate::state::DialogKind::Command && d.fields.get("capture").and_then(Value::as_bool) == Some(true))
}

/// Text of a key press for the shortcut editor (`Cmd+Shift+K`), `None` for bare modifiers.
pub fn shortcut_text(key: egui::Key, m: egui::Modifiers) -> Option<String> {
    if is_modifier_key(key) {
        return None;
    }
    let mut parts = Vec::new();
    if m.command || m.mac_cmd {
        parts.push(tl!("Cmd").to_string());
    }
    if m.ctrl && !m.command {
        parts.push(tl!("Ctrl").into());
    }
    if m.alt {
        parts.push(tl!("Alt").into());
    }
    if m.shift {
        parts.push(tl!("Shift").into());
    }
    let name = match key {
        egui::Key::Equals => "=",
        egui::Key::Minus => "-",
        egui::Key::OpenBracket => "[",
        egui::Key::CloseBracket => "]",
        egui::Key::Semicolon => ";",
        egui::Key::Quote => "'",
        egui::Key::Plus => "",
        k => k.name(),
    };
    parts.push(name.to_string());
    prefs::normalize_shortcut(&parts.join("+"))
}

/// A modifier key on its own. egui reports Ctrl, Shift, Alt and ⌘ presses as key events too
/// (`ControlLeft`…), before the key pressed with them: they are never a shortcut's key, so
/// shortcut capture waits for the real key (#292: Ctrl+F was recorded as "Ctrl+ControlLeft").
pub fn is_modifier_key(key: egui::Key) -> bool {
    use egui::Key::*;
    matches!(key, ShiftLeft | ShiftRight | ControlLeft | ControlRight | AltLeft | AltRight | SuperLeft | SuperRight)
}

/// Modifier bits a physical modifier-key press contributes when egui leaves them out of
/// [`Modifiers`] on the key event (#7190).
fn modifier_key_modifiers(key: egui::Key) -> egui::Modifiers {
    use egui::Key::*;
    match key {
        ShiftLeft | ShiftRight => egui::Modifiers::SHIFT,
        ControlLeft | ControlRight => egui::Modifiers::CTRL,
        AltLeft | AltRight => egui::Modifiers::ALT,
        SuperLeft | SuperRight => egui::Modifiers::COMMAND,
        _ => egui::Modifiers::NONE,
    }
}

fn capture_mods_field(v: Option<&Value>) -> egui::Modifiers {
    let o = v.and_then(Value::as_object);
    egui::Modifiers {
        shift: o.and_then(|o| o.get("shift")).and_then(Value::as_bool).unwrap_or(false),
        ctrl: o.and_then(|o| o.get("ctrl")).and_then(Value::as_bool).unwrap_or(false),
        alt: o.and_then(|o| o.get("alt")).and_then(Value::as_bool).unwrap_or(false),
        command: o.and_then(|o| o.get("command")).and_then(Value::as_bool).unwrap_or(false),
        mac_cmd: o.and_then(|o| o.get("mac_cmd")).and_then(Value::as_bool).unwrap_or(false),
    }
}

fn capture_mods_json(m: egui::Modifiers) -> Value {
    json!({
        "shift": m.shift,
        "ctrl": m.ctrl,
        "alt": m.alt,
        "command": m.command,
        "mac_cmd": m.mac_cmd,
    })
}

/// While capture is active: refresh held modifiers (including across frames via dialog
/// `captureMods`) and, if a non-modifier key was pressed this frame, return it with the
/// modifiers held with it.
///
/// #7190 root cause: capture used to [`find_map`](Iterator::find_map) the first non-modifier
/// [`Event::Key`] and pass only that event's `modifiers` into [`shortcut_text`]. On Linux and
/// Windows, winit/egui often reports `ShiftLeft`/`ShiftRight` (and sometimes the letter key) with
/// [`Modifiers::NONE`], and the modifier down and the letter down often land in separate UI
/// frames. Shift was then missing from the stored binding (`F6` instead of `Shift+F6`). Like #292
/// for `ControlLeft`, we walk the frame's events, OR in [`modifier_key_modifiers`] for modifier
/// keys, and persist accumulated modifiers in `captureMods` until capture ends.
pub(crate) fn capture_key_press(capture_mods: egui::Modifiers, i: &egui::InputState) -> (egui::Modifiers, Option<(egui::Key, egui::Modifiers)>) {
    let mut mods = capture_mods | i.modifiers;
    let mut pressed = None;
    for e in &i.events {
        if let egui::Event::ModifiersChanged(m) = e {
            mods = *m;
        }
        if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
            if is_modifier_key(*key) {
                mods = *modifiers | modifier_key_modifiers(*key);
                continue;
            }
            pressed = Some((*key, mods | *modifiers));
            break;
        }
        if let egui::Event::Key { key, pressed: false, modifiers, .. } = e
            && is_modifier_key(*key)
        {
            mods = *modifiers;
        }
    }
    (mods, pressed)
}

/// Keyboard Shortcuts and Menus: a searchable list of commands by menu with editable shortcuts
/// (click a shortcut, press keys; ⌫ removes it), menu visibility and colours, toolbar tools.
/// The dialog field holding the key pressed into the open field until it is committed. The
/// `__` prefix keeps it out of the dialog's ⌘Z history: only a committed key is an edit.
const PENDING: &str = "__pending";

/// The box below the list while no shortcut is being edited (Photoshop's wording).
const INSTRUCTIONS: &str = "To edit keyboard shortcuts:\n1) Click on the New Set button to create a copy of the selected set, or select a set to modify.\n2) Click in the \"Shortcut\" column for a command and press the keyboard shortcut to assign.\n3) Save the set when you are done editing to save all your changes.";

/// F1–F35 with a modifier: Photoshop warns that such a key, free here, can be taken by actions.
fn function_key_with_modifiers(key: egui::Key, m: egui::Modifiers) -> bool {
    use egui::Key::*;
    let function = matches!(
        key,
        F1 | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
            | F25
            | F26
            | F27
            | F28
            | F29
            | F30
            | F31
            | F32
            | F33
            | F34
            | F35
    );
    function && (m.ctrl || m.command || m.alt || m.shift || m.mac_cmd)
}

/// The message once a field's key is in the set: a conflict keeps its warning (and its Accept
/// and Go to Conflict / Undo Changes, re-stamped with the new bindings); any other key is assigned.
fn commit_message(message: &mut String, warning: &mut bool, conflict: Option<&mut Value>, sc: &str) {
    if let Some(c) = conflict.and_then(Value::as_object_mut) {
        c.remove("state");
    } else {
        *message = crate::i18n::fmt(tl!("{shortcut} assigned."), &[("shortcut", &crate::shortcuts::pretty(sc))]);
        *warning = false;
    }
}

pub(super) fn shortcuts_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mut tab = f.get("tab").and_then(Value::as_u64).unwrap_or(0);
    ui.horizontal(|ui| {
        for (i, name) in [tl!("Keyboard Shortcuts"), tl!("Menus"), tl!("Toolbar")].iter().enumerate() {
            if crate::widgets::pill_tab(ui, name, tab == i as u64).clicked() {
                tab = i as u64;
                f.insert("capture".into(), json!(false));
            }
        }
    });
    f.insert("tab".into(), json!(tab));
    ui.add_space(6.0);
    if tab == 2 {
        toolbar_tab(ui, f);
        return;
    }
    // A malformed map (set over automation) is left as it is, so OK rejects it instead of
    // saving the empty map drawn in its place.
    let parsed = f.get("overrides").map(|v| serde_json::from_value::<BTreeMap<String, String>>(v.clone()));
    let overrides_ok = parsed.as_ref().is_none_or(Result::is_ok);
    let mut overrides = parsed.and_then(Result::ok).unwrap_or_default();
    // A command's further keys (#3665): id → keys, replacing its default alternates.
    let parsed_alts = f.get("alternates").map(|v| serde_json::from_value::<BTreeMap<String, Vec<String>>>(v.clone()));
    let alternates_ok = parsed_alts.as_ref().is_none_or(Result::is_ok);
    let mut alt_overrides = parsed_alts.and_then(Result::ok).unwrap_or_default();
    let mut capture_alt = f.get("captureAlt").and_then(Value::as_bool).unwrap_or(false);
    // A key just assigned that another command holds: Photoshop's Accept and Go to Conflict /
    // Undo Changes (an object: the key, the other command, and this one's keys before).
    let mut conflict: Option<Value> = f.get("conflict").filter(|c| c.is_object()).cloned();
    let mut hidden: Vec<String> = f.get("hidden").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let mut colors: BTreeMap<String, String> = f.get("colors").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let mut selected = f.get("selected").and_then(Value::as_str).unwrap_or("").to_string();
    let mut capture = f.get("capture").and_then(Value::as_bool).unwrap_or(false);
    let mut message = f.get("message").and_then(Value::as_str).unwrap_or("").to_string();
    let mut message_warning = f.get("messageWarning").and_then(Value::as_bool).unwrap_or(false);
    // The key pressed into the open field, not in the set yet: Accept, Enter or moving to
    // another row commits it, Escape drops it (Photoshop's pending edit, #14310).
    let mut pending: Option<String> = f.get(PENDING).and_then(Value::as_str).filter(|_| capture).map(str::to_string);
    // The field open when this frame began: the row (and main key or one more) a pending key belongs to.
    let editing = (capture && tab == 0 && !selected.is_empty()).then(|| (selected.clone(), capture_alt));
    let mut reopened = false;
    let items = shortcut_items(app);
    // The Set row, then the search (the Menus tab searches too).
    if tab == 0 {
        set_row(app, ui, f, &mut overrides, &mut alt_overrides, &items, &mut message, &mut message_warning);
        ui.add_space(4.0);
    }
    let mut filter = f.get("filter").and_then(Value::as_str).unwrap_or("").to_string();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Search")).color(t.text_dim));
        ui.add(egui::TextEdit::singleline(&mut filter).desired_width(260.0).hint_text(tl!("command or shortcut")));
    });
    f.insert("filter".into(), json!(filter));
    // Menu path and command label as shown in the menus, in the UI language.
    let shown = |id: &str, label: &str, path: &[String]| -> (Vec<String>, String) {
        let lang = crate::i18n::current();
        (path.iter().map(|p| crate::i18n::tr(lang, p).to_string()).collect(), crate::i18n::tr_id(lang, id, label).to_string())
    };
    let eff = |overrides: &BTreeMap<String, String>, id: &str, def: &Option<String>| -> Option<String> {
        match overrides.get(id) {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s.clone()),
            None => def.clone(),
        }
    };
    // A command's further keys as the dialog stands: its edited list, else its defaults.
    let eff_alts = |alts: &BTreeMap<String, Vec<String>>, id: &str, main: Option<&str>| -> Vec<String> {
        let mut p = prefs::Preferences::default();
        if let Some(list) = alts.get(id) {
            p.alternate_shortcuts.insert(id.to_string(), list.clone());
        }
        p.alternates(id, main)
    };
    // Writes a committed key into the set: the command's main key, or one more key.
    let assign = |overrides: &mut BTreeMap<String, String>, alt_overrides: &mut BTreeMap<String, Vec<String>>, command: &str, sc: &str, alt: bool| {
        if alt {
            let def = items.iter().find(|i| i.0 == command).and_then(|i| i.3.clone());
            let cur = eff(overrides, command, &def);
            let mut keys = eff_alts(alt_overrides, command, cur.as_deref());
            if cur.as_deref().and_then(prefs::normalize_shortcut).as_deref() != Some(sc) && !keys.iter().any(|k| k == sc) && keys.len() < prefs::MAX_ALTERNATES
            {
                keys.push(sc.to_string());
            }
            alt_overrides.insert(command.to_string(), keys);
        } else {
            overrides.insert(command.to_string(), sc.to_string());
        }
    };
    let mut commit = false;
    let mut capture_mods = capture_mods_field(f.get("captureMods"));
    // Key capture for the selected command: its main key, or (Add Shortcut) one more key.
    if capture && tab == 0 && !selected.is_empty() {
        let (next_mods, pressed) = ui.input(|i| capture_key_press(capture_mods, i));
        capture_mods = next_mods;
        if let Some((key, m)) = pressed {
            ui.input_mut(|i| {
                i.consume_key(m, key);
            });
            if key == egui::Key::Escape {
                // The field goes back to the key it had; the dialog stays open.
                capture = false;
                pending = None;
                conflict = None;
                message.clear();
                message_warning = false;
            } else if key == egui::Key::Enter {
                // Enter commits the field; it is never recorded as a key.
                commit = true;
            } else if matches!(key, egui::Key::Backspace | egui::Key::Delete) && m.is_none() {
                if capture_alt {
                    alt_overrides.insert(selected.clone(), Vec::new());
                } else {
                    overrides.insert(selected.clone(), String::new());
                }
                capture = false;
                pending = None;
                conflict = None;
                message = tl!("Shortcut removed.").into();
                message_warning = false;
            } else if let Some(sc) = shortcut_text(key, m).filter(|sc| reserved_shortcut(sc)) {
                message = crate::i18n::fmt(tl!("{shortcut} is reserved by the system and can't be assigned."), &[("shortcut", &crate::shortcuts::pretty(&sc))]);
                message_warning = true;
                capture = false;
                pending = None;
                conflict = None;
            } else if let Some(sc) = shortcut_text(key, m) {
                let clash_ids: Vec<&str> = items
                    .iter()
                    .filter(|(id, _, _, def)| {
                        let cur = eff(&overrides, id, def);
                        *id != selected
                            && (cur.as_deref().and_then(prefs::normalize_shortcut) == Some(sc.clone())
                                || eff_alts(&alt_overrides, id, cur.as_deref()).contains(&sc))
                    })
                    .map(|(id, ..)| id.as_str())
                    .collect();
                // The menu path as Photoshop writes it here: " > " between the parts, no trailing "…".
                let clash: Vec<String> = items
                    .iter()
                    .filter(|(id, ..)| clash_ids.contains(&id.as_str()))
                    .map(|(id, label, path, _)| {
                        let (path, label) = shown(id, label, path);
                        path.iter().map(String::as_str).chain([label.trim_end_matches('…')]).collect::<Vec<_>>().join(" > ")
                    })
                    .collect();
                // What Undo Changes puts back: the command's main key and further keys as they were.
                conflict = clash_ids.first().map(|first| {
                    json!({
                        "shortcut": sc,
                        "with": first,
                        // Every command holding the key: Accept takes it from all of them.
                        "holders": clash_ids,
                        "command": selected,
                        "main": overrides.get(&selected),
                        "alternates": alt_overrides.get(&selected),
                    })
                });
                let shown_key = crate::shortcuts::pretty(&sc);
                message = if !clash.is_empty() {
                    let template = "{shortcut} is already in use and will be removed from {commands} if accepted.";
                    crate::i18n::fmt(tl!(template), &[("shortcut", &shown_key), ("commands", &clash.join(", "))])
                } else if function_key_with_modifiers(key, m) {
                    let template = "{shortcut} can be assigned to actions which will override this command.";
                    crate::i18n::fmt(tl!(template), &[("shortcut", &shown_key)])
                } else {
                    String::new()
                };
                message_warning = !message.is_empty();
                // The key only fills the field; it joins the set when the field is committed.
                pending = Some(sc);
            }
        }
    }
    if commit {
        if let Some(sc) = pending.take() {
            assign(&mut overrides, &mut alt_overrides, &selected, &sc, capture_alt);
            commit_message(&mut message, &mut message_warning, conflict.as_mut(), &sc);
        }
        capture = false;
    }
    let needle = filter.trim();
    egui::ScrollArea::vertical().max_height(420.0).id_salt("shortcuts-scroll").show(ui, |ui| {
        let mut last_top = String::new();
        egui::Grid::new("shortcut-grid").num_columns(3).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
            for (id, label, path, def) in &items {
                if tab == 1 && crate::tool_shortcuts::tool(id).is_some() {
                    continue;
                }
                let cur = eff(&overrides, id, def);
                let alts = eff_alts(&alt_overrides, id, cur.as_deref());
                let (shown_path, shown_label) = shown(id, label, path);
                // Match what is shown and the English name (commands are documented in English).
                let local = format!("{} {}", shown_path.join(" "), shown_label);
                let english = format!("{} {} {} {} {id}", path.join(" "), label, cur.clone().unwrap_or_default(), alts.join(" "));
                if !needle.is_empty()
                    && !crate::i18n::current().contains_ignore_case(&local, needle)
                    && !crate::i18n::Lang::EN.contains_ignore_case(&english, needle)
                {
                    continue;
                }
                if tab == 0 && path.is_empty() && def.is_none() && cur.is_none() && alts.is_empty() {
                    continue;
                }
                let top = shown_path.first().cloned().unwrap_or_else(|| tl!("Other").into());
                if top != last_top {
                    ui.label(RichText::new(&top).font(crate::theme::semibold(12.5)).color(t.text));
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                    last_top = top;
                }
                let name = match shown_path.get(1..) {
                    Some(rest) if !rest.is_empty() => format!("{} › {}", rest.join(" › "), shown_label),
                    _ => shown_label,
                };
                let sel = selected == *id;
                let name_row = ui.selectable_label(sel, RichText::new(format!("   {name}")).color(t.text_dim));
                if name_row.clicked() {
                    selected = id.clone();
                }
                if tab == 0 {
                    let field_open = sel && capture && !capture_alt;
                    let text = match (field_open, pending.as_deref().or(cur.as_deref())) {
                        // The open field shows the key pressed into it, else the key it had.
                        (true, Some(k)) => crate::shortcuts::pretty(k),
                        (true, None) => tl!("Press keys…").to_string(),
                        (false, _) => cur.as_deref().map(crate::shortcuts::pretty).unwrap_or_default(),
                    };
                    let changed = overrides.contains_key(id) || alt_overrides.contains_key(id);
                    let r = ui.add(
                        egui::Button::new(RichText::new(tl!(&text)).size(12.0).color(if changed { t.accent_text } else { t.text }))
                            .min_size(vec2(120.0, 18.0))
                            .selected(field_open),
                    );
                    // An unassigned shortcut shows no text: name it by the command it binds.
                    let r = if text.is_empty() { r.labelled_by(name_row.id) } else { r };
                    if r.clicked() {
                        reopened = *id != selected;
                        selected = id.clone();
                        capture = true;
                        capture_alt = false;
                        message.clear();
                        message_warning = false;
                    }
                    // The further keys (F3 beside ⌘C), each removable on its own, as Photoshop
                    // lists them as rows of their own (#3665).
                    ui.horizontal(|ui| {
                        for k in &alts {
                            let b = egui::Button::new(RichText::new(format!("{}  ×", crate::shortcuts::pretty(k))).size(11.0).color(t.text)).small();
                            if ui.add(b).on_hover_text(tl!("Delete Shortcut")).clicked() {
                                alt_overrides.insert(id.clone(), alts.iter().filter(|x| *x != k).cloned().collect());
                                selected = id.clone();
                                message = tl!("Shortcut removed.").into();
                                message_warning = false;
                            }
                        }
                        if sel && capture && capture_alt {
                            let text = pending.as_deref().map_or_else(|| tl!("Press keys…").to_string(), crate::shortcuts::pretty);
                            ui.label(RichText::new(text).size(11.0).color(t.accent_text));
                        }
                        if changed {
                            ui.label(RichText::new(tl!("modified")).color(t.text_faint).size(10.5));
                        }
                    });
                } else {
                    // Edit › Menus, Keyboard Shortcuts, Preferences, Save and Exit always stay (#5096).
                    let hideable = photocraft_engine::prefs::MenuCustomization::hideable(id);
                    let mut visible = !hideable || !hidden.contains(id);
                    ui.add_enabled_ui(hideable, |ui| crate::widgets::checkbox(ui, &mut visible, tl!("Visible")));
                    if visible {
                        hidden.retain(|h| h != id);
                    } else if !hidden.contains(id) {
                        hidden.push(id.clone());
                    }
                    let mut col = colors.get(id).cloned().unwrap_or_else(|| "none".into());
                    let opts: Vec<(String, &str)> =
                        ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"].iter().map(|c| (c.to_string(), *c)).collect();
                    crate::widgets::dropdown(ui, &format!("menu-color-{id}"), tl!("Color"), &mut col, &opts, 90.0);
                    if col == "none" {
                        colors.remove(id);
                    } else {
                        colors.insert(id.clone(), col);
                    }
                }
                ui.end_row();
            }
        });
    });
    ui.add_space(6.0);
    if tab == 0 {
        ui.horizontal(|ui| {
            let has_sel = !selected.is_empty();
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Add Shortcut"))).clicked() {
                capture = true;
                capture_alt = true;
                message.clear();
                message_warning = false;
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Use Default"))).clicked() {
                overrides.remove(&selected);
                alt_overrides.remove(&selected);
                message = tl!("Default restored.").into();
                message_warning = false;
            }
            if ui.add_enabled(has_sel, egui::Button::new(tl!("Delete Shortcut"))).clicked() {
                overrides.insert(selected.clone(), String::new());
                message = tl!("Shortcut removed.").into();
                message_warning = false;
            }
            if ui.button(tl!("Reset All to Defaults")).clicked() {
                overrides.clear();
                alt_overrides.clear();
                message = tl!("All shortcuts reset to PhotoCraft defaults.").into();
                message_warning = false;
            }
            // A Photoshop set (`.kys`) fills the overrides; the file arrives after this frame
            // and `kys_import` writes the dialog's fields then (#1163).
            if ui.button(tl!("Import Shortcuts…")).on_hover_text(tl!("Load a keyboard shortcut set saved as .kys")).clicked()
                && let Err(e) = crate::kys_import::import_dialog(app)
            {
                message = e;
                message_warning = false;
            }
        });
    } else if ui.button(tl!("Show All Menu Items")).clicked() {
        hidden.clear();
        colors.clear();
    }
    // Moving to another row (or from the main key to Add Shortcut) commits the open field
    // where it was opened; a click on another row's key opens that row's field instead.
    if let Some((row, alt)) = editing.as_ref().filter(|(row, alt)| *row != selected || *alt != capture_alt) {
        if let Some(sc) = pending.take() {
            assign(&mut overrides, &mut alt_overrides, row, &sc, *alt);
            commit_message(&mut message, &mut message_warning, conflict.as_mut(), &sc);
        }
        if *row != selected {
            capture = reopened;
        }
    }
    if !message.is_empty() {
        ui.horizontal(|ui| {
            if message_warning {
                // Photoshop's yellow warning mark before the text.
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 13.0), egui::Sense::hover());
                crate::color_picker_ui::warning_triangle(ui.painter(), r, t.warning, t.card);
            }
            ui.label(RichText::new(&message).color(if message_warning { t.warning } else { t.text_dim }));
        });
    }
    // The conflict describes the bindings as the capture left them: any later change (Use
    // Default, Delete, ×, Reset All, another command or set) makes it stale, and it goes.
    let state = conflict_state(&overrides, &alt_overrides, &selected, f.get("activeSet"));
    if conflict.as_ref().and_then(|c| c.get("state")).is_some_and(|s| s.as_u64() != Some(state)) {
        conflict = None;
    }
    if tab == 0 {
        ui.horizontal(|ui| {
            // Accept: enabled while a field is open; commits it like Enter.
            if ui.add_enabled(capture, egui::Button::new(tl!("Accept"))).on_hover_text(tl!("Enter the selected shortcut into the set")).clicked() {
                if let Some(sc) = pending.take() {
                    assign(&mut overrides, &mut alt_overrides, &selected, &sc, capture_alt);
                    commit_message(&mut message, &mut message_warning, conflict.as_mut(), &sc);
                }
                capture = false;
            }
            // An unused function key with modifiers: Undo Changes drops the key from the field.
            if conflict.is_none()
                && pending.is_some()
                && message_warning
                && ui.button(tl!("Undo Changes")).on_hover_text(tl!("Put this command's shortcuts back as they were")).clicked()
            {
                pending = None;
                capture = false;
                message = tl!("Changes undone.").into();
                message_warning = false;
            }
            let Some(c) = conflict.clone() else { return };
            let (key, with, command) =
                (c["shortcut"].as_str().unwrap_or_default(), c["with"].as_str().unwrap_or_default(), c["command"].as_str().unwrap_or_default());
            if ui.button(tl!("Accept and Go to Conflict")).on_hover_text(tl!("Take the shortcut from the other command and select it")).clicked() {
                // A key still in the field joins the set first.
                if let Some(sc) = pending.take() {
                    assign(&mut overrides, &mut alt_overrides, &selected, &sc, capture_alt);
                }
                capture = false;
                let holders: Vec<String> = serde_json::from_value(c["holders"].clone()).unwrap_or_else(|_| vec![with.to_string()]);
                for holder in &holders {
                    let def = items.iter().find(|i| i.0 == *holder).and_then(|i| i.3.clone());
                    let cur = eff(&overrides, holder, &def);
                    if cur.as_deref().and_then(prefs::normalize_shortcut).as_deref() == Some(key) {
                        overrides.insert(holder.clone(), String::new());
                    }
                    let keys = eff_alts(&alt_overrides, holder, eff(&overrides, holder, &def).as_deref());
                    if keys.iter().any(|k| k == key) {
                        alt_overrides.insert(holder.clone(), keys.into_iter().filter(|k| k != key).collect());
                    }
                }
                selected = with.to_string();
                filter.clear();
                message = tl!("Choose a new shortcut for this command, or leave it without one.").into();
                message_warning = false;
                conflict = None;
            }
            if ui.button(tl!("Undo Changes")).on_hover_text(tl!("Put this command's shortcuts back as they were")).clicked() {
                pending = None;
                capture = false;
                match c["main"].as_str() {
                    Some(m) => overrides.insert(command.to_string(), m.to_string()),
                    None => overrides.remove(command),
                };
                match serde_json::from_value::<Vec<String>>(c["alternates"].clone()) {
                    Ok(list) => alt_overrides.insert(command.to_string(), list),
                    Err(_) => alt_overrides.remove(command),
                };
                selected = command.to_string();
                message = tl!("Changes undone.").into();
                message_warning = false;
                conflict = None;
            }
        });
        if !capture {
            // While nothing is being edited, Photoshop's instructions sit below the list.
            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(tl!(INSTRUCTIONS)).color(t.text_dim));
            });
        }
    }
    if overrides_ok {
        f.insert("overrides".into(), json!(overrides));
    }
    if alternates_ok {
        f.insert("alternates".into(), json!(alt_overrides));
    }
    f.insert("captureAlt".into(), json!(capture && capture_alt));
    // A conflict just found is stamped with the bindings it describes.
    if let Some(c) = conflict.as_mut().filter(|c| c.get("state").is_none()) {
        c["state"] = json!(conflict_state(&overrides, &alt_overrides, &selected, f.get("activeSet")));
    }
    f.insert("conflict".into(), conflict.unwrap_or(Value::Null));
    f.insert("hidden".into(), json!(hidden));
    f.insert("colors".into(), json!(colors));
    f.insert("selected".into(), json!(selected));
    f.insert("capture".into(), json!(capture));
    if capture {
        f.insert("captureMods".into(), capture_mods_json(capture_mods));
    } else {
        f.remove("captureMods");
    }
    match pending.filter(|_| capture) {
        Some(sc) => f.insert(PENDING.into(), json!(sc)),
        None => f.remove(PENDING),
    };
    f.insert("message".into(), json!(message));
    f.insert("messageWarning".into(), json!(message_warning));
}

/// A command's keys as the dialog stands, main key first: the edited main key (or its default)
/// and its further keys.
fn dialog_keys(overrides: &BTreeMap<String, String>, alternates: &BTreeMap<String, Vec<String>>, id: &str, default: Option<&str>) -> Vec<String> {
    let main = match overrides.get(id) {
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s.as_str()),
        None => default,
    };
    let mut p = prefs::Preferences::default();
    if let Some(list) = alternates.get(id) {
        p.alternate_shortcuts.insert(id.to_string(), list.clone());
    }
    let mut keys: Vec<String> = main.and_then(prefs::normalize_shortcut).into_iter().collect();
    keys.extend(p.alternates(id, main));
    keys
}

/// Edit › Keyboard Shortcuts › Set, as in Photoshop: choose a saved set (its bindings replace
/// the dialog's), save the bindings as a new set or over the one in use, delete it, export it as
/// a `.kys` file or summarize it as an HTML page. Everything but Export and Summarize waits for
/// OK, like the rest of the dialog.
#[allow(clippy::too_many_arguments)]
fn set_row(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    f: &mut Map<String, Value>,
    overrides: &mut BTreeMap<String, String>,
    alternates: &mut BTreeMap<String, Vec<String>>,
    items: &[(String, String, Vec<String>, Option<String>)],
    message: &mut String,
    warning: &mut bool,
) {
    use photocraft_engine::shortcut_sets::{self, ShortcutSet};
    let mut sets: BTreeMap<String, ShortcutSet> = f.get("sets").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let mut active = f.get("activeSet").and_then(Value::as_str).unwrap_or("").to_string();
    if !active.is_empty() && !sets.contains_key(&active) {
        active.clear();
    }
    // The engine decides what a set holds and when it is modified (not the actions' F-keys).
    let now = ShortcutSet::from_bindings(overrides, alternates);
    let modified = shortcut_sets::differs(&sets, &active, overrides, alternates);
    let defaults = tl!("PhotoCraft Defaults").to_string();
    let shown = |name: &str| -> String {
        let base = if name.is_empty() { defaults.clone() } else { name.to_string() };
        if name == active && modified { crate::i18n::fmt(tl!("{set} (modified)"), &[("set", &base)]) } else { base }
    };
    let names: Vec<(String, String)> = std::iter::once(String::new()).chain(sets.keys().cloned()).map(|n| (n.clone(), shown(&n))).collect();
    let mut new_name: Option<String> = f.get("newSetName").and_then(Value::as_str).map(str::to_string);
    let set_label = if active.is_empty() { defaults.clone() } else { active.clone() };
    let mut export = false;
    let mut summarize = false;
    ui.horizontal(|ui| {
        ui.label(tl!("Set:"));
        let mut chosen = active.clone();
        let pairs: Vec<(String, &str)> = names.iter().map(|(n, l)| (n.clone(), l.as_str())).collect();
        if crate::widgets::dropdown(ui, "shortcut-set", tl!("Set:"), &mut chosen, &pairs, 220.0) && chosen != active {
            // As `useSet`: the set's bindings, keeping the actions' F-keys.
            sets.get(&chosen).cloned().unwrap_or_default().apply(overrides, alternates);
            active = chosen;
            message.clear();
        }
        if ui.button(tl!("New Set…")).on_hover_text(tl!("Save these shortcuts as a new set")).clicked() {
            new_name = Some(String::new());
        }
        if ui.add_enabled(!active.is_empty() && modified, egui::Button::new(tl!("Save"))).on_hover_text(tl!("Save the changes to this set")).clicked() {
            sets.insert(active.clone(), now.clone());
            *message = tl!("Set saved.").into();
            *warning = false;
        }
        if ui.add_enabled(!active.is_empty(), egui::Button::new(tl!("Delete"))).on_hover_text(tl!("Delete this set")).clicked() {
            sets.remove(&active);
            active.clear();
            *message = tl!("Set deleted.").into();
            *warning = false;
        }
        export = ui.button(tl!("Export…")).on_hover_text(tl!("Save these shortcuts as a .kys file")).clicked();
        summarize = ui.button(tl!("Summarize…")).on_hover_text(tl!("Save a list of these shortcuts as a web page")).clicked();
    });
    if let Some(name) = new_name.as_mut() {
        let mut create = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label(tl!("Name:"));
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(220.0).hint_text(tl!("set name")));
            create = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            create |= ui.button(tl!("Create")).clicked();
            cancel = ui.button(tl!("Cancel")).clicked();
        });
        if create {
            match shortcut_sets::set_name(name) {
                Ok(n) if !sets.contains_key(&n) && sets.len() >= shortcut_sets::MAX_SETS => {
                    *message = tl!("There are too many sets: delete one first.").into();
                    *warning = true;
                }
                Ok(n) => {
                    sets.insert(n.clone(), now.clone());
                    active = n;
                    new_name = None;
                    *message = tl!("Set saved.").into();
                    *warning = false;
                }
                Err(e) => {
                    *message = e;
                    *warning = true;
                }
            }
        } else if cancel {
            new_name = None;
        }
    }
    if export || summarize {
        let result = if export {
            let entries: Vec<photocraft_engine::kys::Entry> = items
                .iter()
                .map(|(id, label, _, def)| photocraft_engine::kys::Entry {
                    id: id.clone(),
                    label: label.clone(),
                    tool: id.starts_with("tools.select."),
                    keys: dialog_keys(overrides, alternates, id, def.as_deref()),
                })
                .collect();
            let text = photocraft_engine::kys::write(&set_label, &entries);
            save_text(app, &format!("{set_label}.kys"), text)
        } else {
            let mut sections: Vec<shortcut_sets::Section> = Vec::new();
            for (id, label, path, def) in items {
                let section = path.first().cloned().unwrap_or_else(|| tl!("Other").to_string());
                let name = path.iter().skip(1).chain(std::iter::once(label)).cloned().collect::<Vec<_>>().join(" › ");
                let row = (name, dialog_keys(overrides, alternates, id, def.as_deref()));
                match sections.iter_mut().find(|(s, _)| *s == section) {
                    Some((_, rows)) => rows.push(row),
                    None => sections.push((section, vec![row])),
                }
            }
            let html = shortcut_sets::summarize(&set_label, &sections, cfg!(target_os = "macos"));
            save_text(app, &format!("{set_label}.html"), html)
        };
        if let Err(e) = result {
            *message = e;
            *warning = true;
        }
    }
    f.insert("sets".into(), json!(sets));
    f.insert("activeSet".into(), json!(active));
    f.insert("newSetName".into(), new_name.map_or(Value::Null, Value::String));
}

/// Save `text` through the save dialog under the suggested file name.
fn save_text(app: &mut PhotocraftApp, suggested: &str, text: String) -> Result<Value, String> {
    // A set name can hold characters a file name can't.
    let name: String = suggested.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    app.pick_save(&name, move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, text.as_bytes())?;
        Ok(json!({"path": path}))
    })
}

#[cfg(test)]
#[path = "shortcut_pending_edit_tests.rs"]
mod shortcut_pending_edit_tests;
