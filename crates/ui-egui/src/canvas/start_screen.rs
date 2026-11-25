//! The Home screen: the framework start page with PhotoCraft's recent files, links and drop hint, and the Home rule
//! the options bar's button, the rail's Home link and `window.home` share.

use craft::behavior::IconId;
use craft::ui::contract::Bind;
use craft::ui::runtime::env::{Label, Word, WordRows, WordsOverride, with_env};
use craft::ui::shell::{StartAction, StartLink, StartScreen};

use super::PhotocraftApp;

/// Recent files listed on the Home screen.
const HOME_RECENT: usize = 11;

/// The recent cards keep one size, as many to a row as fit (six on a desktop window), instead of growing to fill it:
/// a factor on the card thumbnail.
const HOME_CARD_ZOOM: f32 = 1.65;

/// The Home screen in the document area. The recent list is empty on the web, which has no paths to reopen, and with
/// Show Home Recent Files off.
pub(super) fn start_screen(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let show_recent = !cfg!(target_arch = "wasm32") && app.session.prefs().interface.show_home_recent_files;
    let recent = if show_recent { crate::home_recent_ui::recent_files(app, HOME_RECENT) } else { Vec::new() };
    let provider_recent =
        app.session.prefs().interface.show_home_recent_files && app.services.file_provider.as_ref().is_some_and(|provider| provider.enabled());
    if provider_recent {
        crate::file_providers::home_recent(app, ui);
        ui.add_space(16.0);
    }
    let drop_hint = if provider_recent && recent.is_empty() {
        if cfg!(target_arch = "wasm32") {
            tl!("Use the browser file picker to reopen a file from this device.")
        } else {
            tl!("There are no recent local files.")
        }
    } else {
        tl!("Drop an image or PSD anywhere to open it.")
    };
    let thumbs = crate::home_recent_ui::thumbnails(&ctx);
    // The source reads the app, so the page binds copies of the view and the sort and they are written back after.
    let (mut view, mut sort) = (app.ui.chrome.home_recent_view, app.ui.chrome.home_recent_sort);
    let links = links();
    let footer = footer_links();
    let actions = {
        let source = crate::home_recent_ui::source(app, &ctx);
        let screen = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &recent)
            .thumbnails(&thumbs, &source)
            .links(&links[..])
            .home_layout(Label::from(crate::native_menu::APP_NAME), &footer[..])
            .max_recent(HOME_RECENT)
            .card_zoom(HOME_CARD_ZOOM)
            .drop_hint(Label::from(drop_hint));
        if provider_recent {
            // The framework owns the local grid and list; provider resources keep their own section.
            let words = WordRows::new().row(Word::keyed("startScreen.recent", "Recent"), tl!("Local Recent Files"));
            with_env(ui, |set| set.words = Some(WordsOverride::Overlay(words)), |ui| screen.show(ui).actions)
        } else {
            screen.show(ui).actions
        }
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
        // A speech bubble and a chain link, as the rail showed before the framework move (#15449).
        StartLink::url(tl!("Discord"), Some(IconId::new("note")), crate::links::DISCORD),
        StartLink::url(tl!("GitHub"), Some(IconId::new("link_layers")), crate::links::GITHUB),
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

#[cfg(test)]
mod provider_tests {
    use craft::ui::shell::RecentView;
    use egui_kittest::kittest::Queryable;

    use super::*;

    const LOCAL_RECENT: &str = "fixtures/Local.psd";

    fn home(width: f32, provider: Option<crate::file_providers::Host>) -> egui_kittest::Harness<'static, (Box<PhotocraftApp>, bool)> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: provider, ..Default::default() });
        *app.ui.recent_files = vec![LOCAL_RECENT.into()];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, 760.0)).with_context(app.ctx().clone()).build_ui_state(
            |ui, (app, ready): &mut (Box<PhotocraftApp>, bool)| {
                if *ready {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| start_screen(app, ui));
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.state_mut().1 = true;
        h.run_steps(3);
        h
    }

    fn provider() -> crate::file_providers::Host {
        let provider = crate::file_providers::Host::default();
        provider.set_session("home-provider-instance".into()).unwrap();
        let connection = provider.connect("storage".into(), "Example Storage".into(), "account".into()).unwrap();
        provider
            .set_resources(&connection, vec![crate::file_providers::Resource { id: "remote".into(), name: "Remote.psd".into(), updated_at: String::new() }])
            .unwrap();
        provider
    }

    #[test]
    fn provider_sections_preserve_the_framework_home_grid_and_local_files() {
        for width in [480.0, 1100.0] {
            let provider = provider();
            let mut h = home(width, Some(provider.clone()));
            assert!(h.query_by_label("Remote.psd").is_some());
            assert!(h.query_all_by_label("Local Recent Files").next().is_some());
            assert!(h.query_by_label("Local.psd").is_some());
            h.get_by_label("List").click();
            h.run_steps(3);
            assert_eq!(h.state().0.ui.chrome.home_recent_view, RecentView::List);
            assert!(h.query_by_label("Remote.psd").is_some());
            assert!(h.query_by_role_and_label(egui::accesskit::Role::ListItem, "Local.psd, fixtures").is_some());
            assert_eq!(h.state().0.ui.recent_files.as_slice(), [LOCAL_RECENT]);
            assert_eq!(provider.resources().iter().map(|resource| resource.id.as_str()).collect::<Vec<_>>(), ["remote"]);
        }
    }

    #[test]
    fn inactive_provider_and_hidden_recents_leave_provider_resources_out_of_home() {
        let h = home(1100.0, None);
        assert!(h.query_by_label("Local.psd").is_some());
        assert!(h.query_by_label("Local Recent Files").is_none());

        let provider = provider();
        provider.set_enabled(false).unwrap();
        let mut h = home(1100.0, Some(provider.clone()));
        assert!(h.query_by_label("Remote.psd").is_none());
        assert!(h.query_by_label("Local Recent Files").is_none());
        assert!(h.query_by_label("Local.psd").is_some());

        provider.set_enabled(true).unwrap();
        h.state_mut().0.session.edit_prefs(|prefs| prefs.interface.show_home_recent_files = false);
        h.run_steps(3);
        assert!(h.query_by_label("Remote.psd").is_none());
        assert!(h.query_by_label("Local.psd").is_none());
    }
}
