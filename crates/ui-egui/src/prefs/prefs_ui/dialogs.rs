//! The Edit-menu front end (`invoke`) and the Preferences window as a dialog on the framework's host.
//!
//! The window is a [`SettingsDocument`] over the engine's preferences, shown by `PreferencesWindow` once per pass
//! while its [`PrefsDialog`] is open. Apply and OK leave the keys they changed in the document and the dialog files
//! them; `take` writes just those through `prefs.set` and `prefs.reset`, so a value changed elsewhere while the
//! window is open survives. The dialog runs inside the host and has no app: `feed` hands it the app's facts before
//! each pass, and `take` carries its requests (edits, a colour to pick) out after it.

use std::sync::Arc;

use craft::host::Host;
use craft::i18n::Lang;
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
#[cfg(test)]
use craft::ui::patterns::preferences::PrefValue;
use craft::ui::patterns::preferences::{Closed, PageInfo, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, SettingsDocument, StandardPage};
use craft::ui::runtime::env::{Words, WordsOverride, with_env};
use craft::ui::runtime::memory::SaveKey;
use craft::ui::runtime::requests::ColorRequest;
use photocraft_engine::prefs::SECTIONS;
use serde_json::{Value, json};

use super::custom_rows::PrefsCustom;
use super::fields::{PrefPages, rules};
use crate::PhotocraftApp;
use crate::color_picker_ui::PickWindow;

/// The document the window edits between passes. The custom rows borrow the app, so each pass attaches `PrefsCustom`
/// for its own duration and takes it off again before the app is used mutably.
type Doc = SettingsDocument<PrefPages, ()>;

// ------------------------------------------------------------------ menu routing

/// Shell front ends for Edit-menu commands invoked without parameters (dialogs, pickers).
/// `None` when `id` isn't handled here.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|o| o.is_empty());
    if !empty {
        return None;
    }
    if let Some(section) = id.strip_prefix("edit.preferences.") {
        open_preferences(app, section);
        return Some(Ok(json!({"dialog": "preferences"})));
    }
    let dialog = |d: Result<DialogId, String>| Some(d.map(|d| json!({"dialog": d.value().to_string()})));
    match id {
        "edit.keyboardShortcuts" => {
            super::shortcuts::open(app, 0);
            Some(Ok(json!({"dialog": "shortcuts"})))
        }
        "edit.menus" => {
            super::shortcuts::open(app, 1);
            Some(Ok(json!({"dialog": "shortcuts"})))
        }
        "edit.toolbar" => {
            crate::panels::toolbar::open_drawer(app);
            Some(Ok(json!({})))
        }
        "edit.colorSettings" => {
            // What Monitor Profile resolves to, so a fallback to sRGB is visible (#569): read the displays again now. The
            // dialog draws the line from the note's cell, which `read_now` fills before the dialog's first frame.
            crate::monitor_status::read_now(app);
            dialog(crate::filter_dialog::open(app, "edit.colorSettings"))
        }
        "edit.fade" | "edit.findAndReplaceText" | "edit.defineBrushPreset" | "edit.defineCustomShape" | "edit.autoAlignLayers" | "edit.autoBlendLayers" => {
            if !app.session.is_enabled(id) {
                return Some(Err(photocraft_engine::commands::find(id)
                    .map_or("not available".into(), |c| format!("{} is not available right now", c.label.trim_end_matches('…')))));
            }
            dialog(crate::filter_dialog::open(app, id))
        }
        // Photoshop's Content-Aware Fill workspace (caf_ui); the engine command with params
        // still runs directly.
        "edit.contentAwareFill" => Some(crate::caf_ui::open(app).map(|_| json!({"contentAwareFill": crate::caf_ui::describe(app)}))),
        // Photoshop has no dialog: the menu opens the scale box (with params: the engine).
        crate::cas_transform::COMMAND => Some(crate::cas_transform::begin(app, ctx)),
        "edit.presets.presetManager" => Some(Ok(json!({"dialog": super::presets::open_manager(app).value().to_string()}))),
        "edit.presets.exportImportPresets" => Some(Ok(json!({"dialog": super::presets::open_io(app).value().to_string()}))),
        _ => None,
    }
}

// ------------------------------------------------------------------ the window

/// The edits Apply or OK made, as the `values` of `prefs.set` and the paths `prefs.reset` takes.
type Edits = (Value, Vec<String>);

/// What OK answers with: the edits the window made that no pass has handed on yet.
pub(crate) struct PrefsAnswer(Vec<Edits>);

/// The open Preferences window, a dialog on the framework's host under the key `preferences`.
pub(crate) struct PrefsDialog {
    /// The saved values, the draft and the rows' rules. Taken out while a pass runs, because the custom rows attach to
    /// it by value.
    doc: Option<Doc>,
    /// The page, search text and Apply error the window keeps.
    state: PreferencesState,
    /// The engine's preferences revision the document was last based on.
    saved_rev: u64,
    /// The language the document's labels are in: the draft's language, which previews before Apply.
    labels: Lang,
    /// Cached framework words for that same draft language, including the footer and window title.
    words: Arc<Words>,
    /// What the custom rows read from the app, as of the last `feed`.
    custom: PrefsCustom,
    /// The colour row whose swatch was pressed, until `take` opens the picker for it.
    pick: Option<ColorRequest>,
    /// The edits Apply or OK made, until `take` writes them.
    edits: Vec<Edits>,
}

impl PrefsDialog {
    /// Moves the keys the document's last Apply changed into `edits`.
    fn file_edits(&mut self) {
        if let Some(edits) = self.doc.as_mut().and_then(Doc::take_applied_edits) {
            self.edits.push(PrefPages::edits_to_values(edits));
        }
    }

    /// Applies the draft as the Apply button does. `Err` when a colour picker the window opened is still open or the
    /// draft doesn't fit the settings; the message also shows in the window's footer.
    fn apply(&mut self) -> Result<(), String> {
        let Some(doc) = self.doc.take() else { return Err("Preferences is not open".into()) };
        let mut with_custom = doc.custom(self.custom.clone());
        let applied = if with_custom.nested_open() { Err(tl!("Close the Color Picker first.").to_string()) } else { with_custom.apply() };
        self.doc = Some(with_custom.custom(()));
        self.file_edits();
        self.state.error = applied.as_ref().err().cloned();
        applied
    }

    /// Updates what the window reads of the app before a pass: the custom rows' facts, the saved values (an unedited row
    /// follows them; `Err` from the rebase keeps the document as it was) and the labels' language.
    fn feed(&mut self, custom: PrefsCustom, saved: Option<(u64, PrefPages)>, host: &Host) {
        self.custom = custom;
        let Some(doc) = self.doc.as_mut() else { return };
        if let Some((rev, pages)) = saved {
            let _ = doc.rebase(pages);
            self.saved_rev = rev;
        }
        // The draft's language previews inside the window; the main window keeps the committed one.
        let wanted = draft_language(doc).map_or_else(|| host.lang(), |code| language_of(host, &code));
        if wanted != self.labels {
            doc.relabel(&host.with_lang(wanted));
            self.labels = wanted;
            self.words = Arc::new(Words::from_i18n(Arc::clone(crate::i18n::framework_i18n()), wanted));
        }
    }
}

impl StackDialog for PrefsDialog {
    type Result = PrefsAnswer;

    fn id(&self) -> DialogId {
        dialog_id("photocraft.preferences")
    }

    fn key(&self) -> Option<&str> {
        Some("preferences")
    }

    /// A pass applies only its own output: `pick_color`, `closed` and the applied edits are each filed once, so a second
    /// pass in the same frame, which gets no input, files nothing again.
    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<PrefsAnswer>> {
        let doc = self.doc.take()?;
        let mut with_custom = doc.custom(self.custom.clone());
        // Relabeling the document covers its rows, but the framework draws its footer/title from Env.
        // Scope those words to the draft as well, without changing the main window's language.
        let words = Arc::clone(&self.words);
        let out = with_env(
            ui,
            |set| set.words = Some(WordsOverride::Replace(words)),
            |ui| PreferencesWindow::new(Bind::bound(&mut self.state)).remember(SaveKey::new("photocraft.preferences")).show(ui, &mut with_custom),
        );
        self.doc = Some(with_custom.custom(()));
        if let Some(request) = out.pick_color {
            self.pick = Some(request);
        }
        self.file_edits();
        out.closed.map(|closed| match closed {
            Closed::Ok => DialogResult::Ok(PrefsAnswer(std::mem::take(&mut self.edits))),
            _ => DialogResult::Cancel,
        })
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<PrefsAnswer> {
        self.apply().ok()?;
        Some(PrefsAnswer(std::mem::take(&mut self.edits)))
    }

    fn json_fields(&self) -> Option<Value> {
        let values = self.doc.as_ref()?.draft().to::<Value>()?;
        Some(json!({"values": values, "page": self.state.selected}))
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "values" => self.doc.as_mut().ok_or("Preferences is not open")?.draft_mut().replace(&value).map_err(|e| format!("`values` is not valid: {e}")),
            "page" => {
                self.state.selected = value.as_str().ok_or("`page` is not valid: not text")?.to_string();
                Ok(())
            }
            _ => Err(format!("the Preferences window has no field `{field}`")),
        }
    }
}

/// Whether the Preferences window is open.
pub fn window_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.is_open::<PrefsDialog>()
}

/// Opens Edit › Preferences on `section` (`general` for an unknown one). With the window open only its page changes.
pub fn open_preferences(app: &mut PhotocraftApp, section: &str) {
    let section = if SECTIONS.iter().any(|(id, _)| *id == section) { section } else { "general" };
    if let Some(open) = app.framework.dialogs.get_mut::<PrefsDialog>() {
        open.state.selected = section.to_string();
        return;
    }
    let doc = match SettingsDocument::new(PrefPages::of(app.session.prefs())) {
        Ok(doc) => doc,
        Err(error) => {
            app.ui.status = error.to_string();
            return;
        }
    };
    let pages: Vec<PageInfo> = StandardPage::ALL.into_iter().map(StandardPage::info).collect();
    let doc = doc.pages(pages).rules(rules());
    // Agents reach the desktop app only; the web build shows the settings it had before as rows.
    #[cfg(not(target_arch = "wasm32"))]
    let doc = doc.custom_page(super::custom_rows::AGENTS_PAGE);
    let mut doc = doc;
    let host = Arc::clone(app.framework.host());
    doc.relabel(&host);
    let mut state = PreferencesState::default();
    state.selected = section.to_string();
    let custom = PrefsCustom::new(app, false);
    let saved_rev = app.session.prefs.rev();
    let words = Arc::new(Words::from_i18n(Arc::clone(crate::i18n::framework_i18n()), host.lang()));
    app.framework.dialogs.open(PrefsDialog { doc: Some(doc), state, saved_rev, labels: host.lang(), words, custom, pick: None, edits: Vec::new() });
}

/// The language the draft's `interface.language` names, as the code stored in the draft.
fn draft_language(doc: &Doc) -> Option<String> {
    doc.pages().iter().filter(|page| page.id == "interface").flat_map(|page| page.groups.iter()).flat_map(|group| group.rows.iter()).find_map(|row| match &row
        .kind
    {
        RowKind::Choice { value, .. } if row.key == "interface.language" => Some((*value).to_string()),
        _ => None,
    })
}

/// The language `code` stands for, `host`'s own where it names none.
fn language_of(host: &Host, code: &str) -> Lang {
    let Some(i18n) = host.i18n_shared() else { return host.lang() };
    if code == "auto" { i18n.system_lang() } else { i18n.from_code(code).unwrap_or_else(|| host.lang()) }
}

/// Writes the keys Apply or OK changed through the engine, and returns the `prefs.set` reply. Only these keys go,
/// so a setting changed elsewhere while the window is open is not overwritten. Keys Reset Section removed go through
/// `prefs.reset`.
fn write_edits(app: &mut PhotocraftApp, (values, resets): Edits) -> Result<Value, String> {
    let mut reply = json!({});
    if values.as_object().is_some_and(|map| !map.is_empty()) {
        reply = app.run("prefs.set", json!({"values": values}))?;
    }
    for path in resets {
        app.run("prefs.reset", json!({"path": path}))?;
    }
    Ok(reply)
}

/// Writes every batch of edits; the reply of the last `prefs.set`. The first refusal stops the rest.
fn write_all(app: &mut PhotocraftApp, batches: Vec<Edits>) -> Result<Value, String> {
    let mut reply = json!({});
    for edits in batches {
        reply = write_edits(app, edits)?;
    }
    Ok(reply)
}

/// Hands the open window what it reads of the app, before the host runs it.
pub(crate) fn feed(app: &mut PhotocraftApp) {
    let Some(shown_rev) = app.framework.dialogs.get::<PrefsDialog>().map(|d| d.saved_rev) else { return };
    let custom = PrefsCustom::new(&*app, crate::color_picker_ui::window_picker_open(app, PickWindow::Preferences));
    let rev = app.session.prefs.rev();
    let saved = (rev != shown_rev).then(|| (rev, PrefPages::of(app.session.prefs())));
    let host = Arc::clone(app.framework.host());
    if let Some(dialog) = app.framework.dialogs.get_mut::<PrefsDialog>() {
        dialog.feed(custom, saved, &host);
    }
}

/// Carries the window's requests out after a host pass: the answer it ended with, the edits Apply filed, the colour
/// picker its swatch asked for and the picker's answer. A window that is gone takes its picker with it.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut batches = match app.framework.dialogs.take::<PrefsDialog>() {
        Some(DialogResult::Ok(PrefsAnswer(edits))) => edits,
        _ => Vec::new(),
    };
    let mut picked = None;
    if let Some(dialog) = app.framework.dialogs.get_mut::<PrefsDialog>() {
        batches.append(&mut dialog.edits);
        picked = dialog.pick.take();
    } else {
        crate::color_picker_ui::cancel_window_picker(app, ctx, PickWindow::Preferences);
    }
    if let Some(request) = picked {
        crate::color_picker_ui::open_for_window(app, PickWindow::Preferences, request);
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for_window(app, PickWindow::Preferences)
        && let Some(doc) = app.framework.dialogs.get_mut::<PrefsDialog>().and_then(|d| d.doc.as_mut())
    {
        doc.set_color(&key, color);
    }
    if let Err(error) = write_all(app, batches)
        && let Some(dialog) = app.framework.dialogs.get_mut::<PrefsDialog>()
    {
        dialog.state.error = Some(error);
    }
}

/// Applies the open window's draft as its Apply button does and keeps the window open. `Err` when it is closed, a
/// colour picker it opened is still open, or the engine refuses a value.
pub fn apply_window(app: &mut PhotocraftApp) -> Result<Value, String> {
    // The window judges Apply on what it reads of the app (an open picker among it), as its button does after a feed.
    feed(app);
    let Some(dialog) = app.framework.dialogs.get_mut::<PrefsDialog>() else { return Err("Preferences is not open".into()) };
    let applied = dialog.apply().map(|()| std::mem::take(&mut dialog.edits));
    let result = applied.and_then(|batches| write_all(app, batches));
    if let Err(error) = &result
        && let Some(dialog) = app.framework.dialogs.get_mut::<PrefsDialog>()
    {
        dialog.state.error = Some(error.clone());
    }
    result
}

/// Sets a row of the open window's draft as the user editing it would (type and range checks included).
/// `false` when the window is closed.
#[cfg(test)]
pub(crate) fn set_draft(app: &mut PhotocraftApp, key: &str, value: PrefValue) -> bool {
    match app.framework.dialogs.get_mut::<PrefsDialog>().and_then(|d| d.doc.as_mut()) {
        Some(doc) => {
            doc.set(key, value);
            true
        }
        None => false,
    }
}
