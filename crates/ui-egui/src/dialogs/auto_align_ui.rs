//! Edit › Auto-Align Layers: the reference application's dialog (#14196). A Projection group of six radios with a
//! small picture under each (3 x 2), a Lens Correction group with Vignette Removal and Geometric Distortion, and a
//! text block at the foot: the selected projection's description and the warnings. OK and Cancel; Alt does not turn
//! Cancel into Reset.
//!
//! It stays the command's schema dialog (`filter_dialog::open`), so Enter / Escape, `ui.dialog.*`, action step
//! dialogs and the no-match alert work as for every other command dialog; only its rows and buttons differ. The
//! command keeps its `interpolation` and `registration` params for scripts: the dialog has no rows for them, so the
//! command's own defaults apply.
//!
//! Memory: the projection lives in a hidden radio row ([`LAST`]) the framework remembers on OK beside the two boxes.
//! A run that did not come from this dialog (a script, the control channel, an action step) lands in the session's
//! journal; opening the dialog copies the last journaled run into those remembered values, so it reopens with the
//! projection (and the lens keys) that run used.

use craft::behavior::ColumnWidth;
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Grid, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};
use serde_json::{Map, Value};

use crate::PhotocraftApp;
use crate::theme::Tokens;

pub const COMMAND: &str = "edit.autoAlignLayers";
/// The projection the command runs with. The custom row writes it; an agent's `ui.dialog.set` may too.
pub const PROJECTION: &str = "projection";
/// The hidden radio row that keeps the projection the framework remembers (a custom row's value is not remembered).
pub const LAST: &str = "__projection";
/// The Geometric Distortion box: the command's `geometricCorrection`.
pub const GEOMETRIC: &str = "geometricCorrection";
/// The Vignette Removal box: the command's `vignetteRemoval`, once the engine takes it (#14197).
pub const VIGNETTE: &str = "vignetteRemoval";

/// The projections in the reference application's order, row by row over three columns: id, label and the
/// description line it shows while selected.
pub const PROJECTIONS: [(&str, &str, &str); 6] = [
    ("auto", "Auto", "Determine the best projection automatically."),
    ("perspective", "Perspective", "Only allow perspective image transform."),
    ("collage", "Collage", "Allow image rotation, scale and translation."),
    ("cylindrical", "Cylindrical", "Only allow cylindrical image transform."),
    ("spherical", "Spherical", "Only allow spherical image transform."),
    ("reposition", "Reposition", "Only allow image translation."),
];
/// The projections that grey and clear both Lens Correction boxes.
const NO_LENS: [&str; 2] = ["collage", "reposition"];
const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];
/// The picture under each radio, in points.
const PICTURE: egui::Vec2 = vec2(64.0, 40.0);

/// The text under the boxes when the layers carry no camera metadata.
pub const METADATA_WARNING: &str = "Warning: Could not detect camera metadata. Use the \"File > Scripts > Load Images into Stack...\" script to preserve metadata for Vignette Removal and/or Geometric Distortion.";
/// The text under the boxes while Vignette Removal is ticked.
pub const VIGNETTE_WARNING: &str =
    "Warning: For better Vignette Removal results, shoot with the same camera, lens, aperture, resolution, orientation, focal length and focus settings.";

/// `id` when it names one of the six projections.
fn known(id: &str) -> Option<&'static str> {
    PROJECTIONS.iter().map(|(p, _, _)| *p).find(|p| *p == id)
}

/// Whether the projection `id` lets the Lens Correction boxes take input.
pub fn lens_allowed(id: &str) -> bool {
    !NO_LENS.contains(&id)
}

/// Whether the engine's command takes `vignetteRemoval` yet (#14197); until then the box is shown disabled.
pub fn vignette_supported() -> bool {
    photocraft_engine::commands::find(COMMAND).is_some_and(|c| Param::parse_list(c.params).iter().any(|p| p.key == VIGNETTE))
}

/// The projection the dialog's values hold: the row's own value (text, or the JSON string `ui.dialog.set` writes),
/// else the remembered one, else Auto.
pub fn projection(fields: &DialogFields) -> &'static str {
    let own = match fields.get(PROJECTION) {
        Some(FieldValue::Text(id)) => known(id),
        Some(FieldValue::Json(Value::String(id))) => known(id),
        _ => None,
    };
    own.or_else(|| fields.text(LAST).and_then(known)).unwrap_or("auto")
}

/// The save key the schema dialog remembers this command's values under.
fn remembered_key() -> SaveKey {
    SaveKey::named(super::params::memory_scope(COMMAND))
}

/// Whether the active document carries camera metadata (a camera make or model in its EXIF).
fn has_camera_metadata(app: &PhotocraftApp) -> bool {
    app.session.active().and_then(|st| st.doc.metadata.exif.as_ref()).is_some_and(|exif| {
        let info = photocraft_algo::exif::read(exif);
        info.make.is_some_and(|m| !m.trim().is_empty()) || info.model.is_some_and(|m| !m.trim().is_empty())
    })
}

/// Copies the session's last journaled run of the command into the values the dialog remembers, so a run from a
/// script, the control channel or an action step decides what the dialog reopens with, as one confirmed with OK
/// does. Keys the run left out are their defaults: a scripted run without the lens keys leaves both boxes clear.
pub(crate) fn adopt_last_run(app: &PhotocraftApp) {
    let Some((_, params)) = app.session.journal.iter().rev().find(|(id, _)| id == COMMAND) else { return };
    let ctx = app.ctx();
    let key = remembered_key();
    let mut saved = saved_value::<Option<DialogFields>>(ctx, &key).unwrap_or_default();
    let id = params.get(PROJECTION).and_then(Value::as_str).and_then(known).unwrap_or("auto");
    let lens = lens_allowed(id);
    let flag = |k: &str| lens && params.get(k).and_then(Value::as_bool).unwrap_or(false);
    saved.set(LAST, FieldValue::Text(id.to_owned()));
    saved.set(GEOMETRIC, FieldValue::Bool(flag(GEOMETRIC)));
    saved.set(VIGNETTE, FieldValue::Bool(flag(VIGNETTE)));
    set_saved_value(ctx, &key, Some(saved));
}

/// The dialog's rows, in place of the schema's: the Projection group, the Lens Correction group and the text block.
/// Opening also adopts the last journaled run ([`adopt_last_run`]).
pub(crate) fn params(app: &PhotocraftApp) -> Vec<Param> {
    adopt_last_run(app);
    let metadata = has_camera_metadata(app);
    let lens: Vec<String> = PROJECTIONS.iter().map(|(id, _, _)| *id).filter(|id| lens_allowed(id)).map(str::to_owned).collect();
    let vignette_on: Vec<String> = if vignette_supported() { lens.clone() } else { Vec::new() };
    let options = PROJECTIONS.iter().map(|(id, label, _)| ((*id).to_owned(), Label::from(crate::i18n::t(label).to_string()))).collect();
    vec![
        Param::section(tl!("Projection")),
        // The remembered projection: never shown, never sent (keys under `__` are not command params).
        Param::new(LAST, tl!("Projection"), ParamKind::radio(options, Some("auto".into()))).visible_when("__hidden", true),
        Param::custom(PROJECTION, tl!("Projection"), |ui, fields, _| projection_row(ui, fields)).full_width(),
        Param::section(tl!("Lens Correction")),
        Param::new(VIGNETTE, tl!("Vignette Removal"), ParamKind::bool(false)).enabled_when_in(LAST, vignette_on),
        Param::new(GEOMETRIC, tl!("Geometric Distortion"), ParamKind::bool(false)).enabled_when_in(LAST, lens),
        Param::custom("__notes", Label::Empty, move |ui, fields, _| notes(ui, fields, metadata)).full_width(),
    ]
}

/// The command params of the dialog's values: the projection the row shows (the remembered one when the row never
/// drew), no lens correction under Collage or Reposition, and no `vignetteRemoval` until the engine takes it.
pub(crate) fn command_params(map: &mut Map<String, Value>, fields: &DialogFields) {
    let id = projection(fields);
    map.insert(PROJECTION.into(), Value::String(id.to_owned()));
    if !lens_allowed(id) {
        map.insert(GEOMETRIC.into(), Value::Bool(false));
        map.insert(VIGNETTE.into(), Value::Bool(false));
    }
    if !vignette_supported() {
        map.remove(VIGNETTE);
    }
}

/// Keeps the values in step with the projection: both keys hold it as text, and Collage or Reposition clears the
/// Lens Correction boxes (switching back leaves them clear).
fn settle(fields: &mut DialogFields, id: &str) {
    if fields.get(PROJECTION) != Some(&FieldValue::Text(id.to_owned())) {
        fields.set(PROJECTION, FieldValue::Text(id.to_owned()));
    }
    if fields.text(LAST) != Some(id) {
        fields.set(LAST, FieldValue::Text(id.to_owned()));
    }
    if !lens_allowed(id) {
        for key in [GEOMETRIC, VIGNETTE] {
            if fields.bool(key) != Some(false) {
                fields.set(key, FieldValue::Bool(false));
            }
        }
    }
}

/// The six radios over three columns, each with its picture under it. A press on a picture selects nothing.
fn projection_row(ui: &mut egui::Ui, fields: &mut DialogFields) -> egui::Response {
    let current = projection(fields);
    settle(fields, current);
    let group = RadioGroup::new(tl!("Projection").to_owned(), Bind::given(current.to_owned())).show(ui, |ui, _| {
        Grid::new(COLUMNS).gap(Gap::Section).show(ui, |ui, _| {
            for (id, label, _) in PROJECTIONS {
                Stack::new().show(ui, |ui, _| {
                    Radio::new(id.to_owned()).label(crate::i18n::t(label).to_string()).show(ui);
                    picture(ui, id);
                });
            }
        });
    });
    if let Some(picked) = group.output.value().and_then(|id| known(id)) {
        settle(fields, picked);
    }
    group.output.response.clone()
}

/// The lines of the text block at the foot, in English: the projection `id`'s description, the metadata warning when
/// the layers carry no camera `metadata`, then the vignette warning while `vignette` is ticked.
pub fn lines(id: &str, vignette: bool, metadata: bool) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = PROJECTIONS.iter().filter(|(p, _, _)| *p == id).map(|(_, _, d)| *d).collect();
    if !metadata {
        out.push(METADATA_WARNING);
    }
    if vignette {
        out.push(VIGNETTE_WARNING);
    }
    out
}

/// The text block at the foot ([`lines`]): the description, then the warnings, dimmer.
fn notes(ui: &mut egui::Ui, fields: &mut DialogFields, metadata: bool) -> egui::Response {
    let shown = lines(projection(fields), fields.bool(VIGNETTE) == Some(true), metadata);
    Stack::new()
        .show(ui, |ui, _| {
            for (k, line) in shown.iter().enumerate() {
                let text = Text::new(crate::i18n::t(line).to_string());
                if k == 0 {
                    text.show(ui)
                } else {
                    text.shade(Shade::Muted).show(ui)
                };
            }
        })
        .output
}

/// The small line drawing under a projection's radio (look only; it takes no press).
fn picture(ui: &mut egui::Ui, id: &str) {
    let (rect, _) = ui.allocate_exact_size(PICTURE, egui::Sense::hover());
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    let ink = Stroke::new(1.0, t.icon);
    let fill = t.accent_soft;
    let c = rect.center();
    let image = Rect::from_center_size(c, vec2(22.0, 15.0));
    let poly = |points: Vec<Pos2>| Shape::convex_polygon(points, fill, ink);
    match id {
        "auto" => {
            painter.rect(image, 0.0, fill, ink, egui::StrokeKind::Inside);
            for (corner, out) in [
                (image.left_top(), vec2(-6.0, -5.0)),
                (image.right_top(), vec2(6.0, -5.0)),
                (image.left_bottom(), vec2(-6.0, 5.0)),
                (image.right_bottom(), vec2(6.0, 5.0)),
            ] {
                painter.arrow(corner, out, ink);
            }
        }
        "perspective" => {
            painter.add(poly(vec![pos2(c.x - 26.0, c.y - 6.0), pos2(c.x - 12.0, c.y - 10.0), pos2(c.x - 12.0, c.y + 10.0), pos2(c.x - 26.0, c.y + 6.0)]));
            painter.rect(Rect::from_center_size(c, vec2(22.0, 20.0)), 0.0, fill, ink, egui::StrokeKind::Inside);
            painter.add(poly(vec![pos2(c.x + 12.0, c.y - 10.0), pos2(c.x + 26.0, c.y - 6.0), pos2(c.x + 26.0, c.y + 6.0), pos2(c.x + 12.0, c.y + 10.0)]));
        }
        "collage" => {
            let turn = egui::emath::Rot2::from_angle(-0.3);
            let corners = [vec2(-11.0, -7.5), vec2(11.0, -7.5), vec2(11.0, 7.5), vec2(-11.0, 7.5)];
            painter.add(poly(corners.iter().map(|v| c + turn * *v).collect()));
            let (a, b) = (pos2(c.x + 10.0, c.y + 14.0), pos2(c.x + 24.0, c.y + 4.0));
            painter.arrow(a, b - a, ink);
            painter.arrow(b, a - b, ink);
        }
        "cylindrical" => {
            let mut top: Vec<Pos2> =
                (0..=8).map(|k| k as f32 / 8.0).map(|s| pos2(c.x - 18.0 + 36.0 * s, c.y - 10.0 + 4.0 * (s - 0.5).powi(2) * 4.0 - 4.0)).collect();
            let bottom: Vec<Pos2> =
                (0..=8).rev().map(|k| k as f32 / 8.0).map(|s| pos2(c.x - 18.0 + 36.0 * s, c.y + 10.0 - 4.0 * (s - 0.5).powi(2) * 4.0 + 4.0)).collect();
            top.extend(bottom);
            painter.add(Shape::closed_line(top.clone(), ink));
            painter.add(Shape::line(vec![pos2(c.x, c.y - 14.0), pos2(c.x, c.y + 14.0)], Stroke::new(1.0, t.text_faint)));
        }
        "spherical" => {
            // An image whose corners are pulled in: each side bows inward.
            let (l, r, tp, b) = (c.x - 16.0, c.x + 16.0, c.y - 12.0, c.y + 12.0);
            let side = |from: Pos2, to: Pos2, bow: egui::Vec2| -> Vec<Pos2> {
                (0..8).map(|k| k as f32 / 8.0).map(|s| from + (to - from) * s - bow * (1.0 - (2.0 * s - 1.0).powi(2))).collect()
            };
            let mut outline = side(pos2(l, tp), pos2(r, tp), vec2(0.0, -4.0));
            outline.extend(side(pos2(r, tp), pos2(r, b), vec2(4.0, 0.0)));
            outline.extend(side(pos2(r, b), pos2(l, b), vec2(0.0, 4.0)));
            outline.extend(side(pos2(l, b), pos2(l, tp), vec2(-4.0, 0.0)));
            painter.add(Shape::closed_line(outline, ink));
        }
        _ => {
            // Reposition: the image with blocks around it.
            painter.rect(image, 0.0, fill, ink, egui::StrokeKind::Inside);
            for offset in [vec2(-20.0, -9.0), vec2(20.0, -9.0), vec2(-20.0, 9.0), vec2(20.0, 9.0)] {
                painter.rect(Rect::from_center_size(c + offset, vec2(10.0, 8.0)), 0.0, Color32::TRANSPARENT, ink, egui::StrokeKind::Inside);
            }
        }
    }
}
