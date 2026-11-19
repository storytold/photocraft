//! Generative Fill in the app: the Generative Fill and Remove dialogs' rows, the Preferences › Integrations model
//! row, and the worker-thread model lookups behind both.
//!
//! A model row follows the server its backend and host name: it asks that server for its models once, again when
//! either changes (after a short pause for typing), and on Refresh. While no list has come back the model is a text
//! box; once one has, it is a menu of what the server has. A list from a server other than the one shown is dropped.

use std::sync::{Arc, Mutex, PoisonError};

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::Text;
use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use craft::ui::patterns::preferences::SettingsDraft;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use photocraft_engine::generative_fill_cmds;
use serde_json::{Map, Value, json};

use super::PhotocraftApp;

/// The commands whose dialog these rows belong to.
pub(crate) const COMMANDS: [&str; 2] = ["edit.generativeFill", "edit.generativeRemove"];

/// The rows the dialog keeps behind its Advanced Options check box, so the ordinary dialog is the prompt and the
/// model (#6054).
const ADVANCED: [&str; 8] = ["negativePrompt", "backend", "host", "steps", "guidance", "strength", "resolution", "seed"];

/// The dialog's Advanced Options check box. Its key starts with `__`, so it never reaches the command.
const SHOW_ADVANCED: &str = "__advanced";

/// How long a typed backend or host must stay unchanged before its server is asked.
const TYPING_PAUSE: f64 = 0.6;

const PREF_BACKEND: &str = "integrations.generativeFillBackend";
const PREF_HOST: &str = "integrations.generativeFillHost";
pub(super) const PREF_MODEL: &str = "integrations.generativeFillModel";

/// A model lookup running on a worker thread; its reply lands here when it ends.
type LookupSlot = Arc<Mutex<Option<Result<Value, String>>>>;

/// Ask a server for its models on a worker thread. The lookup waits on the network and resolves the host name, so it
/// must not run on the interface's thread: the window stays responsive, Cancel included, while a slow or silent
/// server is asked.
fn start_lookup(backend: String, host: String) -> LookupSlot {
    let slot = LookupSlot::default();
    let out = slot.clone();
    let work = move || {
        let reply = generative_fill_cmds::lookup_models(&backend, &host).map_err(|e| e.to_string());
        *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(reply);
    };
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("model-lookup".into()).spawn(work).is_err() {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(Err("couldn't start a worker thread".into()));
    }
    // The browser has no threads; the lookup refuses at once there.
    #[cfg(target_arch = "wasm32")]
    work();
    slot
}

fn take_reply(slot: &LookupSlot) -> Option<Result<Value, String>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// The model names in a lookup reply, the server's default, and the one-line note about it. Names the dialog can't
/// tell apart from its notation (`|` and `"`) are left out.
pub(super) fn lookup_summary(reply: &Result<Value, String>) -> (Vec<String>, String, String) {
    match reply {
        Ok(v) => {
            let names: Vec<String> =
                v["models"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| !n.contains(['|', '"'])).map(str::to_string).collect();
            let server = v["server"].as_str().or_else(|| v["host"].as_str()).unwrap_or_default();
            let default = v["default"].as_str().unwrap_or_default().to_string();
            let note = if names.is_empty() {
                crate::i18n::fmt(tl!("{server} has no models installed"), &[("server", server)])
            } else {
                crate::i18n::fmt(tl!("{count} models found on {server}"), &[("count", names.len().to_string().as_str()), ("server", server)])
            };
            (names, default, note)
        }
        Err(e) => (Vec::new(), String::new(), crate::i18n::fmt(tl!("Couldn't list models: {error}"), &[("error", e.as_str())])),
    }
}

/// The host a backend and a typed host name: a blank host stands for the usual one on this machine.
pub(super) fn server_host(backend: &str, typed: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() { generative_fill_cmds::default_host_for(backend).unwrap_or_default() } else { typed.to_string() }
}

/// The model list of one row, kept true to the server the row's backend and host name.
#[derive(Default)]
pub(super) struct ModelList {
    /// The `backend|host` the row names now, and when it started naming it.
    want: String,
    want_at: f64,
    /// The `backend|host` the list below belongs to; empty before any answer.
    listed: String,
    /// The lookup in flight, as the `backend|host` it asks about and its slot.
    running: Option<(String, LookupSlot)>,
    pub(super) models: Vec<String>,
    pub(super) default: String,
    pub(super) note: String,
    /// The model the row put in itself rather than the user: it yields to the next server's default.
    seed: Option<String>,
}

impl ModelList {
    /// Follows the server `backend` at `host` at time `now` (seconds): takes a finished lookup's answer if it is for
    /// that server, and asks again when the list is for another server (after the typing pause, or at once on the
    /// first pass or `refresh`). Returns whether the row should be drawn again soon.
    pub(super) fn follow(&mut self, backend: &str, host: &str, now: f64, refresh: bool) -> bool {
        let want = format!("{backend}|{host}");
        let first = self.want.is_empty();
        if self.want != want {
            self.want.clone_from(&want);
            self.want_at = now;
        }
        if let Some((asked, slot)) = &self.running
            && let Some(reply) = take_reply(slot)
        {
            if *asked == want {
                let (models, default, note) = lookup_summary(&reply);
                self.models = models;
                self.default = default;
                self.note = note;
                self.listed.clone_from(&want);
            }
            self.running = None;
        }
        if refresh {
            self.listed.clear();
        }
        let asking = self.running.as_ref().is_some_and(|(asked, _)| *asked == want);
        if self.listed == want || asking {
            return asking;
        }
        if first || refresh || now - self.want_at >= TYPING_PAUSE {
            self.running = Some((want, start_lookup(backend.to_string(), host.to_string())));
            self.models.clear();
            self.note = crate::i18n::fmt(tl!("Looking for models on {host}…"), &[("host", host)]);
        }
        true
    }

    /// Settles the model shown once the server has answered. A model the row filled in itself (the saved one, or
    /// nothing) becomes the first of `saved`, the server's default and its first model that the server has. A model
    /// the user typed or picked stays theirs, and the note says when the server doesn't list it.
    pub(super) fn settle(&mut self, current: &mut String, saved: &str) {
        if self.listed != self.want || self.models.is_empty() || self.models.contains(current) && self.seed.as_deref() != Some(current.as_str()) {
            return;
        }
        let ours = current.is_empty() || self.seed.as_deref() == Some(current.as_str());
        if !ours {
            if !self.note.contains(current.as_str()) {
                self.note = crate::i18n::fmt(tl!("{note} ({model} isn't one of them)"), &[("note", self.note.as_str()), ("model", current.as_str())]);
            }
            return;
        }
        let chosen = [current.as_str(), saved, self.default.as_str()].into_iter().find(|m| self.models.iter().any(|n| n == m)).unwrap_or(&self.models[0]);
        *current = chosen.to_string();
        self.seed = Some(current.clone());
    }

    /// Marks `model` as one the row filled in itself.
    pub(super) fn seed_with(&mut self, model: &str) {
        self.seed = Some(model.to_string()).filter(|m| !m.is_empty());
    }
}

/// The model picker: a menu of `models` (plus `auto` as an empty choice, and a current model the server doesn't list,
/// so opening never changes it), or a text box while there is no list. Returns whether `current` changed.
fn model_picker(ui: &mut egui::Ui, salt: &str, label: &str, current: &mut String, models: &[String], auto: Option<&str>) -> bool {
    let before = current.clone();
    if models.is_empty() {
        TextField::new(label.to_string(), Bind::bound(current)).id_salt((salt, "text")).show(ui);
    } else {
        let mut entries: Vec<Entry<String>> = Vec::new();
        if let Some(auto) = auto {
            entries.push(Entry::new(String::new(), auto.to_string()));
        }
        entries.extend(models.iter().map(|m| Entry::new(m.clone(), m.clone())));
        if !current.is_empty() && !models.contains(current) {
            entries.push(Entry::new(current.clone(), current.clone()));
        }
        if let Some(picked) = Select::new(label.to_string(), Bind::given(current.clone()), &entries).id_salt((salt, "menu")).show(ui).value().cloned() {
            *current = picked;
        }
    }
    *current != before
}

// --- The dialog ----------------------------------------------------------------------------------------------------

/// Whether `key` of `command` belongs behind the dialog's Advanced Options.
pub(crate) fn is_advanced_param(command: &str, key: &str) -> bool {
    COMMANDS.contains(&command) && ADVANCED.contains(&key)
}

/// The Generative Fill and Remove dialog rows, from the command's own: the prompts are a few lines that wrap, the
/// sampler controls and the server override sit behind Advanced Options, and the model is a row that asks the server
/// the dialog names for its models.
pub(crate) fn dialog_rows(app: &PhotocraftApp, command: &str, params: &mut Vec<Param>) {
    if !COMMANDS.contains(&command) {
        return;
    }
    let saved = app.session.prefs().integrations.generative_fill_model.clone();
    let rows = std::mem::take(params).into_iter().map(|p| {
        if p.key == "model" {
            return model_row(saved.clone());
        }
        let p = if p.key.to_ascii_lowercase().ends_with("prompt") { p.multiline(3) } else { p };
        if is_advanced_param(command, &p.key) { p.visible_when(SHOW_ADVANCED, true) } else { p }
    });
    // The rows that stay on the dialog itself, then the Advanced Options check box and the rows behind it.
    let (rest, advanced): (Vec<Param>, Vec<Param>) = rows.partition(|p| !is_advanced_param(command, &p.key));
    *params = rest;
    if !advanced.is_empty() {
        params.push(Param::new(SHOW_ADVANCED, Label::from(tl!("Advanced Options").to_string()), ParamKind::bool(false)).not_remembered());
        params.extend(advanced);
    }
}

/// The dialog's model row over a lookup of its own; it opens with the model saved in Preferences.
fn model_row(saved: String) -> Param {
    let list: Arc<Mutex<ModelList>> = Arc::default();
    list.lock().unwrap_or_else(PoisonError::into_inner).seed_with(&saved);
    let opening = saved.clone();
    Param::custom("model", Label::from(tl!("Model").to_string()), move |ui, fields, _| {
        let backend = fields.text("backend").unwrap_or("comfyui").to_string();
        let host = server_host(&backend, fields.text("host").unwrap_or_default());
        let mut current = fields.text("model").unwrap_or_default().to_string();
        let now = ui.input(|i| i.time);
        let mut list = list.lock().unwrap_or_else(PoisonError::into_inner);
        let response = ui
            .vertical(|ui| {
                if list.follow(&backend, &host, now, false) {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                }
                list.settle(&mut current, &saved);
                model_picker(ui, "gf-dialog-model", tl!("Model"), &mut current, &list.models, None);
                if !list.note.is_empty() {
                    Text::new(list.note.clone()).role(TypeRole::Caption).show(ui);
                }
            })
            .response;
        fields.set("model", current);
        response
    })
    .opens_with(opening)
}

/// The values the dialog opens with: the server a fill would use (the environment, then Preferences ›
/// Integrations), so a fill goes to the server whose models are shown.
pub(crate) fn dialog_defaults(app: &PhotocraftApp, command: &str, map: &mut Map<String, Value>) {
    if !COMMANDS.contains(&command) {
        return;
    }
    let saved = &app.session.prefs().integrations;
    let (backend, host) = generative_fill_cmds::effective_server(&app.session, &json!({}))
        .unwrap_or_else(|_| (saved.generative_fill_backend.name().to_string(), saved.generative_fill_host.clone()));
    let host = server_host(&backend, &host);
    map.insert("backend".into(), json!(backend));
    map.insert("host".into(), json!(host));
    if !saved.generative_fill_model.is_empty() {
        map.insert("model".into(), json!(saved.generative_fill_model));
    }
}

// --- Preferences › Integrations ------------------------------------------------------------------------------------

/// Preferences › Integrations › Generative Fill Model: the models of the server the (unsaved) backend and host name,
/// as a menu with "Automatic"; a text box while no server has answered. Its lookup lives in the context's memory, so
/// it outlasts the window's passes.
pub(super) fn model_pref_row(ui: &mut egui::Ui, draft: &mut SettingsDraft) {
    let backend: String = draft.get(PREF_BACKEND).unwrap_or_else(|| "comfyui".into());
    let typed: String = draft.get(PREF_HOST).unwrap_or_default();
    let host = server_host(&backend, &typed);
    let stored: String = draft.get(PREF_MODEL).unwrap_or_default();
    let mut current = stored.clone();
    let list: Arc<Mutex<ModelList>> = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<ModelList>>>(egui::Id::unique("prefs-gf-models")).clone());
    let mut list = list.lock().unwrap_or_else(PoisonError::into_inner);
    let label = tl!("Generative Fill Model");
    Text::new(label.to_string()).role(TypeRole::Body).show(ui);
    let mut refresh = false;
    ui.horizontal(|ui| {
        model_picker(ui, "gf-pref-model", label, &mut current, &list.models, Some(tl!("Automatic (the server's choice)")));
        refresh = Button::new(tl!("Refresh")).show(ui).clicked();
    });
    let now = ui.input(|i| i.time);
    if list.follow(&backend, &host, now, refresh) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if !list.note.is_empty() {
        Text::new(list.note.clone()).role(TypeRole::Caption).show(ui);
    }
    if current != stored {
        draft.set(PREF_MODEL, &current);
    }
}

#[cfg(test)]
mod tests;
