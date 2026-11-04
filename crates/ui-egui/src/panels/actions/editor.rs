//! Runtime-only Actions editor page. Definitions change through owned backend commands only.
use std::collections::BTreeSet;

use egui::{Color32, Id};
use serde_json::{Map, Value, json};

use super::editor_picker::{self, Choice};
use crate::{PhotocraftApp, filter_dialog, icons, theme::Tokens};

const CONFIRM_PREF: &str = "actions.editor.save.confirmation";

/// The shared playback adapter owns these conditions, including suspended step dialogs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Activity {
    pub playback_busy: bool,
    pub finishing_step: bool,
}

/// Parent navigation and playback effects. The page never substitutes committed-library playback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Saved { source: Option<usize> },
    Closed { saved: bool, source: Option<usize> },
    CancelledLeave,
    Preview { draft: u64, from: usize },
}

#[derive(Default)]
struct Buffer {
    text: String,
    acknowledged: String,
}

impl Buffer {
    fn dirty(&self) -> bool {
        self.text != self.acknowledged
    }

    fn refresh(&mut self, authoritative: &str) {
        if !self.dirty() {
            self.text = authoritative.to_string();
        }
        self.acknowledged = authoritative.to_string();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Save,
    Leave,
}

#[derive(Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

#[derive(Clone)]
enum Target {
    Insert { at: usize, len: usize, previous: Option<Value> },
    Replace { at: usize, original: Value },
}

#[derive(Clone)]
struct StepEdit {
    draft: u64,
    target: Target,
    command: String,
    params: Map<String, Value>,
}

/// Buffers and view state only: no Action/Session/document copy and no persistence attachment.
pub struct Page {
    draft: u64,
    name: Buffer,
    description: Buffer,
    shortcut: Buffer,
    selected: Option<usize>,
    expanded: BTreeSet<usize>,
    prompt: Option<Intent>,
    dont_show_again: bool,
    prompt_focused: bool,
    editing: Option<StepEdit>,
    error: Option<String>,
    source: Option<usize>,
    closed: bool,
}

fn action_text<'a>(view: &'a Value, key: &str) -> &'a str {
    view.get("action").and_then(|action| action.get(key)).and_then(Value::as_str).unwrap_or("")
}

fn steps(view: &Value) -> &[Value] {
    view.get("action").and_then(|action| action.get("steps")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn step_parts(step: &Value) -> Option<(&str, &Value)> {
    let pair = step.as_array()?;
    Some((pair.first()?.as_str()?, pair.get(1)?))
}

fn flagged(view: &Value, key: &str, step: usize) -> bool {
    view.get("action")
        .and_then(|action| action.get(key))
        .and_then(Value::as_array)
        .is_some_and(|flags| flags.iter().any(|flag| flag.as_u64().and_then(|index| usize::try_from(index).ok()) == Some(step)))
}

fn simple(kind: &filter_dialog::Kind) -> bool {
    matches!(
        kind,
        filter_dialog::Kind::Range { .. }
            | filter_dialog::Kind::Choice { .. }
            | filter_dialog::Kind::Bool(_)
            | filter_dialog::Kind::Int { .. }
            | filter_dialog::Kind::Text
    )
}

fn editable(command: &str) -> bool {
    if command == photocraft_engine::atn_cmds::STOP_MESSAGE {
        return true;
    }
    command != photocraft_engine::atn_cmds::PHOTOSHOP_STEP
        && photocraft_engine::commands::find(command)
            .is_some_and(|spec| filter_dialog::parse_spec(spec.params).iter().any(|param| !param.key.starts_with("__") && simple(&param.kind)))
}

fn summary(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(value) => (if *value { tl!("On") } else { tl!("Off") }).to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => tl!("None").to_string(),
        _ => tl!("Stored settings").to_string(),
    }
}

impl Page {
    /// Open once at an actual New/Edit entry point; the backend allocates name and ownership.
    pub fn open(app: &mut PhotocraftApp, params: Value) -> Result<Self, String> {
        let view = app.run("actions.editor.open", params)?;
        Self::attach(&view)
    }

    /// Attach the actual page to a backend draft opened through the control channel.
    pub(super) fn attach(view: &Value) -> Result<Self, String> {
        let draft = view.get("draft").and_then(Value::as_u64).ok_or("The editor returned no draft identity")?;
        let mut page = Self {
            draft,
            name: Buffer::default(),
            description: Buffer::default(),
            shortcut: Buffer::default(),
            selected: None,
            expanded: BTreeSet::new(),
            prompt: None,
            dont_show_again: false,
            prompt_focused: false,
            editing: None,
            error: None,
            source: None,
            closed: false,
        };
        page.accept(view)?;
        Ok(page)
    }

    pub fn draft(&self) -> u64 {
        self.draft
    }

    /// Parent inspection/close routing must retain this runtime page rather than serialize it.
    pub fn has_modal(&self) -> bool {
        self.prompt.is_some() || self.editing.is_some()
    }

    /// Validate the response's draft identity and shape before refreshing buffers or step selection.
    fn accept(&mut self, view: &Value) -> Result<(), String> {
        if view.get("draft").and_then(Value::as_u64) != Some(self.draft)
            || view.get("action").and_then(|action| action.get("name")).and_then(Value::as_str).is_none()
            || view.get("shortcut").and_then(Value::as_str).is_none()
            || view.get("dirty").and_then(Value::as_bool).is_none()
            || view.get("recording").and_then(Value::as_bool).is_none()
        {
            return Err(tl!("The editor response does not belong to this draft").into());
        }
        self.name.refresh(action_text(view, "name"));
        self.description.refresh(action_text(view, "description"));
        self.shortcut.refresh(view.get("shortcut").and_then(Value::as_str).unwrap_or(""));
        self.source = view.get("source").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok());
        let len = steps(view).len();
        self.selected = self.selected.filter(|index| *index < len);
        self.expanded.retain(|index| *index < len);
        Ok(())
    }

    /// Bind every backend edit to this draft and retain its error without replacing the page.
    fn call(&mut self, app: &mut PhotocraftApp, verb: &str, params: Value) -> Result<Value, String> {
        if self.closed {
            return Err(tl!("This editor page is closed").into());
        }
        let mut params = params.as_object().cloned().unwrap_or_default();
        params.insert("draft".into(), json!(self.draft));
        let result = app.run(&format!("actions.editor.{verb}"), Value::Object(params));
        match result {
            Ok(view) => {
                if verb != "discard"
                    && let Err(error) = self.accept(&view)
                {
                    self.error = Some(error.clone());
                    return Err(error);
                }
                if verb != "get" {
                    self.error = None;
                }
                Ok(view)
            }
            Err(error) => {
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// Refresh this draft from backend inspection without executing a new editor command.
    fn get(&mut self, app: &mut PhotocraftApp) -> Result<Value, String> {
        match photocraft_engine::actions_cmds::editor::inspect(&app.session, self.draft).map_err(|error| error.to_string()) {
            Ok(view) => {
                self.accept(&view)?;
                Ok(view)
            }
            Err(error) => {
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// Include unsubmitted text buffers as well as backend edits when deciding whether leaving needs confirmation.
    fn dirty(&self, view: &Value) -> bool {
        view.get("dirty").and_then(Value::as_bool) == Some(true) || self.name.dirty() || self.description.dirty() || self.shortcut.dirty()
    }

    /// Submit only changed metadata fields, leaving unknown and untouched Action values with the backend.
    fn stage(&mut self, app: &mut PhotocraftApp) -> Result<Value, String> {
        let mut params = Map::new();
        for (key, buffer) in [("name", &self.name), ("description", &self.description), ("shortcut", &self.shortcut)] {
            if buffer.dirty() {
                params.insert(key.into(), json!(buffer.text));
            }
        }
        if params.is_empty() { self.get(app) } else { self.call(app, "update", Value::Object(params)) }
    }

    fn suppressed(app: &PhotocraftApp) -> bool {
        app.session.prefs().dialogs.get(CONFIRM_PREF).and_then(|value| value.get("skip")).and_then(Value::as_bool) == Some(true)
    }

    /// Offer Save only for a dirty draft and apply the stored skip preference through the same answer path.
    fn ask_save(&mut self, app: &mut PhotocraftApp, activity: Activity) -> Result<Option<Event>, String> {
        let view = self.get(app)?;
        if self.dirty(&view) {
            self.prompt = Some(Intent::Save);
            self.dont_show_again = false;
            self.prompt_focused = false;
            if Self::suppressed(app) {
                return self.answer(app, Answer::Save, activity);
            }
        }
        Ok(None)
    }

    /// Parent stores its destination until Closed; CancelledLeave clears that pending navigation.
    pub fn request_leave(&mut self, app: &mut PhotocraftApp, activity: Activity) -> Result<Option<Event>, String> {
        if activity.playback_busy || self.has_modal() {
            return Err(tl!("Confirm or cancel the open action dialog first").into());
        }
        let view = self.get(app)?;
        if self.dirty(&view) || (activity.finishing_step && view.get("recording").and_then(Value::as_bool) == Some(true)) {
            self.prompt = Some(Intent::Leave);
            self.dont_show_again = false;
            self.prompt_focused = false;
            if Self::suppressed(app) {
                return self.answer(app, Answer::Save, activity);
            }
            Ok(None)
        } else {
            self.call(app, "discard", json!({}))?;
            self.closed = true;
            Ok(Some(Event::Closed { saved: false, source: self.source }))
        }
    }

    /// Resolve Save/Discard/Cancel without closing the page on backend refusal or an unfinished recorded step.
    fn answer(&mut self, app: &mut PhotocraftApp, answer: Answer, activity: Activity) -> Result<Option<Event>, String> {
        let intent = self.prompt.ok_or("There is no Save Action prompt")?;
        if matches!(answer, Answer::Cancel) {
            self.prompt = None;
            return Ok((intent == Intent::Leave).then_some(Event::CancelledLeave));
        }
        if activity.playback_busy || (activity.finishing_step && matches!(answer, Answer::Save)) {
            let error = tl!("Wait for the current action step to finish").to_string();
            self.error = Some(error.clone());
            return Err(error);
        }
        if matches!(answer, Answer::Save) {
            self.stage(app)?;
            self.call(app, "save", json!({}))?;
            if self.dont_show_again {
                let previous = app.session.prefs().dialogs.get(CONFIRM_PREF).cloned();
                app.session.edit_prefs(|prefs| {
                    prefs.dialogs.insert(CONFIRM_PREF.into(), json!({"skip": true}));
                });
                if let Err(error) = crate::prefs_ui::save_preferences(app) {
                    app.session.edit_prefs(|prefs| {
                        if let Some(previous) = previous {
                            prefs.dialogs.insert(CONFIRM_PREF.into(), previous);
                        } else {
                            prefs.dialogs.remove(CONFIRM_PREF);
                        }
                    });
                    let error = crate::i18n::fmt(tl!("Action saved, but confirmation preference could not be saved: {error}"), &[("error", &error)]);
                    self.error = Some(error.clone());
                    return Err(error); // Keep prompt and owned draft for an explicit retry or cancel.
                }
            }
            if intent == Intent::Save {
                self.prompt = None;
                return Ok(Some(Event::Saved { source: self.source }));
            }
        }
        self.call(app, "discard", json!({}))?;
        self.prompt = None;
        self.closed = true;
        Ok(Some(Event::Closed { saved: matches!(answer, Answer::Save), source: self.source }))
    }

    /// Stage local metadata before requesting preview of this draft from the selected original step index.
    fn preview(&mut self, app: &mut PhotocraftApp, from: usize) -> Result<Event, String> {
        let view = self.stage(app)?;
        if from >= steps(&view).len() {
            return Err(tl!("Select an existing action step to play").into());
        }
        Ok(Event::Preview { draft: self.draft, from })
    }

    fn step(&mut self, app: &mut PhotocraftApp, params: Value) -> Result<(), String> {
        let op = params.get("op").and_then(Value::as_str).unwrap_or("");
        let at = params.get("step").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok());
        let to = params.get("to").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok());
        let selected = self.selected;
        let expanded = self.expanded.clone();
        self.call(app, "step", params.clone())?;
        let remap = |index: usize| -> Option<usize> {
            let Some(at) = at else { return Some(index) };
            match op {
                "insert" => Some(if index >= at { index.saturating_add(1) } else { index }),
                "duplicate" => Some(if index > at { index.saturating_add(1) } else { index }),
                "delete" => {
                    if index == at {
                        None
                    } else {
                        Some(if index > at { index - 1 } else { index })
                    }
                }
                "move" => {
                    let Some(to) = to else { return Some(index) };
                    Some(if index == at {
                        to
                    } else if at < to && index > at && index <= to {
                        index - 1
                    } else if at > to && index >= to && index < at {
                        index.saturating_add(1)
                    } else {
                        index
                    })
                }
                _ => Some(index),
            }
        };
        self.selected = selected.and_then(remap);
        self.expanded = expanded.into_iter().filter_map(remap).collect();
        if op == "insert" {
            self.selected = at;
        } else if op == "duplicate" {
            self.selected = at.map(|index| index.saturating_add(1));
        }
        Ok(())
    }

    fn insert_stop(&mut self, view: &Value) {
        let list = steps(view);
        let at = self.selected.filter(|index| *index < list.len()).map_or(list.len(), |index| index.saturating_add(1));
        let previous = at.checked_sub(1).and_then(|index| list.get(index)).cloned();
        self.editing = Some(StepEdit {
            draft: self.draft,
            target: Target::Insert { at, len: list.len(), previous },
            command: photocraft_engine::atn_cmds::STOP_MESSAGE.into(),
            params: json!({"message": "", "allowContinue": false}).as_object().cloned().unwrap_or_default(),
        });
    }

    fn edit_settings(&mut self, view: &Value, at: usize) {
        let Some(original) = steps(view).get(at) else { return };
        let Some((command, params)) = step_parts(original) else { return };
        if editable(command)
            && let Some(params) = params.as_object()
        {
            self.editing = Some(StepEdit {
                draft: self.draft,
                target: Target::Replace { at, original: original.clone() },
                command: command.into(),
                params: params.clone(),
            });
        }
    }

    fn accept_step(&mut self, app: &mut PhotocraftApp) -> Result<(), String> {
        let edit = self.editing.as_ref().ok_or("There is no step being edited")?.clone();
        if edit.draft != self.draft {
            return Err(tl!("This step dialog belongs to another editor").into());
        }
        let view = self.get(app)?;
        let list = steps(&view);
        let (op, at, valid) = match edit.target {
            Target::Replace { at, original } => ("replace", at, list.get(at) == Some(&original)),
            Target::Insert { at, len, previous } => {
                let current = at.checked_sub(1).and_then(|index| list.get(index)).cloned();
                ("insert", at, list.len() == len && at <= len && current == previous)
            }
        };
        if !valid {
            let error = tl!("The draft steps changed. Cancel this dialog and reopen the step.").to_string();
            self.error = Some(error.clone());
            return Err(error);
        }
        self.step(app, json!({"op": op, "step": at, "id": edit.command, "params": edit.params}))?;
        self.editing = None;
        Ok(())
    }

    fn metadata(&mut self, app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &Value, swatches: &[Color32; 12]) -> Option<Value> {
        let mut latest = None;
        let picker_id = ui.scope_id().with(("thumbnail", self.draft));
        let choice = editor_picker::show(ui, picker_id, action_text(view, "icon"), action_text(view, "color"), swatches);
        if let Some(choice) = choice {
            let params = match choice {
                Choice::Icon(icon) => json!({"icon": icon}),
                Choice::Color(color) => json!({"color": color}),
            };
            latest = self.call(app, "update", params).ok();
        }
        let name = ui.label(tl!("Name"));
        let name_edit = ui.add(egui::TextEdit::singleline(&mut self.name.text).desired_width(f32::INFINITY)).labelled_by(name.id);
        let description = ui.label(tl!("Description"));
        let description_edit = ui
            .add(
                egui::TextEdit::multiline(&mut self.description.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(tl!("Add a brief description for this action…")),
            )
            .labelled_by(description.id);
        ui.label(tl!("Keyboard shortcut"));
        let (key, mut shift, mut control) = photocraft_engine::atn_cmds::parse_function_key(&self.shortcut.text).unwrap_or((0, false, false));
        let mut chosen = key;
        let label = if key == 0 && !self.shortcut.text.is_empty() {
            self.shortcut.text.clone()
        } else if key == 0 {
            tl!("None").into()
        } else {
            format!("F{key}")
        };
        let mut shortcut_changed = false;
        ui.horizontal(|ui| {
            craft_ui::ComboBox::from_id_salt("action-editor-fn").selected_text(label).show_ui(ui, |ui| {
                for number in 0..=15 {
                    let label = if number == 0 { tl!("None").to_string() } else { format!("F{number}") };
                    shortcut_changed |= ui.selectable_value(&mut chosen, number, label).changed();
                }
            });
            ui.add_enabled_ui(chosen > 0, |ui| {
                shortcut_changed |= ui.checkbox(&mut shift, tl!("Shift")).changed();
                shortcut_changed |= ui.checkbox(&mut control, if cfg!(target_os = "macos") { tl!("Command") } else { tl!("Control") }).changed();
            });
        });
        if shortcut_changed {
            self.shortcut.text = if chosen == 0 { String::new() } else { photocraft_engine::atn_cmds::function_key_name(chosen, shift, control) };
        }
        if (name_edit.lost_focus() || description_edit.lost_focus() || shortcut_changed)
            && (self.name.dirty() || self.description.dirty() || self.shortcut.dirty())
            && let Ok(view) = self.stage(app)
        {
            latest = Some(view);
        }
        latest
    }

    fn insert_menu(&mut self, ui: &mut egui::Ui, view: &Value) {
        for label in ["Insert Action", "Insert Menu Item"] {
            ui.add_enabled(false, egui::Button::new(tl!(label)));
        }
        if ui.button(tl!("Insert Stop")).clicked() {
            self.insert_stop(view);
            ui.close();
        }
        for label in ["Insert Conditional", "Insert Path"] {
            ui.add_enabled(false, egui::Button::new(tl!(label)));
        }
    }

    fn toolbar(&mut self, app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &Value) -> Option<Event> {
        let recording = view.get("recording").and_then(Value::as_bool) == Some(true);
        let mut event = None;
        ui.horizontal(|ui| {
            ui.heading(if recording { tl!("Recording") } else { tl!("Steps") });
            if recording {
                let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, Tokens::get(ui.ctx()).danger);
            }
            if ui.button(if recording { tl!("Stop") } else { tl!("Record") }).clicked() {
                let _ = self.call(app, if recording { "stop" } else { "record" }, json!({}));
            }
            ui.menu_button("+", |ui| self.insert_menu(ui, view)).response.on_hover_text(tl!("Insert…"));
            ui.add_enabled_ui(!steps(view).is_empty(), |ui| {
                ui.menu_button(tl!("Play…"), |ui| {
                    if ui.button(tl!("Play action")).clicked() {
                        event = self.preview(app, 0).ok();
                        ui.close();
                    }
                    if ui.add_enabled(self.selected.is_some(), egui::Button::new(tl!("Play from selected step"))).clicked() {
                        if let Some(from) = self.selected {
                            event = self.preview(app, from).ok();
                        }
                        ui.close();
                    }
                });
                let expanded = self.expanded.len() == steps(view).len();
                if ui.button(if expanded { tl!("Collapse all") } else { tl!("Expand all") }).clicked() {
                    self.expanded = if expanded { BTreeSet::new() } else { (0..steps(view).len()).collect() };
                }
            });
        });
        event
    }

    fn rows(&mut self, app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &Value) {
        if steps(view).is_empty() {
            ui.label(tl!("No steps yet"));
            ui.label(tl!("Start recording to build your action"));
            return;
        }
        for (at, value) in steps(view).iter().enumerate() {
            let Some((command, params)) = step_parts(value) else {
                ui.label(tl!("Stored action step"));
                continue;
            };
            ui.push_id(at, |ui| {
                let mut enabled = !flagged(view, "disabled", at);
                let mut dialog = flagged(view, "dialogs", at);
                let row_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 24.0));
                let controls = ui.input(|input| input.pointer.hover_pos()).is_some_and(|point| row_rect.contains(point) && ui.clip_rect().contains(point))
                    || self.selected == Some(at);
                ui.horizontal(|ui| {
                    let expanded = self.expanded.contains(&at);
                    if icons::button(ui, if expanded { "chevron-down" } else { "chevron-right" }, 20.0, false, tl!("Show step settings")).clicked() {
                        if expanded {
                            self.expanded.remove(&at);
                        } else {
                            self.expanded.insert(at);
                        }
                    }
                    let label = super::step_label(command, params);
                    let text = egui::RichText::new(label).color(if enabled { Tokens::get(ui.ctx()).text } else { Tokens::get(ui.ctx()).text_faint });
                    let row = ui.selectable_label(self.selected == Some(at), text);
                    if row.clicked() {
                        self.selected = Some(at);
                    }
                    if controls || !enabled {
                        if ui.checkbox(&mut enabled, "").on_hover_text(tl!("Enable step")).changed() {
                            let _ = self.step(app, json!({"op": "enabled", "step": at, "enabled": enabled}));
                        }
                    } else {
                        ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                    }
                    if controls {
                        if ui.checkbox(&mut dialog, "").on_hover_text(tl!("Show dialog during playback")).changed() {
                            let _ = self.step(app, json!({"op": "dialog", "step": at, "enabled": dialog}));
                        }
                    } else {
                        ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                    }
                    if editable(command) {
                        if icons::button(ui, "sliders-horizontal", 24.0, false, tl!("Edit step settings")).clicked() {
                            self.edit_settings(view, at);
                        }
                    } else {
                        ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                    }
                    if controls {
                        let menu = icons::button(ui, "ellipsis", 24.0, false, tl!("Steps"));
                        egui::Popup::menu(&menu).show(|ui| {
                            self.selected = Some(at);
                            self.insert_menu(ui, view);
                            ui.separator();
                            for (label, op) in [(tl!("Duplicate step"), "duplicate"), (tl!("Delete step"), "delete")] {
                                if ui.button(label).clicked() {
                                    let _ = self.step(app, json!({"op": op, "step": at}));
                                    ui.close();
                                }
                            }
                            if ui.add_enabled(at > 0, egui::Button::new(tl!("Move step up"))).clicked() {
                                let _ = self.step(app, json!({"op": "move", "step": at, "to": at.saturating_sub(1)}));
                                ui.close();
                            }
                            if ui.add_enabled(at.saturating_add(1) < steps(view).len(), egui::Button::new(tl!("Move step down"))).clicked() {
                                let _ = self.step(app, json!({"op": "move", "step": at, "to": at.saturating_add(1)}));
                                ui.close();
                            }
                        });
                    } else {
                        ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                    }
                });
                if self.expanded.contains(&at)
                    && let Some(params) = params.as_object()
                {
                    ui.indent("settings", |ui| {
                        for (key, value) in params.iter().filter(|(key, _)| !key.starts_with("__")) {
                            ui.label(format!("{}: {}", filter_dialog::label(key), summary(value)));
                        }
                    });
                }
            });
        }
    }

    fn confirmation(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context, activity: Activity) -> Option<Event> {
        self.prompt?;
        let mut answer = None;
        let modal = egui::Modal::new(Id::unique("action-editor-save").with(self.draft)).show(ctx, |ui| {
            ui.heading(tl!("Save Action"));
            ui.label(tl!("There are unsaved changes. Do you want to save or discard them?"));
            ui.checkbox(&mut self.dont_show_again, tl!("Don't show again"));
            if let Some(error) = &self.error {
                ui.colored_label(Tokens::get(ui.ctx()).danger, error);
            }
            ui.horizontal(|ui| {
                let enabled = !activity.playback_busy && !activity.finishing_step;
                let save = ui.add_enabled(enabled, egui::Button::new(tl!("Save")));
                if !self.prompt_focused && enabled {
                    save.request_focus();
                    self.prompt_focused = true;
                }
                if save.clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.add_enabled(!activity.playback_busy, egui::Button::new(tl!("Discard"))).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button(tl!("Cancel and continue editing")).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(Answer::Cancel);
        }
        answer.and_then(|answer| self.answer(app, answer, activity).ok()).flatten()
    }

    fn step_dialog(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        let Some(edit) = self.editing.as_mut() else { return };
        let mut accept = false;
        let mut cancel = false;
        let modal = egui::Modal::new(Id::unique("action-editor-step").with(self.draft)).show(ctx, |ui| {
            ui.heading(if matches!(&edit.target, Target::Insert { .. }) { tl!("Insert Stop") } else { tl!("Edit step settings") });
            if edit.command == photocraft_engine::atn_cmds::STOP_MESSAGE {
                stop_fields(ui, &mut edit.params);
            } else if let Some(spec) = photocraft_engine::commands::find(&edit.command) {
                for param in filter_dialog::parse_spec(spec.params).into_iter().filter(|param| !param.key.starts_with("__")) {
                    field(ui, &param, &mut edit.params);
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(Tokens::get(ui.ctx()).danger, error);
            }
            ui.horizontal(|ui| {
                accept = ui.button(tl!("OK")).clicked();
                cancel = ui.button(tl!("Cancel")).clicked();
            });
        });
        if cancel || modal.should_close() {
            self.editing = None;
        } else if accept {
            let _ = self.accept_step(app);
        }
    }

    /// Render only after parent extraction from its runtime state, passing the shared adapter's activity.
    pub fn show(&mut self, app: &mut PhotocraftApp, ui: &mut egui::Ui, swatches: &[Color32; 12], activity: Activity) -> Option<Event> {
        if self.closed {
            return None;
        }
        let mut view = match self.get(app) {
            Ok(view) => view,
            Err(_) => {
                ui.label(&self.name.text);
                if let Some(error) = &self.error {
                    ui.colored_label(Tokens::get(ui.ctx()).danger, error);
                }
                let event = self.confirmation(app, ui.ctx(), activity);
                self.step_dialog(app, ui.ctx());
                return event; // Never rebind a stale page to whichever draft is now open.
            }
        };
        let mut event = None;
        ui.push_id(self.draft, |ui| {
            ui.add_enabled_ui(!activity.playback_busy && !self.has_modal(), |ui| {
                if icons::button(ui, "chevron-left", 28.0, false, tl!("Back")).clicked() {
                    event = self.request_leave(app, activity).ok().flatten();
                }
                if self.closed || self.has_modal() {
                    return;
                }
                if let Some(updated) = self.metadata(app, ui, &view, swatches) {
                    view = updated;
                }
                event = self.toolbar(app, ui, &view).or(event.take());
                egui::ScrollArea::vertical()
                    .id_salt("draft-steps")
                    .max_height((ui.available_height() - 48.0).max(100.0))
                    .show(ui, |ui| self.rows(app, ui, &view));
                if activity.finishing_step {
                    ui.label(tl!("Finishing step…"));
                }
                if ui.add_enabled(self.dirty(&view) && !activity.finishing_step, egui::Button::new(tl!("Save"))).clicked() {
                    event = self.ask_save(app, activity).ok().flatten().or(event.take());
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(Tokens::get(ui.ctx()).danger, error);
            }
        });
        if self.prompt.is_some() {
            event = self.confirmation(app, ui.ctx(), activity).or(event);
        }
        if self.editing.is_some() {
            self.step_dialog(app, ui.ctx());
        }
        event
    }

    /// A pending leave question must remain visible when another dock tab is active.
    pub(super) fn show_modals(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context, activity: Activity) -> Option<Event> {
        let event = if self.prompt.is_some() { self.confirmation(app, ctx, activity) } else { None };
        if self.editing.is_some() {
            self.step_dialog(app, ctx);
        }
        event
    }
}

fn stop_fields(ui: &mut egui::Ui, values: &mut Map<String, Value>) {
    let label = ui.label(tl!("Message:"));
    let mut message = values.get("message").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::multiline(&mut message).desired_rows(4).desired_width(320.0)).labelled_by(label.id).changed() {
        values.insert("message".into(), json!(message));
    }
    let mut allow = values.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    if ui.checkbox(&mut allow, tl!("Allow Continue")).changed() {
        values.insert("allowContinue".into(), json!(allow));
    }
}

/// Pure definition widgets: only explicitly changed fields are staged, retaining unknown params.
fn field(ui: &mut egui::Ui, param: &filter_dialog::Param, values: &mut Map<String, Value>) {
    use filter_dialog::Kind;
    let label = filter_dialog::label(&param.key);
    let mut update = None;
    ui.horizontal(|ui| {
        let named = ui.label(&label);
        match &param.kind {
            Kind::Bool(default) => {
                let mut value = values.get(&param.key).and_then(Value::as_bool).unwrap_or(*default);
                if ui.checkbox(&mut value, "").labelled_by(named.id).changed() {
                    update = Some(json!(value));
                }
            }
            Kind::Text => {
                let mut value = values.get(&param.key).and_then(Value::as_str).unwrap_or("").to_string();
                if ui.text_edit_singleline(&mut value).labelled_by(named.id).changed() {
                    update = Some(json!(value));
                }
            }
            Kind::Int { default } => {
                let mut value = values.get(&param.key).and_then(Value::as_i64).unwrap_or(*default);
                if ui.add(egui::DragValue::new(&mut value)).labelled_by(named.id).changed() {
                    update = Some(json!(value));
                }
            }
            Kind::Range { min, max, default } if min.is_finite() && max.is_finite() && min <= max => {
                let stored = values.get(&param.key).and_then(Value::as_f64).filter(|value| value.is_finite()).unwrap_or(f64::from(*default));
                let mut value = stored.clamp(f64::from(*min), f64::from(*max));
                if ui.add(egui::DragValue::new(&mut value).range(f64::from(*min)..=f64::from(*max))).labelled_by(named.id).changed() {
                    update = Some(json!(value));
                }
            }
            Kind::Choice { options, default } => {
                let mut value = values
                    .get(&param.key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| default.clone())
                    .or_else(|| options.first().cloned())
                    .unwrap_or_default();
                craft_ui::ComboBox::from_id_salt(&param.key).selected_text(&value).show_ui(ui, |ui| {
                    for option in options {
                        if ui.selectable_value(&mut value, option.clone(), tl!(option)).changed() {
                            update = Some(json!(value));
                        }
                    }
                });
            }
            _ => {
                ui.label(values.get(&param.key).map(summary).unwrap_or_else(|| tl!("None").to_string()));
            }
        }
    });
    if let Some(value) = update {
        values.insert(param.key.clone(), value);
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
