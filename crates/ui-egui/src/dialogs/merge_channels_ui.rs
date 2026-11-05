//! Channels panel › Merge Channels…: the mode (RGB, CMYK or Lab Color) and, for each of its
//! channels, the open grayscale document that becomes it. Photoshop asks in two steps (the mode,
//! then the documents); here both are one dialog. OK runs the engine's `channel.merge`, which
//! closes the sources; agents call it directly with params (#11451).

use photocraft_doc::ColorMode;
use photocraft_engine::channel_cmds;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub const COMMAND: &str = "channel.merge";
const MARK: &str = "__mergeChannels";
/// The most channels a mode has (CMYK).
const SLOTS: usize = 4;
const MODES: [(&str, &str); 3] = [("rgb", "RGB Color"), ("cmyk", "CMYK Color"), ("lab", "Lab Color")];

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

/// The documents that can be merged: the open grayscale ones of the active document's size, the
/// active one first, as the engine picks them without params.
fn sources(app: &PhotocraftApp) -> Vec<(usize, String)> {
    let Some(active) = app.session.active() else { return Vec::new() };
    let fits = |i: usize| app.session.documents().get(i).filter(|d| d.doc.mode == ColorMode::Grayscale && d.doc.size == active.doc.size);
    let current = app.session.active_index();
    let others = (0..app.session.documents().len()).filter(|i| Some(*i) != current);
    current.into_iter().chain(others).filter_map(|i| Some((i, fits(i)?.doc.name.clone()))).collect()
}

/// The dialog's fields: RGB Color, and the merge-able documents in order for the channels (the
/// first again where there are fewer than four), or `None` without a document.
pub fn fields(app: &PhotocraftApp) -> Option<Map<String, Value>> {
    app.session.active()?;
    let sources = sources(app);
    let picks: Vec<usize> = (0..SLOTS).filter_map(|k| sources.get(k).or_else(|| sources.first()).map(|(i, _)| *i)).collect();
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("Merge Channels…"));
    f.insert("__sources".into(), json!(sources));
    f.insert("mode".into(), json!("rgb"));
    f.insert("documents".into(), json!(picks));
    Some(f)
}

/// Opens Merge Channels, or `None` without a document.
pub fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let f = fields(app)?;
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, f))
}

/// The `channel.merge` params for the dialog's fields: the mode and one document per channel.
pub fn params(f: &Map<String, Value>) -> Value {
    let mode = f.get("mode").and_then(Value::as_str).unwrap_or("rgb");
    let n = mode_of(mode).color_channels();
    let docs: Vec<Value> = f.get("documents").and_then(Value::as_array).map(|d| d.iter().take(n).cloned().collect()).unwrap_or_default();
    json!({ "mode": mode, "documents": docs })
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.run(COMMAND, params(f))
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let mut mode = f.get("mode").and_then(Value::as_str).unwrap_or("rgb").to_string();
    let modes: Vec<(String, &str)> = MODES.iter().map(|(k, label)| (k.to_string(), *label)).collect();
    ui.horizontal(|ui| {
        ui.label(tl!("Mode:"));
        if crate::widgets::dropdown(ui, "merge-channels-mode", tl!("Mode:"), &mut mode, &modes, 160.0) {
            f.insert("mode".into(), json!(mode));
        }
    });
    crate::widgets::section_label(ui, tl!("Specify Channels:"));
    crate::widgets::hairline(ui);
    let sources: Vec<(usize, String)> = f.get("__sources").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let options: Vec<(usize, &str)> = sources.iter().map(|(i, name)| (*i, name.as_str())).collect();
    let mut picks: Vec<usize> = f.get("documents").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
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

    #[test]
    fn fields_list_the_sources_and_params_take_one_per_channel() {
        let mut app = app(3);
        app.run("file.new", json!({"width": 9, "height": 8, "mode": "grayscale"})).unwrap();
        app.run("file.new", json!({"width": 8, "height": 8, "mode": "rgb"})).unwrap();
        app.session.set_active(2);
        let f = fields(&app).unwrap();
        assert_eq!(f["__sources"].as_array().unwrap().iter().map(|s| s[0].as_u64().unwrap()).collect::<Vec<_>>(), [2, 0, 1], "the active one first");
        assert_eq!(f["documents"], json!([2, 0, 1, 2]), "the first again for a fourth channel");
        assert_eq!(params(&f), json!({"mode": "rgb", "documents": [2, 0, 1]}));
        let mut cmyk = f.clone();
        cmyk.insert("mode".into(), json!("cmyk"));
        assert_eq!(params(&cmyk)["documents"], json!([2, 0, 1, 2]));
        assert!(confirm(&mut app, &cmyk).is_err(), "a document listed twice is refused, nothing closes");
        assert_eq!(app.session.documents().len(), 5);
        assert!(fields(&PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())).is_none());
    }

    #[test]
    fn ok_merges_the_picked_documents_in_their_channels() {
        let mut app = app(3);
        for (i, gray) in ["#000000", "#808080", "#ffffff"].into_iter().enumerate() {
            app.session.set_active(i);
            app.run("edit.fill", json!({"color": gray})).unwrap();
        }
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
    fn body_renders_every_mode() {
        let app = app(3);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for mode in ["rgb", "cmyk", "lab"] {
            let mut f = fields(&app).unwrap();
            f.insert("mode".into(), json!(mode));
            ctx.run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
            assert!(owns(&f));
        }
    }
}
