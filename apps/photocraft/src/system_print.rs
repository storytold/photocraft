//! Native print confirmation used by File › Print.

use std::io::Write as _;
use std::sync::Arc;

#[cfg(target_os = "windows")]
use photocraft_engine::print_cmds::SystemPrintDuplex;
use photocraft_engine::print_cmds::{SystemPrintDialog, SystemPrintOutcome, SystemPrintSettings};

#[cfg(target_os = "macos")]
const MAC_PRINT_SCRIPT: &str = r#"
on run argv
  tell application "Preview"
    activate
    set printName to item 1 of argv
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
      print printDocument with print dialog
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
end run
"#;

pub fn dialog() -> SystemPrintDialog {
    Arc::new(show)
}

fn show(pdf: &[u8], settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    if photocraft_engine::print_cmds::printing_disabled() {
        return Err("printing is disabled in this process; use dryRun: true".into());
    }
    let mut file = tempfile::Builder::new().prefix("photocraft-print-").suffix(".pdf").tempfile().map_err(|e| e.to_string())?;
    file.write_all(pdf).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    let path = file.path().to_path_buf();
    platform(file.as_file_mut(), &path, settings)
}

#[cfg(target_os = "macos")]
fn platform(_file: &mut std::fs::File, path: &std::path::Path, _settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    // Opening through Launch Services gives sandboxed Preview a file-access extension. Asking
    // Preview to open a private tempfile directly through AppleScript instead produces a
    // misleading "you don't have permission" alert before the print sheet appears.
    let opened = std::process::Command::new("/usr/bin/open")
        .args(["-a", "Preview"])
        .arg(path)
        .status()
        .map_err(|e| format!("could not hand the print document to macOS: {e}"))?;
    if !opened.success() {
        return Err("macOS could not open the temporary print document in Preview".into());
    }
    let name = path.file_name().and_then(std::ffi::OsStr::to_str).ok_or("the temporary print document has no valid file name")?;

    // Wait for Preview to register the exact, uniquely named document opened above. Do not ask
    // AppleScript to open the path again: that bypasses the sandbox extension on some macOS builds.
    let result = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", MAC_PRINT_SCRIPT, "--"])
        .arg(name)
        .output()
        .map_err(|e| format!("could not open the macOS print dialog: {e}"))?;
    if !result.status.success() {
        return Err(format!("macOS print dialog failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
    }
    if String::from_utf8_lossy(&result.stdout).trim() == "cancelled" { Ok(SystemPrintOutcome::Cancelled) } else { Ok(SystemPrintOutcome::Printed) }
}

#[cfg(target_os = "linux")]
fn platform(file: &mut std::fs::File, path: &std::path::Path, settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    use std::io::{Seek as _, SeekFrom};

    file.seek(SeekFrom::Start(0)).map_err(|e| format!("could not rewind the print document: {e}"))?;
    crate::print_portal::confirm(path, settings)
}

#[cfg(target_os = "windows")]
fn platform(_file: &mut std::fs::File, path: &std::path::Path, settings: &SystemPrintSettings) -> Result<SystemPrintOutcome, String> {
    let script = include_str!("system_print_windows.ps1");
    let result = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("PHOTOCRAFT_PRINT_FILE", path)
        .env("PHOTOCRAFT_PRINT_TITLE", &settings.title)
        .env("PHOTOCRAFT_PRINT_PRINTER", settings.printer.as_deref().unwrap_or(""))
        .env("PHOTOCRAFT_PRINT_COPIES", settings.copies.to_string())
        .env("PHOTOCRAFT_PRINT_COLLATE", if settings.collate { "1" } else { "0" })
        .env(
            "PHOTOCRAFT_PRINT_DUPLEX",
            match settings.duplex {
                None => "default",
                Some(SystemPrintDuplex::Simplex) => "simplex",
                Some(SystemPrintDuplex::LongEdge) => "long-edge",
                Some(SystemPrintDuplex::ShortEdge) => "short-edge",
            },
        )
        .output()
        .map_err(|e| format!("could not open the Windows print dialog: {e}"))?;
    if !result.status.success() {
        return Err(format!("Windows print dialog failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
    }
    if String::from_utf8_lossy(&result.stdout).trim() == "cancelled" { Ok(SystemPrintOutcome::Cancelled) } else { Ok(SystemPrintOutcome::Printed) }
}
