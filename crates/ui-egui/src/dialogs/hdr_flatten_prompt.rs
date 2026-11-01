//! Flatten prompt for HDR Toning (#10329). Image › Adjustments › HDR Toning works on the whole
//! image: its command flattens the document before tone mapping, as Photoshop's does, while every
//! other item in the Adjustments menu works on the active layer. So on a layered document the menu
//! item asks first, as Photoshop does, instead of silently replacing the layer stack. Yes flattens
//! the image (its own history state, so Undo brings the layers back) and opens the HDR Toning
//! dialog; No leaves the document alone.
//!
//! The prompt is an ordinary dialog in the UI state (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). It is bound to the document it was asked about: when another
//! document is active by the time it is answered (`ui.window.open`, say), Yes is refused and
//! nothing is flattened. A document with only a Background, or no layers, has nothing to lose: its
//! dialog opens directly. The command with parameters (an action, an agent) skips the
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

/// The document a prompt was asked about.
fn document_of(fields: &Map<String, Value>) -> Option<u64> {
    fields.get("document").and_then(Value::as_u64)
}

/// Open the prompt for the active document, or return the one already open for it (a prompt left
/// over from another document is dropped). Yes flattens and opens the HDR Toning dialog.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let document = app.session.active().map(|st| st.doc.id.0);
    if let Some(d) = app.ui.dialogs.iter().find(|d| owns(&d.fields) && document_of(&d.fields) == document) {
        return d.id;
    }
    app.ui.dialogs.retain(|d| !owns(&d.fields));
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("HDR Toning"));
    f.insert("message".into(), json!(tl!("HDR Toning works on the flattened image. Flatten the layers and continue?")));
    f.insert("document".into(), json!(document));
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn body(ui: &mut egui::Ui, fields: &Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or(""));
}

/// Yes: flatten the image (one history state), then open the HDR Toning dialog. Refused, with
/// nothing flattened, when the document the question was about is no longer the active one.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let active = app.session.active().map(|st| st.doc.id.0);
    if active.is_none() || document_of(fields) != active {
        return Err(tl!("The document changed. Choose HDR Toning again.").into());
    }
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

    /// The prompt is bound to its document: with another one active (a new document, a window
    /// opened for another through `ui.window.open`), Yes flattens nothing.
    #[test]
    fn answering_for_another_document_flattens_nothing() {
        let mut app = app_with(2);
        let first = app.session.active().unwrap().doc.id;
        let id = choose(&mut app, json!({}));
        assert_eq!(app.ui.dialogs[0].fields["document"], json!(first.0));
        app.session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        assert_ne!(app.session.active().unwrap().doc.id, first);
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(app.ui.dialogs.is_empty(), "the stale question is gone");
        for d in app.session.documents() {
            assert_eq!(d.doc.layers.len(), 2, "{:?} keeps its layers", d.doc.id);
        }
        // Asking on the second document replaces a prompt left over from the first.
        app.session.set_active(0);
        let stale = choose(&mut app, json!({}));
        app.session.set_active(1);
        let fresh = choose(&mut app, json!({}));
        assert_ne!(stale, fresh);
        assert_eq!(app.ui.dialogs.len(), 1);
        assert_eq!(app.ui.dialogs[0].fields["document"], json!(app.session.active().unwrap().doc.id.0));
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
