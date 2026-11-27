//! File Info's app-owned pages. The framework owns the dialog, input and field controls; this
//! module keeps the metadata draft and applies it to the document the dialog opened on.

use std::borrow::Cow;

use craft::behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    ActivateOn, Bind, CollectionEvent, Lazy, Props as _, SaveKey, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::Divider;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{DialogFields, FormDialog, Param, ParamKind, ParamRows, set_param_field};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::{saved_value, set_saved_value};
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "file.fileInfo";
const LAST_CATEGORY: SaveKey = SaveKey::new("photocraft.fileInfo.category");
const BASIC_KEYS: [&str; 8] = ["title", "author", "authorTitle", "description", "keywords", "copyrightStatus", "copyright", "copyrightUrl"];

struct Category {
    key: &'static str,
    label: &'static str,
    tooltip: Option<&'static str>,
}

const CATEGORIES: [Category; 12] = [
    Category { key: "basic", label: "Basic", tooltip: None },
    Category { key: "cameraData", label: "Camera Data", tooltip: Some("Camera Data :: Contains the most important TIFF and Exif properties.") },
    Category { key: "origin", label: "Origin", tooltip: Some("Origin :: Displays information about the origin of the asset") },
    Category { key: "iptc", label: "IPTC", tooltip: Some("IPTC :: International Press Telecommunications Council Core schema properties") },
    Category { key: "iptcExtension", label: "IPTC Extension", tooltip: None },
    Category { key: "gpsData", label: "GPS Data", tooltip: None },
    Category { key: "audioData", label: "Audio Data", tooltip: None },
    Category { key: "videoData", label: "Video Data", tooltip: None },
    Category { key: "history", label: "History", tooltip: None },
    Category { key: "dicom", label: "DICOM", tooltip: None },
    Category { key: "assetTags", label: "Asset Tags", tooltip: None },
    Category { key: "rawData", label: "Raw Data", tooltip: Some("Raw Data :: Displays the XMP packet in raw RDF/XML form.") },
];

struct Categories;

impl DisplayOrder<String> for Categories {
    fn len(&self) -> usize {
        CATEGORIES.len()
    }

    fn key(&self, at: usize) -> String {
        CATEGORIES.get(at).map_or_else(String::new, |category| category.key.to_owned())
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        CATEGORIES.get(at).map(|category| Cow::Borrowed(category.label))
    }
}

impl ItemSource<String> for Categories {}

fn basic_rows() -> Vec<Param> {
    vec![
        Param::new("title", tl!("Document Title"), ParamKind::text("")),
        Param::new("author", tl!("Author"), ParamKind::text("")),
        Param::new("authorTitle", tl!("Author Title"), ParamKind::text("")),
        Param::new("description", tl!("Description"), ParamKind::text("")).multiline(3),
        Param::new("keywords", tl!("Keywords"), ParamKind::text("")).multiline(3),
        Param::new(
            "copyrightStatus",
            tl!("Copyright Status"),
            ParamKind::choice(
                vec![
                    ("unknown".to_owned(), Label::from(tl!("Unknown"))),
                    ("copyrighted".to_owned(), Label::from(tl!("Copyrighted"))),
                    ("publicDomain".to_owned(), Label::from(tl!("Public Domain"))),
                ],
                Some("unknown".to_owned()),
            ),
        ),
        Param::new("copyright", tl!("Copyright Notice"), ParamKind::text("")).multiline(3),
        Param::new("copyrightUrl", tl!("Copyright Info URL"), ParamKind::text("")),
    ]
}

/// The reference's ten rows are always present; the four older extra rows keep their values
/// and stay below them when present. The engine alone formats camera values.
fn camera_rows(camera: &Value) -> (Vec<Param>, DialogFields) {
    let lang = crate::i18n::current();
    let mut rows = Vec::new();
    let mut fields = DialogFields::new();
    for (heading, entries) in [
        ("Camera Information", &[("make", "Make"), ("model", "Model"), ("owner", "Owner"), ("lens", "Lens")][..]),
        (
            "Shot Information",
            &[
                ("focalLength", "Focal Length"),
                ("exposure", "Exposure"),
                ("imageSize", "Image Size"),
                ("orientation", "Orientation"),
                ("resolution", "Resolution"),
                ("flash", "Flash"),
            ][..],
        ),
    ] {
        rows.push(Param::section(tl!(heading)));
        for (key, label) in entries {
            let value = camera.get(*key).and_then(Value::as_str).unwrap_or_default();
            let field = format!("camera/{key}");
            fields.set(field.clone(), value);
            rows.push(Param::read_only_text(field, crate::i18n::tr_ctx(lang, "cameraData", label), value.to_owned()));
        }
    }
    for (key, label) in [("dateTaken", "Date Taken"), ("fNumber", "F-Stop"), ("iso", "ISO Speed"), ("focalLength35mm", "Focal Length (35 mm)")] {
        if let Some(value) = camera.get(key).and_then(Value::as_str).filter(|value| !value.is_empty()) {
            let field = format!("camera/{key}");
            fields.set(field.clone(), value);
            rows.push(Param::read_only_text(field, crate::i18n::tr_ctx(lang, "cameraData", label), value.to_owned()));
        }
    }
    (rows, fields)
}

fn initial_fields(info: &Value) -> DialogFields {
    let mut fields = DialogFields::new();
    for key in BASIC_KEYS {
        let text = if key == "keywords" {
            info[key].as_array().map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default()
        } else if key == "copyrightStatus" {
            info[key].as_str().unwrap_or("unknown").to_owned()
        } else {
            info[key].as_str().unwrap_or_default().to_owned()
        };
        fields.set(key, text);
    }
    fields.snapshot();
    fields
}

fn command_params(fields: &DialogFields) -> Value {
    Value::Object(BASIC_KEYS.into_iter().map(|key| (key.to_owned(), json!(fields.text(key).unwrap_or_default()))).collect())
}

/// Opens on the last page shown, keeping Basic's values from this document, not a previous form.
pub(crate) fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let document = app.active_doc_id()?;
    let info = app.session.execute(COMMAND, json!({})).map_err(|error| error.to_string())?;
    let selected = saved_value::<String>(app.ctx(), &LAST_CATEGORY);
    let category = CATEGORIES.iter().find(|category| category.key == selected).map_or("basic", |category| category.key);
    let fields = initial_fields(&info);
    let (camera_params, camera_fields) = camera_rows(&info["camera"]);
    Ok(app.framework.dialogs.open(FileInfoDialog {
        document,
        opening: fields.clone(),
        fields,
        camera: info["camera"].clone(),
        camera_params,
        camera_fields,
        category,
        reset_scroll: true,
        held: HeldAnswer::new(),
    }))
}

struct FileInfoDialog {
    document: DocId,
    fields: DialogFields,
    opening: DialogFields,
    camera: Value,
    camera_params: Vec<Param>,
    camera_fields: DialogFields,
    category: &'static str,
    reset_scroll: bool,
    held: HeldAnswer<Answer>,
}

#[derive(Clone)]
struct Answer {
    fields: DialogFields,
    programmatic: bool,
}

impl FileInfoDialog {
    fn file_ok(&mut self, programmatic: bool) {
        if !self.held.is_waiting() && !self.held.is_accepted() {
            self.held.file(Answer { fields: self.fields.clone(), programmatic });
        }
    }
}

impl StackDialog for FileInfoDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(COMMAND)
    }
    fn key(&self) -> Option<&str> {
        Some(COMMAND)
    }
    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        if self.held.finish().is_some() {
            Some(())
        } else {
            self.file_ok(true);
            None
        }
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let measuring = craft::ui::runtime::measure::active(ui.ctx());
        if !measuring && self.held.finish().is_some() {
            return Some(DialogResult::Ok(()));
        }
        let before = self.fields.clone();
        let old_category = self.category;
        let mut selected = self.category;
        let mut reset_scroll = self.reset_scroll;
        let waiting = self.held.is_waiting();
        let error = self.held.refusal().map(|(error, _)| error.clone());
        let rows = basic_rows();
        let shown = FormDialog::new(tl!("File Info"), Bind::bound(&mut self.fields))
            .opening(self.opening.clone())
            .history(true)
            .collection_body(true)
            .accept_stays_open(true)
            .ok_enabled(!waiting)
            .footer_extra(|ui| {
                // These future File Info subdialogs are outside #13828; don't invent their behaviour.
                Row::new().show(ui, |ui, _| {
                    Button::new(tl!("Preferences")).enabled(false).show(ui);
                    Button::new(tl!("Template Folder")).enabled(false).show(ui);
                });
            })
            .shaped(|dialog| {
                dialog.id(dialog_id(COMMAND)).resizable(true).overlay(|overlay| {
                    // design: allow[literal-size/vec2](File Info's resizable opening and minimum sizes in points)
                    overlay.size(egui::vec2(900.0, 640.0)).min_size(egui::vec2(560.0, 360.0))
                })
            })
            .show(ui, |ui, _, fields| {
                Row::new().fill().enabled(!waiting).show(ui, |ui, _| {
                    let list = List::<String>::new(tl!("File Info Categories"))
                        .select(SelectMode::Single)
                        .activate(ActivateOn::Click)
                        .allow_empty(false)
                        .header(false)
                        .sizing(Sizing::Content)
                        .selection(Bind::given(Selection::single(selected.to_owned())))
                        .show_items(ui, &Categories, &mut |key, _| {
                            let category = CATEGORIES.iter().find(|category| category.key == key.as_str());
                            let row = ListRow::new(category.map_or_else(Label::default, |category| Label::from(tl!(category.label))));
                            match category.and_then(|category| category.tooltip) {
                                Some(tooltip) => row.tooltip(tl!(tooltip)),
                                None => row,
                            }
                        });
                    for event in list.events {
                        if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                            && let Some(category) = keys.iter().find_map(|key| CATEGORIES.iter().find(|category| category.key == key.as_str()))
                            && category.key != selected
                        {
                            selected = category.key;
                            reset_scroll = true;
                        }
                    }
                    Divider::vertical().show(ui);
                    Stack::new().fill().scroll(Scroll::vertical()).label(tl!("File Info Page")).show(ui, |ui, _| {
                        // One stable scroll owner; reveal its first child on a switch instead of leaking
                        // one new scroll identity for every visit. Wheel input can interrupt the glide.
                        Lazy::new("file-info-page", 0.0).reveal(reset_scroll).show(ui, |ui| match selected {
                            "basic" => {
                                ParamRows::new(tl!("Basic"), &rows, Bind::bound(fields)).show(ui);
                            }
                            "cameraData" => {
                                ParamRows::new(tl!("Camera Data"), &self.camera_params, Bind::bound(&mut self.camera_fields)).read_only(true).show(ui);
                            }
                            _ => {}
                        });
                    });
                });
                if let Some(error) = &error {
                    Notice::new(Tone::Bad, error.clone()).layout(NoticeLayout::Inline).show(ui);
                }
                true
            });
        if measuring {
            return None;
        }
        self.reset_scroll = false;
        self.category = selected;
        if selected != old_category {
            set_saved_value(ui.ctx(), &LAST_CATEGORY, selected.to_owned());
        }
        if self.fields != before || selected != old_category {
            self.held.clear_refusal();
        }
        if shown.picked == Some(0) || shown.accepted.is_some() {
            self.file_ok(false);
        }
        if shown.closed.is_some() && !self.held.is_waiting() { Some(DialogResult::Cancel) } else { None }
    }

    fn json_fields(&self) -> Option<Value> {
        let Value::Object(mut fields) = serde_json::to_value(&self.fields).ok()? else { return None };
        fields.insert("category".into(), json!(self.category));
        fields.insert("camera".into(), self.camera.clone());
        fields.insert("document".into(), json!(self.document.0));
        Some(Value::Object(fields))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if self.held.is_waiting() || self.held.is_accepted() {
            return Err(FieldRefusal::Inactive.message(field));
        }
        if field == "category" {
            let category = value
                .as_str()
                .and_then(|key| CATEGORIES.iter().find(|category| category.key == key))
                .ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            if self.category != category.key {
                self.category = category.key;
                self.reset_scroll = true;
                set_saved_value(ctx, &LAST_CATEGORY, category.key.to_owned());
            }
        } else if field == "camera" || field == "document" || field.starts_with("camera/") {
            return Err(FieldRefusal::ReadOnly.message(field));
        } else {
            if self.category != "basic" {
                return Err(FieldRefusal::Inactive.message(field));
            }
            set_param_field(&basic_rows(), &mut self.fields, field, value)?;
        }
        self.held.clear_refusal();
        Ok(())
    }
}

/// The held OK applies through the normal command path before the host closes the dialog.
/// A refused command keeps the draft and captured document; a successful one runs only once.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let closed = app
        .framework
        .dialogs
        .get_by_id::<FileInfoDialog>(dialog_id(COMMAND))
        .filter(|dialog| !app.session.documents().iter().any(|state| state.doc.id == dialog.document))
        .map(StackDialog::id);
    if let Some(id) = closed {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
    let owner = app
        .framework
        .dialogs
        .get_by_id::<FileInfoDialog>(dialog_id(COMMAND))
        .and_then(|dialog| app.framework.dialogs.open_serial(dialog.id()).map(|serial| (dialog.id(), serial, dialog.document)));
    let pending = owner.and_then(|(id, serial, document)| {
        app.framework.dialogs.get_mut_by_id::<FileInfoDialog>(id).and_then(|dialog| dialog.held.take().map(|answer| (id, serial, document, answer)))
    });
    if let Some((id, serial, document, answer)) = pending {
        let params = command_params(&answer.fields);
        let result = app.with_document(document, |app| {
            // ui.dialog.confirm judges command dialogs even without an injected input pass. The
            // generic control adapter recognizes HostedParamDialog; this app-owned family keeps
            // that same gate here while OK is still held. Injected input is already gated by run.
            if answer.programmatic
                && !app.automation_input
                && let Some(authorize) = app.services.automation_command.as_ref()
            {
                authorize(COMMAND, &params)?;
            }
            app.run(COMMAND, params)
        });
        // A command may open/replace a dialog through a script event. Its answer belongs only
        // to this opening, never a replacement with the same family/id.
        if app.framework.dialogs.open_serial(id) == Some(serial)
            && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<FileInfoDialog>(id)
        {
            dialog.held.answer(result.map(|_| ()).map_err(Label::from));
        }
    }
    // The result is only a completion notification: the command already succeeded while held.
    while app.framework.dialogs.take::<FileInfoDialog>().is_some() {}
}
