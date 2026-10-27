//! File › Import › WIA Support. Discovery is a query; acquisition uses the Windows scanner
//! dialogs (or an explicit device for automation) and opens a new, unsaved document.
//! The Windows Automation bridge runs in a separate process so driver calls can be cancelled
//! and the desktop remains responsive. No COM pointers cross the safe Rust engine boundary.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

#[cfg(target_os = "windows")]
mod windows;

const COMMAND: &str = "file.import.wiaSupport";

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Action {
    #[default]
    List,
    Acquire,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Color,
    Gray,
    Text,
}

fn show_ui() -> bool {
    true
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    action: Action,
    #[serde(default, rename = "deviceId")]
    device_id: Option<String>,
    #[serde(default)]
    dpi: Option<u32>,
    #[serde(default)]
    mode: Mode,
    #[serde(default = "show_ui", rename = "showUI")]
    show_ui: bool,
}

fn request(p: &Value) -> Result<Request> {
    let bad = |msg: String| EngineError::BadParams { cmd: COMMAND.into(), msg };
    let r: Request = serde_json::from_value(p.clone()).map_err(|e| bad(e.to_string()))?;
    if r.device_id.as_ref().is_some_and(|id| id.trim().is_empty() || id.len() > 4096 || id.contains('\0')) {
        return Err(bad("`deviceId` must be a nonempty WIA device ID (at most 4096 bytes)".into()));
    }
    if r.dpi.is_some_and(|dpi| !(1..=12800).contains(&dpi)) {
        return Err(bad("`dpi` must be an integer from 1 to 12800; the driver must support it".into()));
    }
    if r.action == Action::Acquire && !r.show_ui && r.device_id.is_none() {
        return Err(bad("`showUI: false` requires an explicit `deviceId` from device discovery".into()));
    }
    Ok(r)
}

#[cfg(any(target_os = "windows", test))]
struct Scan {
    bytes: Vec<u8>,
    extension: String,
    dpi: Option<f32>,
}

#[cfg(any(target_os = "windows", test))]
struct Acquisition {
    report: Value,
    image: Option<Scan>,
}

#[cfg(any(target_os = "windows", test))]
struct Prepared {
    report: Value,
    imported: Option<photocraft_io::ImportResult>,
}

#[cfg(any(target_os = "windows", test))]
fn prepare(acquired: Acquisition, ctx: &crate::jobs::JobCtx) -> Result<Prepared> {
    ctx.check()?;
    let imported = match acquired.image {
        None => None,
        Some(scan) => {
            crate::allocation::checkpoint("importing scanner image")?;
            let name = format!("Scan.{}", scan.extension);
            let mut imported = ctx.stage(0.8, 1.0, "Opening scan", |ctl| photocraft_io::import_with(&name, &scan.bytes, ctl)).map_err(|e| match e {
                photocraft_io::IoError::Cancelled => EngineError::Cancelled,
                e => EngineError::Other(format!("cannot open WIA scan: {e}")),
            })?;
            imported.document.name = "Scan".into();
            if let Some(dpi) = scan.dpi.filter(|dpi| dpi.is_finite() && *dpi > 0.0 && *dpi <= 12800.0) {
                imported.document.resolution_dpi = dpi;
            }
            Some(imported)
        }
    };
    ctx.check()?;
    Ok(Prepared { report: acquired.report, imported })
}

#[cfg(any(target_os = "windows", test))]
fn apply(s: &mut Session, prepared: Prepared) -> Result<Value> {
    let mut report = prepared.report;
    let object = report.as_object_mut().ok_or_else(|| EngineError::Other("invalid WIA acquisition report".into()))?;
    if let Some(imported) = prepared.imported {
        if let Some(question) = s.color.open_question(&imported.document) {
            let held = s.hold_open(imported.document, false);
            object.insert("held".into(), json!(held));
            object.insert("color".into(), question);
            object.insert("warnings".into(), json!(imported.warnings));
            return Ok(report);
        }
        let (index, color) = s.open_document(imported.document, None);
        let document = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
        // A scan has no durable source file. Closing it must offer to save the acquired pixels.
        if let Some(st) = s.active_mut() {
            st.saved_revision = 0;
        }
        object.insert("document".into(), json!(index));
        object.insert("docId".into(), json!(document.0));
        object.insert("color".into(), color);
        object.insert("warnings".into(), json!(imported.warnings));
        object.insert("acquired".into(), json!(true));
    }
    Ok(report)
}

#[cfg(any(target_os = "windows", test))]
fn run_with(s: &mut Session, request: Request, backend: impl FnOnce(&Request, &crate::jobs::JobCtx) -> Result<Acquisition> + Send + 'static) -> Result<Value> {
    crate::jobs::run(
        s,
        "WIA scanner",
        false,
        move |ctx| {
            ctx.progress(0.0, "Connecting to scanner");
            prepare(backend(&request, ctx)?, ctx)
        },
        apply,
    )
}

fn wia(s: &mut Session, p: &Value) -> Result<Value> {
    let request = request(p)?;
    #[cfg(target_os = "windows")]
    {
        run_with(s, request, windows::run)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = s;
        if request.action == Action::Acquire {
            return Err(EngineError::Other("WIA scanner import is available on Windows only".into()));
        }
        Ok(json!({"available": false, "devices": [], "acquired": false, "cancelled": false, "note": "WIA scanner import is available on Windows only"}))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "WIA Support…",
        menu: &["File", "Import"],
        shortcut: None,
        params: r#"{"action":"list|acquire"="list","deviceId":string?,"showUI":bool=true,"dpi":integer?,"mode":"color|gray|text"="color"} → {available, devices, acquired, cancelled, document?}: discover WIA scanners or acquire into a new document (Windows only); headless acquisition requires deviceId"#,
        enabled: |_s| Ok(()),
        journal: false,
        run: wia,
    }]
}

#[cfg(test)]
mod tests;
