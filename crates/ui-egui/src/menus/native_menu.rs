//! The macOS menu bar: the same menus as the in-window bar ([`crate::menus::items::model`]), laid
//! out the way a Mac app's are by craft-menu's [`mac_layout`] and handed to craft-menu's platform
//! backend.
//!
//! craft-menu owns the layout, the clash rules, the key equivalents and the driver. This module
//! supplies what is PhotoCraft's: the app name, the translated labels, the shortcuts bound outside
//! the menus and the gate and modal rules ([`crate::menus::menu_gate`]). The menus are laid out again
//! only when the model's bar or the keymap changed.
//!
//! A menu click is a fired command ([`poll`] fires it with `FireSource::Menu`); the runtime's
//! drain runs it like any other fired command, so the menu has no command path of its own.

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
    let keys = menu_keys(app, ctx);
    let options = LayoutOptions::new(APP_NAME).label(&label).keys(&keys);
    mac_layout(&items::model(app, ctx).bar, &options)
}

/// What a layout's system items give way to: the command the keymap gives a key to, in a menu or not (tool keys,
/// canvas keys). With ⌘H Hides PhotoCraft on, only a ⌘H the user gave a command keeps Hide off ⌘H: a workspace's
/// scoped ⌘H (Content-Aware Fill's Hide Sampling Area) works only inside that workspace.
pub(crate) fn menu_keys<'a>(app: &PhotocraftApp, ctx: &'a egui::Context) -> impl Fn(&craft::keymap::Chord) -> Option<craft::keymap::CommandRef> + 'a {
    keys_under(cmd_hides_app(app).then(|| user_cmd_h(app.session.prefs())), move |chord| holder(ctx, chord))
}

/// [`menu_keys`] over the keymap's `holder`: while ⌘H Hides PhotoCraft is in effect, `user_cmd_h` is the command the
/// user gave ⌘H, if any, and stands in for the keymap's holder of it.
fn keys_under(
    user_cmd_h: Option<Option<craft::keymap::CommandRef>>,
    holder: impl Fn(&craft::keymap::Chord) -> Option<craft::keymap::CommandRef>,
) -> impl Fn(&craft::keymap::Chord) -> Option<craft::keymap::CommandRef> {
    let cmd_h = craft::keymap::Chord::parse("Cmd+H").ok();
    move |chord| match &user_cmd_h {
        Some(own) if Some(chord) == cmd_h.as_ref() => own.clone(),
        _ => holder(chord),
    }
}

/// Whether ⌘H Hides PhotoCraft is in effect: it moved View › Extras off ⌘H.
fn cmd_hides_app(app: &PhotocraftApp) -> bool {
    app.session.prefs().interface.default_shortcut("view.extras", Some("Cmd+H")) != Some("Cmd+H")
}

/// The command the user gave ⌘H, as a main or further key: with ⌘H Hides PhotoCraft on, no default of PhotoCraft's
/// has ⌘H (View › Extras moved to ⌃⌘H), so only such a key keeps Hide off it. Read from the preferences rather than
/// the runtime's keymap, which follows them a frame later.
fn user_cmd_h(prefs: &photocraft_engine::prefs::Preferences) -> Option<craft::keymap::CommandRef> {
    use photocraft_engine::prefs::normalize_shortcut;
    let cmd_h = normalize_shortcut("Cmd+H");
    let main = prefs.shortcuts.iter().find(|(_, key)| normalize_shortcut(key) == cmd_h).map(|(id, _)| id);
    let further = || prefs.alternate_shortcuts.iter().find(|(_, keys)| keys.iter().any(|key| normalize_shortcut(key) == cmd_h)).map(|(id, _)| id);
    main.or_else(further).map(|id| crate::shortcut_dispatch::keymap_command(id))
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
/// when what they show changed: the model's bar (its revision) or the keymap whose keys the layout
/// gives way to.
pub fn sync(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Taken out for the pass: the layout borrows the whole app.
    let Some(mut native) = app.services.native_menu.take() else { return };
    let (gate, rules) = crate::menus::menu_gate(app);
    native.set_gate(gate.text_editing(WindowHost::of(ctx).typing()));
    native.set_modal_rules(rules);
    let shown = (items::model(app, ctx).bar.revision(), app.framework.keymap_inputs());
    for c in native.sync_with(&shown, || laid_out(app, ctx)) {
        log::info!("menu: {} is {} on macOS and {} in PhotoCraft; {}", c.shortcut.portable(), c.system, c.command.id, c.resolution);
    }
    app.services.native_menu = Some(native);
}

#[cfg(test)]
mod tests;
