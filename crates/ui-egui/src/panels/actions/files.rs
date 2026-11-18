//! Load Actions… and Save Actions…: the `.atn` file dialogs of the Actions panel.

use serde_json::{Value, json};

use craft::host::{PickRequest, Picked};

use super::ActionSelection;
use crate::PhotocraftApp;
use crate::panel_docking::show_panel;

/// Command params for a picked file: a real path keeps the journal small; browser files arrive
/// as bytes.
pub(super) fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import one set and reveal its first action, for both native paths and browser contents.
pub(super) fn load_source(app: &mut PhotocraftApp, name: &str, params: Value) -> Result<Value, String> {
    let r = app.run("actions.load", params)?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let unsupported = r["unsupported"].as_array().map_or(0, Vec::len);
    let shown = crate::file_open::display_name(name);
    let loaded =
        crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "Loaded {n} action from {file}", "Loaded {n} actions from {file}"), &[("file", &shown)]);
    app.ui.status_error = false;
    app.ui.status = if unsupported > 0 {
        let note = crate::i18n::trn(
            crate::i18n::current(),
            unsupported as u64,
            "{n} step has no PhotoCraft equivalent yet",
            "{n} steps have no PhotoCraft equivalent yet",
        );
        let template = "{message} ({note})";
        crate::i18n::fmt(tl!(template), &[("message", &loaded), ("note", &note)])
    } else {
        loaded
    };
    // Show what arrived: select the first loaded action, expanded.
    let first = r["actions"][0].as_str().and_then(|first| {
        let list = &app.session.actions.list;
        list.actions().iter().position(|a| a.name == first).map(|i| (i, first.to_string()))
    });
    if let Some((i, first)) = first {
        app.ui.actions.panel.selected = Some(ActionSelection::Action(i));
        app.ui.actions.panel.expanded.insert(first);
    }
    // Showing the panel cannot fail for the Actions panel; a missing group has nothing to reveal.
    let _ = show_panel(app, "actions", Some(true));
    Ok(r)
}

/// Load every chosen action file in order, retaining failures and unsupported-step warnings.
pub(super) fn load_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let mut request = PickRequest::open_many("actions").filter("Actions", &["atn"]);
    if let Some(dir) = crate::file_dialog::last_used_dir(&app.ui.recent_files) {
        request = request.directory(dir);
    }
    app.ask_file(request, |app, picked| {
        // Desktop answers paths (read by the engine, after its size check); the browser answers contents.
        let files: Vec<(String, Option<Vec<u8>>)> = match picked {
            Picked::Paths(paths) => paths.into_iter().map(|path| (path.to_string_lossy().into_owned(), None)).collect(),
            Picked::Contents(files) => files.into_iter().map(|(name, bytes)| (name, Some(bytes))).collect(),
            Picked::Cancelled => return Err(crate::file_dialog::CANCELLED.into()),
            Picked::Unavailable(why) => return Err(why),
        };
        let count = files.len();
        if count == 0 {
            return Err(crate::file_dialog::CANCELLED.into());
        }
        let mut loaded = 0usize;
        // What the panel holds afterwards, per set name: `actions.load` replaces a set of the same
        // name, so a later file's set takes the earlier one's place (and its counts).
        let mut sets: Vec<(String, String, usize, usize)> = Vec::new(); // set, file, actions, unsupported
        let mut replaced = Vec::new();
        let mut errors = Vec::new();
        let mut last = Value::Null;
        for (name, bytes) in files {
            let params = match bytes {
                Some(bytes) => source(&name, &bytes),
                None => json!({"path": name}),
            };
            match load_source(app, &name, params) {
                Ok(r) => {
                    loaded += 1;
                    let shown = crate::file_open::display_name(&name);
                    let set = r["set"].as_str().unwrap_or_default().to_string();
                    let n = r["actions"].as_array().map_or(0, Vec::len);
                    let unsupported = r["unsupported"].as_array().map_or(0, Vec::len);
                    if let Some(entry) = sets.iter_mut().find(|e| e.0 == set) {
                        replaced.push(crate::i18n::fmt(
                            tl!("{set} from {file} replaced the one from {earlier}"),
                            &[("file", &shown), ("earlier", &entry.1), ("set", &set)],
                        ));
                        *entry = (set, shown, n, unsupported);
                    } else {
                        sets.push((set, shown, n, unsupported));
                    }
                    last = r;
                }
                Err(e) => errors.push(format!("{}: {e}", crate::file_open::display_name(&name))),
            }
        }
        if count == 1 {
            return match errors.pop() {
                Some(e) => Err(e),
                None => Ok(last),
            };
        }
        let actions = sets.iter().fold(0usize, |sum, e| sum.saturating_add(e.2));
        let unsupported = sets.iter().fold(0usize, |sum, e| sum.saturating_add(e.3));
        let lang = crate::i18n::current();
        let files = crate::i18n::trn(lang, loaded as u64, "{n} action file loaded", "{n} action files loaded");
        let mut notes = vec![crate::i18n::trn(lang, actions as u64, "{n} action", "{n} actions")];
        if unsupported > 0 {
            notes.push(crate::i18n::trn(lang, unsupported as u64, "{n} step has no PhotoCraft equivalent yet", "{n} steps have no PhotoCraft equivalent yet"));
        }
        notes.extend(replaced);
        if !errors.is_empty() {
            let failed = crate::i18n::trn(lang, errors.len() as u64, "{n} file could not be loaded: {errors}", "{n} files could not be loaded: {errors}");
            notes.push(crate::i18n::fmt(&failed, &[("errors", &errors.join("; "))]));
        }
        let template = "{message} ({note})";
        let summary = crate::i18n::fmt(tl!(template), &[("message", &files), ("note", &notes.join("; "))]);
        if !errors.is_empty() {
            return Err(summary);
        }
        app.ui.status = summary;
        app.ui.status_error = false;
        Ok(json!({"files": loaded, "actions": actions, "unsupported": unsupported}))
    })
}

/// Save Actions…: the given set, else the selected set folder, else the selected action's set,
/// else every action, as a `.atn` file Photoshop loads. An empty set still saves.
pub(super) fn save_dialog(app: &mut PhotocraftApp, set: Option<String>) -> Result<Value, String> {
    let set = set.or_else(|| super::selected_set(app).map(str::to_string)).or_else(|| super::selected_action(app).and_then(|a| a.set.clone()));
    let params = match &set {
        Some(set) => json!({"set": set, "skipUnsupported": true}),
        None => json!({"skipUnsupported": true}),
    };
    let r = app.run("actions.save", params)?;
    let bytes = r["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("the export returned no data")?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let skipped = r["skipped"].as_array().map_or(0, Vec::len);
    // A set name can hold characters a file name can't (as in Keyboard Shortcuts' Save), or be a
    // Windows device name (`CON`); the file keeps the set's own name.
    let stem: String = r["set"]
        .as_str()
        .unwrap_or("Actions")
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    // A file name holds at most 255 bytes: the stem gets what `.atn` leaves, cut at a character.
    let mut stem = photocraft_engine::file_cmds::portable_device_name(stem);
    let mut cut = 255 - ".atn".len();
    while !stem.is_char_boundary(cut.min(stem.len())) {
        cut -= 1;
    }
    stem.truncate(cut);
    let file = format!("{stem}.atn");
    app.pick_save(&file, move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let shown = crate::file_open::display_name(&path);
        let saved =
            crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "Saved {n} action to {file}", "Saved {n} actions to {file}"), &[("file", &shown)]);
        app.ui.status_error = false;
        app.ui.status = if skipped > 0 {
            let note = crate::i18n::trn(
                crate::i18n::current(),
                skipped as u64,
                "{n} step with no Photoshop equivalent left out",
                "{n} steps with no Photoshop equivalent left out",
            );
            let template = "{message} ({note})";
            crate::i18n::fmt(tl!(template), &[("message", &saved), ("note", &note)])
        } else {
            saved
        };
        Ok(json!({"path": path, "actions": n, "skipped": skipped}))
    })
}
