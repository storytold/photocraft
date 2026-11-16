//! Native print confirmation used by File › Print.

use std::io::Write as _;
use std::sync::Arc;

#[cfg(target_os = "windows")]
use photocraft_engine::print_cmds::SystemPrintDuplex;
use photocraft_engine::print_cmds::{SystemPrintDialog, SystemPrintOutcome, SystemPrintSettings};

#[cfg(target_os = "macos")]
const MAC_PRINT_SCRIPT: &str = r#"
on run argv
  with timeout of 86400 seconds
    tell application "Preview"
    activate
    set printName to item 1 of argv
    set printCopies to (item 2 of argv) as integer
    set printCollating to (item 3 of argv) is "true"
    set printPrinter to item 4 of argv
    set printDocument to missing value
    repeat 200 times
      if exists document printName then
        set printDocument to document printName
        exit repeat
      end if
      delay 0.05
    end repeat
    if printDocument is missing value then error "Preview did not open the PhotoCraft print document"
    try
      if printPrinter is "" then
        print printDocument with properties {copies:printCopies, collating:printCollating} with print dialog
      else
        print printDocument with properties {copies:printCopies, collating:printCollating, target printer:printPrinter} with print dialog
      end if
      close printDocument
      return "printed"
    on error errorMessage number errorNumber
      close printDocument
      if errorNumber is -128 then
        return "cancelled"
      end if
      error errorMessage number errorNumber
    end try
    end tell
  end timeout
end run
"#;

/// Returns the desktop callback that opens the platform print confirmation for a rendered PDF.
pub fn dialog() -> SystemPrintDialog {
    Arc::new(show)
}

/// Keeps the temporary PDF alive while the synchronous platform confirmation is open.
fn show(pdf: &[u8], settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    if photocraft_engine::print_cmds::printing_disabled() {
        return Err("printing is disabled in this process; use dryRun: true".into());
    }
    let mut file = tempfile::Builder::new().prefix("photocraft-print-").suffix(".pdf").tempfile().map_err(|e| e.to_string())?;
    file.write_all(pdf).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    // Close the writer before the OS opens the document. In particular, WinRT's StorageFile can
    // reject a path whose temporary-file writer is still open. TempPath keeps the file alive until
    // the synchronous platform dialog has finished.
    let path = file.into_temp_path();
    platform(path.as_ref(), settings)
}

#[cfg(target_os = "macos")]
/// Serializes the Preview settings supported by its public AppleScript dictionary.
fn mac_print_arguments(settings: &SystemPrintSettings) -> [String; 3] {
    [settings.copies.to_string(), if settings.collate { "true" } else { "false" }.into(), settings.printer.clone().unwrap_or_default()]
}

#[cfg(target_os = "macos")]
/// Opens the PDF in Preview and waits for its macOS print sheet to finish or be cancelled.
fn platform(path: &std::path::Path, settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    // Opening through Launch Services gives sandboxed Preview a file-access extension. Asking
    // Preview to open a private tempfile directly through AppleScript instead produces a
    // misleading "you don't have permission" alert before the print sheet appears.
    let opened = std::process::Command::new("/usr/bin/open")
        .args(["-a", "Preview"])
        .arg(path)
        .status()
        .map_err(|_| "macOS could not start Preview for print confirmation".to_string());
    let Ok(opened) = opened else { return Ok(SystemPrintOutcome::Unavailable) };
    if !opened.success() {
        return Ok(SystemPrintOutcome::Unavailable);
    }
    let name = path.file_name().and_then(std::ffi::OsStr::to_str).ok_or("the temporary print document has no valid file name")?;

    // Wait for Preview to register the exact, uniquely named document opened above. Do not ask
    // AppleScript to open the path again: that bypasses the sandbox extension on some macOS builds.
    let result = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", MAC_PRINT_SCRIPT, "--"])
        .arg(name)
        .args(mac_print_arguments(settings))
        .output()
        .map_err(|e| format!("could not open the macOS print dialog: {e}"))?;
    if !result.status.success() {
        let error = String::from_utf8_lossy(&result.stderr);
        if error.contains("-1743") || error.contains("not authorized") || error.contains("Not authorized") {
            return Ok(SystemPrintOutcome::Unavailable);
        }
        return Err(format!("macOS print dialog failed: {}", error.trim()));
    }
    if String::from_utf8_lossy(&result.stdout).trim() == "cancelled" { Ok(SystemPrintOutcome::Cancelled) } else { Ok(SystemPrintOutcome::Printed) }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn preview_receives_supported_initial_print_settings() {
        let settings = SystemPrintSettings {
            title: "proof".into(),
            printer: Some("Studio".into()),
            copies: 3,
            collate: false,
            duplex: Some(photocraft_engine::print_cmds::SystemPrintDuplex::LongEdge),
            paper: (612.0, 792.0),
            landscape: false,
            driver_options: Vec::new(),
            driver_settings: None,
            grayscale: false,
            raster_dpi: 300.0,
        };
        assert_eq!(mac_print_arguments(&settings), ["3", "false", "Studio"]);
    }
}

#[cfg(target_os = "linux")]
/// Opens the desktop's XDG Print portal confirmation.
fn platform(path: &std::path::Path, settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    match crate::print_portal::confirm(path, settings) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            log::warn!("native print portal unavailable; falling back to the regular spooler: {error}");
            Ok(SystemPrintOutcome::Unavailable)
        }
    }
}

#[cfg(target_os = "windows")]
/// Opens the native Windows print dialog through the bundled PowerShell adapter.
fn platform(path: &std::path::Path, settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    let script = include_str!("system_print_windows.ps1");
    let mut command = std::process::Command::new("powershell.exe");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let result = command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("PHOTOCRAFT_PRINT_FILE", path)
        .env("PHOTOCRAFT_PRINT_TITLE", &settings.title)
        .env("PHOTOCRAFT_PRINT_PRINTER", settings.printer.as_deref().unwrap_or(""))
        .env("PHOTOCRAFT_PRINT_COPIES", settings.copies.to_string())
        .env("PHOTOCRAFT_PRINT_COLLATE", if settings.collate { "1" } else { "0" })
        .env("PHOTOCRAFT_PRINT_GRAYSCALE", if settings.grayscale { "1" } else { "0" })
        .env("PHOTOCRAFT_PRINT_RASTER_DPI", settings.raster_dpi.round().to_string())
        .env("PHOTOCRAFT_PRINT_DRIVER_SETTINGS", settings.driver_settings.as_deref().unwrap_or(""))
        .env(
            "PHOTOCRAFT_PRINT_DUPLEX",
            match settings.duplex {
                None => "default",
                Some(SystemPrintDuplex::Simplex) => "simplex",
                Some(SystemPrintDuplex::LongEdge) => "long-edge",
                Some(SystemPrintDuplex::ShortEdge) => "short-edge",
            },
        )
        .output();
    let Ok(result) = result else { return Ok(SystemPrintOutcome::Unavailable) };
    if !result.status.success() {
        return Err(format!("Windows print dialog failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
    }
    if String::from_utf8_lossy(&result.stdout).trim() == "cancelled" { Ok(SystemPrintOutcome::Cancelled) } else { Ok(SystemPrintOutcome::Printed) }
}
