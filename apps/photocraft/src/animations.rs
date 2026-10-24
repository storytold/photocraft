//! The operating system's animation setting, for Preferences › Interface › Animation Effects ›
//! Use System Setting (`Services::system_animations`). `None` means the platform gave no answer;
//! the app then keeps animations on.
//!
//! | Platform | Read from |
//! |---|---|
//! | Windows | Settings › Accessibility › Visual effects › Animation effects (`UISettings.AnimationsEnabled`) |
//! | macOS | System Settings › Accessibility › Display › Reduce motion |
//! | Linux | The settings portal, followed as it changes; it also works inside a Flatpak |
//! | Linux without the portal, the BSDs | The desktop's own files and `gsettings` (`freedesktop.rs`), read again when the window gets the focus back |
//!
//! No `unsafe`: every OS call goes through a safe binding.

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
mod freedesktop;

pub use platform::service;

/// Windows and macOS answer at once, so the setting is asked for again on frames that run anyway
/// (coming back from the system's settings is one), at most once a second.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn polled(read: fn() -> Option<bool>) -> photocraft_ui_egui::SystemAnimationsFn {
    const INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
    let last = std::cell::Cell::new(None::<(std::time::Instant, Option<bool>)>);
    Box::new(move |_| {
        if let Some((at, animations)) = last.get()
            && at.elapsed() < INTERVAL
        {
            return animations;
        }
        let animations = read();
        last.set(Some((std::time::Instant::now(), animations)));
        animations
    })
}

#[cfg(target_os = "windows")]
mod platform {
    /// Settings › Accessibility › Visual effects › Animation effects.
    pub fn service() -> Option<photocraft_ui_egui::SystemAnimationsFn> {
        Some(super::polled(read))
    }

    /// A safe WinRT call; it reports what Windows applies to this session, so a Remote Desktop
    /// connection with animations off counts too.
    pub(super) fn read() -> Option<bool> {
        windows::UI::ViewManagement::UISettings::new().and_then(|settings| settings.AnimationsEnabled()).ok()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    /// System Settings › Accessibility › Display › Reduce motion.
    pub fn service() -> Option<photocraft_ui_egui::SystemAnimationsFn> {
        Some(super::polled(read))
    }

    /// Animations are on unless Reduce motion is.
    fn read() -> Option<bool> {
        Some(!objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
    }
}

/// Linux and the BSDs. Reading the setting can mean waiting for the session bus or running
/// `gsettings`, so a worker thread does it and the UI reads its latest answer.
#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};

    use super::freedesktop;

    /// Starts the worker; the returned reader gives its latest answer and asks it to read again
    /// when the window gets the focus back.
    pub fn service() -> Option<photocraft_ui_egui::SystemAnimationsFn> {
        let latest = Arc::new(Mutex::new(None));
        let wake: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
        // One pending request is enough: focus changes while the worker is busy, or while it
        // follows the portal and takes none, add nothing.
        let (refresh, refreshes) = mpsc::sync_channel::<()>(1);
        let publisher = Publisher { latest: Arc::clone(&latest), wake: Arc::clone(&wake) };
        std::thread::Builder::new().name("animations-setting".into()).spawn(move || worker(&publisher, &refreshes)).ok()?;
        let last_focus = std::cell::Cell::new(true);
        Some(Box::new(move |ctx| {
            let _ = wake.set(ctx.clone());
            let focused = ctx.input(|i| i.focused);
            let was_focused = last_focus.replace(focused);
            // Back from the desktop's settings, perhaps: read again.
            if focused && !was_focused {
                let _ = refresh.try_send(());
            }
            *lock(&latest)
        }))
    }

    /// Follows the portal when it has an answer; otherwise reads the desktop's own setting now
    /// and on every refresh request.
    fn worker(publisher: &Publisher, refreshes: &mpsc::Receiver<()>) {
        #[cfg(target_os = "linux")]
        if portal::follow(publisher) {
            return;
        }
        publisher.publish(desktop::read());
        for () in refreshes {
            publisher.publish(desktop::read());
        }
    }

    /// The worker's side of the latest answer. A change wakes the UI.
    struct Publisher {
        latest: Arc<Mutex<Option<bool>>>,
        wake: Arc<OnceLock<egui::Context>>,
    }

    impl Publisher {
        /// Stores the worker's answer, and wakes the UI when it changed.
        fn publish(&self, animations: Option<bool>) {
            let previous = std::mem::replace(&mut *lock(&self.latest), animations);
            if previous != animations
                && let Some(ctx) = self.wake.get()
            {
                ctx.request_repaint();
            }
        }
    }

    /// The latest answer; a panic elsewhere while holding it doesn't lose it.
    fn lock(latest: &Mutex<Option<bool>>) -> MutexGuard<'_, Option<bool>> {
        latest.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(target_os = "linux")]
    mod portal {
        use zbus::zvariant::{OwnedValue, Value};

        use super::freedesktop;

        /// Reads a setting's value as animations on or off.
        type Parse = fn(&str) -> Option<bool>;

        /// The settings the portal may answer for, most general first: the cross-desktop one,
        /// then GNOME's and KDE's own, which older portals pass on.
        const SETTINGS: [(&str, &str, Parse); 3] = [
            ("org.freedesktop.appearance", "reduced-motion", freedesktop::from_reduced_motion),
            ("org.gnome.desktop.interface", "enable-animations", freedesktop::from_enabled),
            ("org.kde.kdeglobals.KDE", "AnimationDurationFactor", freedesktop::from_duration_factor),
        ];

        /// Publishes the portal's answer, then every change to it. `false` when the portal has
        /// none: no session bus, no portal, or one that knows none of the settings.
        pub fn follow(publisher: &super::Publisher) -> bool {
            let Some(proxy) = crate::appearance::portal_proxy() else { return false };
            // Subscribe before the first read, so a change during start-up isn't lost.
            let signals = proxy.receive_signal("SettingChanged").ok();
            let Some(first) = read(&proxy) else { return false };
            publisher.publish(Some(first));
            for message in signals.into_iter().flatten() {
                let Ok((namespace, key, _)) = message.body().deserialize::<(String, String, OwnedValue)>() else {
                    continue;
                };
                // Read them all again: which setting answers first may have changed too.
                if SETTINGS.iter().any(|(ns, k, _)| *ns == namespace && *k == key) {
                    publisher.publish(read(&proxy));
                }
            }
            true
        }

        /// The answer of the first setting in [`SETTINGS`] the portal knows.
        fn read(proxy: &zbus::blocking::Proxy<'_>) -> Option<bool> {
            SETTINGS.iter().find_map(|(namespace, key, parse)| {
                let value: OwnedValue = proxy.call("Read", &(*namespace, *key)).ok()?;
                parse(&as_text(value)?)
            })
        }

        /// A setting's value as the text a settings file would hold.
        fn as_text(value: OwnedValue) -> Option<String> {
            let mut value: Value<'_> = value.into();
            // The Settings.Read result is a variant; some portals nest another variant inside it.
            for _ in 0..4 {
                match value {
                    Value::Value(inner) => value = *inner,
                    Value::Bool(on) => return Some(on.to_string()),
                    Value::U32(n) => return Some(n.to_string()),
                    Value::I32(n) => return Some(n.to_string()),
                    Value::F64(n) => return Some(n.to_string()),
                    Value::Str(text) => return Some(text.to_string()),
                    _ => return None,
                }
            }
            None
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            /// `value` as the portal hands it over.
            fn owned(value: Value<'_>) -> OwnedValue {
                OwnedValue::try_from(value).unwrap()
            }

            /// Each value type a portal sends, nested or not, reads as the text a settings file
            /// would hold; other types aren't settings.
            #[test]
            fn portal_values_read_as_text() {
                let nested = Value::Value(Box::new(Value::Value(Box::new(Value::U32(1)))));
                assert_eq!(as_text(owned(nested)).as_deref(), Some("1"));
                assert_eq!(as_text(owned(Value::from(false))).as_deref(), Some("false"));
                assert_eq!(as_text(owned(Value::F64(0.5))).as_deref(), Some("0.5"));
                assert_eq!(as_text(owned(Value::from("0"))).as_deref(), Some("0"));
                assert_eq!(as_text(owned(Value::from(vec![1u32, 2]))), None);
                assert_eq!(SETTINGS[0].1, "reduced-motion", "the cross-desktop setting is asked first");
            }
        }
    }

    /// The desktop's own setting, without the portal.
    mod desktop {
        use std::path::PathBuf;

        use super::freedesktop;

        /// A settings file is a few kilobytes; a larger one isn't read.
        const MAX_CONFIG_BYTES: u64 = 1 << 20;

        /// The setting of the desktop this session runs (`XDG_CURRENT_DESKTOP`).
        pub fn read() -> Option<bool> {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            freedesktop::without_portal(&desktop, &config_file, &gsettings)
        }

        /// `$XDG_CONFIG_HOME`, or `~/.config` when it isn't set.
        fn config_home() -> Option<PathBuf> {
            let var = |key: &str| std::env::var_os(key).filter(|value| !value.is_empty()).map(PathBuf::from);
            var("XDG_CONFIG_HOME").filter(|path| path.is_absolute()).or_else(|| Some(var("HOME")?.join(".config")))
        }

        /// The text of `name` in the user's config directory, if it is there and not too large.
        fn config_file(name: &str) -> Option<String> {
            use std::io::Read as _;
            let mut text = String::new();
            std::fs::File::open(config_home()?.join(name)).ok()?.take(MAX_CONFIG_BYTES).read_to_string(&mut text).ok()?;
            Some(text)
        }

        /// `gsettings get`, by absolute path (`/usr/local` and `/usr/pkg` are where the BSDs
        /// install it).
        fn gsettings(schema: &str, key: &str) -> Option<String> {
            let exe = ["/usr/bin/gsettings", "/bin/gsettings", "/usr/local/bin/gsettings", "/usr/pkg/bin/gsettings"]
                .into_iter()
                .find(|path| std::path::Path::new(path).is_file())?;
            let output = std::process::Command::new(exe).args(["get", schema, key]).output().ok()?;
            if !output.status.success() {
                return None;
            }
            String::from_utf8(output.stdout).ok()
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod platform {
    /// No reader here: animations stay on.
    pub fn service() -> Option<photocraft_ui_egui::SystemAnimationsFn> {
        None
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    /// The real setting of the machine the tests run on: whatever it is, reading it doesn't
    /// panic and the service passes it on (`--nocapture` shows it).
    #[test]
    fn windows_reports_its_animation_setting() {
        let animations = super::platform::read();
        println!("UISettings.AnimationsEnabled: {animations:?}");
        let service = super::platform::service().expect("service");
        assert_eq!(service(&egui::Context::default()), animations);
    }
}
