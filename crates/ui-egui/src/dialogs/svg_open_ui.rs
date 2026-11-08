//! Opening an SVG asks for its size first (#8910): an SVG has no pixel size of its own, so a
//! small icon would otherwise open small. Width and Height start at the drawing's own size and
//! keep its proportions; OK opens the drawing scaled to them, its shapes still shapes, so it stays
//! sharp at any size.
//!
//! Only files picked in File › Open or dropped on the window ask. `file.open {"path"}` from a
//! script or agent never opens a dialog: it opens at the drawing's own size, or at `"svgScale"` ×
//! it. The drawing's size is read in a background job, like the open itself, so a large file
//! doesn't hold up the window. The dialog is an ordinary one in the UI state, so `ui.inspect`,
//! `ui.dialog.set` and `ui.dialog.confirm` drive it.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__svgOpen";
/// The drawing's own size in px.
const NATURAL: &str = "__natural";
/// The tab slot a dropped file goes to.
const SLOT: &str = "__slot";
/// The job that reads a drawing's size before the dialog opens.
pub const MEASURE_JOB: &str = "file.open.svgSize";

/// An SVG whose size is being read: the job, the file and the tab slot it was dropped on.
pub struct Measure {
    pub job: photocraft_engine::jobs::JobId,
    pub path: String,
    pub slot: Option<usize>,
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Does opening the file at `path` from File › Open or a drop ask for its size?
pub fn asks(path: &str) -> bool {
    cfg!(not(target_arch = "wasm32")) && std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
}

/// Ask for the size of the SVG at `path` (its tab going to `slot`), then open it. A file already
/// open is shown instead. With background jobs on, the drawing's size is read on a worker and
/// the dialog opens when it lands ([`on_measure_event`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn ask(app: &mut PhotocraftApp, path: &str, slot: Option<usize>) -> Result<(), String> {
    if app.show_if_open(path) {
        return Ok(());
    }
    let read = |p: &str| -> photocraft_engine::Result<(f32, f32)> {
        let bytes = photocraft_format::read_file(std::path::Path::new(p)).map_err(|e| photocraft_engine::EngineError::Other(format!("{p}: {e}")))?;
        photocraft_io::svg::drawing_size(&bytes).map_err(|e| photocraft_engine::EngineError::Other(e.to_string()))
    };
    if !app.background_jobs {
        return match read(path) {
            Ok(size) => {
                open_dialog(app, path, slot, size);
                Ok(())
            }
            // The usual open reports why the file can't be read.
            Err(_) => app.open_path_placed(path, slot, None).map(|_| ()),
        };
    }
    let p = path.to_string();
    let work = move |_: &photocraft_engine::jobs::JobCtx| read(&p);
    let apply = |_: &mut photocraft_engine::Session, (w, h): (f32, f32)| Ok(json!([w, h]));
    let label = crate::i18n::fmt(tl!("Reading {name}"), &[("name", &crate::file_open::display_name(path))]);
    match app.session.start_job(MEASURE_JOB, json!({"path": path}), &label, false, work, apply).map_err(|e| e.to_string())? {
        photocraft_engine::jobs::Started::Done(v) => finish_measure(app, path, slot, Ok(&v)),
        photocraft_engine::jobs::Started::Job(job) => app.svg_measures.push(Measure { job, path: path.to_string(), slot }),
    }
    Ok(())
}

/// The web opens files from their bytes and never asks ([`asks`] is false there).
#[cfg(target_arch = "wasm32")]
pub fn ask(app: &mut PhotocraftApp, path: &str, slot: Option<usize>) -> Result<(), String> {
    app.open_path_placed(path, slot, None).map(|_| ())
}

/// A finished [`MEASURE_JOB`]: opens the dialog, or (when the file can't be read as an SVG) the
/// usual open, which reports why. A cancelled read opens nothing. `true` when the event was one
/// of these jobs.
pub fn on_measure_event(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    use photocraft_engine::jobs::JobOutcome;
    let Some(at) = app.svg_measures.iter().position(|m| m.job == e.id) else { return false };
    let m = app.svg_measures.remove(at);
    match &e.outcome {
        JobOutcome::Done(v) => finish_measure(app, &m.path, m.slot, Ok(v)),
        JobOutcome::Failed(_) => finish_measure(app, &m.path, m.slot, Err(())),
        JobOutcome::Cancelled => {}
    }
    true
}

fn finish_measure(app: &mut PhotocraftApp, path: &str, slot: Option<usize>, size: Result<&Value, ()>) {
    let dim = |v: &Value, i: usize| v.get(i).and_then(Value::as_f64).filter(|x| x.is_finite() && *x > 0.0).map(|x| x as f32);
    match size.ok().and_then(|v| dim(v, 0).zip(dim(v, 1))) {
        Some(size) => open_dialog(app, path, slot, size),
        None => {
            if let Err(e) = app.open_path_placed(path, slot, None) {
                app.open_failed(&crate::file_open::display_name(path), &e);
            }
        }
    }
}

fn open_dialog(app: &mut PhotocraftApp, path: &str, slot: Option<usize>, (w, h): (f32, f32)) {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!("file.open"));
    f.insert("__label".into(), json!(tl!("Open SVG")));
    f.insert("path".into(), json!(path));
    f.insert(NATURAL.into(), json!([w, h]));
    if let Some(slot) = slot {
        f.insert(SLOT.into(), json!(slot));
    }
    f.insert("width".into(), json!(w.round().max(1.0)));
    f.insert("height".into(), json!(h.round().max(1.0)));
    app.ui.open_dialog(DialogKind::Command, f);
}

fn natural(f: &Map<String, Value>) -> Option<(f32, f32)> {
    let get = |i: usize| f.get(NATURAL).and_then(|v| v.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0).map(|v| v as f32);
    get(0).zip(get(1))
}

/// The scale the dialog's Width and Height ask for. They must be sizes of at least 1 px that keep
/// the drawing's proportions (within the rounding of a whole pixel each).
fn scale(f: &Map<String, Value>) -> Result<f32, String> {
    let (nw, nh) = natural(f).ok_or("the drawing's size is unknown")?;
    let side = |k: &str| f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 1.0).map(|v| v as f32);
    let (w, h) = side("width").zip(side("height")).ok_or_else(|| tl!("Width and Height must be at least 1 px").to_string())?;
    let (kw, kh) = (w / nw, h / nh);
    // Each side was rounded to a whole pixel, so they may disagree by up to half a pixel each.
    if (kw - kh).abs() > 0.5 / nw + 0.5 / nh {
        return Err(tl!("Width and Height must keep the drawing's proportions").to_string());
    }
    Ok(kw)
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let Some((nw, nh)) = natural(f) else { return };
    let max = photocraft_io::svg::MAX_SIDE as f32;
    let mut w = f.get("width").and_then(Value::as_f64).unwrap_or(f64::from(nw)) as f32;
    let mut h = f.get("height").and_then(Value::as_f64).unwrap_or(f64::from(nh)) as f32;
    let name = f.get("path").and_then(Value::as_str).map(crate::file_open::display_name).unwrap_or_default();
    ui.label(name);
    ui.add_space(6.0);
    egui::Grid::new("svg-open-dialog").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Width"));
        // Proportions are kept: Width and Height move together.
        if crate::widgets::value_field(ui, &mut w, 1.0..=max, "px", 110.0).changed() {
            h = (w * nh / nw).round().max(1.0);
        }
        ui.end_row();
        ui.label(tl!("Height"));
        if crate::widgets::value_field(ui, &mut h, 1.0..=max, "px", 110.0).changed() {
            w = (h * nw / nh).round().max(1.0);
        }
        ui.end_row();
    });
    f.insert("width".into(), json!(w.round()));
    f.insert("height".into(), json!(h.round()));
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let path = f.get("path").and_then(Value::as_str).ok_or("no file to open")?.to_string();
    let slot = f.get(SLOT).and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok());
    match scale(f) {
        Ok(k) => app.open_svg_scaled(&path, k, slot).map(|warnings| json!({"warnings": warnings})),
        Err(e) => {
            // `dialogs::confirm` has already closed it: open it again with the typed values.
            let mut again = f.clone();
            again.insert("__error".into(), json!(e));
            app.ui.open_dialog(DialogKind::Command, again);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_and_height_set_the_scale_and_svg_files_ask() {
        let mut f = Map::new();
        f.insert(NATURAL.into(), json!([20.0, 10.0]));
        f.insert("width".into(), json!(80.0));
        f.insert("height".into(), json!(40.0));
        assert_eq!(scale(&f), Ok(4.0));
        // A missing, hostile or too small side is an error, not a silent 1×.
        for bad in [json!(null), json!("x"), json!(0), json!(-5), json!(f64::NAN)] {
            f.insert("width".into(), bad.clone());
            assert!(scale(&f).is_err(), "{bad}");
        }
        // Width and Height that don't keep the proportions are refused; whole-pixel rounding isn't.
        f.insert("width".into(), json!(80.0));
        f.insert("height".into(), json!(10.0));
        assert!(scale(&f).is_err());
        let mut odd = Map::new();
        odd.insert(NATURAL.into(), json!([3.0, 7.0]));
        odd.insert("width".into(), json!(10.0));
        odd.insert("height".into(), json!(23.0));
        assert!(scale(&odd).is_ok(), "23 is 23.33 rounded");
        assert!(scale(&Map::new()).is_err());
        assert_eq!(asks("/a/Icon.SVG"), cfg!(not(target_arch = "wasm32")));
        assert!(!asks("/a/icon.png"));
    }

    /// Cancelling the "Reading …" job opens nothing; only a failed read falls back to the usual
    /// open, which says why.
    #[test]
    fn a_cancelled_read_opens_nothing_and_a_failed_one_falls_back() {
        use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
        let dir = std::env::temp_dir().join(format!("pc-svg-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.svg");
        std::fs::write(&path, br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"/>"#).unwrap();
        let path = path.to_string_lossy().to_string();
        let event =
            |id: u64, outcome: JobOutcome| JobEvent { id: JobId(id), command: MEASURE_JOB.into(), label: String::new(), document: None, outcome, error: None };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The fallback open on the window thread, so the test sees what it did.
        app.background_jobs = false;
        app.svg_measures.push(Measure { job: JobId(1), path: path.clone(), slot: None });
        assert!(on_measure_event(&mut app, &event(1, JobOutcome::Cancelled)));
        assert!(app.svg_measures.is_empty());
        assert!(app.session.documents().is_empty() && app.ui.dialogs.is_empty() && app.ui.status.is_empty(), "a cancel opens nothing: {}", app.ui.status);
        app.svg_measures.push(Measure { job: JobId(2), path, slot: None });
        assert!(on_measure_event(&mut app, &event(2, JobOutcome::Failed("x".into()))));
        // The test app has no importer, so the usual open reports that: it was tried.
        assert!(app.ui.status.contains("icon.svg"), "a failed read opens the file the usual way: {}", app.ui.status);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
