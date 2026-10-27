//! A separate STA PowerShell process owns the WIA COM objects and modal driver dialogs. This
//! keeps COM/driver failures outside the editor, and cancellation can stop a blocked transfer.

use std::fs::File;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Acquisition, Action, Request, Scan};
use crate::jobs::JobCtx;
use crate::{EngineError, Result};

const BRIDGE: &str = include_str!("windows.ps1");
const MAX_REPORT_BYTES: u64 = 1 << 20;
const MAX_IMAGE_BYTES: u64 = 256 << 20;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(super) fn run(request: &Request, ctx: &JobCtx) -> Result<Acquisition> {
    ctx.check()?;
    let dir = tempfile::Builder::new().prefix("photocraft-wia-").tempdir().map_err(|e| failure(format!("cannot create the WIA temporary directory: {e}")))?;
    let script = dir.path().join("wia.ps1");
    std::fs::write(&script, BRIDGE).map_err(|e| failure(format!("cannot prepare the WIA bridge: {e}")))?;
    let request_json = serde_json::to_string(request).map_err(|e| failure(format!("cannot encode the WIA request: {e}")))?;
    let mut command = powershell()?;
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Sta", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        // Request values are data, never interpolated into executable PowerShell code.
        .env("PHOTOCRAFT_WIA_REQUEST", request_json)
        .env("PHOTOCRAFT_WIA_OUTPUT_DIR", dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Hides the console window; WIA/driver dialogs can still open normally.
        .creation_flags(0x0800_0000);
    let mut child =
        command.spawn().map_err(|e| failure(format!("cannot start Windows PowerShell for WIA: {e}; ensure Windows PowerShell 5.1 is installed")))?;
    let listing = matches!(&request.action, Action::List);
    ctx.progress(0.05, if listing { "Looking for WIA scanners" } else { "Waiting for scanner and WIA settings" });
    let status = wait_for_bridge(&mut child, ctx, if listing { LIST_TIMEOUT } else { ACQUIRE_TIMEOUT })?;
    ctx.check()?;
    let report_path = dir.path().join("result.json");
    if !report_path.is_file() {
        return Err(failure(format!(
            "the WIA bridge ended without a result ({status}); check that Windows PowerShell and the Windows Image Acquisition (WIA) service are available"
        )));
    }
    let report_bytes = read_bounded(&report_path, MAX_REPORT_BYTES)?;
    let report = serde_json::from_slice(&report_bytes).map_err(|e| failure(format!("the WIA bridge returned an invalid result: {e}")))?;
    let (extension, dpi) = validate_report(&report, listing, status.success())?;
    let image = if let Some(extension) = extension {
        ctx.progress(0.8, "Reading the scanned image");
        let bytes = read_bounded(&dir.path().join("scan.bin"), MAX_IMAGE_BYTES)?;
        if bytes.is_empty() {
            return Err(failure("the scanner returned an empty image"));
        }
        Some(Scan { bytes, extension, dpi })
    } else {
        None
    };
    ctx.check()?;
    Ok(Acquisition { report, image })
}

fn failure(message: impl Into<String>) -> EngineError {
    EngineError::Other(format!("WIA: {}", message.into()))
}

fn powershell() -> Result<Command> {
    // Use the Windows installation, rather than searching an arbitrary working directory/PATH.
    let root = std::env::var_os("SystemRoot").ok_or_else(|| failure("Windows SystemRoot is unavailable; Windows PowerShell cannot be located"))?;
    Ok(Command::new(Path::new(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe")))
}

fn stop_bridge(child: &mut Child) {
    // `kill` may fail if the child has just exited. Always reap it before dropping its tempdir.
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for_bridge(child: &mut Child, ctx: &JobCtx, timeout: Duration) -> Result<ExitStatus> {
    let started = Instant::now();
    loop {
        if ctx.cancelled() {
            stop_bridge(child);
            return Err(EngineError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => {
                stop_bridge(child);
                return Err(failure(format!("cannot monitor the WIA bridge: {e}")));
            }
        }
        if started.elapsed() >= timeout {
            stop_bridge(child);
            return Err(failure("the scanner did not finish in time; check the USB connection, close other scanner applications, and try again"));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|e| failure(format!("cannot read the scanner result: {e}")))?;
    let length = file.metadata().map_err(|e| failure(format!("cannot inspect the scanner result: {e}")))?.len();
    if length > max {
        return Err(failure(format!("the scanner result exceeds the {} MiB limit; scan a smaller area or use a lower resolution", max >> 20)));
    }
    // Limit the read as well as the metadata check, so a growing file cannot bypass the cap.
    let mut reader = file.take(max.saturating_add(1));
    let capacity = usize::try_from(length).map_err(|_| failure("the scanner result is too large for this system"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|e| failure(format!("not enough memory to read the scanner result: {e}")))?;
    reader.read_to_end(&mut bytes).map_err(|e| failure(format!("cannot read the scanner result: {e}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(failure("the scanner result grew beyond the allowed size"));
    }
    Ok(bytes)
}

/// The bridge is a separate process: validate its result before trusting a format or importing.
fn validate_report(report: &Value, listing: bool, process_ok: bool) -> Result<(Option<String>, Option<f32>)> {
    if let Some(error) = report.get("error").filter(|v| !v.is_null()) {
        let message = error.get("message").and_then(Value::as_str).unwrap_or("the WIA driver failed");
        let hresult = error.get("hresult").and_then(Value::as_str).unwrap_or("unknown HRESULT");
        return Err(failure(format!("{message} ({hresult})")));
    }
    if !process_ok {
        return Err(failure("the WIA bridge failed; check the Windows Image Acquisition (WIA) service and scanner driver"));
    }
    let available = report.get("available").and_then(Value::as_bool);
    let acquired = report.get("acquired").and_then(Value::as_bool);
    let cancelled = report.get("cancelled").and_then(Value::as_bool);
    if available != Some(true) || !report.get("devices").is_some_and(Value::is_array) || acquired.is_none() || cancelled.is_none() {
        return Err(failure("the WIA bridge returned an incomplete result"));
    }
    if cancelled == Some(true) && acquired == Some(true) {
        return Err(failure("the WIA bridge returned contradictory acquisition and cancellation flags"));
    }
    if acquired != Some(true) {
        if listing || cancelled == Some(true) {
            return Ok((None, None));
        }
        return Err(failure("the scanner returned no image; check the WIA driver, USB connection and Windows Image Acquisition service"));
    }
    if listing {
        return Err(failure("the WIA bridge acquired an image during device enumeration"));
    }
    let extension = report.get("extension").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    if !matches!(extension.as_str(), "bmp" | "png" | "tif" | "tiff" | "jpg" | "jpeg") {
        return Err(failure(format!("the scanner returned an unsupported image format `{extension}`; select TIFF, BMP, PNG or JPEG in the scanner settings")));
    }
    let dpi = match report.get("dpi") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let n =
                value.as_f64().filter(|n| n.is_finite() && *n > 0.0 && *n <= 65535.0).ok_or_else(|| failure("the scanner returned an invalid resolution"))?;
            Some(n as f32)
        }
    };
    Ok((Some(extension), dpi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_actual_format_resolution_and_cancellation() {
        let mut report = json!({"available": true, "devices": [], "acquired": true, "cancelled": false, "extension": "TIFF", "dpi": 600});
        assert_eq!(validate_report(&report, false, true).unwrap(), (Some("tiff".into()), Some(600.0)));
        report["extension"] = json!("../image.ps1");
        assert!(validate_report(&report, false, true).is_err());
        report["extension"] = json!("bmp");
        report["dpi"] = json!(-1);
        assert!(validate_report(&report, false, true).is_err());
        assert_eq!(validate_report(&json!({"available": true, "devices": [], "acquired": false, "cancelled": true}), false, true).unwrap(), (None, None));
        assert!(validate_report(&json!({}), false, true).is_err());
    }

    #[test]
    fn enumeration_cannot_claim_an_acquisition() {
        let report = json!({"available": true, "devices": [], "acquired": false, "cancelled": false});
        assert!(validate_report(&report, true, true).is_ok());
        assert!(validate_report(&report, false, true).is_err());
        assert!(validate_report(&report, true, false).is_err());
        let acquired = json!({"available": true, "devices": [], "acquired": true, "cancelled": false, "extension": "tiff"});
        assert!(validate_report(&acquired, true, true).is_err());
    }

    #[test]
    fn driver_errors_keep_hresult_and_advice() {
        let error = validate_report(&json!({"error": {"message": "Close other scanner applications", "hresult": "0x80210006"}}), false, false).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("0x80210006"));
        assert!(message.contains("Close other scanner applications"));
    }

    #[test]
    fn result_file_read_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image");
        std::fs::write(&path, b"image").unwrap();
        assert_eq!(read_bounded(&path, 5).unwrap(), b"image");
        assert!(read_bounded(&path, 4).is_err());
    }

    #[test]
    fn bridge_uses_property_ids_and_supported_formats() {
        // Exercise the same PowerShell helpers with deliberately shuffled synthetic properties.
        // The main COM/acquisition block is excluded, so no hardware is contacted by this test.
        let helpers = BRIDGE.split_once("$result = [ordered]").unwrap().0;
        let fixture = r#"
$x = [pscustomobject]@{ PropertyID = 6147; Value = 300 }
$y = [pscustomobject]@{ PropertyID = 6148; Value = 600 }
$other = [pscustomobject]@{ PropertyID = 100; Value = 123 }
$properties = @($y, $other, $x)
if ((Get-WiaValue $properties 6147) -ne 300 -or (Get-WiaValue $properties 6148) -ne 600) { throw 'Property ID lookup failed.' }
Initialize-ScannerApi
$v500 = [pscustomobject]@{ Supported = @('DeviceIndependentBitmap') }
$v500 | Add-Member ScriptMethod IsFormatSupported { param($format) return $this.Supported -contains [string]$format }
if ([string](Select-ScannerFormat $v500) -ne 'DeviceIndependentBitmap') { throw 'The BMP-only V500 was given an unsupported format.' }
$v500.Supported = @('DeviceIndependentBitmap', 'Tiff')
if ([string](Select-ScannerFormat $v500) -ne 'Tiff') { throw 'Supported TIFF was not preferred.' }
$v500.Supported = @()
$rejected = $false
try { Select-ScannerFormat $v500 } catch { $rejected = $true }
if (!$rejected) { throw 'An unsupported transfer format was accepted.' }
"#;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("helpers.ps1");
        std::fs::write(&script, format!("{helpers}\n{fixture}")).unwrap();
        let output = powershell()
            .unwrap()
            .args(["-NoProfile", "-NonInteractive", "-Sta", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn cancellation_reaps_the_bridge_process() {
        let mut command = powershell().unwrap();
        let mut child = command
            .args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 60"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        let started = Instant::now();
        assert!(matches!(wait_for_bridge(&mut child, &ctx, LIST_TIMEOUT), Err(EngineError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait().unwrap().is_some());
    }
}
