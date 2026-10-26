//! Opening an SVG asks for its size first (#8910), as Photoshop's Rasterize SVG does: an SVG has
//! no pixel size of its own, so a small icon would otherwise open small. Width and Height start at
//! the drawing's own size and keep its proportions; OK opens the drawing scaled to them, its
//! shapes still shapes, so it stays sharp at any size.
//!
//! Only files picked in File › Open or dropped on the window ask. `file.open {"path"}` from a
//! script or agent never opens a dialog: it opens at the drawing's own size, or at
//! `"svgScale"` × it. The dialog is an ordinary one in the UI state, so `ui.inspect`,
//! `ui.dialog.set` and `ui.dialog.confirm` drive it.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

const MARK: &str = "__svgOpen";
/// The drawing's own size in px.
const NATURAL: &str = "__natural";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Does opening the file at `path` from File › Open or a drop ask for its size?
pub fn asks(path: &str) -> bool {
    cfg!(not(target_arch = "wasm32")) && std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
}

/// Ask for the size of the SVG at `path`, then open it. A file already open is shown instead, and
/// one that can't be read opens the usual way, which reports the error.
pub fn ask(app: &mut PhotocraftApp, path: &str) -> Result<Vec<String>, String> {
    if app.show_if_open(path) {
        return Ok(Vec::new());
    }
    let Some((w, h)) = natural_size(path).filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) else {
        return app.open_path(path);
    };
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!("file.open"));
    f.insert("__label".into(), json!("Open SVG"));
    f.insert("path".into(), json!(path));
    f.insert(NATURAL.into(), json!([w, h]));
    f.insert("width".into(), json!(w.round().max(1.0)));
    f.insert("height".into(), json!(h.round().max(1.0)));
    app.ui.open_dialog(DialogKind::Command, f);
    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
fn natural_size(path: &str) -> Option<(f32, f32)> {
    let bytes = photocraft_format::read_file(std::path::Path::new(path)).ok()?;
    photocraft_io::svg::drawing_size(&bytes).ok()
}

#[cfg(target_arch = "wasm32")]
fn natural_size(_: &str) -> Option<(f32, f32)> {
    None
}

fn natural(f: &Map<String, Value>) -> (f32, f32) {
    let get = |i: usize| f.get(NATURAL).and_then(|v| v.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1.0) as f32;
    (get(0), get(1))
}

/// The scale the dialog's Width asks for (Height follows it).
fn scale(f: &Map<String, Value>) -> f32 {
    let (w, _) = natural(f);
    let width = f.get("width").and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(f64::from(w)) as f32;
    width / w
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let (nw, nh) = natural(f);
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
    app.open_svg_scaled(&path, scale(f)).map(|warnings| json!({"warnings": warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_sets_the_scale_and_svg_files_ask() {
        let mut f = Map::new();
        f.insert(NATURAL.into(), json!([20.0, 10.0]));
        f.insert("width".into(), json!(80.0));
        assert_eq!(scale(&f), 4.0);
        // A missing or hostile width opens at the drawing's own size.
        for bad in [json!(null), json!("x"), json!(0), json!(-5)] {
            f.insert("width".into(), bad.clone());
            assert_eq!(scale(&f), 1.0, "{bad}");
        }
        assert_eq!(natural(&Map::new()), (1.0, 1.0));
        assert_eq!(asks("/a/Icon.SVG"), cfg!(not(target_arch = "wasm32")));
        assert!(!asks("/a/icon.png"));
    }
}
