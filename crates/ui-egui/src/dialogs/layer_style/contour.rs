//! Layer Style contours: the engine's contour parameters as the framework's `Contour`, the user's
//! contour library, and the Contour Editor opened as a child of the Layer Style dialog.

use craft::imaging::layer_style::{Contour, ContourPoint};
use craft::imaging::ui::layer_style::{ContourAction, ContourEditorDialog, ContourRequest, HostedLayerStyle};
use craft::ui::dialog::{DialogId, DialogResult};
use photocraft_doc::Contour as DocContour;
use photocraft_doc::adjust::CurvePoint;
use photocraft_engine::layer_style::{contour_from_param, contour_param, contour_to_shc};
use photocraft_engine::presets::PresetsExt;
use serde_json::{Value, json};

use super::{Child, child_of, take_child};
use crate::PhotocraftApp;

/// The levels of the contour editor's curve (it plots `0..=LEVELS` on both axes).
const LEVELS: f32 = 255.0;

fn from_doc(c: &DocContour) -> Contour {
    match c {
        DocContour::Linear => Contour::linear(),
        DocContour::Custom { name, points, .. } => Contour {
            name: name.clone(),
            points: points.iter().enumerate().map(|(i, p)| ContourPoint { input: p.input, output: p.output, corner: c.is_corner(i) }).collect(),
        },
    }
}

fn to_doc(c: &Contour) -> DocContour {
    if *c == Contour::linear() {
        return DocContour::Linear;
    }
    DocContour::Custom {
        name: c.name.clone(),
        points: c.points.iter().map(|p| CurvePoint { input: p.input, output: p.output }).collect(),
        corners: c.points.iter().map(|p| p.corner).collect(),
    }
}

fn name_of(c: &DocContour) -> String {
    match c {
        DocContour::Linear => "Linear".into(),
        DocContour::Custom { name, .. } => name.clone(),
    }
}

/// The contour a command parameter names (a preset's name or a curve object).
pub(super) fn from_param(v: &Value) -> Option<Contour> {
    contour_from_param(v).map(|c| from_doc(&c))
}

/// The command parameter for `c`: a preset's name when it is that preset, else the curve itself.
pub(super) fn to_param(c: &Contour) -> Value {
    contour_param(&to_doc(c))
}

/// The user's contour library.
pub(super) fn user_contours(app: &PhotocraftApp) -> Vec<Contour> {
    app.session.presets().contours.iter().map(from_doc).collect()
}

/// The engine's interpolation of a contour's points (levels `0..=255` on both axes) as one output
/// level per input level.
pub(super) fn sampler(points: &[[f32; 2]]) -> Vec<f32> {
    let points: Vec<CurvePoint> = points.iter().map(|p| CurvePoint { input: p[0] / LEVELS, output: p[1] / LEVELS }).collect();
    let lut = photocraft_compose::adjust::contour_curve_lut(&points, &[]);
    (0..=LEVELS as usize)
        .map(|level| {
            let at = (level as f32 / LEVELS * lut.len().saturating_sub(1) as f32).round() as usize;
            lut.get(at).copied().unwrap_or(0.0) * LEVELS
        })
        .collect()
}

/// Opens the Contour Editor on `request` as a child of the Layer Style dialog `parent`.
pub(super) fn open_editor(app: &mut PhotocraftApp, parent: DialogId, request: ContourRequest) {
    let (key, initial) = (request.key.clone(), request.contour.clone());
    let id = app.framework.dialogs.open_child(ContourEditorDialog::new(request).sampler(sampler), parent);
    app.layer_style_child = Some((parent, Child::Editor { id, key, initial }));
}

/// Once per pass: while the editor is open its edits are the draft's contour (the preview follows)
/// and its library asks are answered; when it is gone, OK keeps its contour and Cancel restores the
/// one it opened with.
pub(super) fn answered(app: &mut PhotocraftApp, parent: DialogId) {
    let Some(Child::Editor { id, key, initial }) = child_of(app, parent).cloned() else { return };
    if let Some(open) = app.framework.dialogs.get_mut_by_id::<ContourEditorDialog>(id) {
        let action = open.take_action();
        let live = open.changed().then(|| open.current().clone());
        if let Some(contour) = live {
            set_contour(app, parent, &key, contour);
        }
        if let Some(action) = action {
            library_action(app, action);
        }
        return;
    }
    take_child(app, parent);
    match app.framework.dialogs.take_by_id::<ContourEditorDialog>(id) {
        Some(DialogResult::Ok(contour)) => set_contour(app, parent, &key, contour),
        _ => set_contour(app, parent, &key, initial),
    }
}

fn set_contour(app: &mut PhotocraftApp, parent: DialogId, key: &str, contour: Contour) {
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(parent) {
        dialog.set_contour(key, contour);
    }
}

/// Gives the open Layer Style dialog the library as it is now.
fn refresh(app: &mut PhotocraftApp) {
    let list = user_contours(app);
    if let Some((id, ..)) = app.layer_style_target
        && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id)
    {
        dialog.set_user_contours(list);
    }
}

/// Load, Replace, Save, Delete or New for the user's contours: the library lives in the session,
/// the file is an `.shc`.
pub(super) fn library_action(app: &mut PhotocraftApp, action: ContourAction) {
    let r = match action {
        ContourAction::Load | ContourAction::Replace => {
            let replace = action == ContourAction::Replace;
            app.pick_file_bytes_filtered(&["shc"], move |app, name, bytes| {
                let mut p = crate::swatches_ui::source(&name, &bytes);
                p["mode"] = json!(if replace { "replace" } else { "append" });
                let r = app.run("layer.layerStyle.contours.load", p)?;
                refresh(app);
                let template = "Loaded contours from {file}";
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!(template), &[("file", &crate::file_open::display_name(&name))]);
                Ok(r)
            })
        }
        ContourAction::Save => app.run("layer.layerStyle.contours.save", json!({})).and_then(|r| {
            let bytes = r["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("the export returned no data")?;
            app.pick_save("Contours.shc", move |app, path| {
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, &bytes)?;
                let template = "Saved contours to {file}";
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!(template), &[("file", &crate::file_open::display_name(&path))]);
                Ok(json!({"path": path}))
            })
        }),
        ContourAction::Delete(i) => match app.session.presets().contours.get(i).map(name_of) {
            Some(name) => app.run("layer.layerStyle.contours.delete", json!({"name": name})),
            None => Err("no such contour".into()),
        },
        ContourAction::New(contour) => app.run("layer.layerStyle.contours.new", json!({"contour": to_param(&contour), "name": contour.name})),
        // The editor is opened through `LayerStyleAsk::EditContour`.
        _ => return,
    };
    refresh(app);
    report(app, r);
}

/// The Contour Editor's Save…: the contour as an `.shc` file.
#[allow(dead_code)]
fn contour_file(c: &Contour) -> Result<Vec<u8>, String> {
    contour_to_shc(&to_doc(c)).map_err(|e| e.to_string())
}

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status_error = true;
        app.ui.status = e;
    }
}
