//! The macOS menu bar: the same menus as the in-window bar ([`crate::menus::items::model`]), laid
//! out the way a Mac app's are by craft-menu and handed to its platform
//! backend.
//!
//! craft-menu owns the layout, the clash rules, the key equivalents and the driver. This module
//! supplies what is PhotoCraft's: the app name, the translated labels, the shortcuts bound outside
//! the menus and the gate and modal rules ([`crate::menus::menu_gate`]).
//!
//! A menu click is a fired command ([`poll`] fires it with `FireSource::Menu`); the runtime's
//! drain runs it like any other fired command, so the menu has no command path of its own.

#[cfg(any(target_os = "macos", test))]
use craft::menu::Layout;
use craft::menu::{NativeMenu, Target};
use craft::ui::runtime::commands::{FireSource, fire};
use craft::ui::runtime::windows::WindowHost;

use crate::PhotocraftApp;
use crate::menus::items;

/// The name the app menu, About, Hide and Quit show.
pub const APP_NAME: &str = "PhotoCraft";

fn model<'a>(app: &'a mut PhotocraftApp, ctx: &egui::Context) -> &'a items::MenuModel {
    // Installation and sync can run before the drawing language catches up.
    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
    let _language = crate::i18n::language_scope(lang);
    items::model_for(app, ctx, Target::Mac)
}

/// The Mac menu layout in the saved interface language.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn laid_out(app: &mut PhotocraftApp, ctx: &egui::Context) -> Layout {
    model(app, ctx).layout().clone()
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
    let keymap = app.framework.keymap_inputs();
    let model = model(app, ctx);
    let shown = (model.bar.revision(), keymap);
    for c in native.sync_with(&shown, || model.layout().clone()) {
        log::info!("menu: {} is {} on macOS and {} in PhotoCraft; {}", c.shortcut.portable(), c.system, c.command.id, c.resolution);
    }
    app.services.native_menu = Some(native);
}

#[cfg(test)]
mod tests;
