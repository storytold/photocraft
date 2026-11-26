//! Preferences › Tools › Pressure Curve: the pen's pressure response, edited as points on a curve
//! (`tools.pressureCurve`, read by the stylus through `photocraft_engine::prefs::PressureCurve`).
//! Brush Settings uses the same editor for each dynamic's own curve.

use std::cell::Cell;

use craft::behavior::{EditEvent, EditEvents, Mixed};
use craft::imaging::ui::curve::{Curve, CurveEditor};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, BodyCx, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use photocraft_engine::prefs::PressureCurve;

/// The editor's levels: the plot's whole units, so the points keep their unit-square values as percent.
const LEVELS: f32 = 100.0;

/// Preset curves: Linear, Soft (light pressure already gives a lot) and Hard (it takes a firm
/// press to reach full pressure).
const PRESETS: [(&str, &[[f32; 2]]); 3] =
    [("Linear", &[[0.0, 0.0], [1.0, 1.0]]), ("Soft", &[[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]]), ("Hard", &[[0.0, 0.0], [0.6, 0.3], [1.0, 1.0]])];

fn round2(x: f32) -> f64 {
    (f64::from(x.clamp(0.0, 1.0)) * 100.0).round() / 100.0
}

/// The unit-square points of an editor curve, to two decimals: the stored value stays readable and a drag doesn't
/// write float noise.
fn unit_points(curve: &Curve) -> Vec<[f32; 2]> {
    curve.points().unwrap_or_default().iter().map(|p| [round2(p[0] / LEVELS) as f32, round2(p[1] / LEVELS) as f32]).collect()
}

/// The editor over `points` (fewer than two reads as linear), named `label`. `points` holds the latest value the
/// editor proposed; the returned events are the pass's edit in unit points: a drag's begin, update and commit, and a
/// preset click as one `Set`, so a caller files a curve drag under one history key.
pub fn editor(ui: &mut egui::Ui, label: &str, points: &mut Vec<[f32; 2]>) -> EditEvents<Vec<[f32; 2]>> {
    let start: Vec<[f32; 2]> = if points.len() < 2 { PRESETS[0].1.to_vec() } else { points.clone() };
    let mut curve = Curve::smooth(start.iter().map(|p| [p[0] * LEVELS, p[1] * LEVELS]).collect());
    let preset = Cell::new(None);
    let out = CurveEditor::new(label.to_string(), Bind::bound(&mut curve))
        .levels(LEVELS as u32)
        .max_points(PressureCurve::MAX_POINTS)
        .fields(false)
        .between(|ui: &mut egui::Ui, _: &mut BodyCx<'_>| {
            Row::new().show(ui, |ui, _| {
                for (at, (name, _)) in PRESETS.iter().enumerate() {
                    if Button::new(tl!(name)).show(ui).clicked() {
                        preset.set(Some(at));
                    }
                }
            });
        })
        .show(ui);
    let mut events = out.edit.events.map(|curve| unit_points(&curve));
    if let Some(at) = preset.get() {
        events = EditEvents::new();
        events.push(EditEvent::set(Mixed::Same(start), PRESETS[at].1.to_vec(), label.into(), None));
    }
    if let Some(value) = events.value() {
        *points = value.clone();
    }
    events
}
