//! The macOS menu bar: the same menus as the in-window bar ([`crate::menus::items::model`]), laid
//! out the way a Mac app's are by craft-menu's [`mac_layout`] and handed to craft-menu's platform
//! backend.
//!
//! craft-menu owns the layout, the clash rules, the key equivalents and the driver. This module
//! supplies what is PhotoCraft's: the app name, the translated labels, the shortcuts bound outside
//! the menus, the gate and modal rules ([`crate::menus::menu_gate`]) and the hash of the state the
//! menus show ([`state_hash`]).
//!
//! A menu click is a fired command ([`poll`] fires it with `FireSource::Menu`); the runtime's
//! drain runs it like any other fired command, so the menu has no command path of its own.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::menu::{Layout, LayoutOptions, NativeMenu, mac_layout};
use craft::ui::runtime::commands::{FireSource, fire, holder};
use craft::ui::runtime::windows::WindowHost;

use crate::PhotocraftApp;
use crate::menus::items;

/// The name the app menu, About, Hide and Quit show.
pub const APP_NAME: &str = "PhotoCraft";

/// [`items::model`]'s menus laid out as a Mac menu bar, in the UI language.
///
/// The one place that lays the model out for macOS: the first install, every re-sync and the tests
/// use it. The model itself is the one menu definition both bars read.
pub(crate) fn laid_out(app: &mut PhotocraftApp, ctx: &egui::Context) -> Layout {
    let lang = crate::i18n::current();
    let label = |english: &'static str| crate::i18n::tr(lang, english).to_string();
    // The system items give way to the command the keymap gives a key to, in a menu or not (tool keys, canvas keys).
    let keys = |chord: &craft::keymap::Chord| holder(ctx, chord);
    let options = LayoutOptions::new(APP_NAME).label(&label).keys(&keys);
    mac_layout(&items::model(app, ctx).bar, &options)
}

/// Installs the macOS menu bar; `None` where there is none (every other platform).
#[cfg(target_os = "macos")]
pub fn install(ctx: &egui::Context, app: &mut PhotocraftApp) -> Option<NativeMenu> {
    let layout = laid_out(app, ctx);
    craft::menu::mac::install_with(ctx, &layout.bar, craft::menu::mac::MacOptions::default().window_list(false))
}

/// Installs the macOS menu bar; `None` where there is none (every other platform).
#[cfg(not(target_os = "macos"))]
pub fn install(_ctx: &egui::Context, _app: &mut PhotocraftApp) -> Option<NativeMenu> {
    None
}

/// Before the frame: reads the menu's events and fires the clicks the gate allows into the runtime.
/// Key equivalents reach egui as key presses inside craft-menu's driver.
pub fn poll(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(native) = app.services.native_menu.as_mut() else { return };
    native.poll(ctx);
    for command in native.take_clicks(ctx) {
        fire(ctx, command, FireSource::Menu);
    }
}

/// Once per frame after the pass: sets what may run from the menus and brings the menus up to date
/// when anything they show may have changed. Building the rows walks every command's enabled
/// state, so the layout runs only when [`state_hash`] changes or a menu event arrived.
pub fn sync(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Taken out for the pass: the layout borrows the whole app.
    let Some(mut native) = app.services.native_menu.take() else { return };
    let (gate, rules) = crate::menus::menu_gate(app);
    native.set_gate(gate.text_editing(WindowHost::of(ctx).typing()));
    native.set_modal_rules(rules);
    let state = state_hash(app, craft::menu::had_input(ctx).then_some(app.frame));
    for c in native.sync_with(state, || laid_out(app, ctx)) {
        log::info!("menu: {} is {} on macOS and {} in PhotoCraft; {}", c.shortcut.portable(), c.system, c.command.id, c.resolution);
    }
    app.services.native_menu = Some(native);
}

/// What the menus' rows depend on, cheaply: commands run, the documents (their names, in tab
/// order, for the Window menu) and the active one's revision, selection, recent files, panels and
/// view state, the keymap, and the preferences the rows show. `input` forces a change on frames with a click or key press, which
/// covers state the hash doesn't list.
pub(crate) fn state_hash(app: &PhotocraftApp, input: Option<u64>) -> u64 {
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    let s = &app.session;
    s.journal.len().hash(&mut h);
    s.active_index().hash(&mut h);
    for d in s.documents() {
        (d.doc.id.0, &d.doc.name).hash(&mut h);
    }
    s.clipboard.is_some().hash(&mut h);
    // A running job greys the commands that would change its document.
    s.has_jobs().hash(&mut h);
    (crate::actions::library::edit_undo_label(app), crate::vector_ui::pen_can_undo(app)).hash(&mut h);
    if let Some(d) = s.active() {
        (d.doc.id.0, d.revision, d.active_layer.map(|l| l.0), d.selected_layers.len(), d.isolated_layers.len()).hash(&mut h);
    }
    let ui = &app.ui;
    (&ui.recent_files, &app.workspaces.set.current, app.ui.palette.open, ui.transform.is_some(), ui.text_edit.is_some(), app.dialog_count()).hash(&mut h);
    // Cut and Copy are enabled while the typed text has a selection.
    ui.text_edit.as_ref().map(|e| e.anchor != e.caret).hash(&mut h);
    // The Window bars' ticks; the Contextual Task Bar's Hide bar and `ui.set` change them without a menu click (#15489).
    // Not the task bar's place, which moves every frame of a drag.
    let p = &ui.panels;
    (p.options_bar, p.status_bar, p.menu_bar, p.context_task_bar).hash(&mut h);
    (app.discard.is_some(), crate::distort_ui::modal_workspace(app)).hash(&mut h);
    crate::prefs_ui::recording(app).hash(&mut h);
    // The rows show the keymap's chords; the keymap follows these from the frame they change.
    app.framework.keymap_inputs().hash(&mut h);
    // Edit / Clear Selected Guides are enabled only while a guide is selected.
    crate::guide_select::selected(app).hash(&mut h);
    // The dock workspace's revision moves only on `replace`, so the arrangement is read as the panels in the layout and
    // the keys of the hidden and floating ones; the checked flags cover panels opened or closed and the flyout of a
    // panel in a column of icons.
    let docking = &app.ui.docking;
    docking.layout.panels().hash(&mut h);
    docking.hidden.iter().map(|(panel, _)| panel).collect::<Vec<_>>().hash(&mut h);
    docking.floated.iter().map(|(panel, _)| panel).collect::<Vec<_>>().hash(&mut h);
    for panel in crate::panels::registry::PANELS {
        crate::panel_docking::is_checked(app, panel.id).hash(&mut h);
    }
    serde_json::to_string(&(&ui.extras, &ui.view, ui.theme.id().as_str(), ui.theme.is_dark(), ui.tool)).unwrap_or_default().hash(&mut h);
    // The preferences the rows show: the language, Edit › Menus (hidden rows, colours), the Quick Export label and
    // the Font Preview Size tick.
    let prefs = s.prefs();
    prefs.interface.language.hash(&mut h);
    (&prefs.menus.hidden, &prefs.menus.colors, prefs.interface.show_menu_colors).hash(&mut h);
    (prefs.export.quick_export_format, prefs.type_.font_preview).hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests;
