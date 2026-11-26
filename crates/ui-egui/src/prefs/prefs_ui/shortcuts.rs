//! Edit › Keyboard Shortcuts and Menus: the framework's Keyboard Shortcuts dialog over a draft
//! keymap, with PhotoCraft's Menus tab beside it.
//!
//! The session holds everything the dialog edits: the draft keymap, the shortcut sets, the Menus
//! draft and the text of a set waiting to be reviewed. Cancel drops it and the runtime never saw
//! a change; OK writes the preferences once ([`save_keys`], then the menus), and the runtime's
//! keymap follows the new shortcut preferences (`shortcut_dispatch::overrides`).
//!
//! The Toolbar tab is not here: Edit › Toolbar opens the toolbar drawer.

use photocraft_engine::actions_cmds::ActionsExt;
use std::collections::{BTreeMap, BTreeSet};

use craft::behavior::IconId;
use craft::keymap::import::kys;
use craft::keymap::{Builtins, CommandRef, ImportOptions, Keymap, NoneDefined, Overrides, Profiles, Scopes, reserved};
use craft::menu::Row;
use craft::menu::shortcuts::{Area, ConflictMode, DialogState, DialogTab, Format, Options, Outcome, ShortcutWords, ShortcutsDialog, kys_error_text};
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use craft::ui::display::Text;
use craft::ui::layout_components::Row as LayoutRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, with_env};
use craft::ui::runtime::windows;
use photocraft_engine::keymap_defs;
use photocraft_engine::menu_sets::{self, MenuSet};
use photocraft_engine::prefs::{MenuCustomization, Preferences};
use photocraft_engine::shortcut_sets::{DEFAULTS_NAME, set_name};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

use crate::PhotocraftApp;
use crate::dialogs::alerts;

/// The id of the `.kys` format in the dialog's import and export lists.
const KYS_FORMAT: &str = "kys";

/// The colours Edit › Menus gives an item; `none` is no colour.
const COLORS: [&str; 8] = ["none", "red", "orange", "yellow", "green", "blue", "violet", "gray"];

/// The Set row's two questions (#14450). The switch question's text says "shortcuts" on the Menus
/// tab too, as measured, double space and all; the Delete question names the set in its place.
const SWITCH_ASK: &str = "Selecting a different set will result in a loss of all unsaved shortcuts.  Save your changes before switching sets?";
const DELETE_ASK: &str = "Are you sure you want to send {name} to the recycle bin?";

/// Both boxes carry the dialog's own name as their title, as measured for the switch question. The
/// Delete box's title is not measured, and this is the title the reference gives the box next to it.
const BOX_TITLE: &str = "Keyboard Shortcuts and Menus";

/// The number the next dialog session takes: a fresh one for every `open`. See
/// [`ShortcutsSession::id`].
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The open Keyboard Shortcuts and Menus dialog, at `app.prefs_rt.shortcuts`.
pub(crate) struct ShortcutsSession {
    /// Which opening of the dialog this draft belongs to. The dialog's Cancel and `ui.dialog.cancel`
    /// take the draft away and leave an open question standing, and the next `open` starts a fresh
    /// draft of the preferences. A question, and a Save dialog it asked for a name through, names
    /// the session it was opened on, so an answer that arrives late cannot move a draft that never
    /// saw it (#14450).
    pub(crate) id: u64,
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
    /// Set by picking another entry while the rows differ from the set in use: the pass asks the
    /// switch question before anything moves (#14450).
    pub(crate) ask_switch: Option<String>,
    /// Set by a Delete click: the pass asks the Delete question before the set goes (#14450).
    pub(crate) ask_delete: bool,
    /// Set with `ask_name` by Yes of the switch question, when the set in use has no saved copy to
    /// write (the built-in set, or a name the engine refuses): the picked target, applied once the
    /// Save dialog has answered. `ask_new_name` clears it, so a later Save or New cannot pick it up.
    pub(crate) switch_target: Option<String>,
    /// Set by the row's Save on a saved set and by Yes of the switch question on one: OK writes that
    /// set's `.mnu` file, which is where the reference application keeps a set it has saved (#14450).
    /// The value is the file the Save dialog chose, when there was one, or `None` for the set's own
    /// file; a set that never picked one goes to the menu-sets folder, and one the folder was found
    /// holding goes back to that file (`file_paths`). A re-save of a set keeps the destination it
    /// already has rather than replacing it.
    pub(crate) file_write: BTreeMap<String, Option<String>>,
    /// Set when the menu-sets folder is read at open: the file each discovered set was found in, as
    /// the folder lists it. Save and Delete of such a set take that file, because the set's own name
    /// does not always reproduce it — a folder that lists `Darkroom.mnu` has that file, not the
    /// `Darkroom.MNU` `menu_sets::path_of` would name (#14450).
    pub(crate) file_paths: BTreeMap<String, PathBuf>,
    /// Set by Yes of the Delete question: OK takes that set's `.mnu` file away too, so the folder does
    /// not offer the set back on the next open (#14450). The value is the set's file as the draft knew
    /// it at that moment - the one its Save dialog had chosen, or the one the folder scan found it in,
    /// or `None` for the menu-sets folder's own name for the set - taken there so the delete answers
    /// for that path: a later Save of the same name to a different file cannot spare the file the
    /// Delete was made for, and the folder would read the deleted set back from it.
    pub(crate) file_delete: BTreeMap<String, Option<PathBuf>>,
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

    /// The Save dialog's default name: the shown name plus " Copy", ending in the extension the
    /// set's name drops ("<defaults> (modified) Copy.MNU", "<defaults> Copy.MNU"), as measured
    /// (#14450). The extension is part of the name: a set named `Web.v2` would otherwise come back
    /// from `file_stem()` as `Web` and replace an existing `Web` set.
    fn copy_name(&self) -> String {
        format!("{} Copy.MNU", self.shown_name())
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
        // A Save or New click is its own ask: it never carries a switch the row did not finish.
        self.switch_target = None;
        if self.sets.len() >= menu_sets::MAX_SETS {
            self.cap_refused = true;
        } else {
            self.ask_name = true;
        }
    }

    /// Choose the set `name` ("" or the built-in name: the built-in set) and load its menus into
    /// the rows, dropping unsaved edits: what the switch question's "No" does, and what a pick of an
    /// unmodified set does without asking (#14450).
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

    /// Yes of the switch question: save the rows as they stand, then switch to `target`. A saved set
    /// takes the write straight away — the measured rule that Save on a saved set overwrites it
    /// without a dialog — while the built-in set, and a name from a hand-edited settings file that
    /// the engine would refuse, can only get their name from the Save dialog: there the switch waits
    /// on `switch_target`, and a cancelled name dialog leaves the row on its set and its edits
    /// (#14450, and #14326's measurement of the same box on the other tab).
    fn save_then_switch(&mut self, target: &str) {
        if !self.set.is_empty() && set_name(&self.set).is_ok() {
            let name = self.set.clone();
            self.store(&name);
            // Yes saves the set the way its Save button does, and the set's file with it. A file the
            // Save dialog already chose for that set stays its file: only a set with no chosen file
            // is queued to the menu-sets folder, so a re-save cannot send one elsewhere back.
            self.file_write.entry(name).or_insert(None);
            self.choose(target);
        } else if self.sets.len() >= menu_sets::MAX_SETS {
            // The save could only name a set the engine refuses to store: the cap line the row's
            // own New shows, and the row does not move.
            self.cap_refused = true;
        } else {
            self.ask_name = true;
            self.switch_target = Some(target.to_owned());
        }
    }

    /// The file of a set, as the draft knows it at this moment: the one its Save dialog answered
    /// with, or the one the folder scan found it in. `None` leaves it to the folder's own name for
    /// the set.
    fn set_file_of(&self, name: &str) -> Option<PathBuf> {
        match self.file_write.get(name) {
            Some(Some(path)) => Some(PathBuf::from(path)),
            _ => self.file_paths.get(name).cloned(),
        }
    }

    /// Yes of the Delete question: the set it named, if that is still the set in use. The set's file
    /// goes with it, since the folder is where a saved set lives (#14450). The file is taken here,
    /// at the moment the Delete was confirmed: a later Save of the same name can send that name to
    /// another file, and the delete still answers for the file it was made for.
    fn delete_named(&mut self, name: &str) {
        if self.set == name {
            self.delete();
            self.file_delete.insert(name.to_owned(), self.set_file_of(name));
        }
    }
}

/// The Menus tab's question about unsaved edits, asked when the row picks another set while the
/// rows differ from the set in use (#14450). Its tag names the set the row picked and the session
/// it was asked on: the switch, and the draft it belongs to.
pub(crate) struct SetSwitch {
    target: String,
    /// The [`ShortcutsSession::id`] of the dialog this box was opened for.
    session: u64,
}

/// The Menus tab's Delete question (#14450): "Are you sure you want to send <name> to the recycle
/// bin?". Its tag names the set whose Delete was clicked, so an answer only deletes that set.
pub(crate) struct SetDelete {
    name: String,
}

/// The switch question's "Don't show again", kept under `Preferences::dialogs` with the other
/// prompts' (`alerts::dismissed` / `alerts::remember_skip`). The reference asks it on both tabs of
/// this dialog, so one key covers the question; the Delete question has no such box.
pub const SWITCH_PREF: &str = "edit.menus.switchSet";

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
    let rows = match app.framework.catalog_rows_at(app.session.actions().list.revision()) {
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
        id: NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        state: if tab == 1 { DialogState::new().with_tab("menus") } else { DialogState::new() },
        profiles: profiles_of(prefs),
        keymap: draft_keymap(app),
        menus: MenusDraft::of_prefs(prefs),
        pending_import: None,
        remembered: BTreeMap::new(),
    };
    app.prefs_rt.shortcuts = Some(session);
    // The sets the menu-sets folder holds join the row: in the reference application the folder is
    // where a set lives, so a file is the set of that name, and a set the folder has no file for keeps
    // the copy the preferences hold. A file speaks only for the items its rows number: the row's set
    // is that file merged with the preferences' copy, which keeps every item the file has no row for
    // (`menu_sets::merged_with_prefs`). Reading the file whole instead would empty a set whose changes
    // are on items we have no measured number for - the file of such a set holds no rows at all. A
    // file that does not read is said out loud, not skipped in silence (#14450).
    let (from_files, errors) = read_set_files(app);
    if let Some(session) = app.prefs_rt.shortcuts.as_mut() {
        for file in from_files {
            let held = session.menus.sets.get(&file.name).cloned();
            if held.is_none() && session.menus.sets.len() >= menu_sets::MAX_SETS {
                // The folder holds more sets than the engine stores. An entry past the cap could only
                // make OK's `edit.menus` replacement be refused - and then nothing the Menus tab
                // carried would be written - so it never reaches the row, and the engine's own cap
                // line says why. The sets the preferences hold are never the ones dropped.
                session.menus.cap_refused = true;
                continue;
            }
            // The file the folder was found holding IS that set's file: Save and Delete go to it,
            // rather than to the file the set's name would suggest and which a folder spelling the
            // extension differently does not hold.
            session.menus.file_paths.insert(file.name.clone(), file.path);
            session.menus.sets.insert(file.name.clone(), menu_sets::merged_with_prefs(&file.set, held.as_ref()));
        }
    }
    if !errors.is_empty() {
        set_lines(app, tl!("A menu set file was not read: {error}"), &errors);
    }
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
    // The session this pass belongs to: the question and the Save dialog it opens can be answered
    // after this draft is gone and another `open` made a new one.
    let session_id = session.id;
    let rows = crate::menus::base_rows(app);
    let options = options();
    let builtins = builtins();
    let ShortcutsSession { id: _, state, profiles, keymap, menus, pending_import, remembered } = &mut session;
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
            // The failure line names the write that failed: shortcuts and menus are separate
            // subjects, and each of them is written whatever the other one did. A refused
            // `edit.keyboardShortcuts` says its line, but it does not take the Menus tab's sets,
            // hidden items, colours and staged set files down with it - the rule slice 2 set for a
            // stale set entry and for a file that could not be written is that OK says what it could
            // not do rather than skipping the rest of its writes.
            let keys_failure = save_keys(app, keymap, profiles).err().map(|e| (tl!("The keyboard shortcuts were not saved: {error}"), e));
            let menus_failure = save_menus(app, menus).err().map(|e| (tl!("The menus were not saved: {error}"), e));
            if let Some((subject, e)) = keys_failure.or(menus_failure) {
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
        ask_set_name(app, state, &suggested_set_file(app, menus), session_id);
    }
    // The two questions the Set row raises are opened here, beside the pass's own Save-dialog ask:
    // the row has been drawn and its draft state is settled, and the host draws the box over the
    // dialog in this same frame. The draft is left where it was: only the answer moves the row.
    if let Some(target) = menus.ask_switch.take() {
        if alerts::dismissed(app, SWITCH_PREF) {
            // The box was silenced by a ticked "Don't show again": the switch goes ahead, and its
            // unsaved edits go with it, as the reference's "No" does them.
            menus.choose(&target);
        } else {
            ask_switch_set(app, &target, session_id);
        }
    }
    if menus.ask_delete {
        menus.ask_delete = false;
        // Delete is greyed for the built-in set, so there is a saved set to ask about.
        if !menus.set.is_empty() {
            ask_delete_set(app, &menus.set);
        }
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
    let written = app.run("prefs.set", call).map_err(|e| e.to_string())?;
    // OK is also what writes and deletes the sets' `.mnu` files: as with the preferences, nothing the
    // Set row did reaches the disk before it, so a cancelled dialog leaves the folder alone.
    write_set_files(app, menus);
    Ok(written)
}

/// The `.mnu` files of the sets the row saved, and the files of the sets it deleted: the file the Save
/// dialog chose for a set, else the file the menu-sets folder was found holding it in, else the set's
/// own file in that folder, which is where the reference application keeps its sets. The write makes
/// the folder when it is not there. Files that cannot be written or deleted are said on the app's
/// status line rather than undoing the preferences write that came before it, and several of them are
/// said together: five broken files are five sets, not one (#14450).
fn write_set_files(app: &mut PhotocraftApp, menus: &MenusDraft) {
    let Some(dir) = sets_folder(app) else { return };
    let mut refused_write = Vec::new();
    // The two queues carry no order of their own, so they are resolved at OK. The writes are
    // resolved on what the draft ends with: a name whose set is gone - deleted, and not made again
    // before the dialog closed - keeps its write skipped, since there is no set left to write. A
    // write is recorded by what came back, not by what it was aimed at: a save that LANDED records
    // the path it landed in, and a save that was refused - a name with no file to write to, or a
    // path that refused the write - records only that that name's save failed. The queued deletes
    // below step aside for a failed save of their name, and for the very path a landed save wrote.
    let mut saved_at: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut save_refused: BTreeSet<&str> = BTreeSet::new();
    for name in menus.file_write.keys() {
        let Some(set) = menus.sets.get(name) else { continue };
        // Where the write goes: the set's own file, or - spelling out what `write_set` would do -
        // the menu-sets folder's own name for the set.
        let to = match menus.set_file_of(name) {
            Some(path) => Ok(path),
            None => menu_sets::path_of(&dir, name),
        };
        match &to {
            // The result decides what is recorded: the path only for a file that really was written,
            // the name alone when the write came back refused.
            Ok(path) => match menu_sets::write_set_at(path, name, set) {
                Ok(()) => {
                    saved_at.insert(name.as_str(), path.clone());
                }
                Err(e) => {
                    save_refused.insert(name.as_str());
                    refused_write.push(e);
                }
            },
            Err(e) => {
                // The name has no file it could be written to, so there is nothing to land.
                save_refused.insert(name.as_str());
                refused_write.push(e.clone());
            }
        }
    }
    set_lines(app, tl!("The menu set was not saved: {error}"), &refused_write);
    let mut refused_delete = Vec::new();
    // A queued delete steps aside in two cases, and only in those. A Save of that name that wrote the
    // very file the delete was made for - the file it stood in when the Delete question was answered,
    // taken then and not here - rewrote that file, so it is the set's file and not a leftover. A Save
    // of that name that was refused leaves the file the delete was made for as the one copy of the set
    // the disk holds: taking it away after a replacement that never landed would leave the set in the
    // preferences with no file anywhere, and the user would have lost it. A Save that landed
    // ELSEWHERE spares nothing, and the old file goes with the set that moved: a file left standing in
    // the folder would offer the deleted set back at the next open (#14450).
    for (name, target) in &menus.file_delete {
        if save_refused.contains(name.as_str()) {
            // The set's replacement never reached the disk: the file this delete was made for is the
            // only copy of it the folder holds, and it stays for the next open to read. Said by the
            // refused-save line above, so this delete adds no line of its own.
            continue;
        }
        let path = match target {
            Some(path) => path.clone(),
            None => match menu_sets::path_of(&dir, name) {
                Ok(path) => path,
                Err(e) => {
                    refused_delete.push(e);
                    continue;
                }
            },
        };
        if saved_at.get(name.as_str()).is_some_and(|written| *written == path) {
            continue;
        }
        let taken = menu_sets::delete_set_at(&path);
        if let Err(e) = taken {
            refused_delete.push(e);
        }
    }
    set_lines(app, tl!("The menu set was not deleted: {error}"), &refused_delete);
}

/// Where a saved set lives as a file: the menu-sets folder of the settings directory
/// (`menu_sets::folder`), which is where the reference application keeps its sets. `None` where the
/// app has no settings folder at all — the web build, whose `Services` holds no store, and tests that
/// open the dialog without one. Without it the row keeps its sets in the preferences only.
#[cfg(target_arch = "wasm32")]
fn sets_folder(_: &PhotocraftApp) -> Option<std::path::PathBuf> {
    None
}

/// The same on the desktop: the folder of the opened settings store, `menu_sets::SETS_DIR` inside it.
#[cfg(not(target_arch = "wasm32"))]
fn sets_folder(app: &PhotocraftApp) -> Option<std::path::PathBuf> {
    app.services.settings.as_ref().and_then(|(store, _)| store.dir()).map(menu_sets::folder)
}

/// A Menus-tab set line for the app's status line. The framework draws its in-dialog bars only on its
/// own tab, so the Menus tab's set problems are said there, as slice 2 established.
fn set_line(app: &mut PhotocraftApp, subject: &str, error: &str) {
    app.ui.status = crate::i18n::fmt(subject, &[("error", error)]);
    app.ui.status_error = true;
}

/// The same line for several set problems at once: the first one in full, and how many more there
/// were. One problem is said as before; none at all says nothing.
fn set_lines(app: &mut PhotocraftApp, subject: &str, errors: &[String]) {
    let Some((first, rest)) = errors.split_first() else { return };
    let said = if rest.is_empty() { first.clone() } else { crate::i18n::fmt(tl!("{error} and {n} more"), &[("error", first), ("n", &rest.len().to_string())]) };
    set_line(app, subject, &said);
}

/// The set files the menu-sets folder holds, each with the name its file gives and the path it stands
/// at, and one message per file that could not be read. The folder itself is not an error: it comes
/// with the first saved set.
fn read_set_files(app: &PhotocraftApp) -> (Vec<menu_sets::SetFile>, Vec<String>) {
    match sets_folder(app) {
        Some(dir) => menu_sets::read_sets(&dir),
        None => (Vec::new(), Vec::new()),
    }
}

/// The name the Save dialog is given: the measured "<current set name> Copy.MNU", inside the
/// menu-sets folder when the app has one, so the dialog starts there as measured (#14450). The
/// extension is part of the name: `ask_set_name` takes the stem, so only `.MNU` is stripped and a
/// dotted set name (`Web.v2 Copy`) keeps its dots.
fn suggested_set_file(app: &PhotocraftApp, draft: &MenusDraft) -> String {
    let name = draft.copy_name();
    match sets_folder(app) {
        Some(dir) => dir.join(&name).to_string_lossy().into_owned(),
        None => name,
    }
}

/// Save on the built-in set and New ask for the set's name through the system Save dialog, which
/// suggests `suggested` — the menu-sets folder of the settings directory, the "Menu Customization
/// (*.MNU)" type and the default name "<current set name> Copy.MNU", all as measured. The chosen
/// file's name without its extension names the set, and the file itself is written there: a saved set
/// lives as a `.mnu` file, so a set the dialog answered with is stored with the preferences and in
/// that file (#14450).
fn ask_set_name(app: &mut PhotocraftApp, state: &mut DialogState, suggested: &str, session: u64) {
    let asked = app.pick_save(suggested, move |app, path| {
        let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // The engine's own name rules: a name it would refuse (empty, overlong, the built-in one)
        // is an error line, not a set the row could never select; the dialog stays open either way.
        // Its bars belong to the framework's own tab, so the line goes to the app's status line.
        let refused = match app.prefs_rt.shortcuts.as_mut().filter(|draft| draft.id == session) {
            Some(session) => match set_name(&stem) {
                Ok(name) => {
                    session.menus.store(&name);
                    // The file the dialog chose is the set's file, and OK writes it: the dialog
                    // started in the menu-sets folder, so that is where it lands unless the user went
                    // elsewhere. A cancelled name dialog never reaches here, and writes nothing.
                    session.menus.file_write.insert(name, Some(path.clone()));
                    // Yes of the switch question was waiting on this name: the save is in, so the
                    // switch it promised goes ahead. The next Save or New clears the target.
                    if let Some(target) = session.menus.switch_target.take() {
                        session.menus.choose(&target);
                    }
                    None
                }
                Err(e) => Some(e),
            },
            // The dialog went away with the session, or another one opened since: the name belongs
            // to the draft that asked, so nothing is named and nothing is written.
            None => None,
        };
        if let Some(e) = refused {
            set_line(app, tl!("The menu set was not saved: {error}"), &e);
        }
        Ok(control_reply())
    });
    if let Err(e) = asked {
        state.show_error(e);
    }
}

/// The switch question the Menus tab holds a pick of another set behind (#14450): the measured
/// title, text (it says "shortcuts" on this tab too) and Yes / No / Cancel, with the measured
/// "Don't show again" box. Its tag names the picked set, so the answer applies the switch that was
/// held back; the row stays on its own set while the box waits.
///
/// The buttons carry the roles the framework's own Yes / No / Cancel question gives them (`unsaved.rs`
/// in `storytold/craft-libs`): `Proceed` is Yes and is the default button Enter answers, `Cancel` is
/// the box's Cancel and answers Escape and the close action, and `Alternate` is No. The footer places
/// its buttons by role, so Windows stands them Yes, Cancel, No: the reference's literal Yes, No,
/// Cancel order is not expressible through a hosted `StandardPrompt` (see the PR text).
fn ask_switch_set(app: &mut PhotocraftApp, target: &str, session: u64) {
    let prompt = StandardPrompt::custom(tl!(SWITCH_ASK).to_owned())
        .title(tl!(BOX_TITLE).to_owned())
        .button(PromptChoice::Proceed, tl!("Yes").to_owned())
        .button(PromptChoice::Alternate, tl!("No").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .dont_show_again()
        .tag(SetSwitch { target: target.to_owned(), session });
    // An `Err` is a question already open (this one or another family's): the row keeps its set and
    // the next pick asks again.
    let _ = alerts::open_prompt(app, prompt);
}

/// The Delete question for the set `name` (#14450): the measured text naming the set, [Yes] [No].
/// Yes removes the set, No (and Escape, and the close action) keeps it. The reference shows no
/// "Don't show again" box here, so this question has none.
fn ask_delete_set(app: &mut PhotocraftApp, name: &str) {
    let message = crate::i18n::fmt(tl!(DELETE_ASK), &[("name", name)]);
    let prompt = StandardPrompt::custom(message)
        .title(tl!(BOX_TITLE).to_owned())
        .button(PromptChoice::Proceed, tl!("Yes").to_owned())
        .button(PromptChoice::Cancel, tl!("No").to_owned())
        .tag(SetDelete { name: name.to_owned() });
    let _ = alerts::open_prompt(app, prompt);
}

/// The `PROMPTS` row for the Menus tab's two questions. The switch question: Yes saves the rows as
/// they stand and then loads the picked set, No loads it and drops the unsaved edits, and its Cancel
/// keeps set and edits; a ticked "Don't show again" goes to the dialog preferences. The Delete
/// question: Yes removes the set it named, anything else keeps it. `false` for another family's
/// answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    if let Some(SetSwitch { target, session: asked }) = answer.take_tag::<SetSwitch>() {
        let (choice, skip) = (answer.choice, answer.skip);
        // The dialog can have closed while the box waited (its Cancel, `ui.dialog.cancel`): the
        // answer then writes nothing, since the draft it belongs to is gone. A dialog opened again
        // is another session with a fresh draft, so the answer has to name the session the box was
        // opened on before it writes into the draft standing now.
        if let Some(session) = app.prefs_rt.shortcuts.as_mut().filter(|current| current.id == asked) {
            match choice {
                PromptChoice::Proceed => session.menus.save_then_switch(&target),
                PromptChoice::Alternate => session.menus.choose(&target),
                _ => {}
            }
        }
        // The framework reports the check box only with its default button, so a tick is kept with
        // Yes; No and Cancel leave the question asked again next time.
        if skip {
            alerts::remember_skip(app, SWITCH_PREF);
        }
        return true;
    }
    if let Some(SetDelete { name }) = answer.take_tag::<SetDelete>() {
        if answer.choice == PromptChoice::Proceed
            && let Some(session) = app.prefs_rt.shortcuts.as_mut()
        {
            session.menus.delete_named(&name);
        }
        return true;
    }
    false
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
            if draft.modified() {
                // Unsaved edits: the row does not move. The pass asks the switch question, and only
                // its Yes or No loads the picked set (#14450).
                draft.ask_switch = Some(picked);
            } else {
                draft.choose(&picked);
            }
        }
        if save.clicked() {
            if draft.set.is_empty() {
                // The built-in set has no saved copy: the name comes from the Save dialog.
                draft.ask_new_name();
            } else {
                let name = draft.set.clone();
                draft.store(&name);
                // "Save on a modified saved set overwrites its file without a dialog" (#14450): the
                // file is the one that set already has - the one the Save dialog chose for it, or the
                // menu-sets folder's own if it never picked one - and OK is what writes it.
                draft.file_write.entry(name).or_insert(None);
            }
        }
        if new.clicked() {
            draft.ask_new_name();
        }
        if delete.clicked() {
            // Delete is greyed for the built-in set, so there is a saved set to ask about.
            draft.ask_delete = true;
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
            with_env(
                ui,
                |set| set.id_salt = Some(egui::Id::unique(id)),
                |ui| {
                    LayoutRow::new().show(ui, |ui, _| {
                        Text::new(format!("{} › {}", row.path.join(" › "), row.label)).show(ui);
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
                },
            );
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
