//! Layer › Duplicate Layer… dialog (#3348): the copy's name and its destination document, as in
//! Photoshop. Only the menu, palette and Layers panel menu open it; ⌘J, Alt-drag and dragging onto
//! the New Layer button still duplicate at once.
use crate::{PhotocraftApp, state::DialogKind};
use serde_json::{Map, Value, json};
const MARK: &str = "__duplicateLayer";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

pub fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    let index = app.session.active_index().ok_or("no document")?;
    let st = app.session.active().ok_or("no document")?;
    let selected = st.selected_layers();
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    let mut fields = Map::new();
    fields.insert(MARK.into(), json!(true));
    fields.insert("__command".into(), json!("layer.duplicate"));
    fields.insert("destination".into(), json!(index));
    // Photoshop's document-`Name` starts empty; the engine defaults it when empty.
    fields.insert("documentName".into(), json!(""));
    fields.insert("document".into(), json!(st.doc.id.0));
    fields.insert("selected".into(), json!(selected.iter().map(|id| id.0).collect::<Vec<_>>()));
    // Several selected layers keep their names, as in Photoshop's Duplicate Layers.
    if selected.len() > 1 {
        fields.insert("__label".into(), json!("Duplicate Layers…"));
    } else if layer.is_group() {
        fields.insert("__label".into(), json!("Duplicate Group…"));
        fields.insert("name".into(), json!(st.doc.copy_name(&layer.name)));
    } else {
        fields.insert("__label".into(), json!("Duplicate Layer…"));
        fields.insert("name".into(), json!(st.doc.copy_name(&layer.name)));
    }
    let id = app.ui.open_dialog(DialogKind::Command, fields);
    Ok(json!({"dialog": id}))
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let documents: Vec<(String, String)> = app.session.documents().iter().enumerate().map(|(i, d)| (i.to_string(), d.doc.name.clone())).collect();
    let mut options: Vec<(String, &str)> = documents.iter().map(|(k, n)| (k.clone(), n.as_str())).collect();
    options.push(("new".to_string(), tl!("New")));
    let mut destination = match fields.get("destination") {
        Some(Value::String(s)) if s == "new" => "new".to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    if crate::text::direction::is_rtl() {
        // The fork's `egui::Grid` ignores the direction: its cells always advance to the right,
        // so in a right-to-left UI it would render unmirrored. The same rows are drawn by hand
        // in reading order instead: the label at the reading start, the field beside it.
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 6.0);
        if matches!(fields.get("name"), Some(Value::String(_))) {
            crate::text::direction::dialog_row(ui, |ui| name_row(ui, fields));
        }
        crate::text::direction::dialog_row(ui, |ui| {
            ui.label(egui::RichText::new(tl!("Destination")).font(crate::theme::semibold(13.0)));
        });
        crate::text::direction::dialog_row(ui, |ui| document_row(ui, &mut destination, &options));
        // Photoshop's document-`Name` is only there when the destination is a new document.
        if destination == "new" {
            crate::text::direction::dialog_row(ui, |ui| document_name_row(ui, fields));
        }
    } else {
        egui::Grid::new("duplicate-layer").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if matches!(fields.get("name"), Some(Value::String(_))) {
                name_row(ui, fields);
                ui.end_row();
            }
            ui.label(egui::RichText::new(tl!("Destination")).font(crate::theme::semibold(13.0)));
            ui.end_row();
            document_row(ui, &mut destination, &options);
            ui.end_row();
            // Photoshop's document-`Name` is only there when the destination is a new document.
            if destination == "new" {
                document_name_row(ui, fields);
                ui.end_row();
            }
        });
    }
    if destination == "new" {
        fields.insert("destination".into(), json!("new"));
    } else if let Ok(i) = destination.parse::<u64>() {
        fields.insert("destination".into(), json!(i));
    }
}

/// A single-line edit whose text reads right-to-left in a right-to-left UI (Arabic names align
/// to the field's right edge); a left-to-right UI keeps egui's default edit, unchanged.
/// The copy's name: the label names the field for accessibility.
fn name_row(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut name = fields.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    let name_label = ui.label(tl!("Name:"));
    crate::widgets::rtl_singleline_add(ui, &mut name, |e| e.desired_width(260.0)).labelled_by(name_label.id);
    fields.insert("name".into(), json!(name));
}

/// The Document row: the destination document, or "New" for a fresh one.
fn document_row(ui: &mut egui::Ui, destination: &mut String, options: &[(String, &str)]) {
    ui.label(tl!("Document"));
    crate::widgets::dropdown(ui, "duplicate-layer-document", tl!("Document"), destination, options, 260.0);
}

/// The new document's name, shown only when the destination is a new document.
fn document_name_row(ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let mut doc_name = fields.get("documentName").and_then(Value::as_str).unwrap_or("").to_string();
    let name_label = ui.label(tl!("Name:"));
    crate::widgets::rtl_singleline_add(ui, &mut doc_name, |e| e.desired_width(260.0)).labelled_by(name_label.id);
    fields.insert("documentName".into(), json!(doc_name));
}

pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let index = app.session.active_index().ok_or("The document is closed")?;
    let st = app.session.active().ok_or("The document is closed")?;
    if fields.get("document").and_then(Value::as_u64) != Some(st.doc.id.0)
        || fields.get("selected") != Some(&json!(st.selected_layers().iter().map(|id| id.0).collect::<Vec<_>>()))
    {
        return Err("The document or selected layers changed. Choose Duplicate Layer again.".into());
    }
    let mut params = Map::new();
    if let Some(name) = fields.get("name").and_then(Value::as_str) {
        params.insert("name".into(), json!(name));
    }
    match fields.get("destination") {
        Some(Value::String(s)) if s == "new" => {
            if let Some(doc_name) = fields.get("documentName").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                params.insert("documentName".into(), json!(doc_name));
            }
            params.insert("document".into(), json!("new"));
            app.run_command("layer.duplicate", Value::Object(params))
        }
        Some(Value::Number(n)) => {
            let destination =
                n.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < app.session.documents().len()).ok_or("no destination document")?;
            if destination == index {
                app.run_command("layer.duplicate", Value::Object(params))
            } else {
                params.insert("document".into(), json!(destination));
                app.run_command("layer.copyToDocument", Value::Object(params))
            }
        }
        _ => Err("no destination document".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_app() -> PhotocraftApp {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"name":"Poster"})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Ink"})).unwrap();
        PhotocraftApp::new(s, crate::Services::default())
    }
    fn invoke(app: &mut PhotocraftApp) -> u64 {
        let ctx = egui::Context::default();
        crate::menus::invoke(app, &ctx, "layer.duplicate", json!({})).unwrap()["dialog"].as_u64().unwrap()
    }
    fn names(app: &PhotocraftApp, i: usize) -> Vec<String> {
        app.session.documents()[i].doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
    }

    #[test]
    fn the_menu_opens_the_dialog_and_ok_duplicates_under_the_typed_name() {
        let mut app = make_app();
        let dialog = invoke(&mut app);
        assert_eq!(app.session.active().unwrap().doc.layer_count(), 2, "nothing is copied until OK");
        assert_eq!(app.ui.dialogs[0].fields["name"], "Ink copy");
        app.ui.dialog_mut(dialog).unwrap().fields.insert("name".into(), json!("Shadow"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(names(&app, 0).contains(&"Shadow".to_string()));
        assert_eq!(app.session.active().unwrap().doc.layer_count(), 3);
    }

    #[test]
    fn another_destination_document_receives_the_copy() {
        let mut app = make_app();
        app.session.execute("file.new", json!({"width":8,"height":8,"name":"Card"})).unwrap();
        app.session.set_active(0);
        let dialog = invoke(&mut app);
        app.ui.dialog_mut(dialog).unwrap().fields.insert("destination".into(), json!(1));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2);
        assert!(names(&app, 1).contains(&"Ink copy".to_string()));
    }

    #[test]
    fn a_new_destination_makes_a_named_document_holding_just_the_copy() {
        let mut app = make_app();
        let dialog = invoke(&mut app);
        let fields = &mut app.ui.dialog_mut(dialog).unwrap().fields;
        fields.insert("destination".into(), json!("new"));
        fields.insert("documentName".into(), json!("Card"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.documents()[1].doc.name, "Card");
        assert_eq!(app.session.documents()[0].doc.layer_count(), 2, "the source is untouched");
        assert!(names(&app, 1).contains(&"Ink copy".to_string()));
    }

    #[test]
    fn a_new_destination_without_a_name_defaults_it_and_refuses_a_dead_index() {
        let mut app = make_app();
        let dialog = invoke(&mut app);
        app.ui.dialog_mut(dialog).unwrap().fields.insert("destination".into(), json!("new"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.session.documents()[1].doc.name, "Poster copy");
        let dialog = invoke(&mut app);
        app.ui.dialog_mut(dialog).unwrap().fields.insert("destination".into(), json!(7));
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
        assert_eq!(app.session.documents().len(), 2);
    }

    #[test]
    fn a_single_group_gets_the_group_title() {
        let mut app = make_app();
        app.session.execute("layer.new.group", json!({"name": "Pack"})).unwrap();
        let _dialog = invoke(&mut app);
        assert_eq!(app.ui.dialogs[0].fields["__label"], "Duplicate Group…");
    }

    #[test]
    fn a_changed_document_refuses_and_direct_runs_stay_dialog_free() {
        let mut app = make_app();
        let dialog = invoke(&mut app);
        app.session.execute("file.new", json!({"width":8,"height":8})).unwrap();
        assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
        app.session.set_active(0);
        app.run("layer.duplicate", json!({})).unwrap();
        app.run("layer.new.layerViaCopy", json!({})).unwrap();
        assert!(app.ui.dialogs.is_empty());
        assert_eq!(app.session.active().unwrap().doc.layer_count(), 4);
    }
}

/// The Duplicate Layer body mirrors in a right-to-left UI: the captions read from the right and
/// every field follows its caption, with the dialog as wide as its left-to-right twin.
#[cfg(test)]
mod rtl_tests {
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;

    /// The Duplicate Layer… dialog for the document's layer, rendered in `dir`.
    fn duplicate_layer(dir: crate::text::direction::Dir) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let _dir = crate::text::direction::direction_scope(dir);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        open(h.state_mut()).unwrap();
        h.run_steps(4);
        h
    }

    /// The dialog's own frame, as the host sized it.
    fn frame(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> egui::Rect {
        let id = h.state().ui.dialogs.last().unwrap().id;
        h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog")
    }

    /// The body reads from the left in a left-to-right UI and from the right in a right-to-left
    /// one: the Destination header starts at the reading start and the name field follows its
    /// caption towards the reading end.
    #[test]
    fn rtl_the_duplicate_layer_body_mirrors() {
        let ltr = duplicate_layer(crate::text::direction::Dir::Ltr);
        let ltr_frame = frame(&ltr);
        let ltr_header = ltr.get_by_label("Destination").rect();
        let ltr_name = ltr.get_by_label("Name:").rect();
        assert!(ltr_header.left() - ltr_frame.left() < 40.0, "the header reads from the left: {ltr_header:?} in {ltr_frame:?}");
        assert!(ltr_name.left() > ltr_header.left(), "the name field follows its caption: {ltr_name:?} vs {ltr_header:?}");

        let rtl = duplicate_layer(crate::text::direction::Dir::Rtl);
        let rtl_frame = frame(&rtl);
        let rtl_header = rtl.get_by_label("Destination").rect();
        let rtl_name = rtl.get_by_label("Name:").rect();
        assert!(rtl_frame.right() - rtl_header.right() < 40.0, "the header reads from the right: {rtl_header:?} in {rtl_frame:?}");
        assert!(rtl_name.right() < rtl_header.right(), "the name field follows its caption: {rtl_name:?} vs {rtl_header:?}");
        assert!(rtl_frame.right() - rtl_name.right() < 120.0, "the field hugs the reading start: {rtl_name:?} in {rtl_frame:?}");
    }

    /// The mirrored body keeps the dialog as wide as its left-to-right twin.
    #[test]
    fn rtl_the_duplicate_layer_dialog_keeps_its_width() {
        let width = |dir| frame(&duplicate_layer(dir)).width();
        let ltr = width(crate::text::direction::Dir::Ltr);
        let rtl = width(crate::text::direction::Dir::Rtl);
        assert!((ltr - rtl).abs() < 1.0, "LTR {ltr} vs RTL {rtl}");
    }

    /// The dialog body's caption texts, in the order the rows add them.
    const CAPTIONS: [&str; 3] = ["Name:", "Destination", "Document"];

    /// The dialog's captions in reading order: every row top to bottom, each from its reading
    /// start (`rtl` flips only the in-row order; the rows are vertical in both directions).
    /// A label-role node carries its text as its value, not its label.
    fn captions(h: &egui_kittest::Harness<'static, PhotocraftApp>, rtl: bool) -> Vec<String> {
        let f = frame(h);
        let mut rows: Vec<(f32, f32, String)> = h
            .query_all_by_role(egui::accesskit::Role::Label)
            .filter_map(|n| {
                let r = n.rect();
                let a = n.accesskit_node();
                let text = a.value().or_else(|| a.label())?;
                f.contains(r.center()).then_some((r.top(), if rtl { -r.left() } else { r.left() }, text))
            })
            .collect();
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        rows.into_iter().map(|(_, _, t)| t).collect()
    }

    /// The body's captions, without any other label the frame may overlap.
    fn body_captions(c: &[String]) -> Vec<&str> {
        c.iter().map(String::as_str).filter(|t| CAPTIONS.contains(t)).collect()
    }

    /// The row sequence is written once per direction; this twin pins the two together: the same
    /// captions in the same reading order in both, and choosing the destination "New" in the
    /// Document dropdown adds the second "Name:" row under RTL just as under LTR.
    #[test]
    fn rtl_the_caption_sequence_matches_the_ltr_twin_including_the_new_document_row() {
        let captions_in = |dir| captions(&duplicate_layer(dir), dir == crate::text::direction::Dir::Rtl);
        let ltr = captions_in(crate::text::direction::Dir::Ltr);
        let rtl = captions_in(crate::text::direction::Dir::Rtl);
        assert_eq!(body_captions(&ltr), ["Name:", "Destination", "Document"], "the LTR sequence");
        assert_eq!(body_captions(&rtl), body_captions(&ltr), "the RTL twin repeats the captions in the same reading order");

        let drive_new = |dir| {
            let mut h = duplicate_layer(dir);
            // Press the box, drag onto the New option, release there: the dropdown's
            // press-drag-release gesture (#2735).
            let box_at = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Document").rect().center();
            h.hover_at(box_at);
            h.run_steps(1);
            h.drag_at(box_at);
            h.run_steps(3);
            let new_at = h.query_by_label("New").expect("the list opened on the press").rect().center();
            h.hover_at(new_at);
            h.run_steps(1);
            h.drop_at(new_at);
            h.run_steps(3);
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["destination"], "new", "releasing on New picked it");
            h
        };
        let ltr = captions(&drive_new(crate::text::direction::Dir::Ltr), false);
        let rtl = captions(&drive_new(crate::text::direction::Dir::Rtl), true);
        assert_eq!(body_captions(&ltr), ["Name:", "Destination", "Document", "Name:"], "LTR gains the document-name row");
        assert_eq!(body_captions(&rtl), body_captions(&ltr), "RTL gains the second Name: row in the same place");
    }
}
