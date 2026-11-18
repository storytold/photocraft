use std::cell::RefCell;

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _, TooltipSpec};
use craft::ui::shell::{Title, TitleBar};

use super::{PhotocraftApp, brand};

// ----------------------------------------------------------------------------- title bar

/// The app's top bar, in the shell's title slot: brand mark, menus, the document title and the workspace controls.
/// The framework `TitleBar` follows the window's frame: with the app's own frame (Windows and Linux) it is also the
/// window's title bar and the caption buttons take its ends; with the system's frame it shows the content and no title.
pub fn title_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let title = title_text(app);
    // The bar runs `leading` in its body and `trailing` in its end region, one after the other, so the two borrows
    // of the app never overlap.
    let app = RefCell::new(app);
    let out = TitleBar::new().show_titled(
        ui,
        Title::new(title),
        |ui, _| {
            let mut guard = app.borrow_mut();
            let app = &mut **guard;
            brand::mark().show(ui);
            // With the macOS menu bar the menus are at the top of the screen instead.
            if app.services.native_menu.is_none() {
                crate::menus::menu_bar(app, ui);
            }
        },
        |ui, _| {
            let mut guard = app.borrow_mut();
            let app = &mut **guard;
            crate::prefs_ui::appearance_button(app, ui);
            let tip = crate::shortcuts::tip_label(app, "Search commands", "edit.search");
            let search = IconButton::new(IconId::new("search")).label(tip.clone()).tooltip(TooltipSpec::new(tip)).selected(app.ui.palette.open).show(ui);
            if search.clicked() {
                app.ui.palette.open = !app.ui.palette.open;
            }
            crate::workspace_ui::switcher(app, ui);
        },
    );
    // The bar never closes the window: File › Exit asks about unsaved work first.
    if out.close_requested {
        let mut guard = app.borrow_mut();
        let _ = crate::menus::invoke(&mut guard, ui.ctx(), "file.exit", serde_json::json!({}));
    }
}

/// The title the bar shows: the active document's name with a dot when it has unsaved changes, or the app name with
/// no document open. With Preferences › Interface › Show File Name In Title off, the file name drops out and the app
/// name (and the dot) remains, since the tabs already name the file (#5037).
pub(crate) fn title_text(app: &PhotocraftApp) -> String {
    let show_name = app.session.prefs().interface.show_file_name_in_title;
    app.session
        .active()
        .map(|d| format!("{}{}", if show_name { d.doc.name.as_str() } else { "PhotoCraft" }, if d.is_dirty() { "  •" } else { "" }))
        .unwrap_or_else(|| "PhotoCraft".into())
}

/// The OS window title: the active document's name suffixed with the app name (a `*` prefix
/// marks unsaved changes, like the `•` in the app-drawn title), or just the app name with no
/// document open. With Preferences › Interface › Show File Name In Title off, the file name
/// drops out and only the app name (and the `*`) remains (#5037).
pub fn window_title(app: &PhotocraftApp) -> String {
    if !app.session.prefs().interface.show_file_name_in_title {
        return format!("{}PhotoCraft", if app.session.active().is_some_and(|d| d.is_dirty()) { "*" } else { "" });
    }
    app.session.active().map(|d| format!("{}{} \u{2014} PhotoCraft", if d.is_dirty() { "*" } else { "" }, d.doc.name)).unwrap_or_else(|| "PhotoCraft".into())
}

#[cfg(test)]
mod tests;
