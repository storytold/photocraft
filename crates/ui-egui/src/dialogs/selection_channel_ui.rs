//! Select › Save Selection… and Load Selection…: Photoshop's dialogs, with the Document the
//! selection is saved to or loaded from. Photoshop lists the open images with the same pixel
//! dimensions as the active one, and Save Selection adds New, which puts the selection into a new
//! image. The Channel list follows the chosen document. Save Selection's Name is for a new channel
//! only, and its Operation reads New Channel for a new channel and Replace / Add to / Subtract
//! from / Intersect with Channel for an existing one. OK runs `select.saveSelection` or
//! `select.loadSelection`, which agents can also call directly with params.
//!
//! A hosted parameter dialog whose rows are built each pass from its values ([`rows`]). The open
//! documents it can choose from, and their channels, are in its private `__docs` value, which
//! [`reconcile`] keeps up to date with the session every pass, as Match Color keeps its sources;
//! [`normalize`] makes the values agree with them in the builder, so a change of Document or
//! Channel shows its consequences in the same pass.
//!
//! The `channel` value is `"new"`, one of Load Selection's built-in sources (the composite, a
//! colour channel, `"transparency"`, `"mask"`, `"selection"`), or the index of an alpha or spot
//! channel of the chosen document, so two channels of one name, or a channel named like a
//! built-in source ("Mask", "Red"), are told apart.

use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_doc::Document;
use photocraft_engine::DocState;
use photocraft_engine::channel_cmds::{ChanRef, color_names, composite_name, parse_ref};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const SAVE: &str = "select.saveSelection";
pub const LOAD: &str = "select.loadSelection";
/// The open documents the dialog can choose from and their channels, as [`snapshot`] lists them.
const DOCS: &str = "__docs";
/// The document and channel as last made consistent, with the chosen document's alpha and spot
/// channel names then, `[document, channel, names]`: how a change of the Document is told from a
/// change of the Channel, and how the chosen channel is followed when a command adds, deletes or
/// moves channels while the dialog is open. Alt+Cancel (Reset) restores it with the other values,
/// so the open-time channel is followed from the open-time names.
const SEEN: &str = "__seen";
/// The value of a New document or New channel.
const NEW: &str = "new";
const SAVE_OPERATIONS: [&str; 4] = ["replace", "add", "subtract", "intersect"];
const LOAD_OPERATIONS: [&str; 4] = ["new", "add", "subtract", "intersect"];

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == SAVE || key == LOAD
}

/// The document the dialog `key` was opened on: the selection is saved from it or loaded into it.
fn target(app: &PhotocraftApp, key: &str) -> Option<u64> {
    crate::dialogs::params::captured(app, key).get("target").and_then(Value::as_u64)
}

/// Opens the dialog for `command` ([`SAVE`] or [`LOAD`]) on the active document, set to save into
/// a new channel of it or to load its first alpha channel (its composite when it has none).
pub fn open(app: &mut PhotocraftApp, command: &str) -> Result<DialogId, String> {
    let st = app.session.active().ok_or("no document")?;
    let target = st.doc.id.0;
    let save = command == SAVE;
    let mut fields = DialogFields::new();
    fields.set(DOCS, FieldValue::Json(snapshot(app, target)));
    fields.set("document", target.to_string());
    if save {
        fields.set("channel", NEW);
        fields.set("name", "");
    } else {
        fields.set("channel", default_load_channel(st));
        fields.set("invert", false);
    }
    fields.set("operation", NEW);
    normalize(&mut fields, save);
    let label = photocraft_engine::commands::find(command).map_or(command, |c| c.label);
    let title = crate::dialogs::filter_dialog::title_of(label);
    let key = command.to_owned();
    // Photoshop opens both dialogs on these values each time: nothing is remembered.
    Ok(crate::dialogs::params::open_param_unsaved(app, command, fields, json!({"target": target}), move |fields: &mut DialogFields| {
        normalize(fields, save);
        let rows = rows(fields, save);
        ParamDialog::command(key.clone(), title.clone(), rows).values(Bind::bound(fields))
    }))
}

/// The open documents with the target's pixel size, in tab order, each with what the dialog shows
/// of it: `{target, docs: [{id, name, names, load, first}]}`, `names` its alpha and spot channels,
/// `load` Load Selection's Channel list `[[value, label]]` and `first` the channel Load Selection
/// starts from. No documents once the target is closed.
fn snapshot(app: &PhotocraftApp, target: u64) -> Value {
    let documents = app.session.documents();
    let size = documents.iter().find(|d| d.doc.id.0 == target).map(|t| t.doc.size);
    let docs: Vec<Value> = documents
        .iter()
        .filter(|d| Some(d.doc.size) == size)
        .map(|st| json!({"id": st.doc.id.0.to_string(), "name": st.doc.name, "names": channel_names(&st.doc), "load": load_channels(st), "first": default_load_channel(st)}))
        .collect();
    json!({"target": target.to_string(), "docs": docs})
}

/// Load Selection's Channel list of `st` as `[[value, label]]`: the composite and colour channels
/// (loaded by their luminosity, as ⌘-clicking them does), the alpha and spot channels by index,
/// the active layer's transparency and its mask when it has one, and the document's selection.
fn load_channels(st: &DocState) -> Vec<[String; 2]> {
    let d = &st.doc;
    let mode = d.pixel_format().mode;
    let mut out = Vec::new();
    // A Multichannel image is its channels: it has no composite or colour channels of its own.
    if mode != photocraft_color::ColorMode::Multichannel {
        out.push([composite_name(mode).to_string(), composite_name(mode).to_string()]);
        if mode.color_channels() > 1 {
            out.extend(color_names(mode).iter().map(|n| [n.to_string(), n.to_string()]));
        }
    }
    out.extend(d.channels.iter().enumerate().map(|(i, c)| [i.to_string(), c.name.clone()]));
    // Photoshop names the layer: "Layer 1 Transparency", "Layer 1 Mask".
    if let Some(layer) = st.active_layer.and_then(|l| d.layer(l)) {
        out.push(["transparency".into(), format!("{} {}", layer.name, tl!("Transparency"))]);
        if layer.mask.is_some() {
            out.push(["mask".into(), format!("{} {}", layer.name, tl!("Mask"))]);
        }
    }
    out.push(["selection".into(), tl!("Selection").to_string()]);
    out
}

/// Load Selection's first channel: the first alpha channel, else the composite (the selection for
/// a Multichannel image without channels).
fn default_load_channel(st: &DocState) -> String {
    let mode = st.doc.pixel_format().mode;
    match st.doc.channels.first() {
        Some(_) => "0".into(),
        None if mode == photocraft_color::ColorMode::Multichannel => "selection".into(),
        None => composite_name(mode).into(),
    }
}

/// The names of `doc`'s alpha and spot channels, in order.
fn channel_names(doc: &Document) -> Vec<String> {
    doc.channels.iter().map(|c| c.name.clone()).collect()
}

/// The dialog's `__docs`.
fn docs(fields: &DialogFields) -> &[Value] {
    match fields.get(DOCS) {
        Some(FieldValue::Json(docs)) => docs.get("docs").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
        _ => &[],
    }
}

/// The document the dialog was opened on, as `__docs` names it.
fn target_key(fields: &DialogFields) -> Option<&str> {
    match fields.get(DOCS) {
        Some(FieldValue::Json(docs)) => docs.get("target").and_then(Value::as_str),
        _ => None,
    }
}

/// The chosen document's entry in `__docs`, when it is an open one of the Document list (not New).
fn chosen(fields: &DialogFields) -> Option<&Value> {
    let id = fields.text("document")?;
    docs(fields).iter().find(|d| d.get("id").and_then(Value::as_str) == Some(id))
}

/// Whether the dialog saves into a new document.
fn new_document(fields: &DialogFields, save: bool) -> bool {
    save && fields.text("document") == Some(NEW)
}

/// The Channel list of the chosen document as `(value, label)`. Save Selection: New, then the
/// document's alpha and spot channels (only New for a new document). Load Selection: the list
/// [`load_channels`] made. An alpha or spot channel's value is its index.
fn channels(fields: &DialogFields, save: bool) -> Vec<(String, String)> {
    let doc = chosen(fields).filter(|_| !new_document(fields, save));
    if save {
        let mut out = vec![(NEW.to_string(), tl!("New").to_string())];
        let names = doc.and_then(|d| d.get("names")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        out.extend(names.iter().enumerate().filter_map(|(i, n)| Some((i.to_string(), n.as_str()?.to_string()))));
        return out;
    }
    let list = doc.and_then(|d| d.get("load")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    list.iter().filter_map(|pair| Some((pair.get(0)?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))).collect()
}

/// Where chosen channel `i` is now that the chosen document's channel names went from `before`
/// to `now`: the place of its name while exactly one channel had that name then and exactly one
/// has it now, else none, since index `i` may now be another channel.
fn follow(before: &[Value], now: &[Value], i: usize) -> Option<usize> {
    let name = before.get(i)?.as_str()?;
    let once = before.iter().filter(|n| n.as_str() == Some(name)).count() == 1;
    let mut places = now.iter().enumerate().filter(|(_, n)| n.as_str() == Some(name)).map(|(j, _)| j);
    let j = places.next()?;
    (once && places.next().is_none()).then_some(j)
}

/// Keeps the values consistent with the chosen document: a document that closed or no longer has
/// the target's size falls back to the target; the Channel list is the chosen document's, so
/// choosing another document starts from New (Save) or that document's first channel (Load),
/// unless a channel of it was chosen after it, and so does a channel the document lacks; when a
/// command adds, deletes or moves the chosen document's channels while the dialog is open, the
/// chosen channel follows its name, or goes back to the first like a channel the document lacks
/// when its name doesn't tell it apart, never to another channel at its old place; Save
/// Selection's operation is New Channel for a new channel and Replace Channel instead of it for an
/// existing one.
pub(crate) fn normalize(fields: &mut DialogFields, save: bool) {
    if !new_document(fields, save) && chosen(fields).is_none() {
        let Some(target) = target_key(fields).map(str::to_owned) else { return };
        fields.set("document", target);
    }
    let document = fields.text("document").map(str::to_owned);
    let names: Option<Vec<Value>> = chosen(fields).filter(|_| !new_document(fields, save)).and_then(|d| d.get("names")?.as_array().cloned());
    let mut channel = fields.text("channel").map(str::to_owned);
    let seen = match fields.get(SEEN) {
        Some(FieldValue::Json(Value::Array(seen))) => Some(seen.clone()),
        _ => None,
    };
    let seen_document = seen.as_ref().and_then(|s| s.first()).and_then(Value::as_str).map(str::to_owned);
    let seen_channel = seen.as_ref().and_then(|s| s.get(1)).and_then(Value::as_str).map(str::to_owned);
    let same_channel = seen_channel.is_some() && seen_channel == channel;
    // Another document with the channel left as it was: the channel was the old document's.
    let moved = seen.is_some() && seen_document != document && same_channel;
    // The same document and channel, but the document's channels changed under the dialog.
    if seen_document == document
        && same_channel
        && let (Some(before), Some(now), Some(i)) =
            (seen.as_ref().and_then(|s| s.get(2)).and_then(Value::as_array), &names, channel.as_deref().and_then(|c| c.parse::<usize>().ok()))
        && before != now
    {
        channel = follow(before, now, i).map(|j| j.to_string());
    }
    let list = channels(fields, save);
    let listed = |c: &Option<String>| c.as_ref().is_some_and(|c| list.iter().any(|(v, _)| v == c));
    if moved || !listed(&channel) {
        channel = if save { Some(NEW.to_string()) } else { chosen(fields).and_then(|d| d.get("first")?.as_str().map(str::to_owned)) };
    }
    if let Some(c) = &channel
        && fields.text("channel") != Some(c.as_str())
    {
        fields.set("channel", c.clone());
    }
    let op = fields.text("operation").unwrap_or(NEW).to_owned();
    let op = if save {
        match (channel.as_deref() == Some(NEW), op.as_str()) {
            (true, _) => NEW,
            (false, op) if SAVE_OPERATIONS.contains(&op) => op,
            (false, _) => "replace",
        }
    } else if LOAD_OPERATIONS.contains(&op.as_str()) {
        op.as_str()
    } else {
        NEW
    }
    .to_owned();
    if fields.text("operation") != Some(op.as_str()) {
        fields.set("operation", op);
    }
    let now = json!([document, channel, names]);
    if fields.get(SEEN) != Some(&FieldValue::Json(now.clone())) {
        fields.set(SEEN, FieldValue::Json(now));
    }
}

/// The dialog's rows for its values: Document and Channel, Name (Save, for a new channel only) or
/// Invert (Load), and the Operation.
pub(crate) fn rows(fields: &DialogFields, save: bool) -> Vec<Param> {
    // Document and channel names are the user's: shown as they are, not translated.
    let mut documents: Vec<(String, Label)> =
        docs(fields).iter().filter_map(|d| Some((d.get("id")?.as_str()?.to_string(), Label::from(d.get("name")?.as_str()?.to_string())))).collect();
    if save {
        documents.push((NEW.to_string(), Label::from(tl!("New"))));
    }
    let channels: Vec<(String, Label)> = channels(fields, save).into_iter().map(|(v, n)| (v, Label::from(n))).collect();
    let document = fields.text("document").map(str::to_owned);
    let channel = fields.text("channel").map(str::to_owned);
    let mut rows = vec![
        Param::section(if save { tl!("Destination") } else { tl!("Source") }),
        Param::new("document", tl!("Document"), ParamKind::choice(documents, document)),
        Param::new("channel", tl!("Channel"), ParamKind::choice(channels, channel.clone())),
    ];
    let operations: Vec<(String, Label)> = if save {
        let new_channel = channel.as_deref() == Some(NEW);
        // Name is for a new channel only, as in Photoshop.
        rows.push(Param::new("name", tl!("Name"), ParamKind::text("")).enabled_when("channel", NEW));
        // The first choice reads New Channel for a new channel and Replace Channel for an existing
        // one; combining needs an existing channel.
        let first = if new_channel { (NEW, tl!("New Channel")) } else { ("replace", tl!("Replace Channel")) };
        [first, ("add", tl!("Add to Channel")), ("subtract", tl!("Subtract from Channel")), ("intersect", tl!("Intersect with Channel"))]
            .into_iter()
            .map(|(k, l)| (k.to_string(), Label::from(l)))
            .collect()
    } else {
        rows.push(Param::new("invert", tl!("Invert"), ParamKind::bool(false)));
        [
            (NEW, tl!("New Selection")),
            ("add", tl!("Add to Selection")),
            ("subtract", tl!("Subtract from Selection")),
            ("intersect", tl!("Intersect with Selection")),
        ]
        .into_iter()
        .map(|(k, l)| (k.to_string(), Label::from(l)))
        .collect()
    };
    rows.push(Param::section(tl!("Operation")));
    let mut operation = Param::new("operation", tl!("Operation"), ParamKind::radio(operations, fields.text("operation").map(str::to_owned)));
    if save && channel.as_deref() == Some(NEW) {
        operation = operation.disabled_options(["add", "subtract", "intersect"]);
    }
    rows.push(operation);
    rows
}

/// Brings the open dialogs' `__docs` up to date with the session: every pass, before the host
/// shows them, as documents open, close, are renamed, and gain or lose channels.
pub(crate) fn reconcile(app: &mut PhotocraftApp) {
    for key in [SAVE, LOAD] {
        let id = dialog_id(key);
        let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id) else { continue };
        let Some(target) = target(app, key) else { continue };
        let now = snapshot(app, target);
        if dialog.fields().get(DOCS) == Some(&FieldValue::Json(now.clone())) {
            continue;
        }
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
            dialog.set_field(DOCS, FieldValue::Json(now));
        }
    }
}

/// How a step names alpha or spot channel `i` of `doc`: by its name, as steps always have, when
/// the commands read that name as this channel; else by its index. The commands read "Mask",
/// "Selection", a colour channel's name and the like as those sources, "new" and "targeted" as
/// their keywords, and a name two channels share as the first of them.
fn channel_reference(doc: &Document, i: usize) -> Value {
    match doc.channels.get(i) {
        Some(c) if !matches!(c.name.as_str(), NEW | "targeted") && parse_ref(&json!(c.name), doc) == Some(ChanRef::Alpha(i)) => json!(c.name),
        _ => json!(i),
    }
}

/// The message for OK after the document the dialog saves to or loads from was closed.
fn closed(save: bool) -> &'static str {
    if save { tl!("The document was closed. Choose Save Selection again.") } else { tl!("The document was closed. Choose Load Selection again.") }
}

/// The command and engine params OK runs for the dialog `key`'s `fields`, made consistent with the
/// session as it is now; an error when the document the dialog was opened on, or the chosen one,
/// was closed, or another document became active. `document` is given only for another document or
/// New, so a step recorded on the active document replays on whichever document is active, as
/// before; the other document goes by its current index.
pub(crate) fn command(app: &PhotocraftApp, key: &str, fields: &DialogFields) -> Result<(&'static str, Value), String> {
    let save = key == SAVE;
    let target = target(app, key).ok_or_else(|| closed(save))?;
    let documents = app.session.documents();
    let t = documents.iter().find(|d| d.doc.id.0 == target).ok_or_else(|| closed(save))?;
    if app.session.active().map(|st| st.doc.id) != Some(t.doc.id) {
        return Err(tl!("The active document changed. Choose the command again.").into());
    }
    let size = t.doc.size;
    let open = |id: &str| documents.iter().position(|d| d.doc.id.0.to_string() == id && d.doc.size == size);
    // A chosen document that closed meanwhile is refused, not swapped for another one.
    if !new_document(fields, save) && fields.text("document").is_some_and(|id| open(id).is_none()) {
        return Err(closed(save).into());
    }
    let mut f = fields.clone();
    f.set(DOCS, FieldValue::Json(snapshot(app, target)));
    normalize(&mut f, save);
    let mut p = Map::new();
    let chosen = if new_document(&f, save) {
        p.insert("document".into(), json!(NEW));
        None
    } else {
        let i = f.text("document").and_then(open).ok_or_else(|| closed(save))?;
        if documents[i].doc.id != t.doc.id {
            p.insert("document".into(), json!(i));
        }
        Some(&documents[i])
    };
    let channel = f.text("channel").unwrap_or(NEW);
    let index = channel.parse::<usize>().ok();
    p.insert(
        "channel".into(),
        match (index, chosen) {
            (Some(i), Some(st)) => channel_reference(&st.doc, i),
            _ => json!(channel),
        },
    );
    if save {
        // Name is for a new channel only: given with an existing one, the engine would rename it.
        if channel == NEW {
            p.insert("name".into(), json!(f.text("name").unwrap_or_default()));
        }
    } else {
        p.insert("invert".into(), json!(f.bool("invert").unwrap_or(false)));
    }
    p.insert("operation".into(), json!(f.text("operation").unwrap_or(NEW)));
    Ok((if save { SAVE } else { LOAD }, Value::Object(p)))
}

/// Refuses an OK the session no longer allows, so the dialog stays open with the reason.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    command(app, &result.key, &result.fields).map(|_| ()).map_err(Label::from)
}

/// OK: runs the command with the dialog's choices.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let outcome = command(app, &result.key, &result.fields).and_then(|(command, params)| crate::actions::run_step_dialog(app, result.id, command, params));
    if let Err(error) = outcome {
        crate::notices::error(app, error);
    }
}

/// What tests read of an open dialog.
#[cfg(test)]
pub(crate) mod testing {
    use craft::ui::dialog::{DialogId, StackDialog};
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamKind};

    use crate::PhotocraftApp;

    /// The values of the open dialog `id`.
    pub(crate) fn fields(app: &PhotocraftApp, id: DialogId) -> DialogFields {
        app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("an open Save or Load Selection dialog").fields().clone()
    }

    /// The option ids of row `key` of the open dialog `id`, as its rows stand, with whether each is enabled.
    pub(crate) fn options(app: &PhotocraftApp, id: DialogId, key: &str) -> Vec<(String, bool)> {
        let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("an open Save or Load Selection dialog");
        let save = StackDialog::key(dialog) == Some(super::SAVE);
        let rows = super::rows(dialog.fields(), save);
        let row = rows.iter().find(|row| row.key == key).unwrap_or_else(|| panic!("no row {key}"));
        match &row.kind {
            ParamKind::Choice { options, .. } | ParamKind::Radio { options, .. } => {
                options.iter().map(|(id, _)| (id.clone(), row.option_enabled(id))).collect()
            }
            other => panic!("{key} is not a choice: {other:?}"),
        }
    }

    /// Whether row `key` of the open dialog `id` takes input.
    pub(crate) fn enabled(app: &PhotocraftApp, id: DialogId, key: &str) -> bool {
        let fields = fields(app, id);
        let save = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|dialog| StackDialog::key(dialog)) == Some(super::SAVE);
        super::rows(&fields, save).iter().find(|row| row.key == key).unwrap_or_else(|| panic!("no row {key}")).enabled(&fields)
    }
}

#[cfg(test)]
mod tests;
