//! Filter › Plug-ins: one menu item per installed WebAssembly filter plug-in (`plugin.*` engine
//! commands), with a generated dialog and live preview for plug-ins that take parameters.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;

/// Menu ids of installed plug-ins: `plugin.filter.<plug-in id>`.
const PREFIX: &str = "plugin.filter.";
#[cfg(not(target_arch = "wasm32"))]
const NATIVE_PREFIX: &str = "plugin.8bf.filter.";

/// Inserts the installed plug-ins into Filter › Plug-ins, above "Install Plug-in…".
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        let enabled = app.session.is_enabled("plugin.8bf.run");
        let descriptors: Vec<_> = photocraft_plugins::native_8bf::list().iter().map(|plugin| plugin.descriptor.clone()).collect();
        extra.extend(native_labels(&descriptors).into_iter().map(|(id, label)| MenuItem {
            id: format!("{NATIVE_PREFIX}{id}"),
            label,
            path: path.clone(),
            shortcut: None,
            enabled,
            checked: None,
            color: None,
        }));
    }
    if !extra.is_empty() {
        extra.push(MenuItem { id: "---".into(), label: "---".into(), path: path.clone(), shortcut: None, enabled: false, checked: None, color: None });
    }
    for (k, it) in extra.into_iter().enumerate() {
        items.insert(at + k, it);
    }
}

/// `(id, label)` per native filter, sorted by name. Folder scans often find a filter's x86 and
/// x64 builds under one file name: those labels name the architecture, and same-architecture
/// copies also name their folder, so every item stays distinguishable.
#[cfg(not(target_arch = "wasm32"))]
fn native_labels(plugins: &[photocraft_plugins::native_8bf::Descriptor]) -> Vec<(String, String)> {
    use photocraft_plugins::native_8bf::pe::Architecture;
    let arch = |a: Architecture| match a {
        Architecture::X86 => "x86",
        Architecture::X64 => "x64",
    };
    let folder = |p: &photocraft_plugins::native_8bf::Descriptor| p.path.parent().and_then(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned());
    let mut sorted: Vec<_> = plugins.iter().collect();
    let order = |a: Architecture| a == Architecture::X64;
    sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(order(a.architecture).cmp(&order(b.architecture))).then(a.path.cmp(&b.path)));
    sorted
        .iter()
        .map(|p| {
            let same_name = plugins.iter().filter(|q| q.name == p.name).count();
            let same_arch = plugins.iter().filter(|q| q.name == p.name && q.architecture == p.architecture).count();
            let label = match (same_name, same_arch, folder(p)) {
                (1, _, _) => format!("{} (8BF)…", p.name),
                (_, 1, _) | (_, _, None) => format!("{} (8BF {})…", p.name, arch(p.architecture)),
                (_, _, Some(folder)) => format!("{} (8BF {}, {folder})…", p.name, arch(p.architecture)),
            };
            (p.id.clone(), label)
        })
        .collect()
}

/// Enabled state of a plug-in menu item (`None` for other ids).
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(pid) = id.strip_prefix(NATIVE_PREFIX) {
        return Some(photocraft_plugins::native_8bf::get(pid).is_some() && app.session.is_enabled("plugin.8bf.run"));
    }
    let pid = id.strip_prefix(PREFIX)?;
    Some(photocraft_plugins::registry::get(pid).is_some() && app.session.is_enabled("plugin.run"))
}

/// Runs a plug-in menu item: straight away when it has no parameters (or params were given),
/// else through its dialog. "Install Plug-in…" without params opens its path dialog.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let given = params.as_object().is_some_and(|o| !o.is_empty());
    #[cfg(not(target_arch = "wasm32"))]
    {
        if id == "plugin.8bf.install" && !given {
            return Some(Ok(json!({"dialog":crate::filter_dialog::open(app,"plugin.8bf.install")})));
        }
        if let Some(pid) = id.strip_prefix(NATIVE_PREFIX) {
            let mut p = params.as_object().cloned().unwrap_or_default();
            p.insert("id".into(), json!(pid));
            return Some(app.run("plugin.8bf.run", Value::Object(p)));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn same_named_native_filters_get_distinct_labels() {
        use photocraft_plugins::native_8bf::{Descriptor, pe::Architecture};
        let filter = |path: &str, name: &str, architecture| Descriptor {
            id: format!("8bf:{path}"),
            name: name.into(),
            path: path.into(),
            architecture,
            entry: "PluginMain".into(),
        };
        let labels = native_labels(&[
            filter("/p/64/Zombie.8bf", "Zombie", Architecture::X64),
            filter("/p/32/Zombie.8bf", "Zombie", Architecture::X86),
            filter("/p/a/Ink.8bf", "Ink", Architecture::X86),
            filter("/p/b/Ink.8bf", "Ink", Architecture::X86),
            filter("/p/Aether.8bf", "Aether", Architecture::X64),
        ]);
        let labels: Vec<&str> = labels.iter().map(|(_, label)| label.as_str()).collect();
        assert_eq!(labels, ["Aether (8BF)…", "Ink (8BF x86, a)…", "Ink (8BF x86, b)…", "Zombie (8BF x86)…", "Zombie (8BF x64)…"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_install_menu_opens_path_dialog_without_executing_library() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let items = crate::menus::menu_items(&app);
        assert!(items.iter().any(|item| item.id == "plugin.8bf.install" && item.path == ["Filter", "Plug-ins"]));
        let result = crate::menus::invoke(&mut app, &ctx, "plugin.8bf.install", json!({})).unwrap();
        assert!(result.get("dialog").is_some());
        assert!(app.ui.dialogs.iter().any(|dialog| dialog.fields.get("__command").and_then(Value::as_str) == Some("plugin.8bf.install")));
    }

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
        assert!(r.get("dialog").is_some());
    }

    /// Plug-in rows carry the untranslated English menu path like every other item, so they still
    /// land under Filter › Plug-ins when the UI language translates those menu names (#2540).
    #[test]
    fn plugin_menu_path_stays_untranslated_in_other_languages() {
        let ja = crate::i18n::Lang::from_code("ja").expect("registered language");
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
