//! The Adjustment Brush Tool (#12519): its options bar and its stroke. Painting runs
//! `paint.adjustmentBrush`, which makes an adjustment layer of the dropdown's kind whose mask is
//! the stroke, or paints into the mask of the active adjustment layer of that kind. The stroke
//! shows as the painting tools' trail while it is drawn and is applied on release.

use photocraft_engine::adjustment_brush_cmds::{self as ab, ENTRIES, INDEXED_ALERT};
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::options_bar::{opt_label, percent_field, pressure_button, whole_percent_field};
use crate::state::Tool;

/// The kind a stroke paints: the dropdown's entry, or the first offered one when the document's
/// colour mode or bit depth greys it.
pub fn kind(app: &PhotocraftApp) -> &'static str {
    let stored = ab::entry_of(&app.ui.tool_options.adjustment_brush_kind).map_or(ab::DEFAULT_KIND, |e| e.kind);
    match app.session.active() {
        Some(st) => ab::offered_kind(stored, st.doc.mode, st.doc.depth),
        None => stored,
    }
}

/// Selecting an adjustment layer sets the dropdown to its kind, once per selection, so a later
/// pick in the dropdown sticks.
pub fn follow_active_layer(app: &mut PhotocraftApp) {
    let active = app.session.active().and_then(|st| st.active_layer).map(|l| l.0);
    if app.ui.tool_options.adjustment_brush_followed == active {
        return;
    }
    app.ui.tool_options.adjustment_brush_followed = active;
    if let Some(k) = ab::active_kind(&app.session) {
        app.ui.tool_options.adjustment_brush_kind = k.into();
    }
}

/// Subtract (black over white) or Add (white over black): the mode and the colours the stroke
/// paints the mask with.
pub fn set_subtract(app: &mut PhotocraftApp, subtract: bool) {
    app.ui.tool_options.adjustment_brush_subtract = subtract;
    let (fg, bg) = if subtract { ("#000000", "#ffffff") } else { ("#ffffff", "#000000") };
    if let Err(e) = app.run("tools.setColors", json!({"foreground": fg, "background": bg})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A press in an Indexed Color document: the reference application's alert, and no stroke.
pub fn press_refused(app: &mut PhotocraftApp) -> bool {
    let indexed = app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed);
    if indexed {
        crate::brush_refusal::prompt(app, tl!(INDEXED_ALERT));
    }
    indexed
}

/// Applies a finished drag. Returns false for another tool.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], line: bool) -> bool {
    if tool != Tool::AdjustmentBrush {
        return false;
    }
    follow_active_layer(app);
    let mut p = json!({
        "kind": kind(app),
        "subtract": app.ui.tool_options.adjustment_brush_subtract,
        "points": points,
        "zoom": app.point_zoom(),
    });
    crate::canvas::unsmoothed_line(&mut p, line);
    match app.run("paint.adjustmentBrush", p) {
        Ok(_) => {
            // The layer painted is active with its mask targeted, as after a New Adjustment Layer.
            app.ui.mask_target = true;
            app.ui.vector_mask_target = false;
            app.ui.tool_options.adjustment_brush_followed = app.session.active().and_then(|st| st.active_layer).map(|l| l.0);
        }
        Err(e) if e.contains(INDEXED_ALERT) => crate::brush_refusal::prompt(app, tl!(INDEXED_ALERT)),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

/// Does a separator line go above `kind` in the dropdown?
fn starts_group(kind: &str) -> bool {
    let i = ENTRIES.iter().position(|e| e.kind == kind);
    i.and_then(|i| i.checked_sub(1)).and_then(|i| ENTRIES.get(i)).is_some_and(|e| e.separator_after)
}

/// The options bar, left to right as in the reference application: (the tool preset picker) ·
/// Adjustment · Subtract / Add · Brush Preset · Pressure for Size · Choose an object · Overlay ·
/// Opacity · Pressure for Opacity · Flow · Airbrush. Returns false for another tool.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::AdjustmentBrush {
        return false;
    }
    follow_active_layer(app);
    let (mode, depth) = app.session.active().map_or((photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), |st| (st.doc.mode, st.doc.depth));
    opt_label(ui, tl!("Adjustment:"));
    let options: Vec<(&'static str, &str)> = ENTRIES.iter().map(|e| (e.kind, e.label)).collect();
    let mut current = kind(app);
    let chosen = crate::widgets::dropdown_wheel_grouped_enabled(
        ui,
        "adjustment-brush-kind",
        tl!("Adjustment"),
        &mut current,
        &options,
        150.0,
        |k| starts_group(k),
        |k| ab::enabled_in(k, mode, depth),
    )
    .0;
    // Choosing an entry is no history state; the keys may land on a greyed one, which keeps the choice.
    if let Some(&k) = chosen.last()
        && ab::enabled_in(k, mode, depth)
    {
        app.ui.tool_options.adjustment_brush_kind = k.into();
    }
    let subtract = app.ui.tool_options.adjustment_brush_subtract;
    let sub = crate::icons::button(ui, "minus", 24.0, subtract, tl!("Subtract from mask areas to remove adjustment from areas of the image"));
    let add = crate::icons::button(ui, "plus", 24.0, !subtract, tl!("Add to mask areas to apply adjustment to more of the image"));
    if crate::brush_picker::named(sub, "Subtract").clicked() && !subtract {
        set_subtract(app, true);
    } else if crate::brush_picker::named(add, "Add").clicked() && subtract {
        set_subtract(app, false);
    }
    crate::widgets::vline(ui, 22.0);
    crate::panels::brush_preset_chip(ui, &app.session.tools.brush, &mut app.ui);
    crate::brush_picker::settings_toggle(app, ui);
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    if pressure_button(ui, b.pressure_size, tl!("Always use pressure for size"), false).clicked() {
        b.pressure_size = !b.pressure_size;
    }
    crate::widgets::vline(ui, 22.0);
    // Present but inert: what it does was not measured.
    ui.add_enabled_ui(false, |ui| {
        crate::icons::button(ui, "square-dashed-mouse-pointer", 24.0, false, tl!("Choose an object and have the adjustment applied to it"));
    });
    // Its tint over the painted mask is its own item (#12520).
    crate::widgets::checkbox(ui, &mut app.ui.tool_options.adjustment_brush_overlay, tl!("Overlay"));
    crate::widgets::vline(ui, 22.0);
    whole_percent_field(ui, tl!("Opacity"), &mut b.opacity, 1.0..=100.0, 62.0);
    if pressure_button(ui, b.pressure_opacity, tl!("Always use pressure for opacity"), false).clicked() {
        b.pressure_opacity = !b.pressure_opacity;
    }
    percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 62.0);
    let airbrush = crate::icons::button(ui, "sparkles", 24.0, b.build_up, tl!("Enable airbrush-style build-up effects"));
    if crate::brush_picker::named(airbrush, "Enable airbrush-style build-up effects").clicked() {
        b.build_up = !b.build_up;
    }
    crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
    true
}

#[cfg(test)]
#[path = "../adjustment_brush_tool_tests.rs"]
mod adjustment_brush_tool_tests;
