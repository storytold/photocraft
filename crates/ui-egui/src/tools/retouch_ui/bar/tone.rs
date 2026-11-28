//! The Retouch options bar's toning family: Dodge and Burn, Sponge, Blur, Sharpen and Smudge, Color Replacement.

use craft::behavior::{IconId, NumberCodec};
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing, bind_tracked};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Env;
use craft::ui::text::{NumberField, NumberValue, RangeAlert};
use photocraft_color::BlendMode;

use super::heal::{check, choose, string_entries};
use crate::PhotocraftApp;
use crate::option_cells::number;
use crate::state::Tool;

/// The Smudge tool's Painting Modes in the reference application's order (#12959), a separator between its groups.
fn smudge_modes() -> Vec<Entry<BlendMode>> {
    let mode = |m: BlendMode| Entry::new(m, m.label());
    vec![
        mode(BlendMode::Normal),
        Entry::separator(),
        mode(BlendMode::Darken),
        mode(BlendMode::Lighten),
        Entry::separator(),
        mode(BlendMode::Hue),
        mode(BlendMode::Saturation),
        mode(BlendMode::Color),
        mode(BlendMode::Luminosity),
    ]
}

/// The options of Dodge and Burn.
pub(super) fn dodge_burn(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let (tone_range, exposure, protect) = bind_tracked!(app.ui.tool_options => tone_range, exposure, protect_tones);
    let ranges = string_entries(&[("shadows", tl!("Shadows")), ("midtones", tl!("Midtones")), ("highlights", tl!("Highlights"))]);
    choose(ui, tl!("Range"), tone_range, &ranges, true, Some(tl!("Painting Mode")));
    // Whole numbers 1..100 with the range alert and a slider arrow; Dodge shares the field (#12669).
    let exposure =
        number(tl!("Exposure"), exposure, 1.0..=100.0).whole().unit("%").popup_slider().tooltip(tl!("Set exposure for stroke")).show(ui).out_of_range;
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let angle = number(tl!("Angle"), Bind::bound(&mut brush.angle), -180.0..=180.0)
        .whole()
        .unit("°")
        .icon(IconSpec::svg(IconId::new("rotate_cw")))
        .tooltip(tl!("Set the brush angle"))
        .show(ui);
    let mut key = angle.events.iter().find_map(|event| crate::history_keys::coalesce("brush", event));

    check(ui, tl!("Protect Tones"), protect, true, Some(tl!("Minimizes clipping in shadows and highlights, and keeps colors from shifting hue")));

    let pressure = ToggleButton::new(tl!("Always use pressure for size"), Bind::given(brush.pressure_size))
        .icon(IconId::new("target"))
        .icon_only()
        .tooltip(tl!("Always use Pressure for Size. When off, Brush Preset controls pressure"))
        .show(ui);
    if let Some(on) = pressure.value().copied() {
        brush.pressure_size = on;
        key = pressure.events.iter().find_map(|event| crate::history_keys::coalesce("brush", event)).or(key);
    }
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
    exposure.or(angle.out_of_range)
}

/// The options of Sponge.
pub(super) fn sponge(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let (sponge_mode, sponge_flow, vibrance) = bind_tracked!(app.ui.tool_options => sponge_mode, sponge_flow, vibrance);
    let modes = string_entries(&[("desaturate", tl!("Desaturate")), ("saturate", tl!("Saturate"))]);
    choose(ui, tl!("Mode"), sponge_mode, &modes, true, None);
    let flow = number(tl!("Flow"), sponge_flow, 1.0..=100.0).whole().unit("%").popup_slider().show(ui).out_of_range;
    check(ui, tl!("Vibrance"), vibrance, true, None);
    flow
}

/// Smudge keeps the original Angle field's truncation of typed fractions.
/// The shared field still owns parsing, range alerts, stepping and scrubbing.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SmudgeAngle(f32);

impl NumberValue for SmudgeAngle {
    fn number(&self) -> Option<f64> {
        Some(f64::from(self.0))
    }
    fn of_number(number: f64) -> Self {
        Self(number.trunc() as f32)
    }
    fn blank() -> Option<Self> {
        None
    }
    fn named(&self) -> Option<u32> {
        None
    }
    fn from_name(_: u32) -> Option<Self> {
        None
    }
}

fn smudge_angle(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let mut angle = SmudgeAngle(brush.angle);
    let shown = Field::new(tl!("Angle"))
        .side(SideLabel::new().scrub(true).icon(IconSpec::svg(IconId::new("rotate_cw"))).layout(FieldLayout::Inside))
        .sizing(Sizing::Content)
        .show(ui, |ui, _| {
            NumberField::named(tl!("Angle"), Bind::bound(&mut angle))
                .range(-180.0..=180.0)
                .step(1.0)
                .codec(NumberCodec::new(Env::of(ui).units().reader(None), 0))
                .unit("°")
                .scrub_by_label()
                .tooltip(tl!("Set the brush angle"))
                .show(ui)
        })
        .inner;
    let mut key = None;
    let mut alert = None;
    if let Some(shown) = shown {
        brush.angle = angle.0;
        key = shown.edit.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
        alert = shown.out_of_range.filter(|alert| !alert.range.contains(&alert.typed.trunc()));
    }
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
    alert
}

/// Smudge's pressure-for-size switch edits the active tool's own brush through
/// the same command and gesture path as the current Background Eraser controls.
fn smudge_pressure_size(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let pressure = ToggleButton::new(tl!("Always use pressure for size"), Bind::given(brush.pressure_size))
        .icon(IconId::new("target"))
        .icon_only()
        .tooltip(tl!("Always use Pressure for Size. When off, Brush Preset controls pressure").to_owned())
        .show(ui);
    if let Some(on) = pressure.value().copied() {
        brush.pressure_size = on;
    }
    let key = pressure.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
}

/// The options of Blur, Sharpen and Smudge.
pub(super) fn blur_sharpen_smudge(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> Option<RangeAlert> {
    if tool == Tool::Smudge {
        choose(ui, tl!("Mode"), bind_tracked!(app.ui.tool_options => smudge_mode), &smudge_modes(), true, Some(tl!("Painting Mode")));
    }
    // The cell writes only on a commit, so a shared fraction is not rounded (#12650).
    let strength = number(tl!("Strength"), bind_tracked!(app.ui.tool_options => strength), 1.0..=100.0).whole().unit("%").show(ui).out_of_range;
    let angle = match tool {
        Tool::Smudge => smudge_angle(ui, app),
        Tool::Sharpen => brush_angle(ui, app),
        _ => None,
    };
    if let Some(all) = app.ui.tool_options.sample_all.slot(tool)
        && let Some(next) = Checkbox::new(Bind::given(all)).label(tl!("Sample All Layers").to_owned()).show(ui).value().copied()
        && let Some(slot) = app.ui.tool_options.sample_all.slot_mut(tool)
    {
        *slot = next;
    }
    if tool == Tool::Sharpen {
        check(ui, tl!("Protect Detail"), bind_tracked!(app.ui.tool_options => protect_detail), true, None);
    }
    if tool == Tool::Smudge {
        check(ui, tl!("Finger Painting"), bind_tracked!(app.ui.tool_options => finger_painting), true, None);
        smudge_pressure_size(ui, app);
    }
    strength.or(angle)
}

/// The brush Angle cell, as the Background Eraser's: whole degrees -180..180 over the tool's own
/// brush (the shell swaps one in per tool), so it is the angle the Brush Settings panel shows.
fn brush_angle(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let out = number(tl!("Angle"), Bind::bound(&mut brush.angle), -180.0..=180.0)
        .whole()
        .unit("°")
        .icon(IconSpec::svg(IconId::new("rotate_cw")))
        .tooltip(tl!("Set the brush angle"))
        .show(ui);
    // One scrub is one gesture, so one history step.
    let key = out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    crate::brush_panel::commit_keyed(app, &before, &brush, key);
    out.out_of_range
}

/// The options of Color Replacement.
pub(super) fn color_replacement(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let modes = string_entries(&[("hue", tl!("Hue")), ("saturation", tl!("Saturation")), ("color", tl!("Color")), ("luminosity", tl!("Luminosity"))]);
    choose(ui, tl!("Mode"), bind_tracked!(app.ui.tool_options => cr_mode), &modes, true, Some(tl!("Painting Mode")));
    Divider::vertical().show(ui);
    let sampling = [
        Entry::new("continuous".to_owned(), tl!("Sampling: Continuous")).icon(IconSpec::svg(IconId::new("eyedropper"))),
        Entry::new("once".to_owned(), tl!("Sampling: Once")).icon(IconSpec::svg(IconId::new("target"))),
        Entry::new("backgroundSwatch".to_owned(), tl!("Sampling: Background Swatch")).icon(IconSpec::svg(IconId::new("square"))),
    ];
    if let Some(picked) = Segmented::new(tl!("Sampling"), Bind::given(app.ui.tool_options.cr_sampling.clone()), &sampling).show(ui).value().cloned() {
        app.ui.tool_options.cr_sampling = picked;
    }
    Divider::vertical().show(ui);
    let limits = string_entries(&[("discontiguous", tl!("Discontiguous")), ("contiguous", tl!("Contiguous")), ("findEdges", tl!("Find Edges"))]);
    choose(ui, tl!("Limits"), bind_tracked!(app.ui.tool_options => cr_limits), &limits, true, Some(tl!("Determines the spread of replaced color")));
    let tolerance = number(tl!("Tolerance"), bind_tracked!(app.ui.tool_options => cr_tolerance), 1.0..=100.0)
        .whole()
        .unit("%")
        .tooltip(tl!("Tolerance for choosing related colors"))
        .show(ui)
        .out_of_range;
    check(ui, tl!("Anti-alias"), bind_tracked!(app.ui.tool_options => cr_anti_alias), true, Some(tl!("Anti-alias brush application")));
    tolerance
}

#[cfg(test)]
#[path = "../../../sharpen_bar_angle_tests.rs"]
mod sharpen_bar_angle_tests;
