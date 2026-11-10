//! The user's Windows "Haptic signals" mouse setting (Settings › Bluetooth & devices › Mouse).
//! Windows applies it to its own haptics, not to raw HID writes, so PhotoCraft honors it itself.

/// Whether to play haptics, and how strong (0..=1).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Setting {
    pub enabled: bool,
    pub strength: f64,
}

impl Setting {
    /// Windows builds that predate the setting: play at full strength.
    pub const DEFAULT: Self = Self { enabled: true, strength: 1.0 };
    pub const OFF: Self = Self { enabled: false, strength: 0.0 };
}

/// The setting as it is now. Off outside Windows.
pub fn read() -> Setting {
    #[cfg(target_os = "windows")]
    return windows::read();
    #[cfg(not(target_os = "windows"))]
    Setting::OFF
}

/// Turns the raw registry values into a [`Setting`]: `None` for a value that isn't there,
/// `Some(Err(()))` for one that is there but unreadable. A missing value keeps the default; an
/// unreadable one turns haptics off, so PhotoCraft never buzzes against the user's wishes.
pub fn from_values(enabled: Option<Result<u32, ()>>, intensity_bits: Option<Result<u64, ()>>) -> Setting {
    let mut setting = Setting::DEFAULT;
    match enabled {
        Some(Ok(on)) => setting.enabled = on != 0,
        Some(Err(())) => return Setting::OFF,
        None => {}
    }
    match intensity_bits {
        Some(Ok(bits)) => {
            let strength = f64::from_bits(bits);
            if !strength.is_finite() {
                return Setting::OFF;
            }
            setting.strength = strength.clamp(0.0, 1.0);
        }
        Some(Err(())) => return Setting::OFF,
        None => {}
    }
    setting
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{Setting, from_values};
    use std::io::ErrorKind;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Windows\EnhancedPenSupport";
    const ENABLED_VALUE: &str = "MouseFeedbackEnabled";
    /// REG_QWORD holding the bits of an f64: 0.25, 0.5, 0.75 or 1.0.
    const INTENSITY_VALUE: &str = "MouseFeedbackIntensity";

    pub fn read() -> Setting {
        let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY) {
            Ok(key) => key,
            Err(err) if err.kind() == ErrorKind::NotFound => return Setting::DEFAULT,
            Err(_) => return Setting::OFF,
        };
        from_values(present(key.get_value::<u32, _>(ENABLED_VALUE)), present(key.get_value::<u64, _>(INTENSITY_VALUE)))
    }

    fn present<T>(value: std::io::Result<T>) -> Option<Result<T, ()>> {
        match value {
            Ok(v) => Some(Ok(v)),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(_) => Some(Err(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_keep_full_strength() {
        assert_eq!(from_values(None, None), Setting::DEFAULT);
    }

    #[test]
    fn reads_the_switch_and_the_intensity() {
        assert_eq!(from_values(Some(Ok(1)), Some(Ok(0.5f64.to_bits()))), Setting { enabled: true, strength: 0.5 });
        assert!(!from_values(Some(Ok(0)), Some(Ok(1.0f64.to_bits()))).enabled);
    }

    #[test]
    fn anything_unreadable_turns_haptics_off() {
        assert_eq!(from_values(Some(Err(())), None), Setting::OFF);
        assert_eq!(from_values(None, Some(Err(()))), Setting::OFF);
        assert_eq!(from_values(None, Some(Ok(f64::NAN.to_bits()))), Setting::OFF);
    }

    #[test]
    fn clamps_an_out_of_range_intensity() {
        assert_eq!(from_values(None, Some(Ok(7.0f64.to_bits()))).strength, 1.0);
        assert_eq!(from_values(None, Some(Ok((-1.0f64).to_bits()))).strength, 0.0);
    }
}
