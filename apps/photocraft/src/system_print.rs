//! Native print confirmation used by File › Print.

use std::io::Write as _;
use std::sync::Arc;

use photocraft_engine::print_cmds::{SystemPrintDialog, SystemPrintOutcome};

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

fn show(pdf: &[u8], title: &str) -> Result<SystemPrintOutcome, String> {
    if photocraft_engine::print_cmds::printing_disabled() {
        return Err("printing is disabled in this process; use dryRun: true".into());
    }
    let mut file = tempfile::Builder::new().prefix("photocraft-print-").suffix(".pdf").tempfile().map_err(|e| e.to_string())?;
    file.write_all(pdf).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    platform(file.as_file(), file.path(), title)
}

#[cfg(target_os = "macos")]
fn platform(_file: &std::fs::File, path: &std::path::Path, _title: &str) -> Result<SystemPrintOutcome, String> {
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
fn platform(file: &std::fs::File, _path: &std::path::Path, title: &str) -> Result<SystemPrintOutcome, String> {
    use std::collections::HashMap;
    use std::os::fd::AsFd as _;
    use zbus::{
        blocking::Proxy,
        zvariant::{Fd, OwnedObjectPath, OwnedValue, Value},
    };

    let conn = zbus::blocking::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let unique = conn.unique_name().ok_or("print portal connection has no unique name")?.as_str().trim_start_matches(':').replace('.', "_");
    let token = format!("photocraft_print_{}", std::process::id());
    let request_path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    let request = Proxy::new(&conn, "org.freedesktop.portal.Desktop", request_path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    use futures_lite::StreamExt as _;
    let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
    let portal =
        Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Print").map_err(|e| e.to_string())?;
    let options: HashMap<&str, Value<'_>> = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let fd = Fd::from(file.as_fd());
    let _handle: OwnedObjectPath = portal.call("Print", &("", title, fd, options)).map_err(|e| format!("could not open the desktop print portal: {e}"))?;
    let reply = futures_lite::future::block_on(replies.next()).ok_or("the desktop print dialog closed without a result")?;
    let (code, _values): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(SystemPrintOutcome::Printed),
        1 => Ok(SystemPrintOutcome::Cancelled),
        _ => Err("the desktop print portal could not print the document".into()),
    }
}

#[cfg(target_os = "windows")]
fn platform(_file: &std::fs::File, path: &std::path::Path, title: &str) -> Result<SystemPrintOutcome, String> {
    let script = include_str!("system_print_windows.ps1");
    let result = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("PHOTOCRAFT_PRINT_FILE", path)
        .env("PHOTOCRAFT_PRINT_TITLE", title)
        .output()
        .map_err(|e| format!("could not open the Windows print dialog: {e}"))?;
    if !result.status.success() {
        return Err(format!("Windows print dialog failed: {}", String::from_utf8_lossy(&result.stderr).trim()));
    }
    if String::from_utf8_lossy(&result.stdout).trim() == "cancelled" { Ok(SystemPrintOutcome::Cancelled) } else { Ok(SystemPrintOutcome::Printed) }
}
