//! Linux desktop colour scheme, including Wayland compositors that do not report a winit theme.

#[cfg(target_os = "linux")]
const PORTAL_NAMESPACE: &str = "org.freedesktop.appearance";
#[cfg(target_os = "linux")]
const PORTAL_KEY: &str = "color-scheme";

#[cfg(target_os = "linux")]
pub(crate) fn portal_proxy() -> Option<zbus::blocking::Proxy<'static>> {
    let conn = zbus::blocking::connection::Builder::session().ok()?.method_timeout(std::time::Duration::from_secs(2)).build().ok()?;
    zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").ok()
}

#[cfg(target_os = "linux")]
fn portal_theme(proxy: &zbus::blocking::Proxy<'_>) -> Option<egui::Theme> {
    let value: zbus::zvariant::OwnedValue = proxy.call("Read", &(PORTAL_NAMESPACE, PORTAL_KEY)).ok()?;
    portal_code(value).and_then(decode).or_else(gtk_theme)
}

#[cfg(target_os = "linux")]
fn portal_code(value: zbus::zvariant::OwnedValue) -> Option<u32> {
    let mut value: zbus::zvariant::Value<'_> = value.into();
    // The Settings.Read result is a variant; some portals nest another variant inside it.
    for _ in 0..4 {
        match value {
            zbus::zvariant::Value::Value(inner) => value = *inner,
            other => return u32::try_from(other).ok(),
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn decode(code: u32) -> Option<egui::Theme> {
    match code {
        1 => Some(egui::Theme::Dark),
        2 => Some(egui::Theme::Light),
        _ => None,
    }
}

/// GNOME fallback when the portal has no preference. Read only on the appearance worker,
/// using an absolute path rather than the shell's PATH.
#[cfg(target_os = "linux")]
fn gtk_theme() -> Option<egui::Theme> {
    let exe = ["/usr/bin/gsettings", "/bin/gsettings", "/usr/local/bin/gsettings"].into_iter().find(|p| std::path::Path::new(p).is_file())?;
    let output = std::process::Command::new(exe).args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gtk_scheme(std::str::from_utf8(&output.stdout).ok()?)
}

#[cfg(target_os = "linux")]
fn parse_gtk_scheme(s: &str) -> Option<egui::Theme> {
    match s.trim().trim_matches('\'') {
        "prefer-dark" => Some(egui::Theme::Dark),
        "prefer-light" | "default" => Some(egui::Theme::Light),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn code_of(theme: Option<egui::Theme>) -> u8 {
    match theme {
        Some(egui::Theme::Dark) => 1,
        Some(egui::Theme::Light) => 2,
        None => 0,
    }
}

#[cfg(target_os = "linux")]
fn portal_startup<T>(subscribe: impl FnOnce(), read: impl FnOnce() -> T) -> T {
    subscribe();
    read()
}

/// Reads the colour scheme once, then follows the portal's `SettingChanged` signal. There is no
/// polling: the worker sleeps in the signal iterator and wakes the UI only when the value changes.
pub fn service() -> Option<photocraft_ui_egui::SystemThemeFn> {
    #[cfg(target_os = "linux")]
    {
        start(|publisher| {
            let proxy = portal_proxy();
            // Subscribe before the first read, so a change during start-up isn't lost.
            let mut signals = None;
            let initial = portal_startup(
                || signals = proxy.as_ref().and_then(|proxy| proxy.receive_signal("SettingChanged").ok()),
                || proxy.as_ref().and_then(portal_theme).or_else(gtk_theme),
            );
            publisher.publish(initial);
            let Some(signals) = signals else { return };
            for message in signals {
                let Ok((namespace, key, changed)) = message.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>() else {
                    continue;
                };
                if namespace != PORTAL_NAMESPACE || key != PORTAL_KEY {
                    continue;
                }
                publisher.publish(portal_code(changed).and_then(decode).or_else(gtk_theme));
            }
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The worker's handle on the shared value. Every change wakes the registered UI, including the
/// first read when it lands after the start-up wait below.
#[cfg(target_os = "linux")]
struct Publisher {
    value: std::sync::Arc<std::sync::atomic::AtomicU8>,
    wake: std::sync::Arc<std::sync::OnceLock<egui::Context>>,
    ready: std::sync::mpsc::Sender<()>,
}

#[cfg(target_os = "linux")]
impl Publisher {
    fn publish(&self, theme: Option<egui::Theme>) {
        let code = code_of(theme);
        if self.value.swap(code, std::sync::atomic::Ordering::Relaxed) != code
            && let Some(ctx) = self.wake.get()
        {
            ctx.request_repaint();
        }
        let _ = self.ready.send(());
    }
}

/// Runs `worker` on its own thread and waits up to 250 ms for its first value, so a fast desktop
/// answers before the first frame.
#[cfg(target_os = "linux")]
fn start(worker: impl FnOnce(&Publisher) + Send + 'static) -> Option<photocraft_ui_egui::SystemThemeFn> {
    use std::sync::{Arc, OnceLock, atomic::AtomicU8};
    let value = Arc::new(AtomicU8::new(0));
    let wake: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let (ready, wait) = std::sync::mpsc::channel();
    let publisher = Publisher { value: Arc::clone(&value), wake: Arc::clone(&wake), ready };
    std::thread::Builder::new().name("appearance-portal".into()).spawn(move || worker(&publisher)).ok()?;
    let _ = wait.recv_timeout(std::time::Duration::from_millis(250));
    Some(Box::new(move |ctx| {
        let _ = wake.set(ctx.clone());
        decode(u32::from(value.load(std::sync::atomic::Ordering::Relaxed)))
    }))
}

// ---- Desktop accent colour ----
//
// The same portal namespace carries `accent-color` on GNOME 47+ and Plasma 6+. When the portal
// has no answer, GNOME's named accent or KDE's `kdeglobals` are read once at start-up; those
// fallbacks do not update live (only the portal path repaints on change).

#[cfg(target_os = "linux")]
const ACCENT_PORTAL_KEY: &str = "accent-color";

/// GNOME's nine named accents; the empty string is the default appearance (blue).
#[cfg(target_os = "linux")]
fn gnome_named_accent(name: &str) -> Option<egui::Color32> {
    let (r, g, b) = match name.trim().trim_matches('\'') {
        "" | "blue" => (0x35, 0x84, 0xe4),
        "teal" => (0x21, 0x90, 0xa4),
        "green" => (0x3a, 0x94, 0x4a),
        "yellow" => (0xc8, 0x88, 0x00),
        "orange" => (0xed, 0x5b, 0x00),
        "red" => (0xe6, 0x2d, 0x42),
        "pink" => (0xd5, 0x61, 0x99),
        "purple" => (0x91, 0x41, 0xac),
        "slate" => (0x6f, 0x83, 0x96),
        _ => return None,
    };
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Accepts `#rrggbb`, `rrggbb` and `r,g,b` text (KDE writes the last form), plus `(f,f,f)` and
/// `(r,g,b)` tuples that some portals stringify.
#[cfg(target_os = "linux")]
fn parse_accent_text(text: &str) -> Option<egui::Color32> {
    if text.trim_start().starts_with('(')
        && let Some((r, g, b)) = parse_accent_floats_tuple(text)
    {
        // The portal's spec form is 0..1 floats; a stringified byte triple is also accepted.
        return if r <= 1.0 && g <= 1.0 && b <= 1.0 { accent_from_floats(r, g, b) } else { Some(egui::Color32::from_rgb(r as u8, g as u8, b as u8)) };
    }
    let text = text.trim().trim_start_matches('#');
    let radix = if text.contains(',') { 10 } else { 16 };
    let (r, g, b) = if text.contains(',') {
        let mut parts = text.split(',');
        (parts.next()?.trim(), parts.next()?.trim(), parts.next()?.trim())
    } else if text.len() >= 6 {
        (text.get(0..2)?, text.get(2..4)?, text.get(4..6)?)
    } else {
        return None;
    };
    Some(egui::Color32::from_rgb(u8::from_str_radix(r, radix).ok()?, u8::from_str_radix(g, radix).ok()?, u8::from_str_radix(b, radix).ok()?))
}

#[cfg(target_os = "linux")]
fn accent_from_floats(r: f64, g: f64, b: f64) -> Option<egui::Color32> {
    let channel = |v: f64| u8::try_from((v.clamp(0.0, 1.0) * 255.0).round() as i64).ok();
    Some(egui::Color32::from_rgb(channel(r)?, channel(g)?, channel(b)?))
}

#[cfg(target_os = "linux")]
fn parse_accent_floats_tuple(text: &str) -> Option<(f64, f64, f64)> {
    let text = text.trim_start_matches('(').trim_end_matches(')');
    let (r, rest) = text.split_once(',')?;
    let (g, b) = rest.split_once(',')?;
    Some((r.trim().parse().ok()?, g.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// The accent from the portal's `Read` result, unwrapping nested variants. The spec value is an
/// `(ddd)` of 0..1 floats; a text form is accepted for portals that predate it.
#[cfg(target_os = "linux")]
fn portal_accent(proxy: &zbus::blocking::Proxy<'_>) -> Option<egui::Color32> {
    let value: zbus::zvariant::OwnedValue = proxy.call("Read", &(PORTAL_NAMESPACE, ACCENT_PORTAL_KEY)).ok()?;
    decode_accent_value(value)
}

#[cfg(target_os = "linux")]
fn decode_accent_value(value: zbus::zvariant::OwnedValue) -> Option<egui::Color32> {
    let mut value: zbus::zvariant::Value<'_> = value.into();
    for _ in 0..4 {
        match value {
            zbus::zvariant::Value::Value(inner) => value = *inner,
            other => {
                if let Ok((r, g, b)) = <(f64, f64, f64)>::try_from(other.clone()) {
                    return accent_from_floats(r, g, b);
                }
                if let Ok(text) = <&str>::try_from(&other) {
                    return parse_accent_text(text);
                }
                if let Ok(text) = <String>::try_from(other) {
                    return parse_accent_text(&text);
                }
                return None;
            }
        }
    }
    None
}

/// GNOME fallback: the named accent from `gsettings`, read only on the accent worker — and only
/// in a GNOME session, where an unset accent legitimately means the default blue. `gsettings`
/// ships with GLib everywhere (Kubuntu included), and outside GNOME it answers with that same
/// empty string, which would hide the desktop's real accent behind GNOME's blue.
#[cfg(target_os = "linux")]
fn gnome_accent() -> Option<egui::Color32> {
    if !gnome_session(&|k| std::env::var(k).ok()) {
        return None;
    }
    gnome_named_accent(&gsettings_get("org.gnome.desktop.interface", "accent-color")?)
}

/// Whether the session reports itself as GNOME.
#[cfg(target_os = "linux")]
fn gnome_session(var: &dyn Fn(&str) -> Option<String>) -> bool {
    var("XDG_CURRENT_DESKTOP").is_some_and(|d| d.to_uppercase().contains("GNOME"))
}

#[cfg(target_os = "linux")]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let exe = ["/usr/bin/gsettings", "/bin/gsettings", "/usr/local/bin/gsettings"].into_iter().find(|p| std::path::Path::new(p).is_file())?;
    let output = std::process::Command::new(exe).args(["get", schema, key]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(std::str::from_utf8(&output.stdout).ok()?.trim().to_owned())
}

/// KDE fallback: `AccentColor` from `kdeglobals` (`General`), falling back to the scheme's
/// selection background. Read once at start-up with the environment injected for tests.
#[cfg(target_os = "linux")]
fn kde_accent(var: &dyn Fn(&str) -> Option<String>) -> Option<egui::Color32> {
    let config = var("XDG_CONFIG_HOME")
        .filter(|d| std::path::Path::new(d).is_absolute())
        .map(std::path::PathBuf::from)
        .or_else(|| var("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    let text = crate::caption_style::read_text(&config.join("kdeglobals")).unwrap_or_default();
    parse_accent_text(
        &crate::caption_style::ini_value(&text, "General", "AccentColor")
            .or_else(|| crate::caption_style::ini_value(&text, "Colors:View", "SelectionBackground"))?,
    )
}

/// pywal fallback: `color4` from `~/.cache/wal/colors.json` (the palette adapters' accent).
#[cfg(target_os = "linux")]
fn wal_accent(var: &dyn Fn(&str) -> Option<String>) -> Option<egui::Color32> {
    let home = var("HOME").filter(|d| std::path::Path::new(d).is_absolute())?;
    let path = std::path::Path::new(&home).join(".cache/wal/colors.json");
    let json: serde_json::Value = serde_json::from_str(&crate::caption_style::read_text(&path)?).ok()?;
    parse_accent_text(json.get("color4")?.as_str()?)
}

/// Reads the desktop accent once, then follows the portal's `accent-color` changes the same way
/// [`service`] follows the colour scheme. While the portal answers, its accent is authoritative;
/// while it does not, the fallback settings files are polled, so an accent picked in the
/// desktop's settings later still reaches the app.
pub fn accent_service() -> Option<photocraft_ui_egui::AccentFn> {
    #[cfg(target_os = "linux")]
    {
        accent_start(|publisher| {
            let proxy = portal_proxy();
            // Subscribe before the first read, so a change during start-up isn't lost.
            let mut signals = None;
            let via_portal = portal_startup(
                || signals = proxy.as_ref().and_then(|proxy| proxy.receive_signal("SettingChanged").ok()),
                || proxy.as_ref().and_then(portal_accent),
            );
            let initial = via_portal.or_else(fallback_accent);
            if let Some(c) = initial {
                log::info!("desktop accent #{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
            }
            publisher.publish(initial);
            // The portal signals accent changes on GNOME 47+ and Plasma; where it has no answer
            // (older portals, or accents read from files), the fallback files are polled instead.
            let portal_authoritative = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(via_portal.is_some()));
            let mut files_watched = false;
            if via_portal.is_none() {
                publisher.clone().watch_files(std::sync::Arc::clone(&portal_authoritative));
                files_watched = true;
            }
            let Some(signals) = signals else { return };
            for message in signals {
                let Ok((namespace, key, changed)) = message.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>() else {
                    continue;
                };
                if namespace != PORTAL_NAMESPACE || key != ACCENT_PORTAL_KEY {
                    continue;
                }
                let decoded = decode_accent_value(changed);
                let (portal_now, watch_now) = after_signal(decoded.is_some(), files_watched);
                if watch_now && !files_watched {
                    publisher.clone().watch_files(std::sync::Arc::clone(&portal_authoritative));
                }
                files_watched = watch_now;
                portal_authoritative.store(portal_now, std::sync::atomic::Ordering::Relaxed);
                publisher.publish(decoded.or_else(fallback_accent));
            }
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// What a portal accent signal leaves behind: whether the portal still owns the accent, and
/// whether the fallback file poller must be running. A portal that stops answering hands the
/// accent to the fallback files from then on; one that answers again takes it back.
#[cfg(target_os = "linux")]
fn after_signal(portal_answered: bool, files_watched: bool) -> (bool, bool) {
    (portal_answered, files_watched || !portal_answered)
}

/// The accent from the desktop's settings files, when the portal has none: GNOME's named accent,
/// KDE's `kdeglobals`, then pywal's palette. Called on the worker threads only.
#[cfg(target_os = "linux")]
fn fallback_accent() -> Option<egui::Color32> {
    let env = |k: &str| std::env::var(k).ok();
    gnome_accent().or_else(|| kde_accent(&env)).or_else(|| wal_accent(&env))
}

/// The worker's handle on the shared accent. Packed as `0x0100_0000 | rgb`, with 0 meaning
/// "no accent", so even a pure black accent counts as present.
#[cfg(target_os = "linux")]
#[derive(Clone)]
struct AccentPublisher {
    value: std::sync::Arc<std::sync::atomic::AtomicU32>,
    wake: std::sync::Arc<std::sync::OnceLock<egui::Context>>,
    ready: std::sync::mpsc::Sender<()>,
}

#[cfg(target_os = "linux")]
impl AccentPublisher {
    fn publish(&self, accent: Option<egui::Color32>) {
        let packed = accent.map_or(0, |c| 0x0100_0000 | (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b()));
        if self.value.swap(packed, std::sync::atomic::Ordering::Relaxed) != packed
            && let Some(ctx) = self.wake.get()
        {
            ctx.request_repaint();
        }
        let _ = self.ready.send(());
    }

    /// Follow the settings files the portal does not signal (`kdeglobals`, pywal), polling every
    /// couple of seconds; `publish` only wakes the UI on a real change. While
    /// `portal_authoritative` is set, the poller stays quiet so the portal's accent wins.
    fn watch_files(self, portal_authoritative: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let _ = std::thread::Builder::new().name("accent-files".into()).spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if !portal_authoritative.load(std::sync::atomic::Ordering::Relaxed) {
                    self.publish(fallback_accent());
                }
            }
        });
    }
}

/// Runs `worker` on its own thread and waits up to 250 ms for its first value, so a fast desktop
/// answers before the first frame.
#[cfg(target_os = "linux")]
fn accent_start(worker: impl FnOnce(&AccentPublisher) + Send + 'static) -> Option<photocraft_ui_egui::AccentFn> {
    use std::sync::{Arc, OnceLock, atomic::AtomicU32};
    let value = Arc::new(AtomicU32::new(0));
    let wake: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let (ready, wait) = std::sync::mpsc::channel();
    let publisher = AccentPublisher { value: Arc::clone(&value), wake: Arc::clone(&wake), ready };
    std::thread::Builder::new().name("accent-portal".into()).spawn(move || worker(&publisher)).ok()?;
    let _ = wait.recv_timeout(std::time::Duration::from_millis(250));
    Some(Box::new(move |ctx| {
        let _ = wake.set(ctx.clone());
        unpack_accent(value.load(std::sync::atomic::Ordering::Relaxed))
    }))
}

#[cfg(target_os = "linux")]
fn unpack_accent(packed: u32) -> Option<egui::Color32> {
    if packed & 0x0100_0000 == 0 {
        return None;
    }
    Some(egui::Color32::from_rgb(((packed >> 16) & 0xff) as u8, ((packed >> 8) & 0xff) as u8, (packed & 0xff) as u8))
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn a_portal_that_stops_answering_hands_the_accent_to_the_fallback_files() {
        use super::after_signal;
        // A portal that keeps answering stays authoritative and never needs the file poller.
        assert_eq!(after_signal(true, false), (true, false));
        assert_eq!(after_signal(true, true), (true, true));
        // A signal without a portal accent hands the accent to the fallback files — once,
        // even if the initial portal read had an accent and never started the poller.
        assert_eq!(after_signal(false, false), (false, true));
        assert_eq!(after_signal(false, true), (false, true));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_keeps_changes_emitted_during_initial_read() {
        use std::cell::Cell;

        for (initial_value, changed_value) in [(2, 1), (1, 2), (2, 2)] {
            let subscribed = Cell::new(false);
            let mut portal_value = initial_value;
            let mut pending_signal = None;
            let initial = super::portal_startup(
                || subscribed.set(true),
                || {
                    let initial = portal_value;
                    portal_value = changed_value;
                    if subscribed.get() && portal_value != initial {
                        pending_signal = Some(portal_value);
                    }
                    initial
                },
            );
            let cached = pending_signal.unwrap_or(initial);
            assert_eq!(cached, changed_value, "portal changed during startup but cached {cached}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_scheme_values() {
        assert_eq!(super::decode(0), None);
        assert_eq!(super::decode(1), Some(egui::Theme::Dark));
        assert_eq!(super::decode(2), Some(egui::Theme::Light));
        assert_eq!(super::decode(9), None);
        let nested = zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::U32(2)))));
        assert_eq!(super::portal_code(zbus::zvariant::OwnedValue::try_from(nested).unwrap()), Some(2));
        assert_eq!(super::parse_gtk_scheme("'prefer-light'\n"), Some(egui::Theme::Light));
        assert_eq!(super::parse_gtk_scheme("'prefer-dark'\n"), Some(egui::Theme::Dark));
        // GNOME's "default" appearance is light, even though the portal reports no preference.
        assert_eq!(super::parse_gtk_scheme("'default'\n"), Some(egui::Theme::Light));
        assert_eq!(super::parse_gtk_scheme("default"), Some(egui::Theme::Light));
        assert_eq!(super::parse_gtk_scheme("'not-a-scheme'"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn accent_text_forms() {
        assert_eq!(super::parse_accent_text("#3584e4"), Some(egui::Color32::from_rgb(0x35, 0x84, 0xe4)));
        assert_eq!(super::parse_accent_text("3584e4"), Some(egui::Color32::from_rgb(0x35, 0x84, 0xe4)));
        // KDE's byte triple must not be mistaken for the portal's 0..1 floats.
        assert_eq!(super::parse_accent_text("237,91,0"), Some(egui::Color32::from_rgb(237, 91, 0)));
        assert_eq!(super::parse_accent_text("(0.5, 0.25, 0.0)"), Some(egui::Color32::from_rgb(128, 64, 0)));
        assert_eq!(super::parse_accent_text("(237, 91, 0)"), Some(egui::Color32::from_rgb(237, 91, 0)));
        assert_eq!(super::parse_accent_text("#358"), None);
        assert_eq!(super::parse_accent_text("nonsense"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gnome_named_accents() {
        let blue = egui::Color32::from_rgb(0x35, 0x84, 0xe4);
        assert_eq!(super::gnome_named_accent(""), Some(blue), "the default appearance is blue");
        assert_eq!(super::gnome_named_accent("'orange'"), Some(egui::Color32::from_rgb(0xed, 0x5b, 0x00)));
        assert_eq!(super::gnome_named_accent("magenta"), None);
    }

    /// The GNOME gsettings fallback applies only in a GNOME session: elsewhere gsettings (which
    /// ships with GLib everywhere) answers with the same empty "default" string and would hide
    /// the desktop's real accent behind GNOME's blue.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_gnome_fallback_only_applies_in_a_gnome_session() {
        let env = &[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")];
        assert!(super::gnome_session(&|k| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())));
        let kde = &[("XDG_CURRENT_DESKTOP", "KDE")];
        assert!(!super::gnome_session(&|k| kde.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())));
        assert!(!super::gnome_session(&|_| None), "unset: not a GNOME session");
    }

    /// The KDE fallback reads `AccentColor` (byte triple or hex) with the scheme's selection
    /// background behind it, through an injected config directory.
    #[cfg(target_os = "linux")]
    #[test]
    fn kde_accent_reads_kdeglobals() {
        let dir = std::env::temp_dir().join(format!("photocraft-kde-accent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let var = |name: &str| (name == "XDG_CONFIG_HOME").then(|| dir.to_string_lossy().into_owned());
        std::fs::write(dir.join("kdeglobals"), "[General]\nAccentColor=237,91,0\n").unwrap();
        assert_eq!(super::kde_accent(&var), Some(egui::Color32::from_rgb(237, 91, 0)));
        std::fs::write(dir.join("kdeglobals"), "[General]\nKWinPaletteUseDark=true\n[Colors:View]\nSelectionBackground=#3584e4\n").unwrap();
        assert_eq!(super::kde_accent(&var), Some(egui::Color32::from_rgb(0x35, 0x84, 0xe4)));
        std::fs::remove_dir_all(&dir).ok();
        let no_home = |name: &str| (name == "HOME").then(|| "/photocraft-no-home".to_string());
        assert_eq!(super::kde_accent(&no_home), None, "no kdeglobals, no accent");
    }

    /// The "present" flag keeps a pure black accent from reading as "no accent".
    #[cfg(target_os = "linux")]
    #[test]
    fn accent_packing_round_trips_black() {
        assert_eq!(super::unpack_accent(0), None);
        let black = egui::Color32::BLACK;
        let packed = 0x0100_0000 | (u32::from(black.r()) << 16) | (u32::from(black.g()) << 8) | u32::from(black.b());
        assert_eq!(super::unpack_accent(packed), Some(black));
        assert_eq!(super::unpack_accent(0x0100_0000 | 0xed_5b_00), Some(egui::Color32::from_rgb(0xed, 0x5b, 0x00)));
    }

    /// The pywal fallback reads `color4` from the cache file, through an injected home.
    #[cfg(target_os = "linux")]
    #[test]
    fn wal_accent_reads_color4() {
        let home = std::env::temp_dir().join(format!("photocraft-wal-accent-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".cache/wal")).unwrap();
        let var = |name: &str| (name == "HOME").then(|| home.to_string_lossy().into_owned());
        std::fs::write(home.join(".cache/wal/colors.json"), r##"{"color0": "#000000", "color4": "#7aa2f7", "special": {"background": "#1a1b26"}}"##).unwrap();
        assert_eq!(super::wal_accent(&var), Some(egui::Color32::from_rgb(0x7a, 0xa2, 0xf7)));
        let no_home = |name: &str| (name == "HOME").then(|| "/photocraft-no-home".to_string());
        assert_eq!(super::wal_accent(&no_home), None, "no palette, no accent");
        std::fs::remove_dir_all(&home).ok();
    }

    /// #2798: a first read that misses the start-up wait still wakes the idle UI.
    /// #2798: a first read that misses the start-up wait still wakes the idle UI.
    #[cfg(target_os = "linux")]
    #[test]
    fn late_initial_read_requests_repaint() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let reader = super::start(move |publisher| {
            let _ = gate.recv();
            publisher.publish(Some(egui::Theme::Light));
            let _ = done.send(());
        })
        .expect("worker thread");
        let ctx = egui::Context::default();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(reader(&ctx), None, "start-up wait elapsed without an answer");
        release.send(()).unwrap();
        finished.recv().unwrap();
        assert_eq!(repaints.load(Ordering::Relaxed), 1);
        assert_eq!(reader(&ctx), Some(egui::Theme::Light));
    }
}
