//! Select › Save Selection… and Load Selection…: Photoshop's dialogs, with the Document the
//! selection is saved to or loaded from. Photoshop lists the open images with the same pixel
//! dimensions as the active one, and Save Selection adds New, which puts the selection into a
//! new image. The Channel list follows the chosen document. Save Selection's Name is for a new
//! channel only, and its Operation reads New Channel for a new channel and Replace / Add to /
//! Subtract from / Intersect with Channel for an existing one. OK runs `select.saveSelection` or
//! `select.loadSelection`, which agents can also call directly with params.

use photocraft_engine::DocState;
use photocraft_engine::channel_cmds::{color_names, composite_name};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const SAVE: &str = "select.saveSelection";
pub const LOAD: &str = "select.loadSelection";
const MARK: &str = "__selectionChannel";
/// The id of the document the dialog was opened on: the selection is saved from it or loaded
/// into it.
const TARGET: &str = "__document";
/// The value of a New document or New channel.
const NEW: &str = "new";
const SAVE_OPERATIONS: [&str; 4] = ["replace", "add", "subtract", "intersect"];
const LOAD_OPERATIONS: [&str; 4] = ["new", "add", "subtract", "intersect"];

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn saving(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(SAVE)
}

/// Opens the dialog for `command` ([`SAVE`] or [`LOAD`]) on the active document, set to save into
/// a new channel of it or to load its first alpha channel (its composite when it has none).
pub fn open(app: &mut PhotocraftApp, command: &str) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert(TARGET.into(), json!(st.doc.id.0));
    f.insert("document".into(), json!(st.doc.id.0));
    if command == SAVE {
        f.insert("__command".into(), json!(SAVE));
        f.insert("__label".into(), json!("Save Selection…"));
        f.insert("channel".into(), json!(NEW));
        f.insert("name".into(), json!(""));
        f.insert("operation".into(), json!(NEW));
    } else {
        f.insert("__command".into(), json!(LOAD));
        f.insert("__label".into(), json!("Load Selection…"));
        f.insert("channel".into(), json!(default_load_channel(st)));
        f.insert("invert".into(), json!(false));
        f.insert("operation".into(), json!(NEW));
    }
    Ok(json!({"dialog": app.ui.open_dialog(crate::state::DialogKind::Command, f)}))
}

/// The document the dialog was opened on, while it is still open.
fn target<'a>(app: &'a PhotocraftApp, f: &Map<String, Value>) -> Option<&'a DocState> {
    let id = f.get(TARGET).and_then(Value::as_u64)?;
    app.session.documents().iter().find(|d| d.doc.id.0 == id)
}

/// The Document list: the open documents with the target's pixel size, in tab order, as
/// `(document id, name)`; the target itself is among them. Empty once the target is closed.
pub fn documents(app: &PhotocraftApp, f: &Map<String, Value>) -> Vec<(u64, String)> {
    let Some(t) = target(app, f) else { return Vec::new() };
    app.session.documents().iter().filter(|d| d.doc.size == t.doc.size).map(|d| (d.doc.id.0, d.doc.name.clone())).collect()
}

/// The chosen document, when it is an open one of the Document list (not New).
fn chosen<'a>(app: &'a PhotocraftApp, f: &Map<String, Value>) -> Option<&'a DocState> {
    let id = f.get("document").and_then(Value::as_u64)?;
    let size = target(app, f)?.doc.size;
    app.session.documents().iter().find(|d| d.doc.id.0 == id && d.doc.size == size)
}

/// The Channel list of the chosen document as `(value, label)`. Save Selection: New, then the
/// document's alpha and spot channels (only New for a new document). Load Selection: the
/// composite and colour channels (loaded by their luminosity, as ⌘-clicking them does), the
/// alpha and spot channels, the active layer's transparency and its mask when it has one, and
/// the document's selection.
pub fn channels(app: &PhotocraftApp, f: &Map<String, Value>) -> Vec<(String, String)> {
    let doc = chosen(app, f);
    if saving(f) {
        let mut out = vec![(NEW.to_string(), tl!("New").to_string())];
        out.extend(doc.into_iter().flat_map(|d| d.doc.channels.iter().map(|c| (c.name.clone(), c.name.clone()))));
        return out;
    }
    let Some(st) = doc else { return Vec::new() };
    let d = &st.doc;
    let mode = d.pixel_format().mode;
    let mut out = Vec::new();
    // A Multichannel image is its channels: it has no composite or colour channels of its own.
    if mode != photocraft_color::ColorMode::Multichannel {
        out.push((composite_name(mode).to_string(), composite_name(mode).to_string()));
        if mode.color_channels() > 1 {
            out.extend(color_names(mode).iter().map(|n| (n.to_string(), n.to_string())));
        }
    }
    out.extend(d.channels.iter().map(|c| (c.name.clone(), c.name.clone())));
    // Photoshop names the layer: "Layer 1 Transparency", "Layer 1 Mask".
    if let Some(layer) = st.active_layer.and_then(|l| d.layer(l)) {
        out.push(("transparency".into(), format!("{} {}", layer.name, tl!("Transparency"))));
        if layer.mask.is_some() {
            out.push(("mask".into(), format!("{} {}", layer.name, tl!("Mask"))));
        }
    }
    out.push(("selection".into(), tl!("Selection").to_string()));
    out
}

/// Load Selection's first channel: the first alpha channel, else the composite (a Multichannel
/// image's first channel).
fn default_load_channel(st: &DocState) -> String {
    let mode = st.doc.pixel_format().mode;
    match st.doc.channels.first() {
        Some(c) => c.name.clone(),
        None if mode == photocraft_color::ColorMode::Multichannel => "selection".into(),
        None => composite_name(mode).to_string(),
    }
}

/// Keeps the fields consistent with the chosen document: a document that closed or no longer
/// has the target's size falls back to the target; a channel the document lacks goes back to
/// New (Save) or the document's first channel (Load); Save Selection's operation is New
/// Channel for a new channel and Replace Channel instead of it for an existing one.
pub fn normalize(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    let new_document = saving(f) && f.get("document").and_then(Value::as_str) == Some(NEW);
    if !new_document && chosen(app, f).is_none() {
        let Some(id) = f.get(TARGET).cloned() else { return };
        f.insert("document".into(), id);
    }
    let channel = f.get("channel").and_then(Value::as_str).unwrap_or_default().to_string();
    let list = channels(app, f);
    if !list.iter().any(|(v, _)| *v == channel) {
        let first = if saving(f) { Some(NEW.to_string()) } else { chosen(app, f).map(default_load_channel) };
        if let Some(first) = first {
            f.insert("channel".into(), json!(first));
        }
    }
    if saving(f) {
        let new_channel = f.get("channel").and_then(Value::as_str) == Some(NEW);
        let op = f.get("operation").and_then(Value::as_str).unwrap_or(NEW);
        let op = match (new_channel, op) {
            (true, _) => NEW,
            (false, op) if SAVE_OPERATIONS.contains(&op) => op,
            (false, _) => "replace",
        };
        f.insert("operation".into(), json!(op));
    } else if !f.get("operation").and_then(Value::as_str).is_some_and(|op| LOAD_OPERATIONS.contains(&op)) {
        f.insert("operation".into(), json!(NEW));
    }
}

/// The message for OK after the document the dialog saves to or loads from was closed.
fn closed(f: &Map<String, Value>) -> &'static str {
    if saving(f) { tl!("The document was closed. Choose Save Selection again.") } else { tl!("The document was closed. Choose Load Selection again.") }
}

/// The engine params for the dialog's fields. `document` is given only for another document or
/// New, so a step recorded on the active document replays on whichever document is active, as
/// before; the other document goes by its current index.
pub fn params(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let gone = || closed(f);
    let t = target(app, f).ok_or_else(gone)?;
    if app.session.active().map(|st| st.doc.id) != Some(t.doc.id) {
        return Err(tl!("The active document changed. Choose the command again.").into());
    }
    let mut p = Map::new();
    if saving(f) && f.get("document").and_then(Value::as_str) == Some(NEW) {
        p.insert("document".into(), json!(NEW));
    } else {
        let id = f.get("document").and_then(Value::as_u64).ok_or_else(gone)?;
        let size = t.doc.size;
        let i = app.session.documents().iter().position(|d| d.doc.id.0 == id && d.doc.size == size).ok_or_else(gone)?;
        if id != t.doc.id.0 {
            p.insert("document".into(), json!(i));
        }
    }
    // Name is for a new channel only: given with an existing one, the engine would rename it.
    let new_channel = f.get("channel").and_then(Value::as_str) == Some(NEW);
    let keys: &[&str] = match (saving(f), new_channel) {
        (true, true) => &["channel", "name", "operation"],
        (true, false) => &["channel", "operation"],
        (false, _) => &["channel", "invert", "operation"],
    };
    for k in keys {
        if let Some(v) = f.get(*k) {
            p.insert((*k).into(), v.clone());
        }
    }
    Ok(Value::Object(p))
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    // A chosen document that closed meanwhile is refused, not swapped for another one.
    if f.get("document").and_then(Value::as_u64).is_some() && chosen(app, f).is_none() {
        return Err(closed(f).into());
    }
    let mut f = f.clone();
    normalize(app, &mut f);
    let command = if saving(&f) { SAVE } else { LOAD };
    let p = params(app, &f)?;
    app.run(command, p)
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    normalize(app, f);
    let save = saving(f);
    let mut options: Vec<(String, String)> = documents(app, f).into_iter().map(|(id, name)| (id.to_string(), name)).collect();
    if save {
        options.push((NEW.into(), tl!("New").into()));
    }
    let mut document = match f.get("document") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    ui.spacing_mut().item_spacing.y = 6.0;
    crate::widgets::section_label(ui, if save { tl!("Destination") } else { tl!("Source") });
    crate::widgets::hairline(ui);
    egui::Grid::new("selection-channel").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label(tl!("Document:"));
        // Document and channel names are the user's: shown as they are, not translated.
        let docs: Vec<(String, &str)> = options.iter().map(|(k, n)| (k.clone(), n.as_str())).collect();
        if crate::widgets::dropdown_literal(ui, "selection-channel-document", tl!("Document"), &mut document, &docs, 220.0) {
            let v = document.parse::<u64>().map_or_else(|_| json!(document), |id| json!(id));
            f.insert("document".into(), v);
            normalize(app, f);
        }
        ui.end_row();
        ui.label(tl!("Channel:"));
        let list = channels(app, f);
        let mut channel = f.get("channel").and_then(Value::as_str).unwrap_or_default().to_string();
        let chans: Vec<(String, &str)> = list.iter().map(|(k, n)| (k.clone(), n.as_str())).collect();
        if crate::widgets::dropdown_literal(ui, "selection-channel-channel", tl!("Channel"), &mut channel, &chans, 220.0) {
            f.insert("channel".into(), json!(channel));
            normalize(app, f);
        }
        ui.end_row();
        if save {
            // Name is for a new channel only, as in Photoshop.
            let new_channel = f.get("channel").and_then(Value::as_str) == Some(NEW);
            let mut name = f.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let name_label = ui.label(tl!("Name:"));
            let edit = ui.add_enabled(new_channel, egui::TextEdit::singleline(&mut name).desired_width(220.0)).labelled_by(name_label.id);
            if edit.changed() {
                f.insert("name".into(), json!(name));
            }
            ui.end_row();
        } else {
            ui.label("");
            let mut invert = f.get("invert").and_then(Value::as_bool).unwrap_or(false);
            if ui.checkbox(&mut invert, tl!("Invert")).changed() {
                f.insert("invert".into(), json!(invert));
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Operation"));
    crate::widgets::hairline(ui);
    let mut op = f.get("operation").and_then(Value::as_str).unwrap_or(NEW).to_string();
    ui.indent("selection-channel-operation", |ui| {
        if save {
            let new_channel = f.get("channel").and_then(Value::as_str) == Some(NEW);
            // The first choice reads New Channel for a new channel and Replace Channel for an
            // existing one; combining needs an existing channel.
            let first = if new_channel { (NEW, tl!("New Channel")) } else { ("replace", tl!("Replace Channel")) };
            ui.radio_value(&mut op, first.0.to_string(), first.1);
            for (key, label) in [("add", tl!("Add to Channel")), ("subtract", tl!("Subtract from Channel")), ("intersect", tl!("Intersect with Channel"))] {
                ui.add_enabled_ui(!new_channel, |ui| ui.radio_value(&mut op, key.to_string(), label));
            }
        } else {
            for (key, label) in [
                (NEW, tl!("New Selection")),
                ("add", tl!("Add to Selection")),
                ("subtract", tl!("Subtract from Selection")),
                ("intersect", tl!("Intersect with Selection")),
            ] {
                ui.radio_value(&mut op, key.to_string(), label);
            }
        }
    });
    f.insert("operation".into(), json!(op));
}

#[cfg(test)]
mod tests;
