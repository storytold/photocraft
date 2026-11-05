//! Channels panel › Merge Channels…: the mode (RGB, CMYK or Lab Color) and, for each of its
//! channels, the open grayscale document that becomes it. Photoshop asks in two steps (the mode,
//! then the documents); here both are one dialog. Only the modes the open documents can fill are
//! offered (RGB and Lab need three, CMYK four), OK waits for a different document per channel,
//! and a merge that still fails opens the dialog again with the reason. OK runs the engine's
//! `channel.merge`, which closes the sources; agents call it directly with params (#11451).

use photocraft_doc::ColorMode;
use photocraft_engine::channel_cmds;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "channel.merge";
const MARK: &str = "__mergeChannels";
/// The most channels a mode has (CMYK).
const SLOTS: usize = 4;
const MODES: [(&str, &str); 3] = [("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];
const TOO_FEW: &str = "Not enough grayscale documents for this mode.";
const REPEATED: &str = "Pick a different document for each channel.";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
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

/// The documents that can be merged: the open grayscale ones of the active document's size, the
/// active one first, as the engine picks them without params.
fn sources(app: &PhotocraftApp) -> Vec<(usize, String)> {
    let Some(active) = app.session.active() else { return Vec::new() };
    let fits = |i: usize| app.session.documents().get(i).filter(|d| d.doc.mode == ColorMode::Grayscale && d.doc.size == active.doc.size);
    let current = app.session.active_index();
    let others = (0..app.session.documents().len()).filter(|i| Some(*i) != current);
    current.into_iter().chain(others).filter_map(|i| Some((i, fits(i)?.doc.name.clone()))).collect()
}

fn mode_field(f: &Map<String, Value>) -> &str {
    f.get("mode").and_then(Value::as_str).unwrap_or("rgb")
}

fn sources_field(f: &Map<String, Value>) -> Vec<(usize, String)> {
    f.get("__sources").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

fn picks_field(f: &Map<String, Value>) -> Vec<usize> {
    f.get("documents").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// The dialog's fields: the first mode the documents can fill (RGB Color from three) and the
/// merge-able documents in order, one per channel, or `None` without a document.
pub fn fields(app: &PhotocraftApp) -> Option<Map<String, Value>> {
    app.session.active()?;
    let sources = sources(app);
    let picks: Vec<usize> = sources.iter().take(SLOTS).map(|(i, _)| *i).collect();
    let mode = modes(sources.len()).first().map_or_else(|| "rgb".to_string(), |(key, _)| key.clone());
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Merge Channels…"));
    f.insert("__sources".into(), json!(sources));
    f.insert("mode".into(), json!(mode));
    f.insert("documents".into(), json!(picks));
    Some(f)
}

/// Opens Merge Channels, or `None` without a document.
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let f = fields(app)?;
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// Why OK is greyed (English, for `tl!`): the mode needs more documents than are open, or the
/// picks repeat a document or name one the dialog did not offer. `None` when OK can merge, and
/// for another dialog's fields.
pub fn problem(f: &Map<String, Value>) -> Option<&'static str> {
    if !owns(f) {
        return None;
    }
    let n = mode_of(mode_field(f)).color_channels();
    let sources = sources_field(f);
    let picks: Vec<usize> = picks_field(f).into_iter().take(n).collect();
    if sources.len() < n || picks.len() < n {
        return Some(TOO_FEW);
    }
    let mut seen = std::collections::HashSet::new();
    let offered = |p: &usize| sources.iter().any(|(i, _)| i == p);
    if picks.iter().any(|p| !seen.insert(*p) || !offered(p)) {
        return Some(REPEATED);
    }
    None
}

/// Whether OK is live: always for other dialogs, here while [`problem`] finds none.
pub fn can_confirm(f: &Map<String, Value>) -> bool {
    problem(f).is_none()
}

/// Checked before the dialog closes (`dialogs::confirm_preflight`), so a refused OK keeps it
/// open: the picks are distinct and still open grayscale documents of the active one's size.
pub fn validate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(), String> {
    if let Some(why) = problem(f) {
        return Err(why.to_string());
    }
    let open: Vec<usize> = sources(app).iter().map(|(i, _)| *i).collect();
    let n = mode_of(mode_field(f)).color_channels();
    if picks_field(f).iter().take(n).any(|p| !open.contains(p)) {
        return Err("a picked document is no longer open as a grayscale document of this size".into());
    }
    Ok(())
}

/// The `channel.merge` params for the dialog's fields: the mode and one document per channel.
pub fn params(f: &Map<String, Value>) -> Value {
    let mode = mode_field(f);
    let n = mode_of(mode).color_channels();
    let docs: Vec<usize> = picks_field(f).into_iter().take(n).collect();
    json!({ "mode": mode, "documents": docs })
}

/// Runs the merge. Should the engine still refuse it, the dialog opens again with its picks and
/// the reason, rather than going away.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let result = app.run(COMMAND, params(f));
    if let Err(e) = &result {
        let mut again = f.clone();
        again.insert("__error".into(), json!(e));
        app.ui.open_dialog(crate::state::DialogKind::Command, again);
    }
    result
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let sources = sources_field(f);
    let mut mode = mode_field(f).to_string();
    let modes = modes(sources.len());
    ui.horizontal(|ui| {
        ui.label(tl!("Mode:"));
        if crate::widgets::dropdown(ui, "merge-channels-mode", tl!("Mode:"), &mut mode, &modes, 160.0) {
            f.insert("mode".into(), json!(mode));
            f.remove("__error");
        }
    });
    crate::widgets::section_label(ui, tl!("Specify Channels:"));
    crate::widgets::hairline(ui);
    let options: Vec<(usize, &str)> = sources.iter().map(|(i, name)| (*i, name.as_str())).collect();
    let mut picks = picks_field(f);
    let mut changed = false;
    for (k, channel) in channel_cmds::color_names(mode_of(&mode)).iter().enumerate() {
        let Some(pick) = picks.get_mut(k) else { break };
        ui.horizontal(|ui| {
            let caption = format!("{}:", tl!(channel));
            ui.add_sized([84.0, 22.0], egui::Label::new(&caption));
            changed |= crate::widgets::dropdown_literal(ui, &format!("merge-channels-{k}"), &caption, pick, &options, 200.0);
        });
    }
    if changed {
        f.insert("documents".into(), json!(picks));
        f.remove("__error");
    }
    // Why OK is greyed, or why the last OK was refused (in the engine's words).
    let why = problem(f).map(|w| tl!(w).to_string()).or_else(|| f.get("__error").and_then(Value::as_str).map(str::to_string));
    if let Some(why) = why {
        ui.label(egui::RichText::new(why).color(Tokens::get(ui.ctx()).danger).size(11.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(docs: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..docs {
            app.run("file.new", json!({"width": 8, "height": 8, "mode": "grayscale"})).unwrap();
        }
        app
    }

    fn offered(f: &Map<String, Value>) -> Vec<String> {
        modes(sources_field(f).len()).into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn fields_list_the_sources_and_params_take_one_per_channel() {
        let mut app = app(3);
        app.run("file.new", json!({"width": 9, "height": 8, "mode": "grayscale"})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "rgb"})).unwrap();
        app.session.set_active(2);
        let f = fields(&app).unwrap();
        assert_eq!(f["__sources"].as_array().unwrap().iter().map(|s| s[0].as_u64().unwrap()).collect::<Vec<_>>(), [2, 0, 1], "the active one first");
        assert_eq!((f["mode"].clone(), f["documents"].clone()), (json!("rgb"), json!([2, 0, 1])), "one each, none repeated");
        assert_eq!(params(&f), json!({"mode": "rgb", "documents": [2, 0, 1]}));
        assert!(fields(&PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())).is_none());
    }

    /// Three documents fill RGB and Lab, not CMYK; four fill CMYK too.
    #[test]
    fn only_the_modes_the_documents_fill_are_offered() {
        let f = fields(&app(3)).unwrap();
        assert_eq!(offered(&f), ["rgb", "lab"]);
        assert!(can_confirm(&f));
        let mut cmyk = f.clone();
        cmyk.insert("mode".into(), json!("cmyk"));
        assert_eq!(problem(&cmyk), Some(TOO_FEW), "CMYK from three documents (as automation could ask) greys OK");
        let f = fields(&app(4)).unwrap();
        assert_eq!(offered(&f), ["rgb", "cmyk", "lab"]);
        let mut cmyk = f.clone();
        cmyk.insert("mode".into(), json!("cmyk"));
        assert!(can_confirm(&cmyk));
        assert_eq!(params(&cmyk)["documents"], json!([3, 0, 1, 2]), "the active (last opened) one first");
    }

    /// A document picked for two channels greys OK with the reason; a refused OK keeps the dialog
    /// open and merges nothing.
    #[test]
    fn repeated_picks_grey_ok_and_keep_the_dialog_open() {
        let mut app = app(3);
        let id = open(&mut app).unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("documents".into(), json!([0, 0, 1]));
        assert_eq!(problem(&d.fields), Some(REPEATED));
        assert!(!can_confirm(&d.fields));
        d.fields.insert("documents".into(), json!([0, 1, 7]));
        assert_eq!(problem(&d.fields), Some(REPEATED), "a document the dialog did not offer");
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.iter().any(|d| d.id == id), "the dialog stays open");
        assert_eq!(app.session.documents().len(), 3, "nothing merged or closed");
        assert!(can_confirm(&Map::new()), "other dialogs are not affected");
    }

    /// Should the engine refuse a merge the dialog allowed, the dialog comes back with the reason.
    #[test]
    fn a_refused_merge_opens_the_dialog_again_with_the_reason() {
        let mut app = app(3);
        let f = fields(&app).unwrap();
        app.run("image.mode.rgb", json!({})).unwrap();
        assert!(validate(&app, &f).is_err(), "the active document is no longer grayscale");
        assert!(confirm(&mut app, &f).is_err());
        let again = &app.ui.dialogs.last().expect("the dialog again").fields;
        assert!(owns(again) && again["__error"].is_string());
        assert_eq!(app.session.documents().len(), 3);
    }

    #[test]
    fn ok_merges_the_picked_documents_in_their_channels() {
        let mut app = app(3);
        for (i, gray) in ["#000000", "#808080", "#ffffff"].into_iter().enumerate() {
            app.session.set_active(i);
            app.run("edit.fill", json!({"color": gray})).unwrap();
        }
        app.session.set_active(0);
        let id = open(&mut app).unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        assert!(owns(&d.fields));
        d.fields.insert("documents".into(), json!([0, 2, 1]));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((app.session.documents().len(), doc.mode), (1, ColorMode::Rgb));
        let px = photocraft_compose::render(doc, doc.bounds()).px[0];
        assert_eq!([px[0], px[1], px[2]].map(|v| (v * 255.0).round() as u8), [0, 255, 128], "red, green and blue from the picks");
    }

    #[test]
    fn body_renders_every_mode_and_the_reason() {
        let app = app(4);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (mode, docs) in [("rgb", json!([0, 1, 2])), ("cmyk", json!([0, 1, 2, 2])), ("lab", json!([3, 2, 1]))] {
            let mut f = fields(&app).unwrap();
            f.insert("mode".into(), json!(mode));
            f.insert("documents".into(), docs);
            ctx.run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
            assert!(owns(&f));
        }
    }
}
