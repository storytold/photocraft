use super::PhotocraftApp;
use super::tools::selection::floating_outline;
use crate::state::Tool;
use craft::behavior::EditEvent;
use craft::behavior::geometry::quad::{BoxRules, BoxState, TransformMode};
use craft::behavior::kurbo::{self, BezPath};
use craft::draw::ui::transform::{RotateZone, TransformBox, TransformEdit};
use craft::ui::contract::{Bind, Props as _, Show as _};

// The hit-zone tests (`transform_hit_zone_tests.rs`, tools-w31) read `super::*`.
#[cfg(test)]
#[allow(unused_imports)]
use super::{Color32, LayerContent, Pos2, Rect, Stroke, Vec2, vec2};

/// View › Show › Layer Edges: the active layer's content bounds (cached per tile) where the canvas
/// shows them, as a closed path. During a Move drag that is at the offset the image is drawn
/// with, so the outline moves in lockstep with the layer's pixels (#2413). `None` with Layer Edges off.
pub(crate) fn layer_edges(app: &PhotocraftApp, idx: usize) -> Option<BezPath> {
    if !app.ui.view.shows(app.ui.view.show.layer_edges) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let id = st.active_layer?;
    let b = st.doc.layer(id)?.surface().map(photocraft_compose::bounds::content_bounds)?;
    if b.is_empty() {
        return None;
    }
    let (dx, dy) = crate::move_ui::shown_layer_offset(app, st.doc.id, st.revision, id).unwrap_or((0, 0));
    let b = b.translate(dx, dy);
    let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    let mut path = BezPath::new();
    path.move_to((x0, y0));
    path.line_to((x1, y0));
    path.line_to((x1, y1));
    path.line_to((x0, y1));
    path.close_path();
    Some(path)
}

#[cfg(test)]
/// The layers Move › Show Transform Controls frames: every selected layer, the Background too when
/// it is among several (a handle drag then refuses, [`begin_controls`]), but not the Background
/// alone (#11101).
/// A corner radius drag hides the box, as in Photoshop. So does View › Extras (⌘H, #6699): the
/// controls are an Extra, hidden and inert until Extras comes back, while Show Transform Controls
/// keeps its setting.
pub(crate) fn frame_layers(app: &PhotocraftApp) -> Vec<&photocraft_doc::Layer> {
    framed_layers(app, false)
}

/// [`frame_layers`], with `pressed` keeping the frame while a canvas control holds a press: the
/// press that began on the controls' own box is one, and [`begin_controls`] still frames its layers.
fn framed_layers(app: &PhotocraftApp, pressed: bool) -> Vec<&photocraft_doc::Layer> {
    if app.ui.tool != Tool::Move
        || !app.ui.tool_options.move_show_transform
        || !app.ui.view.extras
        || app.ui.transform.is_some()
        || app.canvas_tools.mover.dragging()
        || (!pressed && crate::shape_controls::editing(app))
    {
        return Vec::new();
    }
    let Some(st) = app.session.active() else { return Vec::new() };
    let selected = st.selected_layers();
    let several = selected.len() > 1;
    selected.into_iter().filter_map(|id| st.doc.layer(id)).filter(|l| several || !crate::doc_props_ui::is_background(&st.doc, l)).collect()
}

/// Move tool › Show Transform Controls: the boxes Free Transform would open on the selected
/// layers, joined into one, in document units.
pub(crate) fn frame(app: &PhotocraftApp) -> Option<kurbo::Rect> {
    framed(app, false)
}

fn framed(app: &PhotocraftApp, pressed: bool) -> Option<kurbo::Rect> {
    let st = app.session.active()?;
    let layers = framed_layers(app, pressed);
    let b = layers.iter().map(|l| photocraft_engine::transform_cmds::transform_frame(&st.doc, l)).filter(|b| !b.is_empty()).reduce(|a, b| a.union(&b))?;
    // Floating selected pixels are framed by their outline, as their ants are (#7538), not by the
    // selection they were cut with; a frame round other selected layers too keeps them all.
    let alone = matches!(layers.as_slice(), [l] if photocraft_engine::float_cmds::floating(st).is_some_and(|f| f.layer == l.id));
    let b = app.session.active_index().filter(|_| alone).and_then(|idx| floating_outline(app, idx)).map_or(b, |o| o.content_bounds());
    let (dx, dy) = crate::move_ui::floating_offset(app).unwrap_or((0, 0));
    let b = b.translate(dx, dy);
    if b.is_empty() {
        return None;
    }
    Some(kurbo::Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)))
}

/// The reference application's alert when a Move-tool transform control is dragged on a frame
/// that holds the Background among other layers (measured, #11101).
pub(crate) const CONTROLS_LOCKED: &str = "Could not transform because the layer is locked.";

/// What a press on the Move tool's transform controls did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlsPress {
    /// Not on a control, or no session could start: the press is the Move tool's.
    Missed,
    /// A Free Transform session started; the press goes on to drive it.
    Started,
    /// Refused with the locked-layer alert: the press goes nowhere else (#11101).
    Refused,
}

/// Enter the existing Free Transform session on the first `Begin` of the controls' box: on every
/// selected layer, as the frame shows them (#6501). With the Background among them it shows the
/// locked-layer alert and changes nothing (#11101).
pub(crate) fn begin_controls(app: &mut PhotocraftApp, ctx: &egui::Context) -> ControlsPress {
    if framed(app, true).is_none() {
        return ControlsPress::Missed;
    }
    let background = app.session.active().is_some_and(|st| photocraft_engine::transform_cmds::background_among(&st.doc, &st.selected_layers()).is_some());
    if background {
        // PLAN: assumed `dialogs::alerts::alert(app: &mut PhotocraftApp, message: &str)` (dialogs-w21).
        crate::dialogs::alerts::alert(app, tl!(CONTROLS_LOCKED));
        return ControlsPress::Refused;
    }
    match crate::transform_tool::begin(app, ctx) {
        Ok(()) => ControlsPress::Started,
        Err(e) => {
            app.ui.status = e;
            ControlsPress::Missed
        }
    }
}

/// The Move tool's transform box in the accessibility tree.
const CONTROLS_LABEL: &str = "Transform controls";

/// Move › Show Transform Controls over the canvas body `rect`: a `TransformBox` on [`frame`] whose inside and outside
/// (past the corner rotate bands) and arrow keys stay the Move tool's. Its first `Begin` runs [`begin_controls`]; once
/// a session starts, this pass's box events go to the session, and from the next pass the session's
/// `TransformOverlay`, salted with the same [`BOX_ID_SALT`](crate::transform_tool::BOX_ID_SALT), carries the drag on.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect, session_box_shown: bool) {
    if session_box_shown {
        return;
    }
    let Some(start) = frame(app) else { return };
    let settings = crate::transform_tool::settings(app);
    // The rules a new session starts with (`TransformSession::box_rules` in Free mode), so the drag reads the same
    // before and after the hand-off.
    let rules = BoxRules::in_mode(TransformMode::Free).with_scale(settings.scale_rule()).with_aspect_linked(settings.aspect_linked);
    let out = TransformBox::new(CONTROLS_LABEL, Bind::given(BoxState::from_rect(start)), start, 0.0)
        .rules(rules)
        .rotate_zone(RotateZone::Band)
        .reference_point(false)
        .inside_moves(false)
        .session_arrows(false)
        .id_salt(crate::transform_tool::BOX_ID_SALT)
        .show_at(ui, rect);
    if !out.edit.events.iter().any(|event| matches!(event, EditEvent::Begin { .. })) {
        return;
    }
    if begin_controls(app, ui.ctx()) != ControlsPress::Started {
        return;
    }
    let now = ui.input(|i| i.time);
    if let Some(t) = app.ui.transform.as_mut() {
        for event in out.edit.events.iter().cloned() {
            t.session.apply(TransformEdit::Box(event), now);
        }
    }
}

#[cfg(test)]
#[path = "../transform_hit_zone_tests.rs"]
mod hit_zone_tests;
