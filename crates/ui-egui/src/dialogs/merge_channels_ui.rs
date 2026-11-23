//! Channels panel › Merge Channels…: the mode (RGB, CMYK or Lab Color) and, for each of its
//! channels, the open grayscale document that becomes it. Photoshop asks in two steps (the mode,
//! then the documents); here both are one dialog. Only the modes the open documents can fill are
//! offered (RGB and Lab need three, CMYK four), OK waits for a different document per channel,
//! and a merge the engine refuses keeps the dialog open with the reason. The merge is the engine's
//! `channel.merge`, which closes the sources; agents call it directly with params (#11451).
//!
//! The dialog names documents by their [`photocraft_doc::DocId`], not their tab index: work that
//! runs while it is open (a script moving or closing a document) can shift the indices, and OK
//! must merge, and close, the documents that were picked. They are looked up again at OK.
//!
//! OK is held ([`check`]): the check resolves the picks and runs the merge, and answers the
//! engine's refusal as the reason shown in the dialog. [`run`] has nothing left to do.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use photocraft_doc::ColorMode;
use photocraft_engine::channel_cmds;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

pub const COMMAND: &str = "channel.merge";
/// The most channels a mode has (CMYK).
const SLOTS: usize = 4;
const MODES: [(&str, &str); 3] = [("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
const TOO_FEW: &str = "Not enough grayscale documents for this mode.";
const REPEATED: &str = "Pick a different document for each channel.";
const CLOSED: &str = "A picked document is no longer open.";
const CHANGED: &str = "A picked document is no longer a grayscale document of the same size.";

/// A document the dialog offers: its id and name.
type Source = (u64, String);

/// Whether `key` is the Merge Channels dialog's.
pub fn owns(key: &str) -> bool {
    key == COMMAND
}

fn mode_of(key: &str) -> ColorMode {
    match key {
        "cmyk" => ColorMode::Cmyk,
        "lab" => ColorMode::Lab,
        _ => ColorMode::Rgb,
    }
}

/// The modes `count` documents can fill, in the menu's order.
fn modes(count: usize) -> Vec<(String, &'static str)> {
    MODES.iter().filter(|(key, _)| mode_of(key).color_channels() <= count).map(|(key, label)| (key.to_string(), *label)).collect()
}

/// The documents that can be merged, by document id and name: the open grayscale ones of the
/// active document's size, the active one first, as the engine picks them without params.
fn sources(app: &PhotocraftApp) -> Vec<Source> {
    let Some(active) = app.session.active() else { return Vec::new() };
    let docs = app.session.documents();
    let fits = |i: usize| docs.get(i).filter(|d| d.doc.mode == ColorMode::Grayscale && d.doc.size == active.doc.size);
    let current = app.session.active_index();
    let others = (0..docs.len()).filter(|i| Some(*i) != current);
    current.into_iter().chain(others).filter_map(|i| fits(i).map(|d| (d.doc.id.0, d.doc.name.clone()))).collect()
}

fn sources_of(captured: &Value) -> Vec<Source> {
    captured.get("sources").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

fn mode_field(f: &DialogFields) -> &str {
    f.text("mode").unwrap_or("rgb")
}

fn channel_key(k: usize) -> String {
    format!("channel{}", k + 1)
}

/// The picked document ids, one per channel of the mode.
fn picks(f: &DialogFields) -> Vec<u64> {
    (0..mode_of(mode_field(f)).color_channels()).filter_map(|k| f.text(&channel_key(k)).and_then(|s| s.parse().ok())).collect()
}

/// The open values: the first mode the documents can fill (RGB Color from three) and the
/// merge-able documents in order, one per channel.
fn open_fields(sources: &[Source]) -> DialogFields {
    let mode = modes(sources.len()).first().map_or_else(|| "rgb".to_string(), |(key, _)| key.clone());
    let mut f = DialogFields::new();
    f.set("mode", FieldValue::Text(mode));
    for (k, (id, _)) in sources.iter().take(SLOTS).enumerate() {
        f.set(channel_key(k), FieldValue::Text(id.to_string()));
    }
    f
}

/// Why OK is greyed (English, for `tl!`): the mode needs more documents than are offered, or the
/// picks repeat a document or name one the dialog did not offer. `None` when OK can merge.
fn problem(sources: &[Source], f: &DialogFields) -> Option<&'static str> {
    let n = mode_of(mode_field(f)).color_channels();
    let picks = picks(f);
    if sources.len() < n || picks.len() < n {
        return Some(TOO_FEW);
    }
    let mut seen = std::collections::HashSet::new();
    let offered = |p: &u64| sources.iter().any(|(id, _)| id == p);
    if picks.iter().any(|p| !seen.insert(*p) || !offered(p)) {
        return Some(REPEATED);
    }
    None
}

/// The picked documents' current tab indices, looked up by id; refused (English, for `tl!`)
/// when one was closed, or is no longer a grayscale document of the others' size.
fn resolve(app: &PhotocraftApp, sources: &[Source], f: &DialogFields) -> Result<Vec<usize>, &'static str> {
    if let Some(why) = problem(sources, f) {
        return Err(why);
    }
    let docs = app.session.documents();
    let indices = picks(f).iter().map(|id| docs.iter().position(|d| d.doc.id.0 == *id).ok_or(CLOSED)).collect::<Result<Vec<_>, _>>()?;
    let first = indices.first().and_then(|&i| docs.get(i)).map(|d| d.doc.size);
    let fits = |i: &usize| docs.get(*i).is_some_and(|d| d.doc.mode == ColorMode::Grayscale && Some(d.doc.size) == first);
    if indices.iter().all(fits) { Ok(indices) } else { Err(CHANGED) }
}

/// The `channel.merge` params for the dialog's values: the mode and the picked documents' current
/// indices, one per channel.
pub fn params(app: &PhotocraftApp, sources: &[Source], f: &DialogFields) -> Result<Value, String> {
    let documents = resolve(app, sources, f).map_err(str::to_string)?;
    Ok(json!({ "mode": mode_field(f), "documents": documents }))
}

/// The rows for the values `f`, rebuilt each pass: the Mode choice, then one choice per channel of
/// the mode over the offered documents.
fn rows(sources: &[Source], f: &DialogFields) -> Vec<Param> {
    let mode = mode_field(f);
    let mode_options = modes(sources.len()).into_iter().map(|(key, label)| (key, tl!(label).into())).collect();
    let mut rows = vec![Param::new("mode", tl!("Mode:"), ParamKind::choice(mode_options, Some(mode.to_string())))];
    for (k, channel) in channel_cmds::color_names(mode_of(mode)).iter().enumerate() {
        let options = sources.iter().map(|(id, name)| (id.to_string(), Label::from(name.clone()))).collect();
        let default = sources.get(k).map(|(id, _)| id.to_string());
        rows.push(Param::new(channel_key(k), format!("{}:", tl!(channel)), ParamKind::choice(options, default)));
    }
    rows
}

fn build(sources: Vec<Source>) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |f: &mut DialogFields| {
        let why = problem(&sources, f);
        let mut dialog = ParamDialog::new(COMMAND, tl!("Merge Channels"), rows(&sources, f)).ok_enabled(why.is_none());
        if let Some(why) = why {
            dialog = dialog.ok_reason(Label::from(tl!(why)));
        }
        dialog
    }
}

/// Opens Merge Channels, or `None` without a document.
pub fn open(app: &mut PhotocraftApp) -> Option<DialogId> {
    app.session.active()?;
    let sources = sources(app);
    let fields = open_fields(&sources);
    let captured = json!({ "sources": sources });

    Some(params::open_param(app, COMMAND, fields, captured, build(sources)))
}

/// The held OK's check: resolves the picks (a document closed or changed while the dialog was
/// open is refused) and runs the merge. The engine's refusal is the reason the dialog shows.
pub fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let sources = sources_of(&params::captured(app, &result.key));
    let merge = params(app, &sources, &result.fields).map_err(Label::from)?;
    app.run(COMMAND, merge).map(drop).map_err(Label::from)
}

/// OK: the merge ran in [`check`], so nothing is left to do.
pub fn run(_app: &mut PhotocraftApp, _ctx: &egui::Context, _result: ParamResult) {}

#[cfg(test)]
mod tests {
    use craft::ui::patterns::form_dialogs::HostedParamDialog;

    use super::*;

    fn app(docs: usize) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..docs {
            app.run("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        }
        app
    }

    fn id(app: &PhotocraftApp, index: usize) -> u64 {
        app.session.documents()[index].doc.id.0
    }

    fn values(app: &PhotocraftApp, dialog: DialogId) -> DialogFields {
        app.framework.dialogs.get_by_id::<HostedParamDialog>(dialog).expect("the dialog is open").fields().clone()
    }

    fn is_open(app: &PhotocraftApp) -> bool {
        app.framework.dialogs.is_open::<HostedParamDialog>()
    }

    fn pick(app: &mut PhotocraftApp, dialog: DialogId, channel: usize, doc: u64) {
        let ctx = app.ctx().clone();
        app.framework.dialogs.set_json_field(&ctx, dialog, &channel_key(channel), json!(doc.to_string())).unwrap();
    }

    /// The merged document's first pixel as 8-bit RGB.
    fn merged_pixel(app: &PhotocraftApp) -> [u8; 3] {
        let doc = &app.session.active().unwrap().doc;
        let px = photocraft_compose::render(doc, doc.bounds()).px[0];
        [px[0], px[1], px[2]].map(|v| (v * 255.0).round() as u8)
    }

    /// Three grayscale documents filled black, mid gray and white, then the active one is `active`.
    fn shaded(active: usize) -> Box<PhotocraftApp> {
        let mut app = app(3);
        for (i, gray) in ["#000000", "#808080", "#ffffff"].into_iter().enumerate() {
            app.session.set_active(i);
            app.run("edit.fill", json!({"color": gray})).unwrap();
        }
        app.session.set_active(active);
        app
    }

    #[test]
    fn fields_list_the_sources_by_id_and_params_take_their_indices() {
        let mut app = app(3);
        app.run("file.new", json!({"width": 9, "height": 8, "mode": "grayscale"})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "rgb"})).unwrap();
        app.session.set_active(2);
        let ids = [id(&app, 2), id(&app, 0), id(&app, 1)];
        let offered = sources(&app);
        assert_eq!(offered.iter().map(|s| s.0).collect::<Vec<_>>(), ids, "the active one first");
        let f = open_fields(&offered);
        assert_eq!(f.text("mode"), Some("rgb"));
        assert_eq!(picks(&f), ids, "one each, none repeated");
        assert_eq!(params(&app, &offered, &f).unwrap(), json!({"mode": "rgb", "documents": [2, 0, 1]}));
        assert!(open(&mut PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())).is_none());
    }

    /// A document picked for two channels greys OK with the reason; a refused OK keeps the dialog
    /// open and merges nothing.
    #[test]
    fn repeated_picks_grey_ok_and_keep_the_dialog_open() {
        let mut app = app(3);
        let ctx = app.ctx().clone();
        let (a, b) = (id(&app, 0), id(&app, 1));
        let dialog = open(&mut app).unwrap();
        let offered = sources(&app);
        pick(&mut app, dialog, 0, a);
        pick(&mut app, dialog, 1, a);
        pick(&mut app, dialog, 2, b);
        assert_eq!(problem(&offered, &values(&app, dialog)), Some(REPEATED));
        // A document the dialog did not offer.
        let host = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(dialog).unwrap();
        host.set_field(channel_key(1), FieldValue::Text("999999".into()));
        host.set_field(channel_key(0), FieldValue::Text(a.to_string()));
        assert_eq!(problem(&offered, &values(&app, dialog)), Some(REPEATED));
        assert!(crate::dialogs::confirm(&mut app, &ctx, dialog).is_err());
        assert!(is_open(&app), "the dialog stays open");
        assert_eq!(app.session.documents().len(), 3, "nothing merged or closed");
    }

    /// Documents moved while the dialog is open: OK still merges the ones picked, by id, into the
    /// channels picked for them.
    #[test]
    fn ok_merges_the_picked_documents_after_a_reorder() {
        let mut app = shaded(0);
        let ctx = app.ctx().clone();
        let (black, gray, white) = (id(&app, 0), id(&app, 1), id(&app, 2));
        let dialog = open(&mut app).unwrap();
        pick(&mut app, dialog, 0, black);
        pick(&mut app, dialog, 1, white);
        pick(&mut app, dialog, 2, gray);
        // A script reorders the tabs while the dialog is open: white first, black last.
        app.session.execute("document.move", json!({"document": 2, "to": 0})).unwrap();
        app.session.execute("document.move", json!({"document": 1, "to": 2})).unwrap();
        assert_eq!([id(&app, 0), id(&app, 1), id(&app, 2)], [white, gray, black]);
        crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
        assert_eq!((app.session.documents().len(), app.session.active().unwrap().doc.mode), (1, ColorMode::Rgb));
        assert_eq!(merged_pixel(&app), [0, 255, 128], "red, green and blue from the documents picked, not their old tabs");
    }

    /// A picked document closed while the dialog is open: OK refuses with the reason, keeps the
    /// dialog and closes nothing, even when another same-size grayscale document took its tab.
    #[test]
    fn a_closed_pick_is_refused_and_no_other_document_merges() {
        let mut app = shaded(0);
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        app.session.set_active(0);
        let spare = id(&app, 3);
        let dialog = open(&mut app).unwrap();
        app.session.execute("file.close", json!({"document": 1})).unwrap();
        assert_eq!(id(&app, 2), spare, "the spare document is where a picked one was");
        assert_eq!(crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap_err(), CLOSED);
        assert!(is_open(&app), "the dialog stays open");
        assert_eq!(app.session.documents().len(), 3, "nothing merged or closed");
        assert!(app.session.documents().iter().any(|d| d.doc.id.0 == spare));
    }

    /// A picked document turned into another mode while the dialog is open is refused too.
    #[test]
    fn a_changed_pick_is_refused_and_keeps_the_dialog() {
        let mut app = app(3);
        let ctx = app.ctx().clone();
        let dialog = open(&mut app).unwrap();
        app.run("image.mode.rgb", json!({})).unwrap();
        assert_eq!(crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap_err(), CHANGED);
        assert!(is_open(&app));
        assert_eq!(app.session.documents().len(), 3);
    }
}
