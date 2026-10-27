//! Vector tools: Shape tools (U), Pen (P), Path Selection (A), path overlays and the Paths panel.
//! Direct Selection (A) lives in `direct_select`.
//! All edits go through the engine's `shape.*` / `path.*` commands.

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::vector::Path;
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Affine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::shape_gradient_ui::GradientChoices;
use crate::shape_paint_ui::{PaintChange, paint_param, swatch};
use crate::state::{Tool, ToolOptions};
use crate::theme::Tokens;

mod curvature;
mod options;
mod overlay;
mod path_ops;
mod paths_panel;
mod pen;
mod shapes;

pub(crate) use curvature::curvature_anchor_at;
pub use curvature::{curvature_down, curvature_move, curvature_toggle, curvature_up};
#[cfg(test)]
use options::{corner_radii_patch, solid_paint};
pub(crate) use options::{enabled_stroke_width, paint_gesture};
pub use options::{options_bar, shape_properties};
pub use overlay::draw_overlay;
#[cfg(test)]
use overlay::{MAX_CURVE_PIECES, bezier};
#[cfg(test)]
pub(crate) use overlay::{curve_points, path_lines, selected_saved_path, shows_anchors};
pub(crate) use overlay::{draw_anchor, draw_handle, draw_outline};
pub use path_ops::{active_path_name, path_selection_finish, path_to_selection, pick_shape_at, selected_document_path, takes_path_mask, with_active_path};
pub(crate) use path_ops::{active_shape_path, delete_key_deletes_work_path, free_transform_path, targeted_vector_mask};
#[cfg(test)]
use paths_panel::path_context_actions;
pub use paths_panel::{PathEntry, PathRow, path_rows, paths_footer, paths_panel};
#[cfg(test)]
pub(crate) use pen::over_first_anchor;
#[cfg(test)]
use pen::pen_to_json;
pub use pen::{cancel_stale, closes_path_at, freeform_finish, pen_break_handle, pen_commit, pen_down, pen_grab_handle, pen_move, pen_undo_last_point, pen_up};
pub(crate) use pen::{pen_path, pen_replace};
pub use shapes::{create_shape, draw_shape_preview, finish_shape, is_shape_tool, shape_bounds};
#[cfg(test)]
use shapes::{shape_geometry, shape_preview};

// "Set as default" is a temporary UI choice, not a document property.
#[derive(Clone, Copy, Default)]
struct ArrowDefaultLink {
    target: Option<(u64, u64)>,
    enabled: bool,
}

fn arrow_default_link_id() -> egui::Id {
    egui::Id::unique("arrow-default-link")
}

/// Reset the checkbox whenever the selected document, layer or shape changes.
pub(crate) fn track_arrow_default_selection(app: &PhotocraftApp, ctx: &egui::Context) {
    let target = app.session.active().and_then(|st| {
        let id = st.active_layer?;
        let layer = st.doc.layer(id)?;
        match &layer.content {
            LayerContent::Shape(sh) if matches!(sh.live.as_ref(), Some(photocraft_doc::vector::LiveShape::ArrowLine { .. })) => Some((st.doc.id.0, id.0)),
            _ => None,
        }
    });

    ctx.data_mut(|data| {
        let key = arrow_default_link_id();
        let previous = data.get_temp::<ArrowDefaultLink>(key).unwrap_or_default();
        if previous.target != target {
            data.insert_temp(key, ArrowDefaultLink { target, enabled: false });
        }
    });
}

/// Copy only the selected arrow's live parameters into new-arrow defaults.
/// Existing shapes and geometry are not modified.
fn copy_arrow_to_tool_defaults(app: &mut PhotocraftApp, id: photocraft_doc::LayerId) {
    let values = app.session.active().and_then(|st| st.doc.layer(id)).and_then(|layer| match &layer.content {
        LayerContent::Shape(sh) => match sh.live.as_ref() {
            Some(photocraft_doc::vector::LiveShape::ArrowLine { start, end, width, length, concavity, weight, .. }) => {
                {
                    // Live dimensions may exceed new-arrow input limits after scaling.
                    // Clamp only the defaults, never the existing shape.
                    let bounded = |v: f64, min: f64, max: f64| -> f32 { if v.is_finite() { v.clamp(min, max) as f32 } else { min as f32 } };
                    Some((
                        *start,
                        *end,
                        bounded(*width, 1.0, 4096.0),
                        bounded(*length, 1.0, 4096.0),
                        bounded(*concavity, 0.0, 100.0),
                        bounded(*weight, 1.0, 10000.0),
                    ))
                }
            }
            _ => None,
        },
        _ => None,
    });

    if let Some((start, end, width, length, concavity, weight)) = values {
        let o = &mut app.ui.tool_options;
        o.arrow_start = start;
        o.arrow_end = end;
        o.arrow_width = width;
        o.arrow_length = length;
        o.arrow_concavity = concavity;
        o.line_weight = weight;
    }
}

/// Pen tool path under construction: knots as [anchor, in, out] (document px).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PenPath {
    pub knots: Vec<[[f64; 2]; 3]>,
    /// Indices of cusp knots whose incoming and outgoing handles are not linked.
    #[serde(default)]
    pub unlinked: Vec<usize>,
    #[serde(skip)]
    pub dragging: bool,
    /// Re-dragging the final anchor changes its outgoing control only; the incoming curve stays put.
    #[serde(skip)]
    pub adjusting_last: bool,
    /// ⌥/Alt-dragging the final anchor's incoming handle: it turns alone (#6077).
    #[serde(skip)]
    pub adjusting_in: bool,
    /// Drawn with the Curvature Pen: handles follow the anchors (`curvature_refresh`) and
    /// `unlinked` lists its corner points.
    #[serde(default)]
    pub curvature: bool,
    /// Curvature Pen: the anchor being dragged.
    #[serde(skip)]
    pub moving: Option<usize>,
    /// Drawing carries on from an endpoint of this open subpath of a document path (#6593): the
    /// commit replaces that subpath instead of adding one.
    #[serde(default)]
    pub resumes: Option<Resumed>,
}

/// The subpath a Pen path carries on: where it is and what it was when drawing resumed. The
/// commit replaces it only while it is still that subpath of that document; otherwise (another
/// document, or the path changed meanwhile) the drawing is added as a new subpath.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resumed {
    pub doc: u64,
    pub path: String,
    pub index: usize,
    pub was: photocraft_doc::vector::Subpath,
}

impl PenPath {
    /// Does `tool` keep drawing this path? Another tool finishes it.
    pub fn drawn_with(&self, tool: Tool) -> bool {
        tool == if self.curvature { Tool::CurvaturePen } else { Tool::Pen }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod path_selection_tests;

#[cfg(test)]
mod pen_draw_tests;

#[cfg(test)]
mod paint_tests;
