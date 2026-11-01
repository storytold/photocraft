//! Flatten prompt for HDR Toning (#10329). Image › Adjustments › HDR Toning works on the whole
//! image: its command flattens the document before tone mapping, as Photoshop's does, while every
//! other item in the Adjustments menu works on the active layer. So on a layered document the menu
//! item asks first, as Photoshop does, instead of silently replacing the layer stack. Yes flattens
//! the image (its own history state, so Undo brings the layers back) and opens the HDR Toning
//! dialog; No leaves the document alone.
//!
//! The prompt is an ordinary dialog in the UI state (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). A document with only a Background, or no layers, has nothing
//! to lose: its dialog opens directly. The command with parameters (an action, an agent) skips the
//! question and flattens, as Photoshop's action playback does.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__hdrFlatten";
/// The command the prompt guards.
pub const COMMAND: &str = "image.adjustments.hdrToning";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Whether HDR Toning would merge layers: the document holds anything but a lone Background.
pub fn layered(doc: &photocraft_doc::Document) -> bool {
    match doc.layers.as_slice() {
        [] => false,
        [only] => !doc.is_background(only.id),
        _ => true,
    }
}

/// Whether choosing HDR Toning on the active document should ask first.
pub fn needed(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| layered(&st.doc))
}

/// Open the prompt, or return the one already open. Yes flattens and opens the HDR Toning dialog.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields)) {
        return d.id;
    }
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("HDR Toning"));
    f.insert("message".into(), json!(tl!("HDR Toning works on the flattened image. Flatten the layers and continue?")));
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or(""));
}

/// Yes: flatten the image (one history state), then open the HDR Toning dialog.
pub fn confirm(app: &mut PhotocraftApp) -> Result<Value, String> {
    if needed(app) {
        app.run("layer.flattenImage", json!({}))?;
    }
    Ok(json!({"dialog": crate::filter_dialog::open(app, COMMAND)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(layers: usize) -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        for _ in 1..layers {
            s.execute("layer.new.layer", json!({})).unwrap();
        }
        PhotocraftApp::new(s, crate::Services::default())
    }

    fn layers(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().doc.layers.len()
    }

    fn choose(app: &mut PhotocraftApp, params: Value) -> u64 {
        crate::menus::invoke(app, &egui::Context::default(), COMMAND, params).unwrap()["dialog"].as_u64().unwrap()
    }

    #[test]
    fn a_layered_document_asks_before_flattening() {
        let mut app = app_with(2);
        let id = choose(&mut app, json!({}));
        assert!(owns(&app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields));
        assert_eq!(layers(&app), 2, "asking changes nothing");
        // Asking again while the question is up keeps the one prompt.
        assert_eq!(choose(&mut app, json!({})), id);
        assert_eq!(app.ui.dialogs.len(), 1);
        // No: the layers stay and nothing opens.
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!(layers(&app), 2);
        assert!(app.ui.dialogs.is_empty());
        // Yes: one undoable Flatten Image, then the HDR Toning dialog.
        let id = choose(&mut app, json!({}));
        let past = app.session.active().unwrap().history.past_len();
        let r = crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(layers(&app), 1);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        let hdr = r["dialog"].as_u64().unwrap();
        let d = app.ui.dialogs.iter().find(|d| d.id == hdr).unwrap();
        assert!(!owns(&d.fields));
        assert_eq!(d.fields["__command"], json!(COMMAND));
        app.session.undo();
        assert_eq!(layers(&app), 2, "undo brings the layers back");
    }

    #[test]
    fn a_lone_background_opens_the_dialog_directly() {
        let mut app = app_with(1);
        assert!(!needed(&app));
        let id = choose(&mut app, json!({}));
        let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
        assert!(!owns(&d.fields));
        assert_eq!(d.fields["__command"], json!(COMMAND));
    }

    #[test]
    fn parameters_skip_the_question() {
        let mut app = app_with(3);
        crate::menus::invoke(&mut app, &egui::Context::default(), COMMAND, json!({"strength": 1.0})).unwrap();
        assert_eq!(layers(&app), 1, "an action's HDR Toning flattens without asking, as in Photoshop");
        assert!(app.ui.dialogs.is_empty());
    }
}
