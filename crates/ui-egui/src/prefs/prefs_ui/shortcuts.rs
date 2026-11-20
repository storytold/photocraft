//! Edit › Keyboard Shortcuts and Menus: the framework's Keyboard Shortcuts dialog over a draft
//! keymap, with PhotoCraft's Menus tab beside it.
//!
//! The session holds everything the dialog edits: the draft keymap, the shortcut sets, the Menus
//! draft and the text of a set waiting to be reviewed. Cancel drops it and the runtime never saw
//! a change; OK writes the preferences once ([`save_keys`], then the menus), and the runtime's
//! keymap follows the new shortcut preferences (`shortcut_dispatch::overrides`).
//!
//! The Toolbar tab is not here: Edit › Toolbar opens the toolbar drawer.

use std::collections::{BTreeMap, BTreeSet};

use craft::behavior::IconId;
use craft::keymap::import::kys;
use craft::keymap::{Builtins, CommandRef, ImportOptions, Keymap, NoneDefined, Overrides, Profiles, Scopes, reserved};
use craft::menu::Row;
use craft::menu::shortcuts::{Area, ConflictMode, DialogState, DialogTab, Format, Options, Outcome, ShortcutWords, ShortcutsDialog, kys_error_text};
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::windows;
use photocraft_engine::keymap_defs;
use photocraft_engine::menu_sets::{self, MenuSet};
use photocraft_engine::prefs::{MenuCustomization, Preferences};
use photocraft_engine::shortcut_sets::{DEFAULTS_NAME, set_name};
use serde_json::{Map, Value, json};
use std::path::Path;

use crate::PhotocraftApp;

/// The id of the `.kys` format in the dialog's import and export lists.
const KYS_FORMAT: &str = "kys";

/// The colours Edit › Menus gives an item; `none` is no colour.
const COLORS: [&str; 8] = ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"];

/// The open Keyboard Shortcuts and Menus dialog, at `app.prefs_rt.shortcuts`.
pub(crate) struct ShortcutsSession {
    /// The dialog's state between passes.
    pub(crate) state: DialogState,
    /// The shortcut sets as the dialog edits them.
    pub(crate) profiles: Profiles,
    /// The draft the dialog edits in place: the rows the runtime catalog binds, with the
    /// preferences' bindings. The runtime's own keymap is not this one.
    pub(crate) keymap: Keymap,
    /// The Menus tab's draft.
    pub(crate) menus: MenusDraft,
    /// The text of a set to review on the next pass, taken by that pass alone: a second pass of
    /// the same frame finds nothing to review.
    pub(crate) pending_import: Option<String>,
    /// The user's answers to earlier import questions, for the next import of this session.
    pub(crate) remembered: BTreeMap<String, CommandRef>,
}

/// What the Menus tab edits: the hidden items and the colour of each item, by command id, plus the
/// named sets the Set row keeps (#14450) and the one in use ("" is the built-in set).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MenusDraft {
    pub(crate) hidden: BTreeSet<String>,
    pub(crate) colors: BTreeMap<String, String>,
    /// The saved sets as the dialog edits them, keyed by name; the same shape the preferences keep.
    pub(crate) sets: BTreeMap<String, MenuSet>,
    /// The set the row names as the one in use; "" is the built-in set.
    pub(crate) set: String,
    /// Set by Save on the built-in set and by New: the pass asks the file service for a name once.
    pub(crate) ask_name: bool,
    /// Set by Save on the built-in set or by New when `sets` is already at the engine's cap: the
    /// pass shows the engine's cap line instead of asking the file service for a name.
    pub(crate) cap_refused: bool,
    /// Set by anything that changed `sets` or `set`: OK writes them through `edit.menus`.
    pub(crate) sets_changed: bool,
}

impl MenusDraft {
    fn of(menus: &MenuCustomization) -> Self {
        MenusDraft { hidden: menus.hidden.iter().cloned().collect(), colors: menus.colors.clone(), ..Default::default() }
    }

    /// The draft the Menus tab starts from: the menus in use, the saved sets, and the set in use.
    fn of_prefs(prefs: &Preferences) -> Self {
        MenusDraft { sets: prefs.menu_sets.clone(), set: prefs.menu_set.clone(), ..MenusDraft::of(&prefs.menus) }
    }

    /// The draft's menus as a set: what Save stores.
    fn menus(&self) -> MenuSet {
        MenuSet { hidden: self.hidden.iter().cloned().collect(), colors: self.colors.clone() }
    }

    /// Do the draft's menus differ from the saved copy of the set in use? The row's "(modified)"
    /// and Save's enablement read this; the engine compares the same way (`menu_sets::differs`).
    fn modified(&self) -> bool {
        menu_sets::differs(&self.sets, &self.set, &self.menus())
    }

    /// The set in use as the row shows it: the built-in set's name for "", with "(modified)" while
    /// the menus differ from it. The first edit renames it in place, as measured (#14450).
    fn shown_name(&self) -> String {
        let base = if self.set.is_empty() { DEFAULTS_NAME.to_string() } else { self.set.clone() };
        if self.modified() { format!("{base} (modified)") } else { base }
    }

    /// The Save dialog's default name: the shown name plus " Copy" ("<defaults> (modified) Copy",
    /// "<defaults> Copy"), as measured (#14450).
    fn copy_name(&self) -> String {
        format!("{} Copy", self.shown_name())
    }

    /// Whether Save can store the menus as they stand: the set is modified, and its name is one the
    /// engine would take. A name from a hand-edited settings file may not be, and Save could not
    /// write it; the built-in set needs a name from the Save dialog, so it has nothing to check.
    fn can_save(&self) -> bool {
        self.modified() && (self.set.is_empty() || set_name(&self.set).is_ok())
    }

    /// Ask the Save dialog for a new set's name, unless `sets` already holds as many sets as the
    /// engine stores (`menu_sets::MAX_SETS`): a 65th name could only produce sets the engine
    /// refuses to write, so the pass shows the engine's cap line and the dialog stays open.
    fn ask_new_name(&mut self) {
        if self.sets.len() >= menu_sets::MAX_SETS {
            self.cap_refused = true;
        } else {
            self.ask_name = true;
        }
    }

    /// Choose the set `name` ("" or the built-in name: the built-in set) and load its menus into
    /// the rows, dropping unsaved edits as the reference's "No" does; the Yes / No / Cancel box is
    /// the next slice.
    fn choose(&mut self, name: &str) {
        // The built-in entry's value is ""; a hand-edited settings file can also hold a set named
        // like the built-in one, and the built-in entry still gives the built-in menus.
        let built_in = name.is_empty() || name.eq_ignore_ascii_case(DEFAULTS_NAME);
        // `set_entries` builds every saved entry out of `sets`, so a picked name that is not the
        // built-in one is one of them; anything else reads as the built-in set rather than guessing.
        let loaded = if built_in { None } else { self.sets.get(name).cloned() };
        let set = loaded.unwrap_or_default();
        self.hidden = set.hidden.iter().cloned().collect();
        self.colors = set.colors.clone();
        self.set = if built_in { String::new() } else { name.to_string() };
        self.sets_changed = true;
    }

    /// Store the menus as they now stand in the set `name` and use it: Save on a saved set, and
    /// what a name from the Save dialog becomes.
    fn store(&mut self, name: &str) {
        self.sets.insert(name.to_owned(), self.menus());
        self.set = name.to_owned();
        self.sets_changed = true;
    }

    /// Delete the saved set in use and move the row back to the built-in set; the menus on screen
    /// stay, as the engine's `deleteSet` leaves them (#14450).
    fn delete(&mut self) {
        self.sets.remove(&self.set);
        self.set.clear();
        self.sets_changed = true;
    }
}

/// The built-in sets: PhotoCraft's defaults.
fn builtins() -> [(&'static str, Overrides); 1] {
    [(DEFAULTS_NAME, Overrides::new())]
}

/// The saved sets of `prefs` as the dialog's `Profiles`. The defaults are the active set when the
/// preferences name none.
pub(crate) fn profiles_of(prefs: &Preferences) -> Profiles {
    let sets: Vec<Value> = prefs.shortcut_sets.iter().map(|(name, set)| json!({"name": name, "overrides": keymap_defs::set_overrides(set)})).collect();
    serde_json::from_value(json!({"active": prefs.shortcut_set, "sets": sets})).unwrap_or_default()
}

/// The keymap the dialog and the offer edit: the rows the runtime catalog binds, the platform the
/// runtime's keymap is built for, and the preferences' bindings as overrides.
pub(crate) fn draft_keymap(app: &PhotocraftApp) -> Keymap {
    let platform = windows::os_of(app.ctx());
    // The runtime catalog's held rows while they are current; built here before the first frame holds them.
    let rows = match app.framework.catalog_rows_at(app.session.actions.list.revision()) {
        Some(rows) => rows.to_vec(),
        None => crate::shortcut_dispatch::command_rows(app),
    };
    Keymap::new(rows.into_iter().chain(crate::shortcut_dispatch::bridged_rows()).collect(), Scopes::new(), platform)
        .with_reserved(reserved::system(platform))
        .with_overrides(keymap_defs::prefs_overrides(app.session.prefs()))
}

/// How a `.kys` set is read for review and for the offer: rows without a key leave their command
/// alone, and the user's earlier answers come first.
pub(crate) fn import_options(remembered: &BTreeMap<String, CommandRef>) -> ImportOptions {
    ImportOptions { remembered: remembered.clone(), none_defined: NoneDefined::Keep, suggest: true, ..ImportOptions::default() }
}

/// The text of a set file's bytes, or why it is not one.
pub(crate) fn kys_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| format!("{name}: not a keyboard shortcut set (.kys): not UTF-8"))
}

/// Opens the dialog (when it is not open) on tab `tab`: 0 Keyboard Shortcuts, 1 Menus.
pub fn open(app: &mut PhotocraftApp, tab: u64) {
    if app.prefs_rt.shortcuts.is_some() {
        return;
    }
    let prefs = app.session.prefs();
    let session = ShortcutsSession {
        state: if tab == 1 { DialogState::new().with_tab("menus") } else { DialogState::new() },
        profiles: profiles_of(prefs),
        keymap: draft_keymap(app),
        menus: MenusDraft::of_prefs(prefs),
        pending_import: None,
        remembered: BTreeMap::new(),
    };
    app.prefs_rt.shortcuts = Some(session);
}

/// Whether the dialog is open: the runtime's keys step aside for its modal.
pub fn shortcuts_open(app: &PhotocraftApp) -> bool {
    app.prefs_rt.shortcuts.is_some()
}

/// The id the control channel knows the dialog by: Edit › Keyboard Shortcuts and Edit › Menus reply with it,
/// `ui.inspect` lists the dialog under it, and `ui.dialog.cancel` closes it. The dialog draws outside the framework's
/// dialog host, which has no id for it (#15544).
pub(crate) const CONTROL_ID: &str = "shortcuts";

/// The reply of a command that opened the dialog.
pub(crate) fn control_reply() -> Value {
    json!({"dialog": CONTROL_ID})
}

/// The dialog as `ui.inspect` lists it among the dialogs while it is open, in the host's entry shape.
pub(crate) fn inspect(app: &PhotocraftApp) -> Option<Value> {
    shortcuts_open(app).then(|| json!({"id": CONTROL_ID, "key": "edit.keyboardShortcuts", "parent": null, "fields": {}}))
}

/// Cancel for a control client, as the dialog's Cancel: the session goes and nothing it edited is saved. False with
/// the dialog closed.
pub(crate) fn cancel(app: &mut PhotocraftApp) -> bool {
    app.prefs_rt.shortcuts.take().is_some()
}

/// Whether the dialog is recording a chord: the key goes to the recorder and runs nothing, and the
/// native menu bar pauses its key equivalents.
pub fn recording(app: &PhotocraftApp) -> bool {
    app.prefs_rt.shortcuts.as_ref().is_some_and(|session| session.state.is_recording())
}

/// Queues the text of a set for review on the dialog's next pass.
pub(crate) fn queue_import(app: &mut PhotocraftApp, text: String) {
    open(app, 0);
    if let Some(session) = app.prefs_rt.shortcuts.as_mut() {
        session.pending_import = Some(text);
    }
}

/// The dialog's options: Photoshop's Shortcuts For areas, the conflict bar, `.kys` files in and
/// out, and the summary.
fn options() -> Options {
    let kys = Format::new(KYS_FORMAT, "Photoshop Keyboard Shortcuts (.kys)", &["kys", "psp"]);
    Options::default().with(|o| {
        o.title = Some(tl!("Keyboard Shortcuts and Menus").to_string());
        o.areas = Area::photoshop();
        o.conflicts = ConflictMode::Ask;
        o.import_formats = vec![kys];
        o.export_formats = vec![kys];
        o.summary = true;
    })
}

/// The dialog over the session's parts for one pass.
fn dialog<'a>(keymap: &'a mut Keymap, profiles: &'a mut Profiles, builtins: &'a Builtins<'a>, options: &'a Options) -> ShortcutsDialog<'a> {
    ShortcutsDialog::new(keymap).sets(profiles, builtins).options(options)
}

/// One pass of the dialog while it is open.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    pass(app, ui, false);
}

/// One pass; with `press_ok` the pass presses OK first. The session is out of `app.prefs_rt`
/// during the pass, so nothing in the pass reads it through [`recording`].
fn pass(app: &mut PhotocraftApp, ui: &mut egui::Ui, press_ok: bool) {
    let Some(mut session) = app.prefs_rt.shortcuts.take() else { return };
    let rows = crate::menus::base_rows(app);
    let options = options();
    let builtins = builtins();
    let ShortcutsSession { state, profiles, keymap, menus, pending_import, remembered } = &mut session;
    if let Some(text) = pending_import.take() {
        review(ui, state, dialog(keymap, profiles, &builtins, &options), &text, remembered);
    }
    if press_ok {
        dialog(keymap, profiles, &builtins, &options).ok(ui, state);
    }
    let mut tabs = [menus_tab(&rows, menus)];
    let output = dialog(keymap, profiles, &builtins, &options).tabs(&mut tabs).show(ui, state);
    drop(tabs);
    match output.outcome {
        Some(Outcome::Ok) => {
            // The failure line names the write that failed: shortcuts and menus are separate subjects.
            let failure = save_keys(app, keymap, profiles)
                .err()
                .map(|e| (tl!("The keyboard shortcuts were not saved: {error}"), e))
                .or_else(|| save_menus(app, menus).err().map(|e| (tl!("The menus were not saved: {error}"), e)));
            if let Some((subject, e)) = failure {
                app.ui.status = crate::i18n::fmt(subject, &[("error", &e)]);
                app.ui.status_error = true;
            }
        }
        Some(Outcome::Import { .. }) => ask_import(app, state),
        Some(Outcome::Export { name, json, .. }) => save_text(app, state, &format!("{name}.json"), json),
        Some(Outcome::ExportAs { format, name, keymap: exported, .. }) if format == KYS_FORMAT => {
            save_text(app, state, &format!("{name}.kys"), kys::write(&name, &exported));
        }
        Some(Outcome::Summary { name, html, .. }) => save_text(app, state, &format!("{name}.html"), html),
        Some(Outcome::Remembered(answers)) => remembered.extend(answers),
        _ => {}
    }
    if menus.ask_name {
        menus.ask_name = false;
        ask_set_name(app, state, &menus.copy_name());
    }
    if menus.cap_refused {
        menus.cap_refused = false;
        // The framework's in-dialog bars are drawn only on its own tab, so the Menus tab's refusal
        // lines go to the app's status line, named by the engine's own cap message.
        let cap = format!("at most {} menu sets", menu_sets::MAX_SETS);
        app.ui.status = crate::i18n::fmt(tl!("The menu set was not saved: {error}"), &[("error", &cap)]);
        app.ui.status_error = true;
    }
    if state.is_open() {
        app.prefs_rt.shortcuts = Some(session);
    }
}

/// Opens the review of the set `text` holds (a `.kys` file, or craft-keymap JSON), or shows why it
/// cannot be read.
fn review(ui: &egui::Ui, state: &mut DialogState, dialog: ShortcutsDialog<'_>, text: &str, remembered: &BTreeMap<String, CommandRef>) {
    if text.trim_start().starts_with(['{', '[']) {
        if let Err(e) = dialog.import_json(ui, state, text) {
            state.show_error(e.to_string());
        }
        return;
    }
    match kys::read(text) {
        Ok(set) => dialog.review_import(ui, state, set, import_options(remembered)),
        Err(e) => state.show_error(kys_error_text(&e, &Env::of(ui).words::<ShortcutWords>())),
    }
}

/// Import…: ask for the file; the text is reviewed on the pass after it arrives.
fn ask_import(app: &mut PhotocraftApp, state: &mut DialogState) {
    let picked = app.pick_file_bytes_filtered(&["kys", "psp", "json"], |app, name, bytes| {
        let text = kys_text(&name, &bytes)?;
        queue_import(app, text);
        Ok(control_reply())
    });
    if let Err(e) = picked {
        state.show_error(e);
    }
}

/// Saves `text` through the save dialog under the suggested file name.
fn save_text(app: &mut PhotocraftApp, state: &mut DialogState, suggested: &str, text: String) {
    // A set name can hold characters a file name can't.
    let name: String = suggested.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let saved = app.pick_save(&name, move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, text.as_bytes())?;
        Ok(json!({"path": path}))
    });
    if let Err(e) = saved {
        state.show_error(e);
    }
}

/// Writes the bindings and sets of the dialog (or of an import) to the preferences in one
/// `edit.keyboardShortcuts` call: the one writer of the shortcut preferences. An action's function
/// key belongs to the action, not to a set, so the preferences' own stay whatever set was chosen.
pub(crate) fn save_keys(app: &mut PhotocraftApp, keymap: &Keymap, profiles: &Profiles) -> Result<Value, String> {
    let (mut set, mut alternates) = keymap_defs::overrides_to_prefs(keymap.overrides());
    let prefs = app.session.prefs();
    let is_action = |id: &String| id.starts_with(photocraft_engine::actions_cmds::PLAY_SHORTCUT_PREFIX);
    set.retain(|id, _| !is_action(id));
    alternates.retain(|id, _| !is_action(id));
    set.extend(prefs.shortcuts.iter().filter(|(id, _)| is_action(id)).map(|(id, key)| (id.clone(), key.clone())));
    alternates.extend(prefs.alternate_shortcuts.iter().filter(|(id, _)| is_action(id)).map(|(id, keys)| (id.clone(), keys.clone())));
    let builtins = builtins();
    let active = profiles.active_name(&builtins);
    // OK saves the dialog's bindings into the set in use.
    let sets: Map<String, Value> = profiles
        .sets()
        .iter()
        .map(|(name, overrides)| {
            let overrides = if name.as_str() == active { keymap.overrides() } else { overrides };
            let (mut shortcuts, mut alternate_shortcuts) = keymap_defs::overrides_to_prefs(overrides);
            shortcuts.retain(|id, _| !is_action(id));
            alternate_shortcuts.retain(|id, _| !is_action(id));
            (name.clone(), json!({"shortcuts": shortcuts, "alternateShortcuts": alternate_shortcuts}))
        })
        .collect();
    let active = if active == DEFAULTS_NAME { "" } else { active };
    let call = json!({"reset": true, "set": set, "alternates": alternates, "sets": sets, "activeSet": active, "allowUnknown": true, "removeConflicts": false});
    app.run("edit.keyboardShortcuts", call).map_err(|e| e.to_string())
}

/// Writes the Menus tab's draft to the preferences: the menus in use as the tab has always written
/// them, and, once the Set row touched them, the saved sets and the one in use through `edit.menus`
/// — the params slice 1 added, which check the set names, colours and hideable ids.
fn save_menus(app: &mut PhotocraftApp, menus: &MenusDraft) -> Result<Value, String> {
    if menus.sets_changed {
        // OK sends only what the engine would store: a set whose name it refuses is left out, and
        // inside a set only items it lets be hidden and colours it knows. Since `sets` replaces
        // every saved set, an entry a stale settings file holds is repaired out of the file;
        // sending it whole, as this did, let one un-storable entry refuse the call, skip both
        // menus writes, and close the dialog on the lost session.
        let sets: Map<String, Value> = menus
            .sets
            .iter()
            .filter_map(|(name, set)| {
                let name = set_name(name).ok()?;
                let hidden: Vec<&String> = set.hidden.iter().filter(|id| MenuCustomization::hideable(id)).collect();
                let colors: Map<String, Value> =
                    set.colors.iter().filter(|(_, c)| COLORS.contains(&c.as_str())).map(|(id, c)| (id.clone(), json!(c))).collect();
                Some((name, json!({"hidden": hidden, "colors": colors})))
            })
            .collect();
        // The set in use, likewise: "" unless the engine would keep both the name and that set.
        let active = set_name(&menus.set).ok().filter(|name| sets.contains_key(name)).unwrap_or_default();
        app.run("edit.menus", json!({"sets": sets, "activeSet": active})).map_err(|e| e.to_string())?;
    }
    let call = json!({"values": {"menus": {"hidden": menus.hidden, "colors": menus.colors}}});
    app.run("prefs.set", call).map_err(|e| e.to_string())
}

/// Save on the built-in set and New ask for the set's name through the system Save dialog, which
/// suggests `suggested`; the chosen file's name without its extension names the set, as measured
/// (#14450). Writing the `.mnu` file, and starting the dialog in the menu-sets folder of the
/// settings directory with the "Menu Customization (*.MNU)" type, belong to that slice: the set is
/// stored with the preferences here, and no file is written.
fn ask_set_name(app: &mut PhotocraftApp, state: &mut DialogState, suggested: &str) {
    let asked = app.pick_save(suggested, |app, path| {
        let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // The engine's own name rules: a name it would refuse (empty, overlong, the built-in one)
        // is an error line, not a set the row could never select; the dialog stays open either way.
        // Its bars belong to the framework's own tab, so the line goes to the app's status line.
        let refused = match app.prefs_rt.shortcuts.as_mut() {
            Some(session) => match set_name(&stem) {
                Ok(name) => {
                    session.menus.store(&name);
                    None
                }
                Err(e) => Some(e),
            },
            None => None,
        };
        if let Some(e) = refused {
            app.ui.status = crate::i18n::fmt(tl!("The menu set was not saved: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
        Ok(control_reply())
    });
    if let Err(e) = asked {
        state.show_error(e);
    }
}

/// The Menus tab's Set row: the set dropdown with the built-in set first and "(modified)" on the
/// one in use, and Save, New and Delete as icon buttons with the measured tooltips and enablement
/// (#14450). The framework's own Set row stays on its tab, so this row is PhotoCraft's.
fn set_row(ui: &mut egui::Ui, draft: &mut MenusDraft) {
    let entries = set_entries(draft);
    let mut picked = draft.set.clone();
    ui.horizontal(|ui| {
        ui.label(tl!("Set:"));
        Select::new(tl!("Set").to_string(), Bind::bound(&mut picked), &entries).show(ui);
        let save = IconButton::new(IconId::new("save"))
            .label(tl!("Save all changes to the current set of menus"))
            .tooltip(tl!("Save all changes to the current set of menus"))
            .enabled(draft.can_save())
            .show(ui);
        let new = IconButton::new(IconId::new("plus"))
            .label(tl!("Create a new set based on the current set of menus"))
            .tooltip(tl!("Create a new set based on the current set of menus"))
            .show(ui);
        let delete = IconButton::new(IconId::new("trash"))
            .label(tl!("Delete the current set of menus"))
            .tooltip(tl!("Delete the current set of menus"))
            .enabled(!draft.set.is_empty())
            .show(ui);
        // The pick and the buttons are read after the row was drawn, as the framework's row does.
        if picked != draft.set {
            draft.choose(&picked);
        }
        if save.clicked() {
            if draft.set.is_empty() {
                // The built-in set has no saved copy: the name comes from the Save dialog.
                draft.ask_new_name();
            } else {
                let name = draft.set.clone();
                draft.store(&name);
            }
        }
        if new.clicked() {
            draft.ask_new_name();
        }
        if delete.clicked() {
            draft.delete();
        }
    });
}

/// The dropdown's entries: the built-in set first (its value is "", the engine's name for it), then
/// the saved sets by name. Only the set in use carries "(modified)", as measured (#14450). A
/// hand-edited settings file can hold a set named like the built-in one; it keeps its own row here,
/// and `MenusDraft::can_save` keeps Save off a name the engine would refuse.
fn set_entries(draft: &MenusDraft) -> Vec<Entry<String>> {
    let shown = draft.shown_name();
    let mut entries = vec![Entry::new(String::new(), if draft.set.is_empty() { shown.clone() } else { DEFAULTS_NAME.to_string() })];
    entries.extend(draft.sets.keys().map(|name| Entry::new(name.clone(), if *name == draft.set { shown.clone() } else { name.clone() })));
    entries
}

/// The Menus tab: a row per menu item with a Visible check and a colour.
fn menus_tab<'a>(rows: &'a [Row], draft: &'a mut MenusDraft) -> DialogTab<'a> {
    DialogTab::new("menus", tl!("Menus").to_string(), move |ui| menus_body(ui, rows, &mut *draft))
}

/// The Menus tab's body; true when the draft changed. `rows` are the menu items before the
/// customization, so a hidden item is listed here to be shown again.
fn menus_body(ui: &mut egui::Ui, rows: &[Row], draft: &mut MenusDraft) -> bool {
    let before = draft.clone();
    set_row(ui, draft);
    if ui.button(tl!("Show All Menu Items")).clicked() {
        draft.hidden.clear();
    }
    let items: Vec<&Row> = rows.iter().filter(|row| row.label != "---").collect();
    let colors: Vec<Entry<&'static str>> = COLORS.iter().map(|c| Entry::new(*c, *c)).collect();
    let height = ui.spacing().interact_size.y;
    // PLAN: the framework `Table` draws a value in each cell and cannot hold a Checkbox or a Select, so the rows are laid out by hand.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, height, items.len(), |ui, range| {
        for row in &items[range] {
            let id = row.command.id.as_str();
            ui.push_id(id, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{} › {}", row.path.join(" › "), row.label));
                    // Items that can't be hidden (File › Close, Save…, Edit › Menus…) have no visibility
                    // control at all, as in the reference application (#5096, #14335).
                    if MenuCustomization::hideable(id) {
                        let visible = !draft.hidden.contains(id);
                        let shown = Checkbox::new(Bind::given(visible)).label(tl!("Visible").to_string()).show(ui);
                        match shown.events.value() {
                            Some(&true) => {
                                draft.hidden.remove(id);
                            }
                            Some(&false) => {
                                draft.hidden.insert(id.to_owned());
                            }
                            None => {}
                        }
                    }
                    let current = draft.colors.get(id).map_or("none", String::as_str).to_owned();
                    let current = COLORS.iter().copied().find(|c| *c == current).unwrap_or("none");
                    let picked = Select::new(tl!("Color").to_string(), Bind::given(current), &colors).show(ui);
                    if let Some(&color) = picked.events.value() {
                        if color == "none" {
                            draft.colors.remove(id);
                        } else {
                            draft.colors.insert(id.to_owned(), color.to_owned());
                        }
                    }
                });
            });
        }
    });
    *draft != before
}

/// Test hook: adds `binding` to `command` in the open dialog's draft keymap, as the dialog's own edit does.
#[cfg(test)]
pub(crate) fn assign(
    app: &mut PhotocraftApp,
    command: &CommandRef,
    binding: craft::keymap::Binding,
    policy: craft::keymap::Policy,
) -> Result<craft::keymap::Assigned, craft::keymap::AssignError> {
    let session = app.prefs_rt.shortcuts.as_mut().ok_or(craft::keymap::AssignError::UnknownCommand)?;
    session.keymap.assign(command, binding, policy)
}

/// Test hook: makes `binding` the only binding of `command` in the open dialog's draft keymap, as recording over its
/// key in the dialog does.
#[cfg(test)]
pub(crate) fn rebind(
    app: &mut PhotocraftApp,
    command: &CommandRef,
    binding: craft::keymap::Binding,
    policy: craft::keymap::Policy,
) -> Result<craft::keymap::Assigned, craft::keymap::AssignError> {
    let session = app.prefs_rt.shortcuts.as_mut().ok_or(craft::keymap::AssignError::UnknownCommand)?;
    session.keymap.set_bindings(command, vec![binding], policy)
}

/// Test hook: presses OK in the open dialog and handles the outcome, in one pass on `ctx`.
#[cfg(test)]
pub(crate) fn press_ok(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::testing::run_ui(ctx, Default::default(), |ui| pass(app, ui, true));
}
