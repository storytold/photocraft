//! Image › Generative Expand's shell side: the provider's lifetime and the privacy notice.
//!
//! Preferences › AI Integrations holds three settings (`Integrations::generative_*`: on/off,
//! the helper's address, the privacy acceptance). While all three are set, the app's factory
//! ([`crate::Services::generative_factory`]) builds a provider for the address and the session
//! carries it; otherwise there is none, the engine command reports "not configured" and the Image
//! menu shows no item (upstream issue #41: off by default, no placeholder menus, privacy text
//! before the first request). [`sync`] runs once per frame and only acts when the address changed.

use egui::RichText;
use serde_json::{Map, Value};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// What the user accepts in Preferences before Generative Expand can send anything; `{address}`
/// is the helper address they typed.
const PRIVACY_NOTICE: &str = "Privacy: Generative Expand sends a copy of the visible image, plus a mask of the area to fill, \
to the helper program at {address}, and places the answer on a new layer. Nothing is sent until you run Image › Generative Expand. \
With IOPaint running on this computer (the default address), the pixels never leave it.";

/// The privacy notice for `address`, in the current language.
pub fn privacy_notice(address: &str) -> String {
    crate::i18n::fmt(tl!(PRIVACY_NOTICE), &[("address", address)])
}

/// The message a menu or command gets while no provider is configured.
pub fn not_configured() -> String {
    tl!("Generative Expand is off. Turn it on under Edit › Preferences › AI Integrations…, enter the helper address and accept the privacy notice.").to_string()
}

/// Preferences › AI Integrations: the notice under the settings grid, with the address being
/// edited (`obj` is the section's working values).
pub(crate) fn privacy_rows(ui: &mut egui::Ui, obj: &Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let address = obj.get("generativeEndpoint").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()).unwrap_or(photocraft_ml::DEFAULT_ENDPOINT);
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(privacy_notice(address)).color(t.text_dim)).wrap());
    ui.add(
        egui::Label::new(RichText::new(tl!("The Generative Expand item appears in the Image menu once all three settings are set.")).color(t.text_faint))
            .wrap(),
    );
}

/// Keep the session's provider in step with Preferences › AI Integrations. Called once per
/// frame; it does nothing while the wanted address is the one the provider was built for.
pub fn sync(app: &mut PhotocraftApp) {
    let wanted = app.session.prefs().integrations.generative_endpoint().map(str::to_owned);
    // Nothing to do while the address is the one the provider was built for (or refused for:
    // a refused address is not retried every frame), unless a provider exists that the
    // preferences no longer justify.
    let stale = wanted.is_none() && app.session.generative.is_some();
    if wanted == app.generative_endpoint && !stale {
        return;
    }
    // A generation still running was started under the old address or consent: stop it
    // before the provider goes (the job's own cancellation path; the document stays unchanged).
    let running: Vec<_> = app.session.jobs().into_iter().filter(|j| j.state == "running" && j.command == "image.generativeExpand").map(|j| j.id).collect();
    for id in running {
        app.session.cancel_job(id);
    }
    app.session.generative = match (&wanted, app.services.generative_factory.as_ref()) {
        (Some(endpoint), Some(make)) => match make(endpoint) {
            Ok(provider) => Some(provider),
            Err(e) => {
                log::warn!("generative provider for {endpoint}: {e}");
                app.ui.status = crate::i18n::fmt(tl!("Generative Expand is unavailable: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
                None
            }
        },
        // Turned off, or a build (web, tests) that has no provider to offer.
        _ => None,
    };
    app.generative_endpoint = wanted;
    // The Image menu gains or loses its item.
    app.menu_cache = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use photocraft_engine::Session;
    use photocraft_ml::{FakeTransport, GenerativeProvider, IoPaintProvider};
    use std::sync::Arc;

    /// An app whose factory accepts `http://` addresses and refuses the rest.
    fn app() -> PhotocraftApp {
        let services = Services {
            generative_factory: Some(Box::new(|endpoint: &str| {
                if endpoint.starts_with("http://") {
                    Ok(Arc::new(IoPaintProvider::new(endpoint, Box::new(FakeTransport::new()))) as Arc<dyn GenerativeProvider>)
                } else {
                    Err(format!("unsupported address {endpoint}"))
                }
            })),
            ..Default::default()
        };
        PhotocraftApp::new(Session::new(), services)
    }

    fn has_menu_item(app: &PhotocraftApp) -> bool {
        crate::menus::menu_items(app).iter().any(|i| i.id == "image.generativeExpand" && i.path == ["Image"])
    }

    #[test]
    fn provider_exists_only_when_enabled_accepted_and_addressed() {
        let mut app = app();
        sync(&mut app);
        assert!(app.session.generative.is_none(), "off by default");
        assert!(!has_menu_item(&app), "no placeholder menu item");

        app.session.edit_prefs(|p| p.integrations.generative_enabled = true);
        sync(&mut app);
        assert!(app.session.generative.is_none(), "enabled but privacy not accepted");

        app.session.edit_prefs(|p| p.integrations.generative_privacy_accepted = true);
        sync(&mut app);
        let provider = app.session.generative.clone().expect("all three set");
        assert_eq!(provider.info().destination, "a helper program at http://127.0.0.1:8080");
        assert!(has_menu_item(&app));

        app.session.edit_prefs(|p| p.integrations.generative_endpoint = "  ".into());
        sync(&mut app);
        assert!(app.session.generative.is_none(), "an empty address is no provider");

        app.session.edit_prefs(|p| p.integrations.generative_endpoint = "http://127.0.0.1:9090/".into());
        sync(&mut app);
        let provider = app.session.generative.clone().expect("new address");
        assert_eq!(provider.info().destination, "a helper program at http://127.0.0.1:9090");

        app.session.edit_prefs(|p| p.integrations.generative_enabled = false);
        sync(&mut app);
        assert!(app.session.generative.is_none(), "turned off again");
        assert!(!has_menu_item(&app));
    }

    #[test]
    fn menu_item_sits_after_canvas_size() {
        let mut app = app();
        app.session.edit_prefs(|p| {
            p.integrations.generative_enabled = true;
            p.integrations.generative_privacy_accepted = true;
        });
        sync(&mut app);
        let items = crate::menus::menu_items(&app);
        let canvas = items.iter().position(|i| i.id == "image.canvasSize").expect("Canvas Size… is in the catalogue");
        assert_eq!(items[canvas + 1].id, "image.generativeExpand");
        assert_eq!(items[canvas + 1].label, "Generative Expand…");
    }

    #[test]
    fn a_refused_address_leaves_no_provider_and_says_why() {
        let mut app = app();
        app.session.edit_prefs(|p| {
            p.integrations.generative_enabled = true;
            p.integrations.generative_privacy_accepted = true;
            p.integrations.generative_endpoint = "https://example.invalid".into();
        });
        sync(&mut app);
        assert!(app.session.generative.is_none());
        assert!(app.ui.status_error);
        assert!(app.ui.status.contains("unsupported address https://example.invalid"), "{}", app.ui.status);
        // The same address is not retried every frame.
        app.ui.status.clear();
        sync(&mut app);
        assert!(app.ui.status.is_empty());
    }

    #[test]
    fn a_provider_the_preferences_no_longer_justify_is_dropped() {
        let mut app = app();
        sync(&mut app);
        // Installed behind sync's back (preferences off, nothing cached): dropped on the next frame.
        app.session.generative = Some(Arc::new(IoPaintProvider::new("http://127.0.0.1:1", Box::new(FakeTransport::new()))));
        sync(&mut app);
        assert!(app.session.generative.is_none());
        assert!(!has_menu_item(&app));
    }

    #[test]
    fn without_a_factory_the_feature_is_absent() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.edit_prefs(|p| {
            p.integrations.generative_enabled = true;
            p.integrations.generative_privacy_accepted = true;
        });
        sync(&mut app);
        assert!(app.session.generative.is_none());
        assert!(!has_menu_item(&app));
    }

    #[test]
    fn privacy_notice_names_the_address() {
        let notice = privacy_notice("http://127.0.0.1:8080");
        assert!(notice.contains("http://127.0.0.1:8080"), "{notice}");
        assert!(!notice.contains("{address}"));
    }
}
