//! Vector tools: Shape tools (U), Pen (P), Path Selection (A), path overlays and the Paths panel.
//! Direct Selection (A) lives in `direct_select`.
//! All edits go through the engine's `shape.*` / `path.*` commands.

use egui::Color32;
#[cfg(test)]
use egui::vec2;
use photocraft_doc::vector::Path;
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Affine;
use serde_json::{Value, json};

use craft::draw::ui::{EditSubpath, VectorMode};

use crate::PhotocraftApp;
use crate::shape_paint_ui::paint_param;
use crate::state::{Tool, ToolOptions};

pub(crate) mod alignment;
mod freeform;
pub(crate) mod host;
mod options;
pub(crate) mod overlay;
mod path_menus;
mod path_ops;
mod paths_panel;
mod pen;
mod pen_resume;
pub(crate) mod shapes;

pub(crate) use options::enabled_stroke_width;
#[cfg(test)]
use options::{corner_radii_patch, solid_paint};
pub use options::{options_bar, shape_properties};
#[cfg(test)]
pub(crate) use overlay::selected_saved_path;
pub(crate) use overlay::selected_saved_path_index;
pub use path_ops::{
    active_path_name, make_shape, path_selection_finish, path_to_selection, pick_shape_at, selected_document_path, takes_path_mask, with_active_path,
};
pub(crate) use path_ops::{
    active_shape_path, active_type_path, delete_key_edits_path, deselect_loaded_path, edit_path, free_transform_path, saved_path_components_selected,
    targeted_vector_mask, work_path_shown,
};
pub use paths_panel::{PathEntry, PathRow, path_rows, paths_panel};
#[cfg(test)]
use pen::pen_path_op;
pub(crate) use pen::{PATH_OPS, path_op_field, pen_path};
pub use pen::{cancel_stale, pen_commit};
pub use shapes::is_shape_tool;
pub(crate) use shapes::{create_geometry, kind_of, tool_history_name};

#[cfg(test)]
use crate::shape_paint_ui::PaintChange;
#[cfg(test)]
pub(crate) use host::remove_last;
pub(crate) use host::{VectorTools, replace_draft};
#[cfg(test)]
pub(crate) use path_ops::delete_key_deletes_work_path;
#[cfg(test)]
pub(crate) use pen::freeform_finish_owned;

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
                        bounded(*concavity, -50.0, 50.0),
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

/// Pen tool path under construction. The draft is the framework's [`EditSubpath`] (document px); the tool reports it
/// whole and the app keeps it here between passes. It is never serialised.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PenPath {
    /// The anchors, their handles and whether the path is closed.
    pub draft: EditSubpath,
    /// The leading anchors that keep their handles when the Curvature Pen reflows the draft
    /// (`EditSubpath::reflow_kept`): those of the path a press carried on.
    pub kept: usize,
    /// Drawn with the Curvature Pen: handles follow the anchors.
    pub curvature: bool,
    /// The document and target the first anchor was placed on; a draft without one is never committed.
    pub(crate) owner: Option<PenOwner>,
    /// The last commit failed: the draft stays as it was and isn't retried on its own.
    pub(crate) commit_failed: bool,
    /// Runtime origin of a refused Freeform release; its own tool keeps the existing anchor Undo.
    pub(crate) freeform_origin: bool,
    /// Drawing carries on from an endpoint of this open subpath of a document path (#6593): the
    /// commit replaces that subpath instead of adding one.
    pub resumes: Option<Resumed>,
    /// The open draft as it was before the press that closed it: a close drag pulls the first anchor's handles in the
    /// closed path only, so Undo gives the path back as this.
    pub(crate) before_close: Option<EditSubpath>,
    /// The draft's shape layer was made on its first anchor (#12909) and takes its anchors as they are placed;
    /// `resumes` names that layer and its subpath.
    pub(crate) live_layer: bool,
    /// The draft's Work Path was made on its first anchor (#12890) and takes each click as its own History step;
    /// `resumes` names that path and its subpath.
    pub(crate) live_path: bool,
    /// The ids of the History states this draft's own live writes minted (#16686 review), oldest first.
    /// A step's name is not proof of who wrote it - `path.set` takes a Pen step name from any caller - while a
    /// History state id (`History::row_id`) is minted by History, never reused, and no parameter can carry one.
    /// A live draft follows its path only onto a state it minted itself, or one whose successor is waiting to be
    /// redone; anything else the document moved to is a step from elsewhere and makes the draft stale.
    pub(crate) live_steps: Vec<u64>,
}

/// The subpath a Pen path carries on: where it is and what it was when drawing resumed. The
/// commit replaces it only while it is still that subpath of that document; otherwise (another
/// document, or the path changed meanwhile) the drawing is added as a new subpath.
#[derive(Clone, Debug, PartialEq)]
pub struct Resumed {
    /// The document the path belongs to.
    pub doc: u64,
    /// A document path's name ("work" or a saved path's), or "layer" for `layer`'s.
    pub path: String,
    /// The shape layer (Shape mode) or targeted vector mask whose path it is.
    pub layer: Option<u64>,
    /// The subpath's index in the path.
    pub index: usize,
    /// The subpath as it was when drawing resumed.
    pub was: photocraft_doc::vector::Subpath,
}

impl PenPath {
    /// Does `tool` keep drawing this path? Another tool finishes it.
    pub fn drawn_with(&self, tool: Tool) -> bool {
        tool == if self.curvature { Tool::CurvaturePen } else { Tool::Pen }
    }

    /// A retained failed Freeform release keeps Undo in its own tool without treating another
    /// tool's retained Pen draft as a Freeform draft. Drawing and commit ownership stay separate.
    pub(crate) fn owns_undo(&self, tool: Tool) -> bool {
        self.drawn_with(tool) || (self.freeform_origin && tool == Tool::FreeformPen)
    }
}

#[cfg(test)]
impl PenPath {
    /// A draft of corner anchors at `points`, owned by `owner`.
    pub(crate) fn corners(points: &[[f64; 2]], owner: Option<PenOwner>) -> Self {
        let knots = points.iter().map(|p| craft::draw::ui::Knot::corner(craft::behavior::kurbo::Point::new(p[0], p[1]))).collect();
        Self { draft: EditSubpath::new(knots, false), owner, ..Default::default() }
    }

    /// The knots as anchor, in handle and out handle.
    pub(crate) fn triples(&self) -> Vec<[[f64; 2]; 3]> {
        self.draft.knots.iter().map(|k| [[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]]).collect()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod path_drag_preview_tests;

#[cfg(test)]
mod path_selection_tests;

#[cfg(test)]
mod pen_draw_tests;

#[cfg(test)]
mod pen_path_ops_tests;

#[cfg(test)]
mod pen_indexed_shape_tests;

#[cfg(test)]
mod pen_shape_mode_options_tests;

#[cfg(test)]
mod pen_rubber_band_tests;

#[cfg(test)]
mod paint_tests;

#[cfg(test)]
mod triangle_box_geometry_tests;
#[cfg(test)]
mod triangle_properties_fields_tests;

#[cfg(test)]
mod pen_resume_tests;

#[cfg(test)]
mod pen_enter_deselects_tests;

#[cfg(test)]
mod ellipse_path_menus_tests;

#[cfg(test)]
mod curvature_delete_key_tests;

#[cfg(test)]
mod shape_properties_tests;

#[cfg(test)]
mod test_input;

#[cfg(test)]
mod line_shift_snap_tests;

#[cfg(test)]
mod ellipse_shift_circle_tests;

#[cfg(test)]
mod polygon_align_edges_tests;
#[cfg(test)]
mod rectangle_align_edges_tests;

#[cfg(test)]
mod triangle_align_edges_tests;

#[cfg(test)]
mod triangle_shift_equilateral_tests;

#[cfg(test)]
mod rectangle_gear_tests;

#[cfg(test)]
mod rectangle_click_in_shape_tests;

#[cfg(test)]
mod custom_shape_live_box_tests;

#[cfg(test)]
use pen_resume::pen_finish_on_redo;
pub(crate) use pen_resume::{FinishedPen, pen_history_enabled, pen_history_step};

/// Where a Pen draft goes, captured at its first anchor.
#[derive(Clone, Debug)]
pub(crate) struct PenOwner {
    document: DocId,
    revision: u64,
    source: Weak<Document>,
    target: PenTarget,
    options: ToolOptions,
    layer: Option<photocraft_doc::LayerId>,
    vector_mask: bool,
    foreground: [f32; 4],
    background: [f32; 4],
}

impl PartialEq for PenOwner {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document && self.revision == other.revision && self.source.ptr_eq(&other.source) && self.target == other.target
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PenTarget {
    /// A new shape layer, with the paint chosen when drawing began.
    Shape {
        fill: Value,
        stroke: Value,
        open_stroke: Value,
    },
    VectorMask(u64),
    /// The work path or a saved path; `append` adds the draft as a subpath (#4721).
    Path {
        name: String,
        append: bool,
    },
}

/// Where the Pen draws now: a new shape layer (Shape mode), else the work or saved path selected
/// in the Paths panel (the new path joins it, #4721), else the targeted vector mask, else a new
/// Work Path that replaces the old one.
pub(crate) fn capture_pen_owner(app: &PhotocraftApp) -> Option<PenOwner> {
    let st = app.session.active()?;
    let target = if app.ui.tool_options.vector_mode == VectorMode::Shape {
        let open_stroke = shapes::open_pen_stroke_param(app);
        PenTarget::Shape { fill: fill_param(app), stroke: stroke_param(app), open_stroke }
    } else if let Some((name, _)) = selected_document_geometry(app) {
        PenTarget::Path { name, append: true }
    } else if let Some(id) = targeted_vector_mask_id(app) {
        PenTarget::VectorMask(id)
    } else {
        PenTarget::Path { name: "work".into(), append: false }
    };
    let mut options = (*app.ui.tool_options).clone();
    // Combining or resuming starts with the layer's displayed setting without replacing the user's
    // New Layer default merely because a different target was selected.
    if options.vector_mode == VectorMode::Shape
        && (pen::pen_path_op(&options).is_some() || app.ui.pen.as_ref().is_some_and(|p| p.resumes.as_ref().is_some_and(|r| r.layer.is_some())))
        && let Some(LayerContent::Shape(shape)) = st.active_layer.and_then(|id| st.doc.layer(id)).map(|layer| &layer.content)
    {
        options.align_edges = shape.use_aligned_rendering;
    }
    Some(PenOwner {
        document: st.doc.id,
        revision: st.revision,
        source: Arc::downgrade(&st.doc),
        target,
        options,
        layer: st.active_layer,
        vector_mask: app.ui.vector_mask_target,
        foreground: app.session.tools.foreground,
        background: app.session.tools.background,
    })
}

impl PenOwner {
    fn valid(&self, app: &PhotocraftApp) -> bool {
        app.session.active().is_some_and(|st| st.doc.id == self.document && st.revision == self.revision && self.source.ptr_eq(&Arc::downgrade(&st.doc)))
    }
}

/// The document, revision and target a vector gesture began on, for a tool that must not preview or finish on another
/// document or target.
#[derive(Clone, Debug)]
pub(crate) struct VectorOwner {
    doc: DocId,
    document: Weak<Document>,
    revision: u64,
    layer: Option<photocraft_doc::LayerId>,
    selected_path: Option<String>,
    vector_mask: bool,
    mode: VectorMode,
    tool: Tool,
}

impl VectorOwner {
    /// The owner of a gesture that begins now; `None` with no document.
    pub(crate) fn capture(app: &PhotocraftApp) -> Option<Self> {
        let st = app.session.active()?;
        Some(Self {
            doc: st.doc.id,
            document: Arc::downgrade(&st.doc),
            revision: st.revision,
            layer: st.active_layer,
            selected_path: app.ui.selected_path.clone(),
            vector_mask: app.ui.vector_mask_target,
            mode: app.ui.tool_options.vector_mode,
            tool: app.ui.tool,
        })
    }

    /// Whether the document, revision and target are still the ones the gesture began on.
    pub(crate) fn valid(&self, app: &PhotocraftApp) -> bool {
        Self::capture(app).is_some_and(|now| {
            self.doc == now.doc
                && self.document.ptr_eq(&now.document)
                && self.revision == now.revision
                && self.layer == now.layer
                && self.selected_path == now.selected_path
                && self.vector_mask == now.vector_mask
                && self.mode == now.mode
                && self.tool == now.tool
        })
    }
}

use photocraft_doc::DocId;
use std::sync::{Arc, Weak};

use path_ops::{selected_document_geometry, targeted_vector_mask_id};
use shapes::{fill_param, stroke_param};

/// Whether Edit Undo currently belongs to the active tool's live or refused Freeform draft.
pub(crate) fn pen_can_undo(app: &PhotocraftApp) -> bool {
    app.ui.pen.as_ref().is_some_and(|pen| {
        pen.owns_undo(app.ui.tool) && pen.owner.as_ref().is_some_and(|owner| owner.valid(app)) && (!pen.draft.knots.is_empty() || pen.resumes.is_some())
    })
}

/// Commit a valid source-owned draft on a tab switch; otherwise cancel with an actionable diagnostic.
pub(crate) fn cancel_stale_pen(app: &mut PhotocraftApp) -> bool {
    // A live shape layer's draft follows the document through Undo, Redo and History (#12909).
    pen::sync_live_draft_from_layer(app);
    let Some(pen) = app.ui.pen.as_ref() else { return false };
    if pen.owner.as_ref().is_some_and(|o| o.valid(app)) {
        return false;
    }
    if pen.commit_failed {
        return false;
    }
    let owner = pen.owner.clone();
    if let Some(owner) = owner.as_ref() {
        let index = app
            .session
            .documents()
            .iter()
            .position(|st| st.doc.id == owner.document && st.revision == owner.revision && owner.source.ptr_eq(&Arc::downgrade(&st.doc)));
        if let Some(index) = index {
            let active = app.session.active_index();
            let saved_ui = app.ui.clone();
            let draft = app.ui.pen.take();
            app.session.set_active(index);
            app.ui.pen = draft;
            // This source matches the immutable owner; normal commit validation still applies.
            let result = pen::try_pen_commit(app, false);
            if let Some(active) = active {
                app.session.set_active(active);
            }
            let failed = app.ui.pen.take();
            let message = app.ui.status.clone();
            // The temporary source commit ran sync_views on that document. Rebind its runtime
            // tracker before restoring the destination UI, so the next frame cannot clear
            // an unrelated saved-path selection in the destination tab.
            app.path_document = app.session.active().map(|st| (st.doc.id, st.revision, Arc::downgrade(&st.doc)));
            app.ui = saved_ui;
            app.ui.pen = if result.is_ok() { None } else { failed };
            if result.is_ok() && app.ui.direct_selection.target == Some(crate::direct_select::PathRef::Pen) {
                app.ui.direct_selection = Default::default();
            }
            if result.is_err() {
                app.ui.status = message;
                app.ui.status_error = true;
            }
            app.ui.pen_doc = app.ui.pen.as_ref().and_then(|p| p.owner.as_ref().map(|o| o.document));
            return true;
        }
    }
    host::drop_draft(app);
    app.ui.status = "Pen draft cancelled because its source document or path changed; newer work was preserved".into();
    app.ui.status_error = true;
    true
}
