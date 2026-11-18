//! The live gradient overlay: `GradientOverlay` over the selected Gradient Fill layer while the Gradient tool is in
//! live mode. A drag of a handle, stop or midpoint proposes a fill for the picture of the same pass; its release (and
//! a click that adds a stop, or a key) runs one `gradient.fill.set`.

use craft::behavior::EditEvent;
use craft::behavior::kurbo::Point;
use craft::color_ui::gradient::{StopRows, StopRules};
use craft::draw::ui::gradient::{GradientLine, GradientOverlay, GradientStyle as OverlayStyle, LineGradient};
use craft::ui::contract::{Bind, Show as _};
use egui::{Rect, Ui};
use photocraft_doc::{Fill, GradientStyle, Layer, LayerContent, LayerId};
use photocraft_engine::gradient_fill_cmds as cmds;
use serde_json::{Value, json};

use super::{active_gradient, edit_stop_color, edited_fill, live_mode, stops};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The overlay's style for a document fill style: Radial and Angle share the circle the overlay draws.
fn style_of(style: GradientStyle) -> OverlayStyle {
    match style {
        GradientStyle::Linear => OverlayStyle::Linear,
        GradientStyle::Radial => OverlayStyle::Radial,
        GradientStyle::Angle => OverlayStyle::Angle,
        GradientStyle::Reflected => OverlayStyle::Reflected,
        GradientStyle::Diamond => OverlayStyle::Diamond,
    }
}

fn point(p: [f32; 2]) -> Point {
    Point::new(f64::from(p[0]), f64::from(p[1]))
}

/// `gradient.fill.set` params for `value` on `layer`: the line in document pixels and the stops.
fn set_params(layer: LayerId, value: &LineGradient) -> Value {
    let at = |p: Point| [p.x as f32, p.y as f32];
    let mut p = stops::fill_params(&value.stops);
    p["layer"] = json!(layer.0);
    p["from"] = json!(at(value.line.start));
    p["to"] = json!(at(value.line.end));
    p
}

fn fill_of(layer: &Layer) -> Option<&Fill> {
    match &layer.content {
        LayerContent::Fill(f @ Fill::Gradient { .. }) => Some(f),
        _ => None,
    }
}

/// Shows the overlay over the selected Gradient Fill layer and returns the fill a running gesture proposes this pass,
/// for the picture of the same pass. `None` when the overlay is not shown, nothing is being edited, or the pass
/// committed (the document then holds the value). `at` is the canvas body rect, as the canvas's other controls take
/// it. A pass applies only its own events, so a second pass in one frame,
/// which carries none of the first pass's input, runs no command twice.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut Ui, at: Rect) -> Option<(LayerId, Fill)> {
    if !live_mode(app) {
        return None;
    }
    // ⌘ is the Move tool (`hold_keys::cmd_moves`): the overlay steps aside while it is held.
    if craft::ui::runtime::pointer::frame(ui.ctx()).mods.cmd && crate::canvas::tools::takes_command_stand_in(Tool::Gradient) {
        return None;
    }
    let (layer, frame, canvas) = active_gradient(app)?;
    let fill = fill_of(&layer)?.clone();
    let (start, end) = photocraft_compose::gradient_fill::fill_handles(&fill, frame)?;
    let Fill::Gradient { style, reverse, .. } = &fill else { return None };
    let value = LineGradient::new(GradientLine::new(point(start), point(end)), stops::of_fill(&fill));
    let sampler = stops::sampler(app.session.tools.foreground, app.session.tools.background);
    let mut overlay =
        GradientOverlay::new("Gradient", Bind::given(value)).style(style_of(*style)).midpoints(true).rules(StopRules::new(StopRows::Split)).sampler(&sampler);
    if *reverse {
        overlay = overlay.reverse();
    }
    let out = overlay.show_at(ui, at);
    if let Some(requested) = &out.color_requested {
        edit_stop_color(app, layer.id, requested.index);
    }
    let events = &out.edit.events;
    match events.ended() {
        Some(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
            if let Err(error) = app.run(cmds::SET, set_params(layer.id, after)) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
            None
        }
        Some(_) => None,
        None => {
            let proposed = events.value()?;
            let fill = edited_fill(app, &layer, canvas, cmds::SET, &set_params(layer.id, proposed))?;
            Some((layer.id, fill))
        }
    }
}
