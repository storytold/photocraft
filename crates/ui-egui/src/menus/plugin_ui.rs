//! Filter › Plug-ins: one menu item per installed WebAssembly filter plug-in (`plugin.*` engine
//! commands), with a generated dialog and live preview for plug-ins that take parameters.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use craft::menu::Row;

/// Menu ids of installed plug-ins: `plugin.filter.<plug-in id>`.
const PREFIX: &str = "plugin.filter.";

/// Held by tests that install a plug-in and by tests that compare two builds of the menus. The
/// plug-in registry is shared by the whole test process, so an install between two builds adds a
/// Filter › Plug-ins row to the second one only.
#[cfg(test)]
pub(crate) fn registry_guard() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The mutex [`registry_guard`] holds.
#[cfg(test)]
pub(crate) static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Inserts the installed plug-ins into Filter › Plug-ins, above "Install Plug-in…".
pub fn rows(app: &PhotocraftApp, rows: &mut Vec<Row>) {
    let plugins = photocraft_plugins::registry::list();
    let Some(at) = rows.iter().position(|r| r.command.id == "plugin.install") else { return };
    // `Row::path` is the untranslated English path from the command's engine spec: the bar groups
    // rows by it and translates only titles afterwards. Translating it here left the plug-in rows
    // under a path no menu node matched, so under any non-English language they were never drawn
    // (#2540). Borrow the path of "Install Plug-in…", the item we sit above.
    let path: Vec<String> = rows[at].path.clone();
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let enabled = app.session.is_enabled("plugin.run");
    let mut extra: Vec<Row> = plugins
        .iter()
        .map(|p| {
            let m = p.manifest();
            let dots = if m.params.is_empty() || m.name.ends_with('…') { "" } else { "…" };
            Row::new(&path, format!("{PREFIX}{}", m.id).as_str(), format!("{}{dots}", m.name)).enabled(enabled)
        })
        .collect();
    if !extra.is_empty() {
        extra.push(Row::separator(&path));
    }
    for (k, it) in extra.into_iter().enumerate() {
        rows.insert(at + k, it);
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
        return Some(crate::filter_dialog::open(app, "plugin.install").map(|d| json!({"dialog": d.value().to_string()})));
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
    Some(Ok(json!({"dialog": d.value().to_string()})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_plugins_appear_under_filter_plugins_and_run() {
        let _registry = registry_guard();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
        app.run("plugin.install", json!({"path": path})).unwrap();
        let rows = crate::menus::menu_rows(&app);
        let pi: Vec<&Row> = rows.iter().filter(|r| r.path == ["Filter", "Plug-ins"]).collect();
        let at = |id: &str| pi.iter().position(|r| r.command.id == id);
        let me = pi.iter().find(|r| r.command.id == "plugin.filter.org.photocraft.example.invert").expect("plug-in listed");
        assert_eq!(me.label, "Invert (WebAssembly)");
        assert!(!me.enabled, "no document yet");
        assert!(at("plugin.filter.org.photocraft.example.invert") < at("plugin.install"), "plug-ins sit above Install Plug-in…");
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        assert!(crate::menus::is_enabled(&app, "plugin.filter.org.photocraft.example.invert"));
        let px = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
        let before = px(&app);
        crate::menus::invoke(&mut app, &ctx, "plugin.filter.org.photocraft.example.invert", json!({})).unwrap();
        assert!((px(&app)[0] - (1.0 - before[0])).abs() < 1e-6);
        assert!(crate::menus::invoke(&mut app, &ctx, "plugin.filter.not.installed", json!({})).is_err());
        // Install Plug-in… without params opens its dialog instead of failing.
        let r = crate::menus::invoke(&mut app, &ctx, "plugin.install", json!({})).unwrap();
        assert!(r.get("dialog").is_some());
    }

    /// Plug-in rows carry the untranslated English menu path like every other item, so they still
    /// land under Filter › Plug-ins when the UI language translates those menu names (#2540).
    #[test]
    fn plugin_menu_path_stays_untranslated_in_other_languages() {
        if !crate::i18n::needs_catalogs() {
            return;
        }
        let _registry = registry_guard();
        let ja = crate::i18n::Lang::from_code("ja").expect("registered language");
        let _lang = crate::i18n::language_scope(ja);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/invert.wasm");
        app.run("plugin.install", json!({"path": path})).unwrap();
        let rows = crate::menus::menu_rows(&app);
        let me = crate::menus::row(&rows, "plugin.filter.org.photocraft.example.invert").expect("plug-in listed");
        assert_eq!(me.path, ["Filter", "Plug-ins"], "plug-in row sits where the menu renderer looks for it");
        let install = crate::menus::row(&rows, "plugin.install").expect("Install Plug-in… listed");
        assert_eq!(me.path, install.path, "same submenu as Install Plug-in…");
    }
}
