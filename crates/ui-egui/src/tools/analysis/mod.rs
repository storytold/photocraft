//! Image › Analysis and Notes in the shell: the Ruler, Count, Note and Color Sampler tools, Window › Measurement Log
//! and Window › Notes, and the Set Measurement Scale / Select Data Points / Place Scale Marker dialogs.
//!
//! Every change goes through engine commands (`image.analysis.*`, `count.*`, `notes.*`, `view.colorSamplers.*`,
//! `measurementLog.*`). This module root holds the view state ([`AnalysisUi`], part of the serialisable UI state so the
//! control channel can read and drive it) and the menu front ends. The tools are in [`canvas`], the options bars in
//! [`bars`], the two panels in [`log_panel`] and [`notes_panel`], the dialogs in [`dialogs`], and the note alerts in
//! [`note_alerts`].

use craft::behavior::Selection;
use photocraft_engine::color_cmds::ColorExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

pub(crate) mod bars;
pub(crate) mod canvas;
pub(crate) mod dialogs;
pub(crate) mod log_panel;
pub(crate) mod note_alerts;
pub(crate) mod notes_panel;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sampler_menu_tests;

#[cfg(test)]
#[path = "../count_readout_tests.rs"]
mod count_readout_tests;

#[cfg(test)]
#[path = "../note_show_on_new_tests.rs"]
mod note_show_on_new_tests;

#[cfg(test)]
#[path = "../count_options_bar_tests.rs"]
mod count_options_bar_tests;

#[cfg(test)]
#[path = "../count_group_name_tests.rs"]
mod count_group_name_tests;

#[cfg(test)]
#[path = "../count_size_alert_tests.rs"]
mod count_size_alert_tests;

/// The registry ids of the two panels the menu items toggle.
pub(crate) const MEASUREMENT_LOG: &str = "measurementLog";
pub(crate) const NOTES: &str = "notes";

/// The Count tool options bar's number (#11764): with one group (or none), its markers; with two
/// or more, the markers of the visible groups, then the current group's in brackets (0 while it
/// is hidden), as "5 (2)". The single-group case counts a hidden group too (not measured, so it
/// keeps the old total). An out-of-range current index means the last group, as in the Group
/// dropdown and `count.add`.
pub(crate) fn count_readout(m: &photocraft_doc::analysis::Measurement) -> String {
    if m.count_groups.len() < 2 {
        return m.count_total().to_string();
    }
    let visible: usize = m.count_groups.iter().filter(|g| g.visible).map(|g| g.points.len()).sum();
    let gi = m.active_count_group.min(m.count_groups.len().saturating_sub(1));
    let current = m.count_groups.get(gi).filter(|g| g.visible).map_or(0, |g| g.points.len());
    format!("{visible} ({current})")
}

/// View state of the analysis tools and panels. Whether the panels are shown is the workspace's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisUi {
    /// Measurement Log rows selected (row ids). The framework selection has no serde form, so it is not saved.
    #[serde(skip)]
    pub log_selected: Selection<u64>,
    /// Notes panel: the note shown (index).
    pub note_selected: Option<usize>,
    /// Note tool options: author of new notes.
    pub note_author: String,
    /// Note tool options: colour of new notes.
    pub note_color: [f32; 3],
    /// Ruler options bar: show values in the measurement scale's units.
    pub use_measurement_scale: bool,
}

impl Default for AnalysisUi {
    fn default() -> Self {
        Self {
            log_selected: Selection::default(),
            note_selected: None,
            note_author: String::new(),
            note_color: photocraft_engine::notes_cmds::DEFAULT_NOTE_COLOR,
            use_measurement_scale: false,
        }
    }
}

const PANELS: [&str; 2] = ["window.panel.measurementLog", "window.panel.notes"];

/// Menu ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    PANELS.contains(&id)
}

/// The check mark of a menu item of this module.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "window.panel.measurementLog" => Some(crate::panel_docking::is_shown(app, MEASUREMENT_LOG)),
        "window.panel.notes" => Some(crate::panel_docking::is_shown(app, NOTES)),
        "image.analysis.rulerTool" => Some(app.ui.tool == Tool::Ruler),
        "image.analysis.countTool" => Some(app.ui.tool == Tool::Count),
        _ => {
            // View › Proof Setup simulations: checked while that proof is shown.
            // A check item with no document too: an item's kind never changes (native menus).
            let kind = id.strip_prefix("view.proofSetup.").and_then(photocraft_engine::proof_sim::ProofKind::from_id)?;
            let Some(d) = app.session.active() else { return Some(false) };
            let pv = app.session.color().proof(d.doc.id);
            Some(pv.enabled && pv.setup.kind == kind)
        }
    }
}

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// Menu front ends: panel toggles, tool selection, dialogs and pickers in front of the engine's
/// analysis commands. `None` when `id` isn't ours (or the engine should run it directly).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let "window.panel.measurementLog" | "window.panel.notes" = id {
        let panel = if id.ends_with("measurementLog") { MEASUREMENT_LOG } else { NOTES };
        let show = params.get("show").and_then(Value::as_bool);
        return Some(crate::panel_docking::show_panel(app, panel, show).map(|visible| json!({"visible": visible})));
    }
    if !no_params(params) {
        return None;
    }
    Some(match id {
        "image.analysis.rulerTool" => {
            app.ui.tool = Tool::Ruler;
            Ok(json!({"tool": "Ruler"}))
        }
        "image.analysis.countTool" => {
            app.ui.tool = Tool::Count;
            Ok(json!({"tool": "Count"}))
        }
        // PLAN: the dialog names in the reply are assumed; `dialogs::open` is `Result<DialogId, String>` (shared.md) and
        // seeds the scale and marker dialogs from the active document.
        "image.analysis.setMeasurementScale" => {
            app.session.active()?;
            dialogs::open(app, dialogs::Kind::Scale).map(|_| json!({"dialog": "measurementScale"}))
        }
        "image.analysis.selectDataPoints" => dialogs::open(app, dialogs::Kind::DataPoints).map(|_| json!({"dialog": "dataPoints"})),
        "image.analysis.placeScaleMarker" => {
            app.session.active()?;
            dialogs::open(app, dialogs::Kind::ScaleMarker).map(|_| json!({"dialog": "scaleMarker"}))
        }
        "image.analysis.recordMeasurements" => {
            // The current tool picks the source, as in Photoshop.
            let source = match app.ui.tool {
                Tool::Ruler => "ruler",
                Tool::Count => "count",
                _ => "auto",
            };
            let r = app.run(id, json!({"source": source}));
            if r.is_ok() {
                let _ = crate::panel_docking::show_panel(app, MEASUREMENT_LOG, Some(true));
            }
            r
        }
        "file.import.notes" => match app.active_doc_id() {
            Ok(doc) => app.pick_file_bytes(move |app, name, bytes| {
                app.refocus(doc)?;
                let r = photocraft_engine::notes_cmds::import_notes_from(&mut app.session, &name, &bytes).map_err(|e| e.to_string());
                if r.is_ok() {
                    let _ = crate::panel_docking::show_panel(app, NOTES, Some(true));
                    app.sync_views();
                }
                r
            }),
            Err(e) => Err(e),
        },
        "measurementLog.export" => export_log(app, None),
        _ => return None,
    })
}

/// Export the Measurement Log (or the given rows) as CSV through the save picker.
pub(crate) fn export_log(app: &mut PhotocraftApp, rows: Option<Vec<u64>>) -> Result<Value, String> {
    let p = rows.map_or(json!({}), |r| json!({"rows": r}));
    let csv = app.run("measurementLog.export", p)?;
    let text = csv["csv"].as_str().unwrap_or_default().to_string();
    app.pick_save("Measurements.csv", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, text.as_bytes())?;
        Ok(json!({"path": path, "rows": csv["rows"]}))
    })
}

/// Create a note at the rounded document position using the Note tool's options.
/// After `notes.add` succeeds, select the note and open the Notes panel. Enable Show Notes
/// while leaving the master Extras switch alone.
pub(crate) fn create_note(app: &mut PhotocraftApp, x: f64, y: f64) -> Result<Value, String> {
    let a = &app.ui.analysis;
    let params = json!({"x": x.round(), "y": y.round(), "author": a.note_author, "color": a.note_color});
    let result = app.run("notes.add", params)?;
    app.ui.analysis.note_selected = result["index"].as_u64().map(|v| v as usize);
    let _ = crate::panel_docking::show_panel(app, NOTES, Some(true));
    app.ui.view.show.notes = true;
    Ok(result)
}

/// Selecting note `i` shows its author and colour in the Note tool's options bar, as Photoshop
/// does (#7940); a new note then takes them unless they are changed.
pub(crate) fn show_note_options(app: &mut PhotocraftApp, i: usize) {
    let Some(note) = app.session.active().and_then(|d| d.doc.notes.get(i)) else { return };
    let (author, color) = (note.author.clone(), note.color.to_rgb());
    app.ui.analysis.note_author = author;
    app.ui.analysis.note_color = color;
}

/// The Color Sampler point under document point `at` on the active document, by the hit test a click uses (the
/// tool's reach), so the right-click menu acts on the sampler a click would grab.
pub(crate) fn sampler_at(app: &PhotocraftApp, at: [f64; 2]) -> Option<usize> {
    if !at.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (samplers, _) = canvas::samplers_of(app)?;
    samplers.hit(at, app.tool_reach())
}

/// Show a command error in the status bar without treating the sample-limit message as a fault.
pub(super) fn status_on_err(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = false;
    }
}
