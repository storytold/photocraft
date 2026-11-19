//! The File-menu forms that run a command from a few values: Contact Sheet II, Create Droplet, Script Events
//! Manager, Package and Export Paths to File. Each is a hosted command form whose key is the command id; OK runs
//! the command through the `filter_dialog` family runner.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;
use serde_json::Value;

use crate::PhotocraftApp;
use crate::dialogs::params;
use crate::menus::view_cmds::dialogs::label_of;

const CONTACT_SHEET: &str = "file.automate.contactSheetII";
const CREATE_DROPLET: &str = "file.automate.createDroplet";
const SCRIPT_EVENTS: &str = "file.scripts.scriptEventsManager";
const PACKAGE: &str = "file.package";
const PATHS_TO_FILE: &str = "file.export.pathsToIllustrator";

/// Whether `key` is the command id of one of these forms.
pub(crate) fn owns(key: &str) -> bool {
    matches!(key, CONTACT_SHEET | CREATE_DROPLET | SCRIPT_EVENTS | PACKAGE | PATHS_TO_FILE)
}

/// The folder of the active document, else the working directory.
pub(crate) fn default_dir(app: &PhotocraftApp) -> String {
    app.session.active().and_then(|d| d.path.as_deref()).and_then(|p| p.rfind(['/', '\\']).map(|i| p[..i].to_string())).unwrap_or_else(|| ".".into())
}

/// A packaged macOS app does not start in a useful writable working directory. A saved
/// document keeps its own folder; otherwise prefer the user's Documents folder.
pub(crate) fn droplet_default_dir(app: &PhotocraftApp) -> String {
    let dir = default_dir(app);
    if dir != "." {
        return dir;
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let documents = home.join("Documents");
        return if documents.is_dir() { documents } else { home }.to_string_lossy().into_owned();
    }
    dir
}

/// The active document's name without its extension.
pub(crate) fn doc_stem(app: &PhotocraftApp) -> String {
    app.session.active().map(|d| d.doc.name.rsplit_once('.').map_or(d.doc.name.clone(), |(a, _)| a.to_string())).unwrap_or_else(|| tl!("Untitled").into())
}

/// Choice options whose labels are the ids read as words.
fn ids(ids: &[&str]) -> Vec<(String, Label)> {
    ids.iter().map(|id| ((*id).to_owned(), Label::from(tl!(&label_of(id)).to_string()))).collect()
}

/// Choice options that are names (actions, sets, paths), shown as they are.
fn names(names: &[String]) -> Vec<(String, Label)> {
    names.iter().map(|name| (name.clone(), Label::from(name.clone()))).collect()
}

fn open_form(app: &mut PhotocraftApp, key: &'static str, title: &str, rows: Vec<Param>) -> DialogId {
    let title = Label::from(title.to_string());
    params::open_param(app, key, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        ParamDialog::command(key, title.clone(), rows.clone()).values(Bind::bound(fields))
    })
}

/// File › Automate › Contact Sheet II.
pub(crate) fn open_contact_sheet(app: &mut PhotocraftApp) -> DialogId {
    let rows = vec![
        Param::new("input", tl!("Input"), ParamKind::text(default_dir(app))).browse(PathKind::Folder),
        // Source Images: the folder, then Include Subfolders (on) and, right below it, Group Images by
        // Folder (off, greyed while Include Subfolders is off).
        Param::new("includeSubfolders", tl!("Include Subfolders"), ParamKind::bool(true)),
        Param::new("groupByFolder", tl!("Group Images by Folder"), ParamKind::bool(false))
            .enabled_when("includeSubfolders", true)
            .tooltip(tl!("Images in subfolders will start on a new Contact Sheet")),
        Param::new("units", tl!("Units"), ParamKind::choice(ids(&["inches", "cm", "pixels"]), Some("inches".into()))),
        Param::new("width", tl!("Width"), ParamKind::number(Some(0.0), None, 8.0, 0.5, 2)),
        Param::new("height", tl!("Height"), ParamKind::number(Some(0.0), None, 10.0, 0.5, 2)),
        Param::new("resolution", tl!("Resolution"), ParamKind::number(Some(1.0), None, 300.0, 1.0, 0)),
        Param::new("mode", tl!("Mode"), ParamKind::choice(ids(&["rgb", "gray", "cmyk", "lab"]), Some("rgb".into()))),
        Param::new("depth", tl!("Depth"), ParamKind::int(Some(1), None, 8)),
        Param::new("columns", tl!("Columns"), ParamKind::int(Some(1), None, 5)),
        Param::new("rows", tl!("Rows"), ParamKind::int(Some(1), None, 6)),
        Param::new("placeAcrossFirst", tl!("Place across first"), ParamKind::bool(true)),
        Param::new("autoSpacing", tl!("Auto spacing"), ParamKind::bool(true)),
        Param::new("rotateForBestFit", tl!("Rotate for best fit"), ParamKind::bool(false)),
        Param::new("caption", tl!("Caption"), ParamKind::bool(true)),
        Param::new("font", tl!("Font"), ParamKind::text(photocraft_text::fonts::DEFAULT_FAMILY)),
        Param::new("fontSize", tl!("Font size"), ParamKind::number(Some(1.0), None, 12.0, 1.0, 1)),
        Param::new("flatten", tl!("Flatten"), ParamKind::bool(false)),
    ];
    open_form(app, CONTACT_SHEET, tl!("Contact Sheet II"), rows)
}

/// File › Automate › Create Droplet, on the Actions panel's selected action. Refused without one.
pub(crate) fn open_create_droplet(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let dir = droplet_default_dir(app);
    let Some(action) = crate::actions::selected_action(app) else {
        return Err("record an action in the Actions panel first".into());
    };
    let (name, set) = (action.name.clone(), action.set.clone());
    let (action_names, set_names) = crate::actions::action_form_choices(app);
    let mut rows = vec![
        Param::new("path", tl!("Path"), ParamKind::text(format!("{dir}/{name}.pcdroplet"))),
        Param::new("name", tl!("Name"), ParamKind::text(name.clone())),
    ];
    // The Set choice picks between same-named actions; an action outside any set has none.
    if let Some(set) = set {
        rows.push(Param::new("set", tl!("Set"), ParamKind::choice(names(&set_names), Some(set))));
    }
    rows.push(Param::new("action", tl!("Action"), ParamKind::choice(names(&action_names), Some(name))));
    rows.push(Param::new("output", tl!("Output"), ParamKind::text(format!("{dir}/droplet-output"))).browse(PathKind::Folder));
    rows.push(Param::new("format", tl!("Format"), ParamKind::choice(ids(&["same", "png", "jpg", "psd", "tiff"]), Some("same".into()))));
    Ok(open_form(app, CREATE_DROPLET, tl!("Create Droplet"), rows))
}

/// File › Scripts › Script Events Manager.
pub(crate) fn open_script_events(app: &mut PhotocraftApp) -> DialogId {
    let enabled = app.session.prefs().script_events.enabled;
    let events: Vec<&str> = photocraft_engine::automate_cmds::EVENTS.iter().map(|e| e.0).collect();
    let rows = vec![
        Param::new("enabled", tl!("Enabled"), ParamKind::bool(enabled)),
        Param::new("event", tl!("Event"), ParamKind::choice(ids(&events), Some("openDocument".into()))),
        Param::new("script", tl!("Script"), ParamKind::text("")),
        Param::new("name", tl!("Name"), ParamKind::text("")),
    ];
    open_form(app, SCRIPT_EVENTS, tl!("Script Events Manager"), rows)
}

/// File › Package.
pub(crate) fn open_package(app: &mut PhotocraftApp) -> DialogId {
    let rows = vec![Param::new("dir", tl!("Dir"), ParamKind::text(default_dir(app))).browse(PathKind::Folder)];
    open_form(app, PACKAGE, tl!("Package"), rows)
}

/// File › Export › Paths to Illustrator.
pub(crate) fn open_paths_to_file(app: &mut PhotocraftApp) -> DialogId {
    let rows = paths_to_file_rows(app);
    open_form(app, PATHS_TO_FILE, tl!("Export Paths to File"), rows)
}

/// The Paths to Illustrator form's rows: the file path and the Paths choice.
pub(crate) fn paths_to_file_rows(app: &PhotocraftApp) -> Vec<Param> {
    let (dir, stem) = (default_dir(app), doc_stem(app));
    // Document Bounds comes first; a document without paths offers only it, selected (#13771).
    let mut options = ids(&["documentBounds"]);
    let mut selectable = vec!["documentBounds".to_string()];
    if let Some(d) = app.session.active().filter(|d| d.doc.work_path.is_some() || !d.doc.paths.is_empty()) {
        options.extend(ids(&["all"]));
        selectable.push("all".into());
        if d.doc.work_path.is_some() {
            options.extend(ids(&["work"]));
            selectable.push("work".into());
        }
        options.extend(names(&d.doc.paths.iter().map(|p| p.name.clone()).collect::<Vec<_>>()));
        selectable.extend(d.doc.paths.iter().map(|p| p.name.clone()));
    }
    let fallback = if selectable.len() > 1 { "all" } else { "documentBounds" };
    // Opens on the Paths panel's selected row (#13777). A shape layer's path ("layer") is not an entry
    // yet, and a row the document no longer has falls back to All Paths (Document Bounds without paths).
    let selected = app.ui.selected_path.as_deref().filter(|s| *s != "layer" && selectable.iter().any(|n| n == s)).unwrap_or(fallback).to_string();
    // The choice is not remembered between openings (#13777).
    vec![
        Param::new("path", tl!("Path"), ParamKind::text(format!("{dir}/{stem}.ai"))),
        Param::new("paths", tl!("Paths"), ParamKind::choice(options, Some(selected))).not_remembered(),
    ]
}

#[cfg(test)]
mod tests;
