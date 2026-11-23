use craft::ui::contract::Show as _;
use craft::ui::shell::{Title, TitleBar};

use super::{PhotocraftApp, brand};

// ----------------------------------------------------------------------------- title bar

/// The app's top bar, in the shell's title slot: brand mark, menus and the document title. Nothing else sits at its
/// far end, as in the reference application: the workspace switcher, command search and appearance mode live in
/// Window › Workspace, Edit › Search and Window › Appearance (#13577).
/// The framework `TitleBar` follows the window's frame: with the app's own frame (Windows and Linux) it is also the
/// window's title bar and the caption buttons take its ends; with the system's frame it shows the content and no title.
pub fn title_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let title = title_text(app);
    let out = TitleBar::new().show_titled(
        ui,
        Title::new(title),
        |ui, _| {
            brand::mark().show(ui);
            // With the macOS menu bar the menus are at the top of the screen instead.
            if app.services.native_menu.is_none() {
                crate::menus::menu_bar(app, ui);
            }
        },
        |_, _| {},
    );
    // The bar never closes the window: File › Exit asks about unsaved work first.
    if out.close_requested {
        let _ = crate::menus::invoke(app, ui.ctx(), "file.exit", serde_json::json!({}));
    }
}

/// The title the bar shows: the active document's name with a dot when it has unsaved changes, or the app name with
/// no document open. With Preferences › Interface › Show File Name In Title off, the file name drops out and the app
/// name (and the dot) remains, since the tabs already name the file (#5037).
pub(crate) fn title_text(app: &PhotocraftApp) -> String {
    title_text_for(app, crate::os_window::identity())
}

fn title_text_for(app: &PhotocraftApp, identity: crate::os_window::LaunchIdentity) -> String {
    let document = app.session.active().map(|d| (d.doc.name.as_str(), d.is_dirty()));
    crate::channel_titles::drawn_title(
        crate::os_window::name_for(identity),
        document,
        app.session.prefs().interface.show_file_name_in_title,
        identity == crate::os_window::LaunchIdentity::NightlyV1,
    )
}

/// The OS window title retains its channel through document/dirty/preference changes.
/// Stable formatting is unchanged; Nightly never falls back to the stable brand name.
pub fn window_title(app: &PhotocraftApp) -> String {
    window_title_for(app, crate::os_window::identity())
}

fn window_title_for(app: &PhotocraftApp, identity: crate::os_window::LaunchIdentity) -> String {
    let document = app.session.active().map(|d| (d.doc.name.as_str(), d.is_dirty()));
    crate::channel_titles::os_title(crate::os_window::name_for(identity), document, app.session.prefs().interface.show_file_name_in_title)
}

#[cfg(test)]
mod tests;
