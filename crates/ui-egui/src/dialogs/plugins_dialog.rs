//! Filter › Plug-ins › Manage Plug-ins…: the installed plug-ins (filters and command plug-ins),
//! installing from a file, a GitHub repository or a catalog, catalogs, updates, and the install
//! confirmation (permissions, kind, source, SHA-256, and whether any catalog in use lists it).
//! Every action dispatches the engine's `plugin.*` commands (`photocraft_engine::plugin`), in
//! [`take`], after the pass that asked for it.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The dialog's key.
const KEY: &str = "photocraft.plugins";

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// What the person typed: a repository and a catalog address.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Typed {
    repo: String,
    catalog: String,
}

/// What a click asked for; [`take`] runs it once.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    /// Pick a `.craftplugin` (or a bare `.wasm` filter) file.
    FromFile,
    /// Review the bundle these `plugin.install` params name.
    Review(Value),
    /// Install the bundle under review.
    Confirm,
    /// Leave the confirmation.
    Back,
    Remove(String),
    AddCatalog(String),
    RemoveCatalog(String),
    Browse,
    Updates,
}

/// Manage Plug-ins.
pub struct PluginsDialog {
    typed: Typed,
    /// `plugin.list`'s answer.
    listing: Value,
    /// The catalogs' packages (`plugin.catalog.list`), once browsed.
    packages: Vec<Value>,
    /// The available updates (`plugin.update`), once checked.
    updates: Vec<Value>,
    /// The bundle under review: the install params that named it and `plugin.install {review}`'s
    /// description of it.
    review: Option<(Value, Value)>,
    status: String,
    action: Option<Action>,
}

/// One line of text.
fn line(ui: &mut egui::Ui, text: &str) {
    Text::new(label(text)).show(ui);
}

/// Where a bundle came from, in words.
fn origin(v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match v.get("type").and_then(Value::as_str) {
        Some("github") => crate::i18n::fmt(tl!("GitHub repository {repo}, release {tag}"), &[("repo", &s("repo")), ("tag", &s("tag"))]),
        Some("catalog") => crate::i18n::fmt(tl!("Catalog {catalog}, package {package}"), &[("catalog", &s("catalog")), ("package", &s("package"))]),
        Some("file") => crate::i18n::fmt(tl!("File {file}"), &[("file", &s("path"))]),
        _ => String::new(),
    }
}

/// The install confirmation: what the plug-in is, what it may do and where it comes from.
fn confirmation(ui: &mut egui::Ui, info: &Value, action: &mut Option<Action>) {
    let s = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    line(ui, &format!("{} {}", s("name"), s("version")));
    if !s("author").is_empty() {
        line(ui, &crate::i18n::fmt(tl!("By {author}"), &[("author", &s("author"))]));
    }
    if !s("description").is_empty() {
        line(ui, &s("description"));
    }
    ui.separator();
    let kind = if s("kind") == "filter" { tl!("Filter: changes the pixels of the active layer") } else { tl!("Command plug-in: runs PhotoCraft commands") };
    line(ui, &crate::i18n::fmt(tl!("Kind: {kind}"), &[("kind", kind)]));
    if s("kind") != "filter" {
        let access = info["permissions"]["access"].as_str().unwrap_or("read");
        line(ui, if access == "write" { tl!("Permissions: read and change the document") } else { tl!("Permissions: read the document only") });
        let commands: Vec<&str> = info["permissions"]["commands"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let which = if commands.is_empty() { tl!("any command plug-ins may use").to_string() } else { commands.join(", ") };
        line(ui, &crate::i18n::fmt(tl!("Commands it may call: {commands}"), &[("commands", &which)]));
        let menu: Vec<&str> = info["commands"].as_array().map(|a| a.iter().filter_map(|c| c["label"].as_str()).collect()).unwrap_or_default();
        line(ui, &crate::i18n::fmt(tl!("Adds: {items}"), &[("items", &menu.join(", "))]));
    }
    line(ui, &crate::i18n::fmt(tl!("Source: {source}"), &[("source", &origin(&info["origin"]))]));
    line(ui, &crate::i18n::fmt(tl!("SHA-256: {hash}"), &[("hash", &s("sha256"))]));
    if !s("license").is_empty() {
        line(ui, &crate::i18n::fmt(tl!("Licence: {license}"), &[("license", &s("license"))]));
    }
    if info["inCatalog"] == json!(false) {
        Notice::new(Tone::Warn, label(tl!("This plug-in is not in any catalog you use. Install it only if you trust where it comes from."))).show(ui);
    }
    if let Some(old) = info["replaces"].as_str() {
        line(ui, &crate::i18n::fmt(tl!("Replaces the installed version {version}."), &[("version", old)]));
    }
    if info["permissionsChanged"] == json!(true) {
        Notice::new(Tone::Warn, label(tl!("This version asks for other permissions than the installed one."))).show(ui);
    }
    if info["stored"] == json!(false) {
        line(ui, tl!("It is installed for this session only."));
    }
    ui.horizontal(|ui| {
        if Button::new(label(tl!("Install"))).show(ui).clicked() {
            *action = Some(Action::Confirm);
        }
        if Button::new(label(tl!("Back"))).show(ui).clicked() {
            *action = Some(Action::Back);
        }
    });
}

/// The installed plug-ins, installing, catalogs and updates.
fn overview(ui: &mut egui::Ui, d: &mut DialogParts<'_>, typed: &mut Typed) {
    let DialogParts { listing, packages, updates, action } = d;
    line(ui, tl!("Installed"));
    let plugins = listing["plugins"].as_array().cloned().unwrap_or_default();
    if plugins.is_empty() {
        line(ui, tl!("No plug-ins are installed."));
    }
    for (i, p) in plugins.iter().enumerate() {
        ui.push_id(("plugin", i), |ui| {
            ui.horizontal(|ui| {
                let kind = if p["kind"] == "command" { tl!("command") } else { tl!("filter") };
                let name = p["name"].as_str().unwrap_or_default();
                let version = p["version"].as_str().unwrap_or_default();
                line(ui, &format!("{name} {version} ({kind})"));
                if Button::new(label(tl!("Remove"))).show(ui).clicked() {
                    **action = Some(Action::Remove(p["id"].as_str().unwrap_or_default().to_string()));
                }
            });
        });
    }
    for problem in listing["problems"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        Notice::new(Tone::Bad, label(problem)).show(ui);
    }
    ui.horizontal(|ui| {
        if Button::new(label(tl!("Install from File…"))).show(ui).clicked() {
            **action = Some(Action::FromFile);
        }
        if Button::new(label(tl!("Check for Updates"))).show(ui).clicked() {
            **action = Some(Action::Updates);
        }
    });
    for (i, u) in updates.iter().enumerate() {
        ui.push_id(("update", i), |ui| {
            ui.horizontal(|ui| {
                let text = crate::i18n::fmt(
                    tl!("{id}: {available} is available ({installed} installed)"),
                    &[
                        ("id", u["id"].as_str().unwrap_or_default()),
                        ("available", u["available"].as_str().unwrap_or_default()),
                        ("installed", u["installed"].as_str().unwrap_or_default()),
                    ],
                );
                line(ui, &text);
                if Button::new(label(tl!("Update…"))).show(ui).clicked() {
                    **action = Some(Action::Review(u["install"].clone()));
                }
            });
        });
    }
    ui.separator();
    line(ui, tl!("Install from a GitHub repository"));
    ui.horizontal(|ui| {
        TextField::new(label(tl!("Repository")), Bind::bound(&mut typed.repo)).placeholder(label("owner/repo")).show(ui);
        if Button::new(label(tl!("Install…"))).show(ui).clicked() && !typed.repo.trim().is_empty() {
            **action = Some(Action::Review(json!({"repo": typed.repo.trim()})));
        }
    });
    ui.separator();
    line(ui, tl!("Catalogs"));
    let catalogs = listing["catalogs"].as_array().cloned().unwrap_or_default();
    for (i, url) in catalogs.iter().filter_map(Value::as_str).enumerate() {
        ui.push_id(("catalog", i), |ui| {
            ui.horizontal(|ui| {
                line(ui, url);
                if Button::new(label(tl!("Remove"))).show(ui).clicked() {
                    **action = Some(Action::RemoveCatalog(url.to_string()));
                }
            });
        });
    }
    ui.horizontal(|ui| {
        TextField::new(label(tl!("Catalog address")), Bind::bound(&mut typed.catalog)).placeholder(label("https://…/index.json")).show(ui);
        if Button::new(label(tl!("Add Catalog"))).show(ui).clicked() && !typed.catalog.trim().is_empty() {
            **action = Some(Action::AddCatalog(typed.catalog.trim().to_string()));
        }
    });
    if !catalogs.is_empty() && Button::new(label(tl!("Browse Catalogs"))).show(ui).clicked() {
        **action = Some(Action::Browse);
    }
    for (i, p) in packages.iter().enumerate() {
        ui.push_id(("package", i), |ui| {
            ui.horizontal(|ui| {
                let name = p["name"].as_str().unwrap_or_default();
                let version = p["version"].as_str().unwrap_or_default();
                let mut text = format!("{name} {version}");
                if let Some(have) = p["installed"].as_str() {
                    text = crate::i18n::fmt(tl!("{plugin} ({version} installed)"), &[("plugin", &text), ("version", have)]);
                }
                line(ui, &text);
                if Button::new(label(tl!("Install…"))).show(ui).clicked() {
                    **action = Some(Action::Review(json!({"catalog": p["catalog"], "id": p["id"]})));
                }
            });
        });
    }
}

/// The dialog's state the overview draws besides what was typed.
struct DialogParts<'a> {
    listing: &'a Value,
    packages: &'a [Value],
    updates: &'a [Value],
    action: &'a mut Option<Action>,
}

impl StackDialog for PluginsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let PluginsDialog { typed, listing, packages, updates, review, status, action } = self;
        let title = if review.is_some() { tl!("Install Plug-in") } else { tl!("Plug-ins") };
        let shown = FormDialog::new(label(title), Bind::bound(typed)).ok_label(label(tl!("Done"))).enter_confirms(false).show(ui, |ui, _form, typed| {
            match review.as_ref() {
                Some((_, info)) => confirmation(ui, info, action),
                None => overview(ui, &mut DialogParts { listing, packages, updates, action }, typed),
            }
            if !status.is_empty() {
                ui.separator();
                line(ui, status);
            }
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"repo": self.typed.repo, "catalog": self.typed.catalog, "review": self.review.as_ref().map(|(_, info)| info), "status": self.status}))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let text = value.as_str().ok_or_else(|| format!("field `{field}` takes text"))?.to_string();
        match field {
            "repo" => self.typed.repo = text,
            "catalog" => self.typed.catalog = text,
            _ => return Err(format!("the Plug-ins dialog has no field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

/// Open Manage Plug-ins (an open one is reused).
pub fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    let listing = app.run("plugin.list", json!({}))?;
    let dialog =
        PluginsDialog { typed: Typed::default(), listing, packages: Vec::new(), updates: Vec::new(), review: None, status: String::new(), action: None };
    let id = match app.framework.dialogs.open_unless_open(dialog) {
        Ok(id) => id,
        Err(open) => open.id(),
    };
    Ok(json!({"dialog": id.value().to_string()}))
}

/// Set the open dialog's status line and refresh its list of plug-ins.
fn settle(app: &mut PhotocraftApp, status: String) {
    let listing = app.run("plugin.list", json!({})).ok();
    if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
        if let Some(listing) = listing {
            d.listing = listing;
        }
        d.status = status;
    }
}

/// Review the bundle `request` names (`plugin.install {…, review: true}`) and show its
/// confirmation, or why it can't be installed.
fn review(app: &mut PhotocraftApp, request: Value) {
    let mut asked = request.clone();
    asked["review"] = json!(true);
    match app.run("plugin.install", asked) {
        Ok(info) => {
            if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                d.review = Some((request, info));
                d.status.clear();
            }
        }
        Err(e) => settle(app, e),
    }
}

/// A picked file: a bundle goes to its confirmation, a bare `.wasm` filter installs as before.
fn picked(app: &mut PhotocraftApp, name: String, bytes: Vec<u8>) -> Result<Value, String> {
    let file = std::path::Path::new(&name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    let data = photocraft_engine::paint::tile::b64_encode(&bytes);
    if photocraft_engine::plugin::is_bundle(&bytes) {
        review(app, json!({"data": data, "name": file}));
        return Ok(Value::Null);
    }
    let r = app.run("plugin.install", json!({"data": data}));
    let status = match &r {
        Ok(v) => crate::i18n::fmt(tl!("Installed {name}."), &[("name", v["name"].as_str().unwrap_or_default())]),
        Err(e) => e.clone(),
    };
    settle(app, status);
    r
}

/// Runs what the open dialog's last click asked for, and drops a closed dialog (the `TAKERS` entry).
pub fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let _ = app.framework.dialogs.take::<PluginsDialog>();
    let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() else { return };
    let Some(action) = d.action.take() else { return };
    let pending = d.review.clone();
    match action {
        Action::FromFile => {
            if let Err(e) = app.pick_file_bytes_filtered(&["craftplugin", "wasm"], picked) {
                settle(app, e);
            }
        }
        Action::Review(request) => review(app, request),
        Action::Back => {
            if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                d.review = None;
            }
        }
        Action::Confirm => {
            let Some((mut request, info)) = pending else { return };
            request["sha256"] = info["sha256"].clone();
            let name = info["name"].as_str().unwrap_or_default().to_string();
            let status = match app.run("plugin.install", request) {
                Ok(_) => {
                    if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                        d.review = None;
                        d.updates.retain(|u| u["id"] != info["id"]);
                    }
                    crate::i18n::fmt(tl!("Installed {name}."), &[("name", &name)])
                }
                Err(e) => e,
            };
            settle(app, status);
        }
        Action::Remove(id) => {
            let status = match app.run("plugin.remove", json!({"id": id})) {
                Ok(_) => crate::i18n::fmt(tl!("Removed {name}."), &[("name", &id)]),
                Err(e) => e,
            };
            settle(app, status);
        }
        Action::AddCatalog(url) => {
            let status = match app.run("plugin.catalog.add", json!({"url": url})) {
                Ok(_) => {
                    if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                        d.typed.catalog.clear();
                    }
                    String::new()
                }
                Err(e) => e,
            };
            settle(app, status);
        }
        Action::RemoveCatalog(url) => {
            let status = app.run("plugin.catalog.add", json!({"url": url, "remove": true})).err().unwrap_or_default();
            settle(app, status);
        }
        Action::Browse => {
            let (packages, status) = match app.run("plugin.catalog.list", json!({})) {
                Ok(r) => {
                    let packages = r["packages"].as_array().cloned().unwrap_or_default();
                    let warnings: Vec<&str> = r["warnings"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    let status = if packages.is_empty() && warnings.is_empty() {
                        tl!("The catalogs list no plug-ins for PhotoCraft.").to_string()
                    } else {
                        warnings.join("\n")
                    };
                    (packages, status)
                }
                Err(e) => (Vec::new(), e),
            };
            if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                d.packages = packages;
            }
            settle(app, status);
        }
        Action::Updates => {
            let (updates, status) = match app.run("plugin.update", json!({})) {
                Ok(r) => {
                    let updates = r["updates"].as_array().cloned().unwrap_or_default();
                    let errors: Vec<String> = r["errors"]
                        .as_array()
                        .map(|a| a.iter().map(|e| format!("{}: {}", e["id"].as_str().unwrap_or_default(), e["error"].as_str().unwrap_or_default())).collect())
                        .unwrap_or_default();
                    let status = if updates.is_empty() && errors.is_empty() { tl!("Every plug-in is up to date.").to_string() } else { errors.join("\n") };
                    (updates, status)
                }
                Err(e) => (Vec::new(), e),
            };
            if let Some(d) = app.framework.dialogs.get_mut::<PluginsDialog>() {
                d.updates = updates;
            }
            settle(app, status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command plug-in that does nothing, for PhotoCraft.
    fn bundle(id: &str) -> Vec<u8> {
        // `(module (memory (export "memory") 1) (func (export "craft_run") (param i32) (result i32) i32.const 0))`
        let mut wasm = b"\0asm\x01\0\0\0\x01\x06\x01\x60\x01\x7f\x01\x7f\x03\x02\x01\0\x05\x03\x01\0\x01\x07\x16\x02".to_vec();
        wasm.extend(b"\x06memory\x02\0\x09craft_run\0\0\x0a\x06\x01\x04\0\x41\0\x0b");
        let manifest = json!({"id": id, "name": "Tidy", "version": "1.0.0", "main": "p.wasm", "apps": {"photocraft": {"api": 6}},
            "permissions": {"access": "write"}, "commands": [{"id": "go", "label": "Go", "params": [{"name": "amount", "type": "slider", "min": 0, "max": 10, "default": 3}]}]});
        photocraft_engine::plugin::craft_plugin::bundle::Bundle { manifest: manifest.to_string(), wasm, assets: vec![] }.encode()
    }

    #[test]
    fn a_picked_bundle_is_confirmed_then_listed_in_the_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        open(&mut app).unwrap();
        picked(&mut app, "/somewhere/tidy.craftplugin".into(), bundle("io.example.tidy")).unwrap();
        let d = app.framework.dialogs.get_mut::<PluginsDialog>().expect("open");
        let (_, info) = d.review.clone().expect("the confirmation shows");
        assert_eq!(info["kind"], "command");
        assert_eq!(info["permissions"]["access"], "write");
        assert_eq!(info["origin"]["path"], "tidy.craftplugin", "only the file's name, not where it is");
        assert!(app.session.plugins.list().is_empty(), "nothing installed before Install");
        app.framework.dialogs.get_mut::<PluginsDialog>().unwrap().action = Some(Action::Confirm);
        take(&mut app, &ctx);
        assert!(app.session.plugins.command("io.example.tidy").is_some());
        let rows = crate::menus::menu_rows(&app);
        let row = crate::menus::row(&rows, "plugin.command.io.example.tidy/go").expect("listed under Filter › Plug-ins");
        assert_eq!(row.label, "Tidy…", "it has parameters: a dialog asks for them");
        assert!(crate::menus::row(&rows, crate::plugin_ui::MANAGE).is_some());
        // Given parameters, it runs straight away.
        crate::menus::invoke(&mut app, &ctx, "plugin.command.io.example.tidy/go", json!({"amount": 2})).unwrap();
        assert_eq!(app.session.plugins.last.as_ref().map(|(_, _, p)| p["amount"].clone()), Some(json!(2.0)));
    }

    #[test]
    fn the_generated_dialog_notation_covers_the_manifest_kinds() {
        let m = photocraft_engine::plugin::craft_plugin::Manifest::parse(
            &json!({"id": "a.b", "name": "N", "main": "p.wasm", "apps": {"photocraft": {"api": 6}}, "commands": [{"id": "go", "params": [
                {"name": "title", "type": "text"},
                {"name": "size", "type": "number", "min": 1, "max": 50, "default": 4},
                {"name": "on", "type": "boolean", "default": true},
                {"name": "mode", "type": "select", "options": ["soft", "hard"], "default": "hard"},
                {"name": "tint", "type": "color", "default": "#FF0000"}
            ]}]})
            .to_string(),
        )
        .unwrap();
        let spec = crate::plugin_ui::notation(&m.commands[0].params);
        assert_eq!(
            spec,
            r##"{"title":text,"size":1..50=4,"on":bool=true,"mode":"soft|hard"=hard,"tint":color=#ff0000}"##
                .replace("#ff0000", m.commands[0].params[4].default_value().as_str().unwrap())
        );
        let rows = craft::ui::patterns::form_dialogs::Param::parse_list(&spec);
        assert_eq!(rows.len(), 5);
    }
}
