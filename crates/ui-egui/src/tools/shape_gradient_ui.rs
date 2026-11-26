//! The shape Fill / Stroke chips' Gradient tab: the framework's `GradientFillEditor` over the
//! document's `Fill::Gradient`. The tab returns the edited gradient and the caller applies it to the
//! next shape or as one `shape.edit`.

use craft::draw::ui::gradient::{GradientFill, GradientFillEditor, GradientGroup, GradientItem, GradientRef};
use craft::ui::contract::Bind;
use craft::ui::patterns::presets::PresetState;
use photocraft_doc::{Fill, GradientStyle};
use photocraft_engine::presets::Group;
use photocraft_engine::presets::gradients::GradientPreset;

// PLAN: `gradient_ui::stops` (`of_preset`, `fill_value`, `apply_value`) is tools-w47's and must be `pub(crate) mod stops` in gradient_ui.rs.
use crate::gradient_ui::stops;

/// How many gradients the Recent row keeps.
const RECENT: usize = 8;

/// What a chip's Gradient tab offers: the preset library, the recently used gradients, and the
/// Gradient tool's current gradient (what a colour switches to), with the tool colours that
/// resolve their Foreground / Background stops.
pub(crate) struct ShapeGradients<'a> {
    pub groups: Vec<GradientGroup>,
    pub current: &'a GradientPreset,
    /// Newest first ([`ShapeGradients::remember`]).
    pub recent: &'a mut Vec<Fill>,
    pub fg: [f32; 4],
    pub bg: [f32; 4],
}

impl ShapeGradients<'_> {
    /// The preset library as the editor's groups, live colours resolved against `fg` and `bg`.
    pub(crate) fn groups_of(presets: &[Group<GradientPreset>], fg: [f32; 4], bg: [f32; 4]) -> Vec<GradientGroup> {
        presets
            .iter()
            .map(|g| {
                // PLAN: assumed `gradient_ui::stops::of_preset(&GradientPreset, [f32; 4], [f32; 4]) -> GradientStops` (tools-w47).
                let items = g.items.iter().map(|p| GradientItem::new(p.name.clone(), stops::of_preset(p, fg, bg))).collect();
                GradientGroup::new(g.name.clone(), items)
            })
            .collect()
    }

    /// Photoshop's shape gradient default: the current gradient, linear at 90°, 100%.
    pub(crate) fn default_fill(&self) -> Fill {
        let base = Fill::gradient(Vec::new(), 90.0, 1.0, GradientStyle::Linear, false);
        stops::apply_value(&base, &GradientFill::new(stops::of_preset(self.current, self.fg, self.bg)))
    }
}

/// [`ShapeGradients::remember`] on the row itself, for a caller that no longer holds the choices.
pub(crate) fn remember_recent(recent: &mut Vec<Fill>, gradient: &Fill) {
    let Fill::Gradient { .. } = gradient else { return };
    recent.retain(|f| !same_stops(f, gradient));
    recent.insert(0, gradient.clone());
    recent.truncate(RECENT);
}

/// Whether two gradients have the same colour stops, opacity stops and midpoints.
fn same_stops(a: &Fill, b: &Fill) -> bool {
    match (a, b) {
        (Fill::Gradient { stops: s1, opacity_stops: o1, midpoints: m1, .. }, Fill::Gradient { stops: s2, opacity_stops: o2, midpoints: m2, .. }) => {
            s1 == s2 && o1 == o2 && m1 == m2
        }
        _ => false,
    }
}

/// The Gradient tab for `fill`: presets (recent first), stops, style, angle, scale, reverse, dither
/// and align. Returns the edited gradient, `None` while nothing changed this pass.
pub(crate) fn editor(ui: &mut egui::Ui, fill: &Fill, gradients: &mut ShapeGradients, state: Bind<PresetState<GradientRef>>) -> Option<Fill> {
    let Fill::Gradient { .. } = fill else { return None };
    let before = stops::fill_value(fill);
    let mut value = stops::fill_value(fill);
    let recent: Vec<_> = gradients.recent.iter().map(|f| stops::fill_value(f).stops).collect();
    // PLAN: the editor's `color_requested` and `screen_pick` are not routed: the stop colour opens in the editor's own popover.
    let _ = GradientFillEditor::new(tl!("Gradient"), Bind::bound(&mut value), &gradients.groups, state).recent(&recent).show(ui);
    (value != before).then(|| stops::apply_value(fill, &value))
}
