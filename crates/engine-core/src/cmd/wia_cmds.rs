//! File › Import › WIA Support. Windows Image Acquisition is a Windows-only API for scanners and
//! cameras; Photoshop shows this item only on Windows. We register it cross-platform and report the
//! available acquisition devices — none off Windows, where the feature is unavailable. Actual WIA
//! device enumeration/transfer needs the Windows COM API plus hardware (not exercised here).

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{Result, Session};

fn wia(_s: &mut Session, _p: &Value) -> Result<Value> {
    #[cfg(target_os = "windows")]
    {
        // A real implementation enumerates WIA devices via COM (IWiaDevMgr) and transfers an image
        // into a new document. No COM binding is wired yet, so report zero devices. The note names
        // the missing device as the reason, so the UI can tell "no device" apart from a found one (#13938).
        Ok(json!({"available": true, "devices": [], "note": "No WIA device was found; WIA device enumeration is not yet wired"}))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(json!({"available": false, "devices": [], "note": "WIA scanner/camera import is available on Windows only"}))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.import.wiaSupport",
        label: "WIA Support…",
        menu: &["File", "Import"],
        shortcut: None,
        params: r#"{} → {available, devices, note}: acquire from a scanner/camera (Windows only)"#,
        enabled: |_s| Ok(()),
        journal: false,
        run: |s, p| wia(s, p),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_availability() {
        let mut s = crate::session();
        let r = s.execute("file.import.wiaSupport", json!({})).unwrap();
        assert!(r["devices"].is_array());
        assert_eq!(r["available"], cfg!(target_os = "windows"));
    }

    /// #13938: on Windows the result reports no device with a reason the UI can tell apart
    /// from a found device, so the no-device alert does not fire once enumeration reports one.
    #[test]
    #[cfg(target_os = "windows")]
    fn windows_reports_no_device_with_a_reason() {
        let mut s = crate::session();
        let r = s.execute("file.import.wiaSupport", json!({})).unwrap();
        assert_eq!(r["devices"].as_array().map(Vec::len), Some(0), "no device is reported");
        let note = r["note"].as_str().unwrap_or_default();
        assert!(note.starts_with("No WIA device was found"), "the note names the reason: {note}");
    }
}
