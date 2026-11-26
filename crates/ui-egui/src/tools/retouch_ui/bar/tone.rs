//! The Retouch options bar's toning family: Dodge and Burn, Sponge, Blur, Sharpen and Smudge, Color Replacement.

use craft::behavior::IconId;
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::primitives::IconSpec;
use craft::ui::text::RangeAlert;
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
    let o = &mut app.ui.tool_options;
    let ranges = string_entries(&[("shadows", tl!("Shadows")), ("midtones", tl!("Midtones")), ("highlights", tl!("Highlights"))]);
    choose(ui, tl!("Range"), &mut o.tone_range, &ranges, true, Some(tl!("Painting Mode")));
    // Whole numbers 1..100 with the range alert and a slider arrow; Dodge shares the field (#12669).
    let exposure =
        number(tl!("Exposure"), &mut o.exposure, 1.0..=100.0).whole().unit("%").popup_slider().tooltip(tl!("Set exposure for stroke")).show(ui).out_of_range;
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let angle = number(tl!("Angle"), &mut brush.angle, -180.0..=180.0)
        .whole()
        .unit("°")
        .icon(IconSpec::svg(IconId::new("rotate_cw")))
        .tooltip(tl!("Set the brush angle"))
        .show(ui);
    let mut key = angle.events.iter().find_map(|event| crate::history_keys::coalesce("brush", event));

    check(ui, tl!("Protect Tones"), &mut o.protect_tones, true, Some(tl!("Minimizes clipping in shadows and highlights, and keeps colors from shifting hue")));

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
    let o = &mut app.ui.tool_options;
    let modes = string_entries(&[("desaturate", tl!("Desaturate")), ("saturate", tl!("Saturate"))]);
    choose(ui, tl!("Mode"), &mut o.sponge_mode, &modes, true, None);
    let flow = number(tl!("Flow"), &mut o.sponge_flow, 1.0..=100.0).whole().unit("%").popup_slider().show(ui).out_of_range;
    check(ui, tl!("Vibrance"), &mut o.vibrance, true, None);
    flow
}

/// The options of Blur, Sharpen and Smudge.
pub(super) fn blur_sharpen_smudge(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> Option<RangeAlert> {
    let o = &mut app.ui.tool_options;
    if tool == Tool::Smudge {
        choose(ui, tl!("Mode"), &mut o.smudge_mode, &smudge_modes(), true, Some(tl!("Painting Mode")));
    }
    // The cell writes only on a commit, so a shared fraction is not rounded (#12650).
    let strength = number(tl!("Strength"), &mut o.strength, 1.0..=100.0).whole().unit("%").show(ui).out_of_range;
    // Sharpen's Angle follows Strength (#12946); the bar shows either field's range alert.
    let angle = if tool == Tool::Sharpen { brush_angle(ui, app) } else { None };
    let strength = strength.or(angle);
    let o = &mut app.ui.tool_options;
    if let Some(all) = o.sample_all.slot(tool) {
        check(ui, tl!("Sample All Layers"), all, true, None);
    }
    if tool == Tool::Sharpen {
        check(ui, tl!("Protect Detail"), &mut o.protect_detail, true, None);
    }
    if tool == Tool::Smudge {
        check(ui, tl!("Finger Painting"), &mut o.finger_painting, true, None);
    }
    strength
}

/// The brush Angle cell, as the Background Eraser's: whole degrees -180..180 over the tool's own
/// brush (the shell swaps one in per tool), so it is the angle the Brush Settings panel shows.
fn brush_angle(ui: &mut egui::Ui, app: &mut PhotocraftApp) -> Option<RangeAlert> {
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    let out = number(tl!("Angle"), &mut brush.angle, -180.0..=180.0)
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
    let o = &mut app.ui.tool_options;
    let modes = string_entries(&[("hue", tl!("Hue")), ("saturation", tl!("Saturation")), ("color", tl!("Color")), ("luminosity", tl!("Luminosity"))]);
    choose(ui, tl!("Mode"), &mut o.cr_mode, &modes, true, Some(tl!("Painting Mode")));
    Divider::vertical().show(ui);
    let sampling = [
        Entry::new("continuous".to_owned(), tl!("Sampling: Continuous")).icon(IconSpec::svg(IconId::new("eyedropper"))),
        Entry::new("once".to_owned(), tl!("Sampling: Once")).icon(IconSpec::svg(IconId::new("target"))),
        Entry::new("backgroundSwatch".to_owned(), tl!("Sampling: Background Swatch")).icon(IconSpec::svg(IconId::new("square"))),
    ];
    if let Some(picked) = Segmented::new(tl!("Sampling"), Bind::given(o.cr_sampling.clone()), &sampling).show(ui).value().cloned() {
        o.cr_sampling = picked;
    }
    Divider::vertical().show(ui);
    let limits = string_entries(&[("discontiguous", tl!("Discontiguous")), ("contiguous", tl!("Contiguous")), ("findEdges", tl!("Find Edges"))]);
    choose(ui, tl!("Limits"), &mut o.cr_limits, &limits, true, Some(tl!("Determines the spread of replaced color")));
    let tolerance = number(tl!("Tolerance"), &mut o.cr_tolerance, 1.0..=100.0)
        .whole()
        .unit("%")
        .tooltip(tl!("Tolerance for choosing related colors"))
        .show(ui)
        .out_of_range;
    check(ui, tl!("Anti-alias"), &mut o.cr_anti_alias, true, Some(tl!("Anti-alias brush application")));
    tolerance
}

#[cfg(test)]
#[path = "../../../sharpen_bar_angle_tests.rs"]
mod sharpen_bar_angle_tests;
