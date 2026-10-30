//! DeviantArt login and request error handling.

use super::{PhotocraftApp, save_settings, settings_from_json, show_result};

/// DeviantArt rotates the refresh token on every use. The keychain has the new one; the
/// dialog still holds the one it sent. Copy the keychain value before the next save.
pub(super) fn remember_rotated_login(app: &mut PhotocraftApp) {
    // Disconnected (or never connected): do not bring a login back from the keychain.
    if app.deviantart.refresh_token.is_empty() {
        return;
    }
    let token = app
        .services
        .deviantart_current_refresh
        .as_ref()
        .map(|current| current())
        .filter(|token| !token.is_empty())
        .or_else(|| app.services.load_deviantart.as_mut().and_then(|load| load()).map(|text| settings_from_json(&text).refresh_token));
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        app.deviantart.refresh_token = token;
        if let Err(error) = save_settings(app) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

pub(super) fn login_was_rejected(error: &str) -> bool {
    error.starts_with("refresh-grant-rejected: ") || error == "The DeviantArt login expired. Connect again."
}

pub(super) fn report_deviantart_error(app: &mut PhotocraftApp, error: &str, adopt_rotation: bool) {
    if login_was_rejected(error) {
        app.deviantart_pending.clear();
        app.deviantart.refresh_token.clear();
        let _ = save_settings(app);
    } else if adopt_rotation {
        remember_rotated_login(app);
    }
    let message = localize_error(error);
    app.ui.status = message.clone();
    app.ui.status_error = true;
    show_result(app, message, None);
}

pub(super) fn disconnect(app: &mut PhotocraftApp) {
    app.deviantart_pending.clear();
    app.deviantart.refresh_token.clear();
    match save_settings(app) {
        Ok(()) => {
            app.ui.status = tl!("DeviantArt was disconnected.").to_string();
            app.ui.status_error = false;
        }
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}

fn localize_error(error: &str) -> String {
    if login_was_rejected(error) {
        return tl!("The DeviantArt login expired. Connect again.").to_string();
    }
    let source = match error {
        "timeout" => "Waiting for DeviantArt timed out.",
        "denied" => "DeviantArt did not grant access.",
        "listen" => "Could not listen for the DeviantArt login.",
        "no-code" => "DeviantArt did not return a code.",
        "state" => "DeviantArt returned an unexpected login.",
        "desktop" => "DeviantArt submitting needs the desktop app.",
        other => other,
    };
    crate::i18n::t(source).to_string()
}
