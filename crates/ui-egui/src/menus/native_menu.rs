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
    // The system items give way to the command the keymap gives a key to, in a menu or not (tool keys, canvas keys).
    let keys = |chord: &craft::keymap::Chord| holder(ctx, chord);
    let options = LayoutOptions::new(APP_NAME).label(&label).keys(&keys);
    mac_layout(items::model(app, ctx).source(), &options)
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
