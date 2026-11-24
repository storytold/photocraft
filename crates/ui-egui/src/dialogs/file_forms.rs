//! The File-menu forms that run a command from a few values: Contact Sheet II, Create Droplet, Script Events
//! Manager, Package and Export Paths to File. Each is a hosted command form whose key is the command id; OK runs
//! the command through the `filter_dialog` family runner.

use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::PathKind;
use serde_json::{Value, json};

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
    // The Play group (#13709): four checkboxes under the Set and Action choices, all off. The droplet saves them and
    // hands them to each run.
    rows.extend([
        Param::new("overrideOpen", tl!("Override Action \"Open\" Commands"), ParamKind::bool(false)),
        Param::new("includeSubfolders", tl!("Include All Subfolders"), ParamKind::bool(false)),
        Param::new("suppressOpenOptions", tl!("Suppress File Open Options Dialogs"), ParamKind::bool(false)),
        Param::new("suppressWarnings", tl!("Suppress Color Profile Warnings"), ParamKind::bool(false)),
    ]);
    rows.push(Param::new("output", tl!("Output"), ParamKind::text(format!("{dir}/droplet-output"))).browse(PathKind::Folder));
    rows.push(Param::new("format", tl!("Format"), ParamKind::choice(ids(&["same", "png", "jpg", "psd", "tiff"]), Some("same".into()))));
    // File Naming rows, the starting serial and the Compatibility checkboxes (#13712). The
    // reference greys them unless Destination is Folder; that dropdown is #13711's (#15562, open),
    // and this form has no destination state, so the rows stay visible like the output row.
    rows.push(naming_rows_param());
    rows.push(Param::new("startingSerial", tl!("Starting Serial #"), ParamKind::int(Some(0), Some(9999), 1)));
    rows.push(compatibility_param());
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

/// The File Naming rows of the Create Droplet form (#13712): six rows of text field plus rule
/// dropdown over the `namingComponents` the batch engine already names with. The Batch dialog's
/// own rows are #13661; until that lands this widget stays with this form.
///
/// The dropdown's `None` entry and an empty field store nothing: a droplet with no naming keeps
/// each input's name, as before this change.
const NAMING_NONE: &str = "none";

/// The rule dropdown's entries in the reference's order, 19 with `None` last: each is the engine
/// component id Batch names with (docs/atn.md) and the entry's label.
const NAMING_RULES: &[(&str, &str)] = &[
    ("name", "Document Name"),
    ("nameLower", "document name"),
    ("nameUpper", "DOCUMENT NAME"),
    ("serial1", "1 Digit Serial Number"),
    ("serial2", "2 Digit Serial Number"),
    ("serial3", "3 Digit Serial Number"),
    ("serial4", "4 Digit Serial Number"),
    ("letter", "Serial Letter (a, b, c...)"),
    ("LETTER", "Serial Letter (A, B, C...)"),
    ("mmddyy", "mmddyy (date)"),
    ("mmdd", "mmdd (date)"),
    ("yyyymmdd", "yyyymmdd (date)"),
    ("yymmdd", "yymmdd (date)"),
    ("yyddmm", "yyddmm (date)"),
    ("ddmmyy", "ddmmyy (date)"),
    ("ddmm", "ddmm (date)"),
    ("extension", "extension"),
    ("EXTENSION", "EXTENSION"),
    (NAMING_NONE, "None"),
];

/// One shown row: the chosen rule id (`NAMING_NONE` for a literal-text row) and the field text.
#[derive(Clone, PartialEq)]
struct NamingRow {
    rule: String,
    text: String,
}

/// The stored value of `key` as JSON, as a custom row and automation keep it.
fn field_json(fields: &DialogFields, key: &str) -> Option<Value> {
    fields.get(key).and_then(|value| serde_json::to_value(value).ok())
}

/// The rule a stored component names, or `None` for a literal text row and for a component this
/// form does not offer: it reads back as an empty `None` row and a later store drops it, as it
/// does the components past the six shown rows.
fn component_rule(part: &Value) -> Option<&'static str> {
    let rule = part.get("component").and_then(Value::as_str)?;
    NAMING_RULES.iter().find(|(id, _)| *id == rule).map(|(id, _)| *id)
}

/// The label of a rule, empty for `None`.
fn rule_label(rule: &str) -> &'static str {
    NAMING_RULES.iter().find(|(id, _)| *id == rule).map_or("", |(_, label)| *label)
}

/// The six shown rows the fields hold: a component row shows its entry's label in the field, a
/// text row shows its text under `None`, and rows past the stored components are empty `None`s.
fn naming_rows(fields: &DialogFields) -> Vec<NamingRow> {
    let stored = field_json(fields, "namingComponents").filter(Value::is_array).unwrap_or_else(|| Value::Array(Vec::new()));
    let parts: &[Value] = stored.as_array().map_or(&[], Vec::as_slice);
    let mut rows: Vec<NamingRow> = parts
        .iter()
        .take(6)
        .map(|part| match component_rule(part) {
            Some(rule) => NamingRow { rule: rule.to_string(), text: rule_label(rule).to_string() },
            None => NamingRow { rule: NAMING_NONE.to_string(), text: part.get("text").and_then(Value::as_str).unwrap_or_default().to_string() },
        })
        .collect();
    rows.resize(6, NamingRow { rule: NAMING_NONE.to_string(), text: String::new() });
    rows
}

/// The components the rows stand for, in row order, empty `None` rows left out; `None` when every
/// row is `None`, which stores no naming and keeps each input's name as before (#13712).
fn naming_components(rows: &[NamingRow]) -> Option<Vec<Value>> {
    let parts: Vec<Value> = rows
        .iter()
        .filter_map(|row| match row.rule.as_str() {
            NAMING_NONE if row.text.is_empty() => None,
            NAMING_NONE => Some(json!({"text": row.text.as_str()})),
            rule => Some(json!({"component": rule})),
        })
        .collect();
    (!parts.is_empty()).then_some(parts)
}

/// The Example line: the name the fields' rows and starting serial give the sample document
/// `MyFile.gif`, planned by the engine's own batch naming; empty when there is nothing to show.
fn naming_example(fields: &DialogFields) -> String {
    let Some(components) = naming_components(&naming_rows(fields)) else { return String::new() };
    let params = json!({"namingComponents": components, "startingSerial": fields.int("startingSerial").unwrap_or(1)});
    let inputs = [photocraft_engine::file_cmds::BatchInput { path: "MyFile.gif".into(), rel_dir: String::new() }];
    photocraft_engine::file_cmds::plan_batch_names(&inputs, "", "same", &params)
        .ok()
        .flatten()
        .and_then(|names| names.into_iter().next())
        .map(|path| path.rsplit(['/', '\\']).next().unwrap_or(path.as_str()).to_owned())
        .unwrap_or_default()
}

/// The File Naming custom row: the Example line, then six rows of text field plus rule dropdown
/// with the reference's static "+" right of rows 1-5. Picking an entry writes its label into the
/// field; typing turns the row into literal text and jumps its dropdown to `None`.
fn naming_rows_param() -> Param {
    Param::custom("namingComponents", Label::from(tl!("File Naming")), |ui, fields, _cx| {
        let before = naming_rows(fields);
        let mut rows = before.clone();
        let mut response = ui
            .horizontal(|ui| {
                ui.label(tl!("Example:"));
                ui.label(naming_example(fields));
            })
            .response;
        let entries: Vec<Entry<String>> = NAMING_RULES.iter().map(|(id, label)| Entry::new((*id).to_string(), Label::from(tl!(label)))).collect();
        for (index, (row, stored)) in rows.iter_mut().zip(before.iter()).enumerate() {
            response = ui
                .horizontal(|ui| {
                    let typed = ui
                        .add(egui::TextEdit::singleline(&mut row.text).desired_width(150.0))
                        .on_hover_text(tl!("Type specific text to be included in final file name"));
                    // Typing into a rule row makes the row literal text, and its dropdown follows (#13661 wording).
                    if row.text != stored.text && row.rule != NAMING_NONE {
                        row.rule = NAMING_NONE.to_string();
                    }
                    let out = Select::new(Label::Empty, Bind::given(row.rule.clone()), &entries).id_salt(("droplet-naming-rule", index)).show(ui);
                    if let Some(rule) = out.value()
                        && rule.as_str() != row.rule.as_str()
                    {
                        row.rule = rule.clone();
                        if rule.as_str() != NAMING_NONE {
                            row.text = tl!(rule_label(rule)).to_string();
                        } else if row.text == tl!(rule_label(&stored.rule)) || row.text == rule_label(&stored.rule) {
                            // Picking `None` on a rule row empties it: the old rule's LABEL is
                            // not literal text (read back raw at naming_rows(), shown tl'd on a
                            // pick — match either). A typed text stays under `None`.
                            row.text = String::new();
                        }
                    }
                    if index < 5 {
                        ui.label("+");
                    }
                    out.response().clone().on_hover_text(tl!("Select a specific rule to be included in final file name")).union(typed)
                })
                .response;
        }
        if rows != before {
            match naming_components(&rows) {
                Some(parts) => fields.set("namingComponents", FieldValue::Json(Value::Array(parts))),
                None => drop(fields.remove("namingComponents")),
            }
        }
        response
    })
    .opens_with(FieldValue::Json(json!([{"component": "name"}, {"component": "extension"}])))
    .tooltip(tl!("Specify a rule for naming final files"))
}

/// The Compatibility custom row: Windows checked and greyed on Windows, Mac OS and Unix off.
fn compatibility_param() -> Param {
    Param::custom("compatibility", Label::from(tl!("Compatibility")), |ui, fields, _cx| {
        let held = field_json(fields, "compatibility").unwrap_or_else(|| json!({}));
        let flag = |name: &str, default: bool| held.get(name).and_then(Value::as_bool).unwrap_or(default);
        let (mut windows, mut macintosh, mut unix) = (flag("windows", cfg!(windows)), flag("macintosh", false), flag("unix", false));
        let before = (windows, macintosh, unix);
        let response = ui
            .horizontal(|ui| {
                // On Windows the box is checked and greyed, as in the reference.
                ui.add_enabled_ui(!cfg!(windows), |ui| {
                    ui.checkbox(&mut windows, tl!("Windows"));
                });
                ui.checkbox(&mut macintosh, tl!("Mac OS"));
                ui.checkbox(&mut unix, tl!("Unix"));
            })
            .response;
        if (windows, macintosh, unix) != before {
            fields.set("compatibility", FieldValue::Json(json!({"macintosh": macintosh, "windows": windows, "unix": unix})));
        }
        response
    })
    .opens_with(FieldValue::Json(json!({"macintosh": false, "windows": cfg!(windows), "unix": false})))
}

#[cfg(test)]
mod tests;
