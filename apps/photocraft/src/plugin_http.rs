//! HTTPS for plug-in installs (catalogs and GitHub releases, `photocraft_engine::plugin`) over the
//! client PhotoCraft checks for updates with (craft-update: proxies, TLS, GitHub's API limits).

use photocraft_engine::plugin::craft_plugin::Fetch;

/// craft-update's client as the plug-in installer's [`Fetch`].
struct UpdateHttp(craft_update::Http);

impl Fetch for UpdateHttp {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        use craft_update::http::Fetched;
        if !url.starts_with("https://api.github.com/") {
            return self.0.get_bytes(url, max);
        }
        match self.0.get_api(url, None)? {
            Fetched::Body { text, .. } if text.len() > max => Err(format!("GitHub's answer is larger than {max} bytes")),
            Fetched::Body { text, .. } => Ok(Some(text.into_bytes())),
            Fetched::NotFound => Ok(None),
            Fetched::RateLimited { .. } => Err("GitHub's API limit for this hour is used up: try again later".into()),
            Fetched::NotModified => Err("GitHub answered 304 Not Modified to a request without a cache".into()),
        }
    }
}

/// Keep plug-ins in `<settings dir>/Plug-ins` (reading the ones installed there) and install over
/// HTTPS. Without a settings folder they are kept for the session only.
pub fn attach(session: &mut photocraft_engine::Session, settings_dir: Option<&std::path::Path>) {
    if let Some(dir) = settings_dir {
        session.plugins.open_store(dir);
    }
    match craft_update::Http::new("PhotoCraft/Plug-ins") {
        Ok(http) => session.plugins.fetch = Some(Box::new(UpdateHttp(http))),
        Err(e) => log::warn!("plug-in downloads are unavailable: {e}"),
    }
}
