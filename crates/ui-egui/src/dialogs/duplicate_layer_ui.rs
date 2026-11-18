//! Layer › Duplicate Layer… dialog (#3348): the copy's name and its destination document, as in
//! Photoshop. Only the menu, palette and Layers panel menu open it; ⌘J, Alt-drag and dragging onto
//! the New Layer button still duplicate at once.
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The command the dialog runs; also the dialog's key.
pub const KEY: &str = "layer.duplicate";

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == KEY
}

/// Where the copy goes.
enum Destination {
    /// A new document holding just the copy.
    New,
    /// The open document at this index.
    Document(usize),
}

/// The destination the `document` field names; one rule for the held OK's check and for the run.
fn destination(app: &PhotocraftApp, fields: &DialogFields) -> Result<Destination, String> {
    match fields.text("document") {
        Some("new") => Ok(Destination::New),
        Some(index) => index
            .parse::<usize>()
            .ok()
            .filter(|i| *i < app.session.documents().len())
            .map(Destination::Document)
            .ok_or_else(|| tl!("no destination document").to_string()),
        None => Err(tl!("no destination document").to_string()),
    }
}

/// Opens the dialog on the selected layers. The document and the selection are captured for [`check`].
pub fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let index = app.session.active_index().ok_or("no document")?;
    let st = app.session.active().ok_or("no document")?;
    let selected = st.selected_layers();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    let captured = json!({"document": st.doc.id.0, "selected": selected.iter().map(|id| id.0).collect::<Vec<_>>()});
    // Several selected layers keep their names, as in Photoshop's Duplicate Layers.
    let (title, name) = if selected.len() > 1 {
        (tl!("Duplicate Layers…"), None)
    } else if layer.is_group() {
        (tl!("Duplicate Group…"), Some(st.doc.copy_name(&layer.name)))
    } else {
        (tl!("Duplicate Layer…"), Some(st.doc.copy_name(&layer.name)))
    };
    let mut options: Vec<(String, Label)> = app.session.documents().iter().enumerate().map(|(i, d)| (i.to_string(), Label::from(d.doc.name.clone()))).collect();
    options.push(("new".to_string(), Label::from(tl!("New"))));
    Ok(params::open_param(app, KEY, DialogFields::new(), captured, move |fields: &mut DialogFields| {
        let mut rows = Vec::new();
        if let Some(name) = &name {
            rows.push(Param::new("name", tl!("As:"), ParamKind::text(name.clone())));
        }
        rows.push(Param::new("document", tl!("Document"), ParamKind::choice(options.clone(), Some(index.to_string()))));
        // Photoshop's document `Name` starts empty; the engine defaults it when empty, and it is only there for a new document.
        rows.push(Param::new("documentName", tl!("Name:"), ParamKind::text("")).visible_when("document", "new"));
        ParamDialog::command(KEY, title, rows).values(Bind::bound(fields))
    }))
}

/// Refuses an OK after the document, the selection or the destination changed, so the dialog stays open with the reason.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let captured = params::captured(app, KEY);
    let st = app.session.active().ok_or_else(|| Label::from(tl!("The document is closed")))?;
    let selected: Vec<u64> = st.selected_layers().iter().map(|id| id.0).collect();
    if captured["document"].as_u64() != Some(st.doc.id.0) || captured["selected"] != json!(selected) {
        return Err(Label::from(tl!("The document or selected layers changed. Choose Duplicate Layer again.")));
    }
    destination(app, &result.fields).map(|_| ()).map_err(Label::from)
}

/// OK: duplicates into the chosen document.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let outcome = duplicate(app, &result.fields);
    super::alerts::report(app, outcome);
}

fn duplicate(app: &mut PhotocraftApp, fields: &DialogFields) -> Result<Value, String> {
    let index = app.session.active_index().ok_or("The document is closed")?;
    let mut params = serde_json::Map::new();
    if let Some(name) = fields.text("name") {
        params.insert("name".into(), json!(name));
    }
    match destination(app, fields)? {
        Destination::New => {
            if let Some(doc_name) = fields.text("documentName").filter(|n| !n.is_empty()) {
                params.insert("documentName".into(), json!(doc_name));
            }
            params.insert("document".into(), json!("new"));
            app.run_command("layer.duplicate", Value::Object(params))
        }
        Destination::Document(destination) if destination == index => app.run_command("layer.duplicate", Value::Object(params)),
        Destination::Document(destination) => {
            params.insert("document".into(), json!(destination));
            app.run_command("layer.copyToDocument", Value::Object(params))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_layer_ui::testing::{set, top};

    fn make_app() -> Box<PhotocraftApp> {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"name":"Poster"})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Ink"})).unwrap();
        PhotocraftApp::new(s, crate::Services::default())
    }
    fn invoke(app: &mut PhotocraftApp) -> DialogId {
        let ctx = app.ctx().clone();
        crate::menus::invoke(app, &ctx, "layer.duplicate", json!({})).unwrap();
        top(app)
    }
    /// OK on `dialog`; the passes after it deliver the held answer to the family that runs it.
    fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: DialogId) -> Result<(), String> {
        let answered = crate::dialogs::confirm(app, ctx, dialog);
        for _ in 0..2 {
            crate::dialogs::testing::show(app);
        }
        answered
    }
    fn names(app: &PhotocraftApp, i: usize) -> Vec<String> {
        app.session.documents()[i].doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
    }

    #[test]
    fn the_menu_opens_the_dialog_and_ok_duplicates_under_the_typed_name() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        let dialog = invoke(&mut app);
        assert_eq!(app.session.active().unwrap().doc.layer_count(), 2, "nothing is copied until OK");
        set(&mut app, dialog, "name", json!("Shadow")).unwrap();
        confirm(&mut app, &ctx, dialog).unwrap();
        assert!(names(&app, 0).contains(&"Shadow".to_string()));
        assert_eq!(app.session.active().unwrap().doc.layer_count(), 3);
    }

    #[test]
    fn another_destination_document_receives_the_copy() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        app.session.execute("file.new", json!({"width":8,"height":8,"name":"Card"})).unwrap();
        app.session.set_active(0);
        let dialog = invoke(&mut app);
        set(&mut app, dialog, "document", json!("1")).unwrap();
        confirm(&mut app, &ctx, dialog).unwrap();
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2);
        assert!(names(&app, 1).contains(&"Ink copy".to_string()));
    }

    #[test]
    fn a_new_destination_makes_a_named_document_holding_just_the_copy() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        let dialog = invoke(&mut app);
        set(&mut app, dialog, "document", json!("new")).unwrap();
        set(&mut app, dialog, "documentName", json!("Card")).unwrap();
        confirm(&mut app, &ctx, dialog).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.documents()[1].doc.name, "Card");
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2, "the source is untouched");
        assert!(names(&app, 1).contains(&"Ink copy".to_string()));
    }

    #[test]
    fn a_new_destination_without_a_name_defaults_it_and_refuses_a_dead_index() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        let dialog = invoke(&mut app);
        set(&mut app, dialog, "document", json!("new")).unwrap();
        confirm(&mut app, &ctx, dialog).unwrap();
        assert_eq!(app.session.documents()[1].doc.name, "Poster copy");
        let dialog = invoke(&mut app);
        // Either the select refuses the index or the held OK does.
        if set(&mut app, dialog, "document", json!("7")).is_ok() {
            assert!(confirm(&mut app, &ctx, dialog).is_err());
        }
        assert_eq!(app.session.documents().len(), 2);
    }

    #[test]
    fn a_changed_document_refuses_and_keeps_the_dialog() {
        let mut app = make_app();
        let ctx = app.ctx().clone();
        let dialog = invoke(&mut app);
        app.session.execute("file.new", json!({"width":8,"height":8})).unwrap();
        assert!(confirm(&mut app, &ctx, dialog).is_err());
        assert!(app.framework.dialogs.stack().contains(dialog), "the dialog stays open");
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2, "nothing was copied");
    }
}
