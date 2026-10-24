//! Where Linux and BSD desktops keep their animation setting, and how to read its value.
//!
//! Plain functions over text, so the tests run on every platform. Each one answers whether
//! animations are on, or `None` when the value doesn't parse (the next source is asked then).

/// GSettings' and the portal's `true`/`false`, or GTK's `1`/`0`.
pub(super) fn from_enabled(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// KDE's animation speed factor: 0 turns animations off.
pub(super) fn from_duration_factor(value: &str) -> Option<bool> {
    value.trim().parse::<f64>().ok().filter(|factor| factor.is_finite()).map(|factor| factor > 0.0)
}

/// The settings portal's `reduced-motion`: 1 asks for less motion; 0 and any value added later
/// mean no preference.
#[cfg(any(test, target_os = "linux"))]
pub(super) fn from_reduced_motion(value: &str) -> Option<bool> {
    value.trim().parse::<u32>().ok().map(|code| code != 1)
}

/// The desktop's setting when there is no settings portal to ask. `desktop` is
/// `XDG_CURRENT_DESKTOP`, `config_file` reads a file in the user's config directory and
/// `gsettings` runs `gsettings get <schema> <key>`.
pub(super) fn without_portal(desktop: &str, config_file: &dyn Fn(&str) -> Option<String>, gsettings: &dyn Fn(&str, &str) -> Option<String>) -> Option<bool> {
    let running = |name: &str| desktop.split(':').any(|part| part.eq_ignore_ascii_case(name));
    let in_file = |file: &str, section: &str, key: &str, parse: fn(&str) -> Option<bool>| {
        let text = config_file(file)?;
        parse(ini_value(&text, section, key)?)
    };

    // KDE Plasma keeps it in its own file. A kdeglobals left over from another desktop doesn't count.
    if running("KDE")
        && let Some(animations) = in_file("kdeglobals", "KDE", "AnimationDurationFactor", from_duration_factor)
    {
        return Some(animations);
    }
    // GNOME and the desktops built on it, or MATE's copy of the same key.
    let schema = if running("MATE") { "org.mate.interface" } else { "org.gnome.desktop.interface" };
    let gnome = gsettings(schema, "enable-animations").and_then(|out| from_enabled(&out));
    if gnome == Some(false) {
        return gnome;
    }
    // `true` may only be the schema's default on a desktop that doesn't use it, so GTK's own
    // file comes first: it is what a bare window manager's user writes, and what KDE keeps in step.
    let gtk = ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"].iter().find_map(|file| in_file(file, "Settings", "gtk-enable-animations", from_enabled));
    gtk.or(gnome)
}

/// The last value of `key` in `[section]` of an INI-style file (KDE's config, GTK's settings).
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_section = name == section;
        } else if in_section && let Some((name, value)) = line.split_once('=') {
            // KDE marks an entry the user may not change as `Key[$i]=value`.
            let name = name.split_once('[').map_or(name, |(name, _)| name);
            if name.trim() == key {
                found = Some(value.trim());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each reader turns its own kind of value into on or off, and anything else into `None`.
    #[test]
    fn each_kind_of_value_reads_as_on_or_off() {
        for (value, animations) in
            [("true\n", Some(true)), ("false\n", Some(false)), ("1", Some(true)), ("0", Some(false)), ("FALSE", Some(false)), ("", None), ("maybe", None)]
        {
            assert_eq!(from_enabled(value), animations, "{value:?}");
        }
        for (value, animations) in [
            ("0", Some(false)),
            ("0.0", Some(false)),
            ("0.5", Some(true)),
            ("1", Some(true)),
            ("-1", Some(false)),
            ("", None),
            ("fast", None),
            ("NaN", None),
            ("inf", None),
        ] {
            assert_eq!(from_duration_factor(value), animations, "{value:?}");
        }
        for (value, animations) in [("0", Some(true)), ("1", Some(false)), ("2", Some(true)), ("true", None), ("-1", None), ("", None)] {
            assert_eq!(from_reduced_motion(value), animations, "{value:?}");
        }
    }

    /// A key is read from its own section only, the last entry wins, and junk reads as nothing.
    #[test]
    fn ini_values_come_from_their_own_section() {
        let kde = "[General]\nAnimationDurationFactor=3\n\n[KDE]\nSingleClick=false\n AnimationDurationFactor = 0 \n[KDE][Sub]\nAnimationDurationFactor=7\n";
        assert_eq!(ini_value(kde, "KDE", "AnimationDurationFactor"), Some("0"));
        assert_eq!(ini_value(kde, "KDE", "Missing"), None);
        assert_eq!(ini_value("[KDE]\nAnimationDurationFactor[$i]=0.5\n", "KDE", "AnimationDurationFactor"), Some("0.5"), "an immutable entry");
        let gtk = "[Settings]\ngtk-enable-animations=1\ngtk-enable-animations=0\n";
        assert_eq!(ini_value(gtk, "Settings", "gtk-enable-animations"), Some("0"), "the last one wins");
        // Hostile text: no section, no `=`, stray brackets, non-ASCII.
        for junk in ["", "AnimationDurationFactor=0", "[KDE", "[KDE]\n=\n[]\n]]][[[\n", "[KDE]\nклавиша=значение\n\u{0}\u{feff}"] {
            assert_eq!(ini_value(junk, "KDE", "AnimationDurationFactor"), None, "{junk:?}");
        }
    }

    /// Without the portal: KDE's file only on KDE, GNOME's or MATE's key, then GTK's file.
    #[test]
    fn without_the_portal_each_desktop_is_read_where_it_keeps_the_setting() {
        let files = |kdeglobals: Option<&'static str>, gtk3: Option<&'static str>| {
            move |file: &str| match file {
                "kdeglobals" => kdeglobals.map(String::from),
                "gtk-3.0/settings.ini" => gtk3.map(String::from),
                _ => None,
            }
        };
        let no_gsettings = |_: &str, _: &str| -> Option<String> { None };
        let gnome = |on: &'static str| {
            move |schema: &str, key: &str| (schema == "org.gnome.desktop.interface" && key == "enable-animations").then(|| format!("{on}\n"))
        };
        let kde_off = Some("[KDE]\nAnimationDurationFactor=0\n");
        let gtk_off = Some("[Settings]\ngtk-enable-animations=false\n");

        // KDE Plasma: its own file, only while Plasma is the desktop.
        assert_eq!(without_portal("KDE", &files(kde_off, None), &no_gsettings), Some(false));
        assert_eq!(without_portal("KDE", &files(Some("[KDE]\nAnimationDurationFactor=0.5\n"), gtk_off), &no_gsettings), Some(true));
        assert_eq!(without_portal("ubuntu:GNOME", &files(kde_off, None), &gnome("true")), Some(true), "a leftover kdeglobals is not GNOME's setting");
        // Plasma without the entry (animations at their default speed) falls through to GTK's file.
        assert_eq!(without_portal("KDE", &files(Some("[KDE]\nSingleClick=false\n"), gtk_off), &no_gsettings), Some(false));
        // GNOME and the desktops that share its schema.
        assert_eq!(without_portal("GNOME", &files(None, None), &gnome("false")), Some(false));
        assert_eq!(without_portal("GNOME", &files(None, None), &gnome("true")), Some(true));
        // MATE keeps the same key in its own schema.
        let mate = |schema: &str, _: &str| (schema == "org.mate.interface").then(|| "false\n".to_string());
        assert_eq!(without_portal("MATE", &files(None, None), &mate), Some(false));
        assert_eq!(without_portal("GNOME", &files(None, None), &mate), None);
        // A bare window manager: GTK's settings file, which also outranks a schema default.
        assert_eq!(without_portal("sway", &files(None, gtk_off), &no_gsettings), Some(false));
        assert_eq!(without_portal("", &files(None, gtk_off), &gnome("true")), Some(false));
        assert_eq!(without_portal("", &files(None, Some("[Settings]\ngtk-enable-animations=1\n")), &no_gsettings), Some(true));
        // Nothing to read, or nothing readable: unknown, never a panic.
        assert_eq!(without_portal("", &files(None, None), &no_gsettings), None);
        let unreadable = files(Some("\u{0}[[[=]]]"), Some("=\n[Settings]\ngtk-enable-animations=perhaps"));
        assert_eq!(without_portal("KDE:\u{0}::", &unreadable, &|_, _| Some("\u{fffd}".into())), None);
    }
}
