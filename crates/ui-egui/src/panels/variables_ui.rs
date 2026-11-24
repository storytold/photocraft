//! Image › Variables (Define… / Data Sets…), the data-driven graphics dialog, hosted on the
//! framework's dialog host as a [`VariablesDialog`]. Image › Apply Data Set… has its own small
//! dialog (`apply_data_set_ui`). All saving drives the
//! headless engine commands (`image.variables.define`, `image.variables.dataSets`,
//! `image.applyDataSet`), so the control channel drives it with `ui.dialog.set` / `confirm`
//! exactly like a human.
//!
//! The dialog's state is [`VariablesState`], serialised in the shape of the engine command params,
//! so [`VariablesState::defs_params`] and [`VariablesState::data_sets_params`] pass it straight through.

use craft::behavior::{DisplayOrder, IconId, ItemSource, SelectionChange};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, with_env};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::TextField;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::borrow::Cow;

use crate::PhotocraftApp;

/// The page of the dialog that shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Page {
    /// The variable definitions.
    #[default]
    Define,
    /// The data sets.
    DataSets,
}

fn fit() -> String {
    "fit".into()
}
fn center() -> String {
    "center".into()
}
fn visibility() -> String {
    "visibility".into()
}

/// One variable definition, shaped as an entry of the `image.variables.define` params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VarDef {
    /// The variable's name.
    pub name: String,
    /// The bound layer's id.
    #[serde(default)]
    pub layer: u64,
    /// `visibility`, `textReplacement` or `pixelReplacement`.
    #[serde(rename = "type", default = "visibility")]
    pub ty: String,
    /// The pixel replacement's fit method.
    #[serde(default = "fit")]
    pub method: String,
    /// The pixel replacement's alignment.
    #[serde(default = "center")]
    pub align: String,
    /// Whether the pixel replacement clips to the layer.
    #[serde(default)]
    pub clip: bool,
}

/// A data set's value for one variable: a flag for visibility, text for the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    /// A visibility value.
    Flag(bool),
    /// Replacement text or an image path.
    Text(String),
}

/// One value of a data set, shaped as an entry of a set's `values` in the `image.variables.dataSets` params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetValue {
    /// The variable's name.
    pub variable: String,
    /// `visibility`, `text` or `pixels`.
    pub kind: String,
    /// The value.
    pub value: Cell,
}

/// One data set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSetRow {
    /// The set's name.
    pub name: String,
    /// One value per variable, in the order the user first showed them.
    #[serde(default)]
    pub values: Vec<SetValue>,
}

/// What the dialog edits. Its JSON is what `ui.inspect` lists and `ui.dialog.set` takes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesState {
    /// The variable definitions.
    pub defs: Vec<VarDef>,
    /// The data sets.
    pub data_sets: Vec<DataSetRow>,
    /// The page that shows.
    pub page: Page,
    /// The data set the Data Sets page shows, always within `data_sets`.
    pub cur: usize,
}

fn method_name(m: &photocraft_doc::PixelMethod) -> &'static str {
    use photocraft_doc::PixelMethod::*;
    match m {
        Fit => "fit",
        Fill => "fill",
        AsIs => "asIs",
        Conform => "conform",
    }
}

fn align_name(a: &photocraft_doc::PixelAlign) -> &'static str {
    use photocraft_doc::PixelAlign::*;
    match a {
        TopLeft => "topLeft",
        TopCenter => "topCenter",
        TopRight => "topRight",
        CenterLeft => "centerLeft",
        Center => "center",
        CenterRight => "centerRight",
        BottomLeft => "bottomLeft",
        BottomCenter => "bottomCenter",
        BottomRight => "bottomRight",
    }
}

impl VariablesState {
    /// The document's variables as dialog state, on `page`, with the active data set selected.
    pub fn from_doc(v: &photocraft_doc::Variables, page: Page) -> Self {
        let defs = v
            .defs
            .iter()
            .map(|d| {
                let (ty, method, align, clip) = match &d.kind {
                    photocraft_doc::VarKind::Visibility => ("visibility", "fit", "center", false),
                    photocraft_doc::VarKind::TextReplacement => ("textReplacement", "fit", "center", false),
                    photocraft_doc::VarKind::PixelReplacement { method, align, clip } => ("pixelReplacement", method_name(method), align_name(align), *clip),
                };
                VarDef { name: d.name.clone(), layer: d.layer.0, ty: ty.into(), method: method.into(), align: align.into(), clip }
            })
            .collect();
        let data_sets = v
            .data_sets
            .iter()
            .map(|s| DataSetRow {
                name: s.name.clone(),
                values: s
                    .values
                    .iter()
                    .map(|dv| {
                        let (kind, value) = match &dv.value {
                            photocraft_doc::variables::Value::Visibility(b) => ("visibility", Cell::Flag(*b)),
                            photocraft_doc::variables::Value::Text(t) => ("text", Cell::Text(t.clone())),
                            photocraft_doc::variables::Value::Pixels(p) => ("pixels", Cell::Text(p.clone())),
                        };
                        SetValue { variable: dv.variable.clone(), kind: kind.into(), value }
                    })
                    .collect(),
            })
            .collect();
        let mut state = VariablesState { defs, data_sets, page, cur: v.active.unwrap_or(0) };
        state.clamp();
        state
    }

    /// The params of `image.variables.define`.
    pub fn defs_params(&self) -> Value {
        json!({"defs": self.defs})
    }

    /// The params of `image.variables.dataSets`.
    pub fn data_sets_params(&self) -> Value {
        json!({"dataSets": self.data_sets})
    }

    /// Keeps `cur` within the rows.
    fn clamp(&mut self) {
        self.cur = self.cur.min(self.data_sets.len().saturating_sub(1));
    }

    /// Gives every data set one value per variable.
    fn fill_values(&mut self) {
        for set in &mut self.data_sets {
            for def in &self.defs {
                if set.values.iter().any(|v| v.variable == def.name) {
                    continue;
                }
                let (kind, value) = match def.ty.as_str() {
                    "visibility" => ("visibility", Cell::Flag(true)),
                    "pixelReplacement" => ("pixels", Cell::Text(String::new())),
                    _ => ("text", Cell::Text(String::new())),
                };
                set.values.push(SetValue { variable: def.name.clone(), kind: kind.into(), value });
            }
        }
    }
}

/// What the dialog answers with at OK.
#[derive(Clone, Debug, PartialEq)]
pub struct VariablesAnswer {
    /// The edited state.
    pub state: VariablesState,
}

/// The Variables dialog, hosted on the framework's dialog host.
pub struct VariablesDialog {
    state: VariablesState,
    /// Layers available to bind, top to bottom: (id, display name, is-text).
    layers: Vec<(u64, String, bool)>,
    /// The row an Apply click asked for; [`take`] runs it once.
    apply: Option<usize>,
    /// The values the dialog opened with, which Alt+Reset restores; handed to the form on its first pass.
    opening: Option<VariablesState>,
}

impl VariablesDialog {
    fn new(state: VariablesState, layers: Vec<(u64, String, bool)>) -> Self {
        VariablesDialog { opening: Some(state.clone()), state, layers, apply: None }
    }
}

const TYPES: &[(&str, &str)] = &[("visibility", "Visibility"), ("textReplacement", "Text Replacement"), ("pixelReplacement", "Pixel Replacement")];
const METHODS: &[(&str, &str)] = &[("fit", "Fit"), ("fill", "Fill"), ("conform", "Conform"), ("asIs", "As Is")];

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

fn entries(options: &[(&str, &str)]) -> Vec<Entry<String>> {
    options.iter().map(|(key, name)| Entry::new((*key).to_string(), label(crate::i18n::t(name)))).collect()
}

/// The two pages as the tab strip's item source, keyed by position.
struct Pages;

impl DisplayOrder<usize> for Pages {
    fn len(&self) -> usize {
        2
    }
    fn key(&self, at: usize) -> usize {
        at
    }
    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < 2).then_some(*key)
    }
    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(if at == 0 { tl!("Define") } else { tl!("Data Sets") }))
    }
}

impl ItemSource<usize> for Pages {}

fn define_page(ui: &mut egui::Ui, layers: &[(u64, String, bool)], state: &mut VariablesState) {
    Text::new(label(tl!("Variables bind a layer's visibility, text or pixels to a named data slot."))).show(ui);
    let layer_entries: Vec<Entry<u64>> = layers.iter().map(|(id, name, _)| Entry::new(*id, label(name))).collect();
    let type_entries = entries(TYPES);
    let method_entries = entries(METHODS);
    let mut remove = None;
    for (i, def) in state.defs.iter_mut().enumerate() {
        with_env(
            ui,
            |env| env.id_salt = Some(egui::Id::unique(i)),
            |ui| {
                Row::new().show(ui, |ui, _| {
                    TextField::new(label(tl!("Name")), Bind::bound(&mut def.name)).show(ui);
                    if let Some(layer) = Select::new(label(tl!("Layer")), Bind::given(def.layer), &layer_entries).show(ui).value() {
                        def.layer = *layer;
                    }
                    if let Some(ty) = Select::new(label(tl!("Type")), Bind::given(def.ty.clone()), &type_entries).show(ui).value() {
                        def.ty.clone_from(ty);
                    }
                    if def.ty == "pixelReplacement"
                        && let Some(method) = Select::new(label(tl!("Method")), Bind::given(def.method.clone()), &method_entries).show(ui).value()
                    {
                        def.method.clone_from(method);
                    }
                    if IconButton::new(IconId::new("minus")).label(label(tl!("Remove Variable"))).show(ui).clicked() {
                        remove = Some(i);
                    }
                });
            },
        );
    }
    if let Some(i) = remove {
        state.defs.remove(i);
    }
    if Button::new(label(tl!("Add Variable"))).show(ui).clicked() {
        let n = state.defs.len() + 1;
        let layer = layers.first().map_or(0, |l| l.0);
        state.defs.push(VarDef { name: format!("var{n}"), layer, ty: visibility(), method: fit(), align: center(), clip: false });
    }
}

/// Edits one data set; returns the row an Apply click asked for.
fn data_sets_page(ui: &mut egui::Ui, state: &mut VariablesState) -> Option<usize> {
    if state.defs.is_empty() {
        Text::new(label(tl!("Define at least one variable first."))).show(ui);
        return None;
    }
    state.fill_values();
    state.clamp();
    let total = state.data_sets.len();
    Row::new().show(ui, |ui, _| {
        if IconButton::new(IconId::new("chevron_left")).label(label(tl!("Previous Data Set"))).enabled(state.cur > 0).show(ui).clicked() {
            state.cur -= 1;
        }
        let name = state.data_sets.get(state.cur).map_or_else(|| tl!("(none)").to_string(), |s| s.name.clone());
        let shown = if total == 0 { 0 } else { state.cur + 1 };
        let template = "Data Set: {name}  ({index}/{total})";
        let text = crate::i18n::fmt(tl!(template), &[("name", &name), ("index", &shown.to_string()), ("total", &total.to_string())]);
        Text::new(label(&text)).show(ui);
        if IconButton::new(IconId::new("chevron_right")).label(label(tl!("Next Data Set"))).enabled(state.cur + 1 < total).show(ui).clicked() {
            state.cur += 1;
        }
        if Button::new(label(tl!("New"))).show(ui).clicked() {
            state.data_sets.push(DataSetRow { name: format!("Data Set {}", total + 1), values: Vec::new() });
            state.cur = state.data_sets.len() - 1;
            state.fill_values();
        }
        if total > 0 && Button::new(label(tl!("Delete"))).show(ui).clicked() {
            state.data_sets.remove(state.cur);
            state.clamp();
        }
    });
    let cur = state.cur;
    let defs = state.defs.clone();
    let set = state.data_sets.get_mut(cur)?;
    TextField::new(label(tl!("Name")), Bind::bound(&mut set.name)).show(ui);
    for (i, def) in defs.iter().enumerate() {
        let Some(value) = set.values.iter_mut().find(|v| v.variable == def.name) else { continue };
        with_env(
            ui,
            |env| env.id_salt = Some(egui::Id::unique(i)),
            |ui| {
                Row::new().show(ui, |ui, _| {
                    Text::new(label(&def.name)).show(ui);
                    match &mut value.value {
                        Cell::Flag(on) => {
                            Checkbox::new(Bind::bound(on)).label(label(tl!("Visible"))).show(ui);
                        }
                        Cell::Text(text) => {
                            TextField::new(label(&def.name), Bind::bound(text)).show(ui);
                        }
                    }
                });
            },
        );
    }
    Button::new(label(tl!("Apply"))).show(ui).clicked().then_some(cur)
}

impl StackDialog for VariablesDialog {
    type Result = VariablesAnswer;

    fn id(&self) -> DialogId {
        dialog_id("photocraft.variables")
    }

    /// An Apply click files its row in `apply` for [`take`]. A click is the pass's own input, so a second pass
    /// of the frame does not file it again.
    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<VariablesAnswer>> {
        let VariablesDialog { state, layers, apply, opening } = self;
        let mut form = FormDialog::new(label(tl!("Variables")), Bind::bound(state));
        if let Some(opening) = opening.take() {
            form = form.opening(opening);
        }
        let mut asked = None;
        let shown = form.show(ui, |ui, _form, state| {
            let mut page = if state.page == Page::Define { 0 } else { 1 };
            let strip = TabStrip::new(label(tl!("Variables")))
                .kind(TabKind::Text)
                .shown(page)
                .show_items(ui, &Pages, &mut |key, _| Tab::new(label(if *key == 0 { tl!("Define") } else { tl!("Data Sets") })));
            for event in strip.response.events {
                if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                    && let Some(key) = keys.into_iter().last()
                {
                    page = key.min(1);
                }
            }
            state.page = if page == 0 { Page::Define } else { Page::DataSets };
            if layers.is_empty() {
                Notice::new(Tone::Info, label(tl!("Open a document to define variables."))).layout(NoticeLayout::Centred).show(ui);
            } else if state.page == Page::Define {
                define_page(ui, layers, state);
            } else {
                asked = data_sets_page(ui, state);
            }
            true
        });
        if asked.is_some() {
            *apply = asked;
        }
        match (shown.accepted, shown.closed) {
            (Some(state), _) => Some(DialogResult::Ok(VariablesAnswer { state })),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<VariablesAnswer> {
        Some(VariablesAnswer { state: self.state.clone() })
    }

    fn json_fields(&self) -> Option<Value> {
        serde_json::to_value(&self.state).ok()
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let bad = |e: serde_json::Error| format!("`{field}` is not valid: {e}");
        match field {
            "defs" => self.state.defs = serde_json::from_value(value).map_err(bad)?,
            "dataSets" => self.state.data_sets = serde_json::from_value(value).map_err(bad)?,
            "page" => self.state.page = serde_json::from_value(value).map_err(bad)?,
            "cur" => self.state.cur = usize::try_from(serde_json::from_value::<u64>(value).map_err(bad)?).unwrap_or(usize::MAX),
            _ => return Err(format!("the Variables dialog has no field `{field}`")),
        }
        self.state.clamp();
        Ok(())
    }
}

/// Layers available to bind, top to bottom: (id, display name, is-text).
fn layer_options(app: &PhotocraftApp) -> Vec<(u64, String, bool)> {
    let Some(st) = app.session.active() else { return Vec::new() };
    st.doc.walk().iter().map(|(_, _, l)| (l.id.0, l.name.clone(), matches!(l.content, photocraft_doc::LayerContent::Text(_)))).collect()
}

/// Menu entry points. Returns None for ids this module doesn't own.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let has_params = params.as_object().is_some_and(|o| !o.is_empty());
    let page = match id {
        "image.variables.define" if !has_params => Page::Define,
        "image.variables.dataSets" if !has_params => Page::DataSets,
        // No data set named: the Apply Data Set dialog picks one from the list.
        "image.applyDataSet" if !has_params => {
            return Some(Ok(json!({"dialog": crate::panels::apply_data_set_ui::open(app).value().to_string()})));
        }
        _ => return None,
    };
    let state = app.session.active().map(|st| VariablesState::from_doc(&st.doc.variables, page)).unwrap_or(VariablesState { page, ..Default::default() });
    let opened = app.framework.dialogs.open(VariablesDialog::new(state, layer_options(app)));
    Some(Ok(json!({"dialog": opened.value().to_string()})))
}

/// Saves the edited definitions and data sets, then applies `index` when given.
fn save(app: &mut PhotocraftApp, state: &VariablesState, index: Option<usize>) -> Result<(), String> {
    app.run("image.variables.define", state.defs_params())?;
    app.run("image.variables.dataSets", state.data_sets_params())?;
    if let Some(index) = index {
        app.run("image.applyDataSet", json!({"index": index}))?;
    }
    Ok(())
}

/// What OK does: saves the definitions and data sets. It applies nothing; the Data Sets page's
/// Apply and Image › Apply Data Set… do.
fn finish(app: &mut PhotocraftApp, answer: VariablesAnswer) -> Result<(), String> {
    save(app, &answer.state, None)
}

/// The dialog's result pump (a `TAKERS` row): runs a pending Apply, then the result of a closed dialog. Each call
/// runs only the request the dialog filed since the last call, and an answer is taken once.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let pending = app.framework.dialogs.get_mut::<VariablesDialog>().and_then(|d| d.apply.take().map(|index| (index, d.state.clone())));
    if let Some((index, state)) = pending {
        match save(app, &state, Some(index)) {
            Ok(()) => {
                let applied = app.session.active().map(|st| VariablesState::from_doc(&st.doc.variables, state.page));
                if let (Some(mut fresh), Some(dialog)) = (applied, app.framework.dialogs.get_mut::<VariablesDialog>()) {
                    fresh.cur = state.cur;
                    fresh.clamp();
                    dialog.state = fresh;
                }
            }
            Err(error) => app.ui.status = error,
        }
    }
    if let Some(DialogResult::Ok(answer)) = app.framework.dialogs.take::<VariablesDialog>()
        && let Err(error) = finish(app, answer)
    {
        app.ui.status = error;
    }
}
