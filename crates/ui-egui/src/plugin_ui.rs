//! Filter › Plug-ins: one menu item per installed WebAssembly filter plug-in (`plugin.*` engine
//! commands), with a generated dialog and live preview for plug-ins that take parameters.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;

/// Menu ids of installed plug-ins: `plugin.filter.<plug-in id>`.
const PREFIX: &str = "plugin.filter.";

/// Inserts the installed plug-ins into Filter › Plug-ins, above "Install Plug-in…".
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = app.session.poll_plugin_folder();
    let plugins = photocraft_plugins::registry::list();
    let Some(at) = items.iter().position(|i| i.id == "plugin.install") else { return };
    // `MenuItem::path` is the untranslated English path from the command's engine spec: the menu
    // renderer groups rows by it and translates only at display time. Translating it here left the
    // plug-in rows under a path no menu node matched, so under any non-English language they were
    // never drawn (#2540). Borrow the path of "Install Plug-in…", the item we sit above.
    let Some(path) = items.get(at).map(|i| i.path.clone()) else { return };
    let enabled = app.session.is_enabled("plugin.run");
    let mut extra: Vec<MenuItem> = plugins
        .iter()
        .map(|p| {
            let m = p.manifest();
            let dots = if m.params.is_empty() || m.name.ends_with('…') { "" } else { "…" };
            MenuItem {
                id: format!("{PREFIX}{}", m.id),
                label: format!("{}{dots}", m.name),
                path: path.clone(),
                shortcut: None,
                enabled,
                checked: None,
                color: None,
            }
        })
        .collect();
    if !extra.is_empty() {
        extra.push(MenuItem { id: "---".into(), label: "---".into(), path: path.clone(), shortcut: None, enabled: false, checked: None, color: None });
    }
    for (k, it) in extra.into_iter().enumerate() {
        items.insert(at + k, it);
    }
}

/// Enabled state of a plug-in menu item (`None` for other ids).
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let pid = id.strip_prefix(PREFIX)?;
    Some(photocraft_plugins::registry::get(pid).is_some() && app.session.is_enabled("plugin.run"))
}

/// Runs a plug-in menu item: straight away when it has no parameters (or params were given),
/// else through its dialog. "Install Plug-in…" without params opens its path dialog.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let given = params.as_object().is_some_and(|o| !o.is_empty());
    if id == "plugin.install" && !given {
        return Some(Ok(json!({"dialog": crate::filter_dialog::open(app, "plugin.install")})));
    }
    let pid = id.strip_prefix(PREFIX)?;
    let Some(plugin) = photocraft_plugins::registry::get(pid) else { return Some(Err(format!("no plug-in {pid:?} is installed"))) };
    let m = plugin.manifest();
    if m.params.is_empty() || given {
        let mut p = params.as_object().cloned().unwrap_or_default();
        p.insert("id".into(), json!(pid));
        return Some(app.run("plugin.run", Value::Object(p)));
    }
    let mut fixed = Map::new();
    fixed.insert("id".into(), json!(pid));
    let d = crate::filter_dialog::open_with_spec(app, "plugin.run", &m.name, &m.params_notation(), fixed);
    Some(Ok(json!({"dialog": d})))
}

/// Whether `fields` belongs to the `Filter › Plug-ins › Install Plug-in…` (`plugin.install`) dialog.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some("plugin.install")
}

/// Whether `fields` belongs to the post-install confirmation modal (`Plug-in Installed`).
pub fn owns_installed_notice(fields: &Map<String, Value>) -> bool {
    fields.get("__plugin_installed").and_then(Value::as_bool) == Some(true)
}

/// `Install Plug-in…` can confirm once a file path or base64 payload has been entered.
pub fn can_confirm(fields: &Map<String, Value>) -> bool {
    if !owns(fields) {
        return true;
    }
    let has_path = fields.get("path").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
    let has_data = fields.get("data").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
    has_path || has_data
}

/// Renders the `Install Plug-in` dialog body with a file path field, a native `Browse…` button
/// (supporting `.wasm`, `.afs`, and `.8bf` plug-ins), and the `Replace` checkbox.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut path = fields.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut browse_clicked = false;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::filter_dialog::label("path")).color(t.text_dim));
        ui.add(egui::TextEdit::singleline(&mut path).desired_width(240.0));
        if crate::widgets::secondary_button(ui, tl!("Browse…"), 92.0).clicked() {
            browse_clicked = true;
        }
    });
    if path.trim().is_empty() {
        fields.remove("path");
    } else {
        fields.insert("path".into(), json!(path));
    }

    ui.add_space(4.0);
    let mut replace = fields.get("replace").and_then(Value::as_bool).unwrap_or(true);
    crate::widgets::checkbox(ui, &mut replace, &crate::filter_dialog::label("replace"));
    fields.insert("replace".into(), json!(replace));

    if browse_clicked {
        let req = crate::FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["wasm".into(), "afs".into(), "8bf".into()]) };
        let _ = app.ask_file(req, |app, answer| {
            if let Some(d) = app.ui.dialogs.iter_mut().rev().find(|d| owns(&d.fields)) {
                match answer {
                    crate::FileDialogAnswer::Paths(paths) => {
                        if let Some(chosen) = paths.into_iter().next() {
                            d.fields.insert("path".into(), json!(chosen));
                            d.fields.remove("data");
                        }
                    }
                    crate::FileDialogAnswer::Contents(name, bytes) => {
                        d.fields.insert("path".into(), json!(name));
                        d.fields.insert("data".into(), json!(b64_encode(&bytes)));
                    }
                    crate::FileDialogAnswer::ContentsMany(files) => {
                        if let Some((name, bytes)) = files.into_iter().next() {
                            d.fields.insert("path".into(), json!(name));
                            d.fields.insert("data".into(), json!(b64_encode(&bytes)));
                        }
                    }
                    crate::FileDialogAnswer::FolderTo(chosen) => {
                        d.fields.insert("path".into(), json!(chosen));
                        d.fields.remove("data");
                    }
                    crate::FileDialogAnswer::SaveTo(_) | crate::FileDialogAnswer::Unavailable(_) => {}
                }
            }
            Ok(Value::Null)
        });
    }
}

/// Renders the confirmation modal shown after a plug-in is imported via `Install Plug-in…`.
pub fn installed_notice_body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let name = fields.get("name").and_then(Value::as_str).unwrap_or("Plug-in");
    let version = fields.get("version").and_then(Value::as_str).unwrap_or("1.0.0");
    let desc = fields.get("description").and_then(Value::as_str).unwrap_or("");

    ui.label(egui::RichText::new(format!("{name} (v{version})")).strong().color(t.text));
    if !desc.trim().is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(desc).color(t.text_dim));
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("The plug-in is now available under Filter › Plug-ins.")).color(t.text_dim));
}

/// Confirms the `Install Plug-in…` dialog, installs the plug-in, refreshes the menu cache, and
/// opens a confirmation modal reporting the installed plug-in.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let mut params = crate::filter_dialog::params_of(fields);
    if let Some(obj) = params.as_object_mut()
        && let Some(data) = fields.get("data").cloned()
    {
        obj.insert("data".into(), data);
    }
    let result = app.run("plugin.install", params)?;
    app.menu_cache = None;

    let mut notice = Map::new();
    notice.insert("__plugin_installed".into(), json!(true));
    notice.insert("__label".into(), json!("Plug-in Installed"));
    if let Some(name) = result.get("name").cloned() {
        notice.insert("name".into(), name);
    }
    if let Some(version) = result.get("version").cloned() {
        notice.insert("version".into(), version);
    }
    if let Some(desc) = result.get("description").cloned() {
        notice.insert("description".into(), desc);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, notice);
    Ok(result)
}

fn b64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((n >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((n >> 12) & 0x3f) as usize]));
        out.push(if chunk.len() > 1 { char::from(TABLE[((n >> 6) & 0x3f) as usize]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[(n & 0x3f) as usize]) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_plugins_appear_under_filter_plugins_and_run() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
        app.run("plugin.install", json!({"path": path})).unwrap();
        let items = crate::menus::menu_items(&app);
        let pi: Vec<&MenuItem> = items.iter().filter(|i| i.path == ["Filter", "Plug-ins"]).collect();
        let me = pi.iter().find(|i| i.id == "plugin.filter.org.photocraft.example.invert").expect("plug-in listed");
        assert_eq!(me.label, "Invert (WebAssembly)");
        assert!(!me.enabled, "no document yet");
        assert!(pi.iter().any(|i| i.id == "plugin.install"), "Install Plug-in… is in the same submenu");
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert!(crate::menus::is_enabled(&app, "plugin.filter.org.photocraft.example.invert"));
        let px = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
        let before = px(&app);
        crate::menus::invoke(&mut app, &ctx, "plugin.filter.org.photocraft.example.invert", json!({})).unwrap();
        assert!((px(&app)[0] - (1.0 - before[0])).abs() < 1e-6);
        assert!(crate::menus::invoke(&mut app, &ctx, "plugin.filter.not.installed", json!({})).is_err());
        // Install Plug-in… without params opens its dialog instead of failing.
        let r = crate::menus::invoke(&mut app, &ctx, "plugin.install", json!({})).unwrap();
        let dlg_id = r.get("dialog").and_then(Value::as_u64).expect("Install Plug-in dialog id");
        app.ui.dialog_mut(dlg_id).unwrap().fields.insert("path".into(), json!(path));
        crate::dialogs::confirm(&mut app, dlg_id).unwrap();
        let notice = app.ui.dialogs.last().expect("Plug-in Installed confirmation modal opened");
        assert!(owns_installed_notice(&notice.fields));
        assert_eq!(notice.fields.get("name").and_then(Value::as_str), Some("Invert (WebAssembly)"));
    }

    /// Plug-in rows carry the untranslated English menu path like every other item, so they still
    /// land under Filter › Plug-ins when the UI language translates those menu names (#2540).
    #[test]
    fn plugin_menu_path_stays_untranslated_in_other_languages() {
        let ja = crate::i18n::Lang::from_code("ja").expect("registered language");
        // The names on the path must really differ from the English, or the test proves nothing.
        assert_ne!(crate::i18n::tr(ja, "Filter"), "Filter");
        assert_ne!(crate::i18n::tr(ja, "Plug-ins"), "Plug-ins");
        let _lang = crate::i18n::language_scope(ja);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
        app.run("plugin.install", json!({"path": path})).unwrap();
        let items = crate::menus::menu_items(&app);
        let me = items.iter().find(|i| i.id == "plugin.filter.org.photocraft.example.invert").expect("plug-in listed");
        assert_eq!(me.path, ["Filter", "Plug-ins"], "plug-in row sits where the menu renderer looks for it");
        let install = items.iter().find(|i| i.id == "plugin.install").expect("Install Plug-in… listed");
        assert_eq!(me.path, install.path, "same submenu as Install Plug-in…");
    }
}
