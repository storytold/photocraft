//! The Home screen: the framework start page with PhotoCraft's recent files, links and drop hint, and the Home rule
//! the options bar's button, the rail's Home link and `window.home` share.

use craft::behavior::IconId;
use craft::ui::contract::Bind;
use craft::ui::runtime::env::Label;
use craft::ui::shell::{StartAction, StartLink, StartScreen};

use super::PhotocraftApp;

/// Recent files listed on the Home screen.
const HOME_RECENT: usize = 11;

/// The Home screen in the document area. The recent list is empty on the web, which has no paths to reopen, and with
/// Show Home Recent Files off.
pub(super) fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let show_recent = !cfg!(target_arch = "wasm32") && app.session.prefs().interface.show_home_recent_files;
    let recent = if show_recent { crate::home_recent_ui::recent_files(app, HOME_RECENT) } else { Vec::new() };
    let thumbs = crate::home_recent_ui::thumbnails(&ctx);
    // The source reads the app, so the page binds copies of the view and the sort and they are written back after.
    let (mut view, mut sort) = (app.ui.chrome.home_recent_view, app.ui.chrome.home_recent_sort);
    let links = links();
    let footer = footer_links();
    let actions = {
        let source = crate::home_recent_ui::source(app, &ctx);
        StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &recent)
            .thumbnails(&thumbs, &source)
            .links(&links[..])
            .home_layout(Label::from(crate::native_menu::APP_NAME), &footer[..])
            .max_recent(HOME_RECENT)
            .drop_hint(Label::from(tl!("Drop an image or PSD anywhere to open it.")))
            .show(ui)
            .actions
    };
    app.ui.chrome.home_recent_view = view;
    app.ui.chrome.home_recent_sort = sort;
    for action in actions {
        // The recent list's own actions and Open; New is the New Document dialog's, Show in Folder the platform's.
        if let StartAction::RevealRecent(path) = &action {
            reveal(app, path);
            continue;
        }
        if crate::home_recent_ui::act(app, &action) {
            continue;
        }
        if action == StartAction::New {
            crate::new_doc_ui::open(app);
        }
    }
}

/// Shows a recent file selected in the system file manager; a failure goes to the status line, as the other open
/// errors do. The start screen offers Show in Folder only where the platform can reveal a file.
fn reveal(app: &mut PhotocraftApp, path: &std::path::Path) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(e) = craft::platform::open::reveal(path) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, path);
}

/// The navigation of the Home rail: Home (the page shown, so selected), which leaves Home for the open documents, then
/// Learn.
fn links() -> [StartLink; 2] {
    [
        StartLink::command(tl!("Home"), Some(IconId::new("home")), "window.home").selected(true),
        StartLink::url(tl!("Learn"), Some(IconId::new("sparkles")), crate::links::TUTORIALS),
    ]
}

/// The community pages, at the bottom of the Home rail.
fn footer_links() -> [StartLink; 3] {
    [
        StartLink::url(tl!("Website"), Some(IconId::new("link")), crate::links::APP_PAGE),
        StartLink::url(tl!("Discord"), Some(IconId::new("globe")), crate::links::DISCORD),
        StartLink::url(tl!("GitHub"), Some(IconId::new("globe")), crate::links::GITHUB),
    ]
}

/// Shows Home over the open documents, or leaves it for them: the rule of the options bar's Home button, the Home
/// rail's Home link and `window.home`. With no document open there is nothing behind Home, so it does nothing.
pub(crate) fn toggle_home(app: &mut PhotocraftApp) {
    let documents = app.session.documents().len();
    if documents == 0 {
        return;
    }
    let auto = app.session.prefs().general.auto_show_home_screen;
    let up = app.ui.chrome.shows_home(documents, auto);
    app.ui.chrome.home = if up { None } else { Some(documents) };
}

#[cfg(test)]
#[path = "start_screen/tests.rs"]
mod tests;
