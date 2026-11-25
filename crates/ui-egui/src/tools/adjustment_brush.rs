//! The Adjustment Brush Tool (#12519): its options bar and its stroke. Painting runs
//! `paint.adjustmentBrush`, which makes an adjustment layer of the dropdown's kind whose mask is
//! the stroke, or paints into the mask of the active adjustment layer of that kind. The stroke
//! shows as the painting tools' trail while it is drawn and is applied on release.

use craft::behavior::IconId;
use craft::ui::buttons::{IconButton, ToggleButton};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::pickers::Select;
use photocraft_engine::adjustment_brush_cmds::{self as ab, ENTRIES, INDEXED_ALERT};
use serde_json::json;

use super::option_cells;
use crate::PhotocraftApp;
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

/// The reference application's alert for a press in an Indexed Color document, in the UI language.
fn indexed_alert() -> &'static str {
    tl!(
        "Could not complete your request because the tool does not work with Indexed Color images. Please convert the image to RGB or another supported mode before using."
    )
}

/// A press in an Indexed Color document: the reference application's alert, and no stroke.
pub fn press_refused(app: &mut PhotocraftApp) -> bool {
    let indexed = app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed);
    if indexed {
        let _ = crate::dialogs::alerts::alert(app, indexed_alert());
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
        Err(e) if e.contains(INDEXED_ALERT) => {
            let _ = crate::dialogs::alerts::alert(app, indexed_alert());
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

/// The Adjustment dropdown's rows: the entries in their groups, those the document can't take greyed.
fn kind_entries(mode: photocraft_doc::ColorMode, depth: photocraft_doc::SampleType) -> Vec<Entry<&'static str>> {
    let mut rows = Vec::new();
    for e in ENTRIES.iter() {
        rows.push(Entry::new(e.kind, tl!(e.label).to_owned()).enabled(ab::enabled_in(e.kind, mode, depth)));
        if e.separator_after {
            rows.push(Entry::separator());
        }
    }
    rows
}

/// The options bar, left to right as in the reference application: (the tool preset picker) ·
/// Adjustment · Subtract / Add · Brush Preset · Pressure for Size · Choose an object · Overlay ·
/// Opacity · Pressure for Opacity · Flow · Airbrush. Returns false for another tool.
pub fn options_bar(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> bool {
    if tool != Tool::AdjustmentBrush {
        return false;
    }
    follow_active_layer(app);
    let (mode, depth) = app.session.active().map_or((photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), |st| (st.doc.mode, st.doc.depth));
    let entries = kind_entries(mode, depth);
    // Choosing an entry is no history state; a greyed entry can't be picked.
    let picked = Select::new(tl!("Adjustment:").to_owned(), Bind::given(kind(app)), &entries).show(ui).events.committed().copied();
    if let Some(k) = picked.filter(|k| ab::enabled_in(k, mode, depth)) {
        app.ui.tool_options.adjustment_brush_kind = k.into();
    }
    let modes = [
        Entry::new(true, tl!("Subtract from mask areas to remove adjustment from areas of the image").to_owned()).icon(IconId::new("minus")),
        Entry::new(false, tl!("Add to mask areas to apply adjustment to more of the image").to_owned()).icon(IconId::new("plus")),
    ];
    let subtract = app.ui.tool_options.adjustment_brush_subtract;
    if let Some(next) = Segmented::new(tl!("Mask mode").to_owned(), Bind::given(subtract), &modes).show(ui).value().copied()
        && next != subtract
    {
        set_subtract(app, next);
    }
    Divider::vertical().show(ui);
    crate::brush_picker::show_field(ui, app, true);
    crate::brush_picker::settings_toggle(ui, app);

    let before = app.session.tools.brush.clone();
    let mut brush = before.clone();
    let mut key = None;
    let pressure =
        ToggleButton::new(tl!("Always use pressure for size").to_owned(), Bind::given(brush.pressure_size)).icon(IconId::new("target")).icon_only().show(ui);
    if let Some(on) = pressure.value().copied() {
        brush.pressure_size = on;
        key = pressure.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    }
    Divider::vertical().show(ui);
    // Present but inert: what it does was not measured.
    let choose = tl!("Choose an object and have the adjustment applied to it");
    IconButton::new(IconId::new("square-dashed-mouse-pointer")).label(choose).tooltip(choose).enabled(false).show(ui);
    // On, the canvas tints what the active adjustment layer's mask reveals (`raster_source::MaskTint`, #12520).
    Checkbox::new(Bind::bound(&mut app.ui.tool_options.adjustment_brush_overlay))
        .label(tl!("Overlay").to_owned())
        .tooltip(tl!("Show or hide the painted mask areas").to_owned())
        .show(ui);
    Divider::vertical().show(ui);

    let mut alerts = Vec::new();
    let mut opacity = (brush.opacity * 100.0).round();
    let out = option_cells::number(tl!("Opacity:"), &mut opacity, 1.0..=100.0).whole().unit("%").show(ui);
    key = out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e)).or(key);
    brush.opacity = opacity / 100.0;
    alerts.extend(out.out_of_range);
    let pressure = ToggleButton::new(tl!("Always use pressure for opacity").to_owned(), Bind::given(brush.pressure_opacity))
        .icon(IconId::new("target"))
        .icon_only()
        .show(ui);
    if let Some(on) = pressure.value().copied() {
        brush.pressure_opacity = on;
        key = pressure.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e)).or(key);
    }
    let mut flow = brush.flow * 100.0;
    let out = option_cells::number(tl!("Flow:"), &mut flow, 1.0..=100.0).unit("%").show(ui);
    key = out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e)).or(key);
    brush.flow = flow / 100.0;
    alerts.extend(out.out_of_range);
    let airbrush = ToggleButton::new(tl!("Enable airbrush-style build-up effects").to_owned(), Bind::given(brush.build_up))
        .icon(IconId::new("sparkles"))
        .icon_only()
        .show(ui);
    if let Some(on) = airbrush.value().copied() {
        brush.build_up = on;
        key = airbrush.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e)).or(key);
    }
    // A field shown without an edit keeps the brush's exact value (no rounding by showing the bar).
    if out_unchanged(&before, &brush) {
        return true;
    }
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
    for alert in &alerts {
        option_cells::range_alert(app, ui, alert);
    }
    true
}

/// Whether showing the bar left the brush as it was, up to the rounding of the percent fields.
fn out_unchanged(before: &photocraft_engine::BrushSettings, after: &photocraft_engine::BrushSettings) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 0.005;
    near(before.opacity, after.opacity)
        && near(before.flow, after.flow)
        && before.pressure_size == after.pressure_size
        && before.pressure_opacity == after.pressure_opacity
        && before.build_up == after.build_up
}

#[cfg(test)]
#[path = "../adjustment_brush_tool_tests.rs"]
mod adjustment_brush_tool_tests;
