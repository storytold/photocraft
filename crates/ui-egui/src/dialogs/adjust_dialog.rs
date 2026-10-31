//! Modal Image › Adjustments dialogs (Levels, Curves, Hue/Saturation, …) built from the same
//! editors as the adjustment-layer Properties panel (`adjust_editors`).
//!
//! The dialog's fields are the command's complete parameter set (so automation can read and set
//! them with `ui.dialog.set`) plus private `__` keys. While it is open the canvas previews the
//! settings as a temporary adjustment layer clipped to the target (`adjust_preview`, composited on
//! the GPU); where that can't match the command, the *real command* runs on the proxy document
//! through the filter-preview machinery (`__filter` + `__preview`). OK runs the command once (one
//! history step) and Cancel drops the preview, leaving the document untouched.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::adjust_editors::{self, EditorCx};
use crate::tone::{self, HistSource};

const PREFIX: &str = "image.adjustments.";
const CURVE_PICKER: &str = "__curvePicker";
const CURVE_PICKER_BASE: &str = "__curvePickerBase";

fn picker(value: Option<&Value>) -> Option<photocraft_engine::adjust_params::CurvesEyedropper> {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    match value.and_then(Value::as_str) {
        Some("black") => Some(CurvesEyedropper::Black),
        Some("gray") => Some(CurvesEyedropper::NeutralGray),
        Some("white") => Some(CurvesEyedropper::White),
        _ => None,
    }
}

pub(crate) fn picker_supported(mode: photocraft_doc::ColorMode, eyedropper: photocraft_engine::adjust_params::CurvesEyedropper) -> bool {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    mode == photocraft_doc::ColorMode::Rgb || (mode == photocraft_doc::ColorMode::Grayscale && eyedropper != CurvesEyedropper::NeutralGray)
}

/// The kind of an adjustment dialog with Set Black/Neutral Gray/White Point eyedroppers (Levels,
/// Curves), if `fields` are one.
fn picker_kind(fields: &Map<String, Value>) -> Option<&str> {
    fields.get("__adjust").and_then(Value::as_str).filter(|k| matches!(*k, "levels" | "curves"))
}

/// Whether the top modal has a supported Levels or Curves eyedropper armed.
pub fn picker_armed(app: &PhotocraftApp) -> bool {
    let Some(dialog) = app.ui.dialogs.last().filter(|d| picker_kind(&d.fields).is_some()) else { return false };
    let Some(eyedropper) = picker(dialog.fields.get(CURVE_PICKER)) else { return false };
    app.session.active().is_some_and(|s| picker_supported(s.doc.mode, eyedropper))
}

/// Sample the committed merged document through the regular canvas sampler and update only the
/// open Levels or Curves dialog. Returns true when an armed picker consumed the pointer event,
/// even if the point was outside the image or transparent.
pub fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some((id, kind, eyedropper, base)) = app.ui.dialogs.last().and_then(|dialog| {
        let kind = picker_kind(&dialog.fields)?.to_string();
        let eyedropper = picker(dialog.fields.get(CURVE_PICKER))?;
        let base = dialog.fields.get(CURVE_PICKER_BASE).cloned().unwrap_or_else(|| crate::filter_dialog::params_of(&dialog.fields));
        Some((dialog.id, kind, eyedropper, base))
    }) else {
        return false;
    };
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !picker_supported(mode, eyedropper) {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
    let params = match photocraft_engine::adjust_params::adjustment_eyedropper(&kind, &base, eyedropper, sample, mode) {
        Ok(adjustment) => photocraft_engine::adjust_params::to_params(&adjustment),
        Err(error) => {
            app.ui.status = error.to_string();
            return true;
        }
    };
    let Some(dialog) = app.ui.dialog_mut(id) else { return true };
    dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
    if let Value::Object(params) = params {
        dialog.fields.retain(|key, _| key.starts_with("__"));
        dialog.fields.extend(params);
    }
    true
}

/// Whether the top modal is the Curves dialog.
pub fn curves_on_top(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| d.fields.get("__adjust").and_then(Value::as_str) == Some("curves"))
}

/// ⌘/Ctrl-click on the image with Curves on top (#7325): a point on the curve shown in the dialog
/// at the clicked pixel's tone, at the output the curve gives there rounded to a whole level, so the
/// image barely changes (where the curve is clipped at 0 or 255 the point re-shapes it, as a click on
/// the graph there does); with `every_channel` (Shift) one on each colour channel instead, at that
/// channel's value.
/// The pixel comes from the same merged composite as the eyedroppers. Returns true when Curves is
/// on top: the click is the dialog's even when nothing is added (pencil curve, Lab or CMYK, a mask
/// or saved channel target, outside the image, a full curve).
pub fn add_point_at(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64, every_channel: bool) -> bool {
    if !curves_on_top(app) {
        return false;
    }
    let Some((id, mut values)) = app.ui.dialogs.last().map(|d| (d.id, crate::filter_dialog::params_of(&d.fields))) else { return false };
    if values.get("pencil").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !matches!(mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale) {
        app.ui.status = tl!("Unavailable in this color mode").into();
        return true;
    }
    // A mask, saved channel or Quick Mask holds its own gray levels, not the composite's.
    let isolated = isolated(app);
    if isolated.as_ref().is_some_and(|c| c.gray.is_some()) {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, x, y) else { return true };
    let gray = adjust_editors::is_gray(mode);
    let (shown, shown_key) = match &isolated {
        Some(c) => (0, c.key),
        None => {
            let ch: usize = ctx.data(|d| d.get_temp(dialog_mem("curves").with("curves-ch"))).unwrap_or(0);
            let chans = adjust_editors::tone_channels(adjust_editors::space_of(&values), gray);
            (ch, chans.get(ch).map_or("", |c| c.key))
        }
    };
    let keys: &[&str] = if every_channel && !gray && isolated.is_none() { &["red", "green", "blue"] } else { &[shown_key] };
    let mut selected = None;
    let mut changed = false;
    for &key in keys {
        let level = match key {
            "red" => sample[0],
            "green" => sample[1],
            "blue" => sample[2],
            // The composite's tone: Gray's mean, as the Grayscale eyedroppers read it, and RGB's
            // Rec. 709 luma, as the Neutral Gray eyedropper does (`adjust_params::rgb_luma`).
            _ if gray => (sample[0] + sample[1] + sample[2]) / 3.0,
            _ => sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722,
        };
        let input = (level.clamp(0.0, 1.0) * 255.0).round();
        let field = if key.is_empty() { "points" } else { key };
        let mut pts = adjust_editors::read_curve(&values, field);
        let curve: Vec<_> = pts.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
        let lut = photocraft_compose::adjust::curve_lut(&curve);
        let at = (input / 255.0 * lut.len().saturating_sub(1) as f32).round() as usize;
        let Some(output) = lut.get(at).map(|o| (o * 255.0).round()) else { continue };
        let before = pts.len();
        let Some(i) = crate::point_curve::insert(&mut pts, [input, output]) else { continue };
        if pts.len() > before {
            // Whole levels, as the editor writes a point clicked on the graph.
            values[field] = json!(pts.iter().map(|q| [q[0].round(), q[1].round()]).collect::<Vec<_>>());
            changed = true;
        }
        if key == shown_key {
            selected = Some(i);
        }
    }
    if let Some(i) = selected {
        let state = dialog_mem("curves").with("curves");
        ctx.data_mut(|d| {
            let mut st: crate::state::CurvesEditorState = d.get_temp(state).unwrap_or_default();
            st.channel = shown;
            st.gesture = crate::state::PointCurveState { selected: Some(i), drag: None };
            d.insert_temp(state, st);
        });
    }
    if changed
        && let Some(dialog) = app.ui.dialog_mut(id)
        && let Value::Object(v) = values
    {
        dialog.fields.retain(|k, _| k.starts_with("__"));
        dialog.fields.extend(v);
        if picker(dialog.fields.get(CURVE_PICKER)).is_some() {
            let base = crate::filter_dialog::params_of(&dialog.fields);
            dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
        }
    }
    true
}

pub(crate) fn picker_button(ui: &mut egui::Ui, selected: bool, enabled: bool, tooltip: &str, color: egui::Color32) -> egui::Response {
    let response = ui.add_enabled_ui(enabled, |ui| crate::icons::button(ui, "pipette", 28.0, selected, tooltip)).inner;
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().circle_filled(response.rect.right_bottom() - egui::vec2(5.5, 5.5), 3.5, color);
    ui.painter().circle_stroke(response.rect.right_bottom() - egui::vec2(5.5, 5.5), 3.5, egui::Stroke::new(1.0, tokens.field_border));
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, tooltip));
    response
}

fn picker_controls(ui: &mut egui::Ui, fields: &mut Map<String, Value>, mode: photocraft_doc::ColorMode) {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    let current = picker(fields.get(CURVE_PICKER));
    let mut chosen = None;
    ui.horizontal(|ui| {
        let tokens = crate::theme::Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Eyedroppers:")).color(tokens.text_dim).size(12.0));
        // The swatch is the tone the picker maps the sample to (image data, like the Curves
        // gradient bars), not chrome, so it is the same in every theme; its ring is a token.
        for (id, eyedropper, label, tone) in [
            ("black", CurvesEyedropper::Black, tl!("Set Black Point"), 0),
            ("gray", CurvesEyedropper::NeutralGray, tl!("Set Neutral Gray Point"), 128),
            ("white", CurvesEyedropper::White, tl!("Set White Point"), 255),
        ] {
            let enabled = picker_supported(mode, eyedropper);
            if picker_button(ui, current == Some(eyedropper), enabled, label, egui::Color32::from_gray(tone)).clicked() {
                chosen = Some((id, eyedropper));
            }
        }
        if !matches!(mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale) {
            ui.label(egui::RichText::new(tl!("Unavailable in this color mode")).color(tokens.text_faint).size(11.0));
        }
    });
    if let Some((id, selected)) = chosen
        && current != Some(selected)
    {
        let base = crate::filter_dialog::params_of(fields);
        fields.insert(CURVE_PICKER.into(), json!(id));
        fields.insert(CURVE_PICKER_BASE.into(), base);
    }
}

/// Whether `command` opens an adjustment dialog (when run without params).
pub fn has_dialog(command: &str) -> bool {
    command.strip_prefix(PREFIX).is_some_and(adjust_editors::has_editor)
}

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__adjust")
}

/// Root of the dialog editor's view state in egui memory.
fn dialog_mem(kind: &str) -> egui::Id {
    egui::Id::unique(("adjust-dialog", kind))
}

/// Whether holding Alt turns Cancel into Reset for this adjustment dialog (every kind).
pub fn resets(fields: &Map<String, Value>) -> bool {
    owns(fields)
}

/// Alt+Cancel (Reset): engine defaults for the kind; Color Balance keeps Preserve Luminosity and
/// shows Midtones (Photoshop 25.4). Dialog stays open.
pub fn reset(app: &PhotocraftApp, ctx: &egui::Context, fields: &mut Map<String, Value>) {
    if !owns(fields) {
        return;
    }
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default().to_string();
    let mode = curve_mode(app, &kind);
    // Color Balance: Photoshop keeps the user's Preserve Luminosity across Reset.
    let preserve = if kind == "colorBalance" { fields.get("preserveLuminosity").cloned() } else { None };
    let Ok(defaults) = photocraft_engine::adjust_params::default_for(&kind, mode) else { return };
    let values = photocraft_engine::adjust_params::to_params(&defaults);
    // Replace parameter keys; keep private `__` chrome (preview, command, …).
    fields.retain(|k, _| k.starts_with("__"));
    if let Value::Object(v) = values {
        fields.extend(v);
    }
    if let Some(p) = preserve {
        fields.insert("preserveLuminosity".into(), p);
    }
    let mut values = crate::filter_dialog::params_of(fields);
    adjust_editors::reset_adjustment_editor(ctx, &kind, dialog_mem(&kind), &mut values);
    if let Value::Object(v) = values {
        fields.extend(v);
    }
}

/// Opens the dialog for `command` with the kind's neutral settings.
pub fn open(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let kind = command.strip_prefix(PREFIX).filter(|k| adjust_editors::has_editor(k))?;
    let spec = photocraft_engine::commands::find(command)?;
    let mode = curve_mode(app, kind);
    let defaults = photocraft_engine::adjust_params::default_for(kind, mode).ok()?;
    let mut fields = Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(spec.label));
    fields.insert("__adjust".into(), json!(kind));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__preview".into(), json!(true));
    if let Value::Object(p) = photocraft_engine::adjust_params::to_params(&defaults) {
        fields.extend(p);
    }
    Some(app.ui.open_dialog(crate::state::DialogKind::Command, fields))
}

/// Opens the dialog for `command` at the settings last confirmed in it (Photoshop's ⌥ plus the
/// adjustment's shortcut, ⌥⌘L / ⌥⌘M, #8900), or at the neutral ones when there are none or they
/// don't fit the current colour mode.
pub fn open_last(app: &mut PhotocraftApp, command: &str) -> Option<u64> {
    let id = open(app, command)?;
    let kind = command.strip_prefix(PREFIX).unwrap_or_default();
    let mode = curve_mode(app, kind);
    let Some(last) = app.session.prefs().dialogs.get(command).cloned() else { return Some(id) };
    let Ok(adjustment) = photocraft_engine::adjust_params::from_params(kind, &last, None, mode) else { return Some(id) };
    if let (Some(dialog), Value::Object(values)) = (app.ui.dialog_mut(id), photocraft_engine::adjust_params::to_params(&adjustment)) {
        dialog.fields.retain(|k, _| k.starts_with("__"));
        dialog.fields.extend(values);
    }
    Some(id)
}

/// Remember an adjustment dialog's settings after a successful OK, for [`open_last`]. Editing a
/// smart filter's settings is not a use of the dialog's own (as `filter_dialog::remember`).
pub(crate) fn remember(app: &mut PhotocraftApp, fields: &Map<String, Value>) {
    if fields.contains_key("__smartFilter") {
        return;
    }
    let Some(command) = fields.get("__command").and_then(Value::as_str).filter(|c| has_dialog(c)) else { return };
    let values = crate::filter_dialog::params_of(fields);
    app.session.prefs.edit(|prefs| prefs.dialogs.insert(command.into(), values));
}

/// The Curves dialog's single-channel target: the Channels panel's, plus the Layers panel's mask
/// selection — both ways of selecting a mask must give the same dropdown (#5335). A targeted
/// filter mask is a gray plane too (FILE-215-3).
fn isolated(app: &PhotocraftApp) -> Option<photocraft_engine::channel_cmds::IsolatedCurveChannel> {
    let st = app.session.active()?;
    if app.filter_mask_targeted() {
        return photocraft_engine::channel_cmds::filter_mask_curve_channel(st);
    }
    photocraft_engine::channel_cmds::isolated_curve_channel_for(st, app.layer_mask_targeted())
}

/// The colour mode the Levels/Curves dialog seeds and parses in: a mask, saved channel or
/// Quick Mask holds gray levels whatever the document's mode, so its curve is a neutral one
/// and the fields carry no Lab or CMYK records (#5335).
fn curve_mode(app: &PhotocraftApp, kind: &str) -> photocraft_doc::ColorMode {
    let mode = app.session.active().map_or(photocraft_doc::ColorMode::Rgb, |s| s.doc.mode);
    if kind == "curves" && isolated(app).is_some_and(|c| c.gray.is_some()) { photocraft_doc::ColorMode::Grayscale } else { mode }
}

/// Dialog body: the kind's editor over the fields, then the Preview checkbox.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    // The blocks right-align in a right-to-left UI and their rows mirror; left to right the
    // layout is egui's default, so the dialog reads as it did before.
    ui.with_layout(crate::direction::body_layout(), |ui| body_blocks(app, ui, fields));
}

/// The dialog body, drawn inside the mirrored body scope.
fn body_blocks(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let kind = fields.get("__adjust").and_then(Value::as_str).unwrap_or_default().to_string();
    let mode = curve_mode(app, &kind);
    if matches!(kind.as_str(), "levels" | "curves") {
        picker_controls(ui, fields, mode);
        ui.add_space(4.0);
    }
    let mut values = crate::filter_dialog::params_of(fields);
    let active = fields
        .get("__smartFilter")
        .and_then(|f| f.get("layer"))
        .and_then(Value::as_u64)
        .map(photocraft_doc::LayerId)
        .or_else(|| app.session.active().and_then(|s| s.active_layer));
    let isolated_curve = isolated(app);
    // Only Curves gets an isolated single-channel target here; Levels keeps its usual fields.
    let gray_plane = (kind == "curves").then(|| isolated_curve.as_ref().and_then(|c| c.gray)).flatten();
    let gray = gray_plane.is_some() || app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode));
    let hist = match active {
        Some(id) if adjust_editors::needs_histogram(&kind) => {
            // A mask or saved channel's curve edits that plane's own levels: the histogram
            // describes it, not the colour layers beneath (#5335).
            let source = match gray_plane {
                Some(surface) => HistSource::Channel { layer: id, surface },
                None => HistSource::Layer(id),
            };
            Some(tone::histograms(app, source, adjust_editors::space_of(&values)))
        }
        _ => None,
    };
    let cx = EditorCx {
        mem: dialog_mem(&kind),
        hist,
        gray,
        swatches: adjust_editors::swatches(app),
        gradients: adjust_editors::gradient_presets(app, &kind),
        dialog: true,
        dynamic_color_sliders: app.session.prefs().interface.dynamic_color_sliders,
        isolated_curve,
    };
    let e = adjust_editors::editor(ui, &kind, &mut values, &cx);
    if e.import_gradients {
        crate::preset_files_ui::import_gradients_dialog(app);
    }
    if e.changed
        && let Value::Object(v) = values
    {
        // Replace the parameter keys (an editor may remove one), keep the private ones.
        fields.retain(|k, _| k.starts_with("__"));
        fields.extend(v);
        if picker(fields.get(CURVE_PICKER)).is_some() {
            let base = crate::filter_dialog::params_of(fields);
            fields.insert(CURVE_PICKER_BASE.into(), base);
        }
    }
    let mut preview = fields.get("__preview").and_then(Value::as_bool).unwrap_or(true);
    ui.add_space(6.0);
    if crate::widgets::checkbox(ui, &mut preview, tl!("Preview")).changed() {
        fields.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rtl_tests {
    use super::*;
    use crate::direction::Dir;
    use egui_kittest::kittest::Queryable;

    /// A Levels dialog drawn in `dir`, with its eyedropper row on screen.
    fn harness(dir: Dir) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let _dir = crate::direction::direction_scope(dir);
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        open(h.state_mut(), "image.adjustments.levels").unwrap();
        h.run_steps(3);
        h
    }

    /// The eyedropper row follows the reading direction: the label starts the row and the
    /// three point pickers follow it, mirrored in a right-to-left UI.
    #[test]
    fn the_eyedropper_row_follows_the_reading_direction() {
        let ltr = harness(Dir::Ltr);
        let ltr_label = ltr.get_by_label("Eyedroppers:").rect();
        let ltr_black = ltr.get_by_label("Set Black Point").rect();
        let ltr_gray = ltr.get_by_label("Set Neutral Gray Point").rect();
        let ltr_white = ltr.get_by_label("Set White Point").rect();
        assert!(ltr_label.right() <= ltr_black.left(), "LTR: the label starts the row: {ltr_label:?} {ltr_black:?}");
        assert!(
            ltr_black.left() < ltr_gray.left() && ltr_gray.left() < ltr_white.left(),
            "LTR: the pickers read left to right: {ltr_black:?} {ltr_gray:?} {ltr_white:?}"
        );

        let rtl = harness(Dir::Rtl);
        let rtl_label = rtl.get_by_label("Eyedroppers:").rect();
        let rtl_black = rtl.get_by_label("Set Black Point").rect();
        let rtl_gray = rtl.get_by_label("Set Neutral Gray Point").rect();
        let rtl_white = rtl.get_by_label("Set White Point").rect();
        assert!(rtl_label.left() >= rtl_black.right(), "RTL: the label starts the row: {rtl_label:?} {rtl_black:?}");
        assert!(
            rtl_black.left() > rtl_gray.left() && rtl_gray.left() > rtl_white.left(),
            "RTL: the pickers read right to left: {rtl_black:?} {rtl_gray:?} {rtl_white:?}"
        );
    }

    /// The dialog's frame rect.
    fn frame(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> egui::Rect {
        let id = h.state().ui.dialogs.last().expect("the dialog is open").id;
        h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog frame")
    }

    /// The mirrored body scope must not stretch the frame: the Levels dialog opens as wide in
    /// RTL as in LTR (dialog-host Risks a).
    #[test]
    fn the_adjust_dialog_opens_as_wide_in_rtl_as_in_ltr() {
        let width = |dir: Dir| {
            let h = harness(dir);
            frame(&h).width()
        };
        let (ltr, rtl) = (width(Dir::Ltr), width(Dir::Rtl));
        assert!((ltr - rtl).abs() < 1.0, "the frame width is the same in both directions: LTR {ltr} vs RTL {rtl}");
    }

    /// The body's blocks hug the reading start: the label leading the eyedropper row sits at
    /// the dialog's content start edge — the right in RTL, the left in LTR (the panel twin
    /// asserts the same anchor, `adjust_editors/levels.rs`).
    #[test]
    fn the_body_blocks_hug_the_reading_start() {
        let (ltr, rtl) = (harness(Dir::Ltr), harness(Dir::Rtl));
        let (ltr_frame, rtl_frame) = (frame(&ltr), frame(&rtl));
        let ltr_label = ltr.get_by_label("Eyedroppers:").rect();
        let rtl_label = rtl.get_by_label("Eyedroppers:").rect();
        assert!(ltr_label.left() - ltr_frame.left() < 40.0, "LTR: the label leads at the left: {ltr_label:?} {ltr_frame:?}");
        assert!(rtl_frame.right() - rtl_label.right() < 40.0, "RTL: the label leads at the right: {rtl_label:?} {rtl_frame:?}");
    }
}
