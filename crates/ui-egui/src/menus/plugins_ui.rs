//! Filter › G'MIC and the About Plug-ins window (Filter › Plug-ins › About Plug-ins…,
//! Help › About Plug-ins…).
//!
//! Filter › G'MIC lists the G'MIC filters by category (`gmic.*` engine commands; G'MIC is a
//! separate program found on this computer) and shows only once G'MIC is found. The WebAssembly
//! plug-ins stay in Filter › Plug-ins (`plugin_ui`). About Plug-ins explains both kinds of
//! plug-in, shows what is installed and whether G'MIC was found, and says how to get G'MIC here.

use photocraft_engine::gmic_cmds::{self, CatalogState};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;

/// Menu ids of G'MIC filters: `gmic.filter.<category/name>`.
const GMIC_PREFIX: &str = "gmic.filter.";
/// G'MIC's download page.
pub const GMIC_DOWNLOAD: &str = "https://gmic.eu/download.html";
/// The plug-in documentation (ABI, examples) and the G'MIC page.
pub const PLUGINS_DOC: &str = "https://github.com/storytold/photocraft/blob/main/docs/plugins.md";
pub const GMIC_DOC: &str = "https://github.com/storytold/photocraft/blob/main/docs/gmic.md";

/// Shell commands this module handles (also listed in `menus::UI_COMMANDS`).
const IDS: &[&str] = &["plugins.about", "help.aboutPlugins", "gmic.refreshFilters", "gmic.get", "gmic.locate", "gmic.checkAgain"];

pub fn handles(id: &str) -> bool {
    IDS.contains(&id) || id.starts_with(GMIC_PREFIX)
}

fn item(id: impl Into<String>, label: impl Into<String>, path: &[&str], enabled: bool) -> MenuItem {
    MenuItem { id: id.into(), label: label.into(), path: path.iter().map(|s| s.to_string()).collect(), shortcut: None, enabled, checked: None, color: None }
}

/// Whether Filter › G'MIC shows: once G'MIC is found on this computer (until then, About
/// Plug-ins says how to get it). With `Services::detect_gmic` this also starts looking for G'MIC
/// and loading its filters in the background.
pub fn gmic_visible(app: &PhotocraftApp) -> bool {
    if app.services.detect_gmic {
        gmic_cmds::detect_in_background();
    }
    matches!(gmic_cmds::found(), Some(Ok(_)))
}

/// Places About Plug-ins… at the end of Filter › Plug-ins and builds Filter › G'MIC after it
/// (replacing the `UI_COMMANDS` / engine-spec rows, which would land at the end of Filter).
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    let g: &[&str] = &["Filter", "G'MIC"];
    let pi: &[&str] = &["Filter", "Plug-ins"];
    items.retain(|i| !i.path.starts_with(&["Filter".to_string(), "G'MIC".to_string()]) && i.id != "plugins.about");
    let mut extra = vec![item("---", "---", pi, false), item("plugins.about", "About Plug-ins…", pi, true)];
    if gmic_visible(app) {
        let filter_ok = app.session.is_enabled(gmic_cmds::FILTER);
        match gmic_cmds::catalog_state() {
            CatalogState::Loaded(cat) => {
                for f in &cat.filters {
                    let dots = if f.params.is_empty() { "" } else { "…" };
                    extra.push(MenuItem {
                        id: format!("{GMIC_PREFIX}{}", f.id),
                        label: format!("{}{dots}", f.name),
                        path: vec!["Filter".into(), "G'MIC".into(), f.category.clone()],
                        shortcut: None,
                        enabled: filter_ok,
                        checked: None,
                        color: None,
                    });
                }
                if !cat.filters.is_empty() {
                    extra.push(item("---", "---", g, false));
                }
            }
            CatalogState::Loading | CatalogState::Idle => extra.push(item("gmic.loading", tl!("Loading Filters…"), g, false)),
            CatalogState::Failed(_) => extra.push(item("gmic.unavailable", tl!("Filter List Unavailable"), g, false)),
        }
        extra.push(item(gmic_cmds::RUN, "G'MIC Command…", g, app.session.is_enabled(gmic_cmds::RUN)));
        extra.push(item("gmic.refreshFilters", "Refresh Filters", g, true));
        extra.push(item("gmic.get", "About G'MIC…", g, true));
    }
    // Right after Filter › Plug-ins (its last row), else at the end of Filter.
    let at = items
        .iter()
        .rposition(|i| i.path == pi)
        .or_else(|| items.iter().rposition(|i| i.path.first().map(String::as_str) == Some("Filter")))
        .map_or(items.len(), |p| p + 1);
    for (k, it) in extra.into_iter().enumerate() {
        items.insert(at + k, it);
    }
}

/// Enabled state of a G'MIC / About Plug-ins item (`None` for other ids).
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if let Some(fid) = id.strip_prefix(GMIC_PREFIX) {
        return Some(gmic_cmds::loaded_catalog().is_some_and(|c| c.get(fid).is_some()) && app.session.is_enabled(gmic_cmds::FILTER));
    }
    IDS.contains(&id).then_some(true)
}

/// Opens About Plug-ins.
pub fn open_about(app: &mut PhotocraftApp) -> u64 {
    let mut fields = Map::new();
    fields.insert("plugins".into(), json!(true));
    app.ui.open_dialog(crate::state::DialogKind::About, fields)
}

/// Whether an About dialog's fields are About Plug-ins.
pub fn is_about(fields: &Map<String, Value>) -> bool {
    fields.get("plugins").and_then(Value::as_bool) == Some(true)
}

/// Runs a G'MIC / About Plug-ins item (`None` for other ids).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let given = params.as_object().is_some_and(|o| !o.is_empty());
    if let Some(fid) = id.strip_prefix(GMIC_PREFIX) {
        let Some(cat) = gmic_cmds::loaded_catalog() else { return Some(Err("the G'MIC filter list isn't loaded yet".into())) };
        let Some(f) = cat.get(fid) else { return Some(Err(format!("no G'MIC filter {fid:?}"))) };
        let visible = f.notation() != "{}";
        if !visible || given {
            let mut p = params.as_object().cloned().unwrap_or_default();
            p.insert("id".into(), json!(fid));
            return Some(app.run(gmic_cmds::FILTER, Value::Object(p)));
        }
        let mut fixed = Map::new();
        fixed.insert("id".into(), json!(fid));
        // Text defaults the notation can't carry.
        for (k, v) in f.defaults() {
            if v.is_string() && !v.as_str().is_some_and(|s| s.starts_with('#')) {
                fixed.insert(k, v);
            }
        }
        let d = crate::filter_dialog::open_with_spec(app, gmic_cmds::FILTER, &f.name, &f.notation(), fixed);
        return Some(Ok(json!({"dialog": d})));
    }
    Some(match id {
        gmic_cmds::RUN if !given => Ok(json!({"dialog": crate::filter_dialog::open(app, gmic_cmds::RUN)})),
        "plugins.about" | "help.aboutPlugins" | "gmic.get" => Ok(json!({"dialog": open_about(app)})),
        "gmic.refreshFilters" => {
            let state = gmic_cmds::request_catalog(true);
            Ok(json!({"loading": matches!(state, CatalogState::Loading)}))
        }
        "gmic.checkAgain" => {
            let s = gmic_cmds::status(true);
            gmic_cmds::detect_in_background();
            Ok(s)
        }
        "gmic.locate" => locate(app),
        // G'MIC Command… with params, and anything else: not ours.
        _ => return None,
    })
}

/// Locate G'MIC…: pick the `gmic` program; it becomes Preferences › Plug-ins › G'MIC executable.
fn locate(app: &mut PhotocraftApp) -> Result<Value, String> {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: None }, |app, answer| {
        let FileDialogAnswer::Paths(paths) = answer else { return Err("G'MIC needs the desktop app".into()) };
        let path = paths.into_iter().next().ok_or("no file chosen")?;
        app.run("prefs.set", json!({"path": "plugIns.gmicPath", "value": path}))?;
        let s = gmic_cmds::status(true);
        gmic_cmds::detect_in_background();
        Ok(s)
    })
}

/// How to install G'MIC here: an explanation and, where there is one, the command to run.
pub fn install_hint() -> (&'static str, Option<&'static str>) {
    if cfg!(target_arch = "wasm32") {
        return ("G'MIC needs the desktop app: the web version can't start other programs.", None);
    }
    if cfg!(target_os = "macos") {
        return ("Install it with Homebrew, then choose Check Again:", Some("brew install gmic"));
    }
    if cfg!(target_os = "windows") {
        return ("Download the command-line version (gmic-…-cli-win64.zip) from gmic.eu, unzip it, then choose Locate G'MIC… and pick gmic.exe.", None);
    }
    match linux_package_command() {
        Some(cmd) => ("Install the gmic package, then choose Check Again:", Some(cmd)),
        None => ("Install the gmic package with your package manager, or download it from gmic.eu, then choose Check Again.", None),
    }
}

/// The package-manager command for this Linux distribution (from `/etc/os-release`).
fn linux_package_command() -> Option<&'static str> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static CMD: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
        *CMD.get_or_init(|| std::fs::read_to_string("/etc/os-release").ok().and_then(|t| package_command(&t)))
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// The install command for an `os-release` file's `ID` / `ID_LIKE`.
#[cfg(not(target_arch = "wasm32"))]
fn package_command(os_release: &str) -> Option<&'static str> {
    let mut ids = Vec::new();
    for line in os_release.lines() {
        if let Some(v) = line.strip_prefix("ID=").or_else(|| line.strip_prefix("ID_LIKE=")) {
            ids.extend(v.trim().trim_matches('"').split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let is = |names: &[&str]| ids.iter().any(|i| names.contains(&i.as_str()));
    if is(&["arch", "manjaro", "endeavouros", "cachyos"]) {
        Some("sudo pacman -S gmic")
    } else if is(&["debian", "ubuntu", "linuxmint", "pop"]) {
        Some("sudo apt install gmic")
    } else if is(&["fedora", "rhel", "centos"]) {
        Some("sudo dnf install gmic")
    } else if is(&["opensuse", "suse", "opensuse-tumbleweed", "opensuse-leap"]) {
        Some("sudo zypper install gmic")
    } else {
        None
    }
}

/// What a button in About Plug-ins asked for (run after drawing).
enum Action {
    Run(&'static str),
    Open(&'static str),
    Copy(&'static str),
}

/// The About Plug-ins window body.
pub fn about_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut action: Option<Action> = None;
    let wrap = |ui: &mut egui::Ui, text: &str| {
        ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_dim)).wrap());
    };
    // WebAssembly plug-ins.
    crate::widgets::section_label(ui, tl!("Plug-ins"));
    wrap(ui, tl!("PhotoCraft plug-ins are sandboxed WebAssembly filters: one .wasm file works on macOS, Windows, Linux and the web. A plug-in only sees the pixels it filters; it can't read files or use the network."));
    ui.add_space(4.0);
    let plugins = photocraft_plugins::registry::list();
    if plugins.is_empty() {
        ui.weak(tl!("No plug-ins are installed."));
    }
    for p in &plugins {
        let m = p.manifest();
        let by = if m.author.is_empty() { String::new() } else { format!(" · {}", m.author) };
        ui.label(format!("{} {}{by}", m.name, m.version));
    }
    ui.add_space(4.0);
    wrap(ui, tl!("Add one with Install Plug-in…, or put .wasm files in the folder set in Preferences › Plug-ins. To write your own, start from the Rust example in examples/plugins/invert-rs."));
    ui.horizontal_wrapped(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Install Plug-in…"), 0.0).clicked() {
            action = Some(Action::Run("plugin.install"));
        }
        if crate::widgets::secondary_button(ui, tl!("Plug-in Preferences…"), 0.0).clicked() {
            action = Some(Action::Run("edit.preferences.plugIns"));
        }
        if ui.link(egui::RichText::new(tl!("Writing plug-ins")).color(t.accent)).on_hover_text(PLUGINS_DOC).clicked() {
            action = Some(Action::Open(PLUGINS_DOC));
        }
    });
    ui.add_space(10.0);
    // G'MIC.
    crate::widgets::section_label(ui, "G'MIC");
    wrap(ui, tl!("G'MIC is a separate, free image-processing program with hundreds of filters. PhotoCraft runs the copy installed on this computer; it isn't included with PhotoCraft."));
    ui.add_space(4.0);
    let found = gmic_cmds::found();
    match &found {
        Some(Ok(g)) => {
            ui.label(crate::i18n::fmt(tl!("G'MIC {version} found: {path}"), &[("version", g.version()), ("path", &g.path().display().to_string())]));
            match gmic_cmds::catalog_state() {
                CatalogState::Loaded(c) => {
                    ui.weak(crate::i18n::fmt(tl!("{count} filters in Filter › G'MIC"), &[("count", &c.filters.len().to_string())]));
                }
                CatalogState::Loading | CatalogState::Idle => {
                    ui.weak(tl!("Loading the filter list… (the first time takes about a minute)"));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                }
                CatalogState::Failed(e) => {
                    ui.colored_label(t.danger, crate::i18n::fmt(tl!("The filter list couldn't be loaded: {error}"), &[("error", &e)]));
                }
            }
        }
        Some(Err(e)) => {
            ui.label(tl!("G'MIC was not found on this computer."));
            ui.add(egui::Label::new(egui::RichText::new(e.as_str()).color(t.text_faint).small()).wrap());
        }
        None if app.services.detect_gmic => {
            gmic_cmds::detect_in_background();
            ui.weak(tl!("Looking for G'MIC…"));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        None => {
            ui.weak(tl!("Choose Check Again to look for G'MIC."));
        }
    }
    if !matches!(found, Some(Ok(_))) {
        ui.add_space(4.0);
        let (hint, command) = install_hint();
        wrap(ui, tl!(hint));
        if let Some(cmd) = command {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(cmd).font(crate::theme::mono(12.0))).selectable(true));
                if crate::widgets::secondary_button(ui, tl!("Copy"), 0.0).clicked() {
                    action = Some(Action::Copy(cmd));
                }
            });
        }
    }
    ui.horizontal_wrapped(|ui| {
        if !matches!(found, Some(Ok(_))) && crate::widgets::primary_button(ui, tl!("Download G'MIC…"), 0.0).on_hover_text(GMIC_DOWNLOAD).clicked() {
            action = Some(Action::Open(GMIC_DOWNLOAD));
        }
        if !cfg!(target_arch = "wasm32") {
            if crate::widgets::secondary_button(ui, tl!("Locate G'MIC…"), 0.0).clicked() {
                action = Some(Action::Run("gmic.locate"));
            }
            if crate::widgets::secondary_button(ui, tl!("Check Again"), 0.0).clicked() {
                action = Some(Action::Run("gmic.checkAgain"));
            }
        }
        if ui.link(egui::RichText::new(tl!("Using G'MIC")).color(t.accent)).on_hover_text(GMIC_DOC).clicked() {
            action = Some(Action::Open(GMIC_DOC));
        }
    });
    let ctx = ui.ctx().clone();
    match action {
        Some(Action::Run(id)) => {
            if let Err(e) = crate::menus::invoke(app, &ctx, id, json!({}))
                && e != crate::file_dialog::CANCELLED
            {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        Some(Action::Open(url)) => {
            crate::links::open(app, &ctx, url);
        }
        Some(Action::Copy(text)) => ctx.copy_text(text.to_string()),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_commands_follow_os_release() {
        assert_eq!(package_command("NAME=\"CachyOS\"\nID=cachyos\nID_LIKE=arch\n"), Some("sudo pacman -S gmic"));
        assert_eq!(package_command("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), Some("sudo apt install gmic"));
        assert_eq!(package_command("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n"), Some("sudo dnf install gmic"));
        assert_eq!(package_command("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n"), Some("sudo zypper install gmic"));
        assert_eq!(package_command("ID=gentoo\n"), None);
        assert_eq!(package_command(""), None);
    }

    #[test]
    fn gmic_lives_under_filter_and_about_plugins_opens_from_filter_and_help() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert!(!crate::menus::TOP_MENUS.contains(&"Plugins"), "no top-level Plugins menu");
        let items = crate::menus::menu_items(&app);
        // About Plug-ins… closes Filter › Plug-ins (after Install Plug-in…) and is in Help.
        let pi: Vec<&str> = items.iter().filter(|i| i.path == ["Filter", "Plug-ins"]).map(|i| i.id.as_str()).collect();
        assert_eq!(pi.last(), Some(&"plugins.about"), "{pi:?}");
        assert!(pi.contains(&"plugin.install"));
        assert!(items.iter().any(|i| i.id == "help.aboutPlugins" && i.path == ["Help"]));
        // Without a G'MIC found (tests never look), there is no Filter › G'MIC.
        assert!(!items.iter().any(|i| i.path.starts_with(&["Filter".to_string(), "G'MIC".to_string()])));
        for id in ["plugins.about", "help.aboutPlugins", "gmic.get"] {
            let r = crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
            let d = app.ui.dialogs.iter().find(|d| Some(d.id) == r["dialog"].as_u64()).expect("dialog");
            assert!(is_about(&d.fields));
            assert_eq!(crate::dialogs::title(d), "About Plug-ins");
        }
        // G'MIC Command… is greyed without a document and opens its dialog with one.
        assert!(!crate::menus::is_enabled(&app, "gmic.run"));
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(crate::menus::is_enabled(&app, "gmic.run"));
        let r = crate::menus::invoke(&mut app, &ctx, "gmic.run", json!({})).unwrap();
        assert!(r.get("dialog").is_some());
    }
}
