use craft::draw::ui::pen::{AnchorsShown, BezierOutput, BezierOverlay, BezierTool, EditPath, PathMode};
use craft::keymap::Mods;
use craft::ui::contract::{Bind, Show as _};
use photocraft_doc::vector::Path;
use photocraft_doc::{DocId, Document, LayerContent};

use super::path_ops::{MovedPath, active_type_path, edit_path, path_selection_preview, targeted_vector_mask, work_path_shown_in};
use crate::PhotocraftApp;
use crate::direct_select::PathRef;
use crate::state::Tool;

/// What a pass of [`show_paths`] reports: the overlay's output and the path each shown `EditPath`
/// stands for, in the order they were given. A report names a path by its key (`PathRef::key`);
/// map it back with `PathRef::from_key`, never by position.
pub(crate) struct BezierShown {
    /// What the overlay reported.
    pub output: BezierOutput,
    /// The `PathRef` of each `EditPath` shown, in order.
    pub refs: Vec<PathRef>,
}

/// Does `tool` draw the anchor squares of the paths it can edit? Path Selection does, and so do
/// the Pen tools: as in Photoshop, a path finished with Enter or by closing it keeps its anchors
/// on screen, so it can still be edited (⌘/Ctrl-drag, Auto Add/Delete) (#6884, #4716).
pub(crate) fn shows_anchors(tool: Tool) -> bool {
    // Add/Delete Anchor Point and Convert Point show the editing overlay instead.
    matches!(tool, Tool::PathSelection | Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen)
}

#[cfg(test)]
/// The saved path selected in the Paths panel: Photoshop's target path, drawn on the canvas with
/// any tool (#6550). `None` when the panel selects the work path, a layer's path or nothing.
pub(crate) fn selected_saved_path(app: &PhotocraftApp) -> Option<&Path> {
    let sel = app.ui.selected_path.as_deref().filter(|s| !matches!(*s, "work" | "layer"))?;
    app.session.active()?.doc.paths.iter().find(|p| p.name == sel).map(|p| &p.path)
}

/// [`selected_saved_path`] with its index in the document's `paths`: the saved path Path
/// Selection and Direct Selection edit (#8478).
pub(crate) fn selected_saved_path_index(app: &PhotocraftApp) -> Option<(usize, Path)> {
    let sel = app.ui.selected_path.as_deref().filter(|s| !matches!(*s, "work" | "layer"))?;
    app.session.active()?.doc.paths.iter().enumerate().find(|(_, p)| p.name == sel).map(|(i, p)| (i, p.path.clone()))
}

/// View › Show › Target Path (under Extras) hides the paths; the Pen's path in progress stays.
/// Free Transform Path draws the path through its box instead (transform_tool).
fn paths_shown(app: &PhotocraftApp) -> bool {
    app.ui.view.shows(app.ui.view.show.target_path) && app.ui.transform.as_ref().is_none_or(|t| t.path.is_none())
}

/// The overlay tool of the editing tools (Direct Selection, the anchor tools, the Pen with ⌘);
/// `None` for Path Selection, the Pen and the shape tools, whose paths are only shown.
fn editing_tool(app: &PhotocraftApp, mods: Mods) -> Option<BezierTool> {
    // PLAN: assumed `direct_select::path_tool(Tool, Mods)` (R21; tools-w39)
    let tool = crate::direct_select::path_tool(app.ui.tool, mods)?;
    let mode = PathMode::resolve(tool, mods, app.ui.pen.is_some(), app.ui.tool_options.pen_auto_add_delete);
    (!matches!(mode.bezier, BezierTool::Anchors)).then_some(mode.bezier)
}

/// The paths a non-editing tool shows in the canvas of `doc`: the Work Path when shown, the
/// selected saved path, the active shape layer's path, a targeted vector mask, a type path, the
/// Path Selection drag's moved copy and the open Pen draft. The Paths-panel selection and the
/// targets belong to the active document.
fn shown_paths(app: &PhotocraftApp, doc: &Document) -> Vec<(PathRef, EditPath)> {
    let active = app.session.active().is_some_and(|st| st.doc.id == doc.id);
    let tool = app.ui.tool;
    let anchors = shows_anchors(tool);
    let mut out: Vec<(PathRef, EditPath)> = Vec::new();
    let mut add = |r: PathRef, p: &Path| out.push((r, edit_path(p, r.key())));
    if paths_shown(app) {
        // A Path Selection drag shows its path where the release will put it.
        let moving = path_selection_preview(app, doc);
        let moved = |which: MovedPath| moving.as_ref().filter(|(m, _)| *m == which).map(|(_, p)| p);
        // The Work Path is drawn only while it is the path selected in the Paths panel or a path
        // tool is active, as in Photoshop: a Work Path left unselected (one a PSD arrived with,
        // or after Ctrl-click Load as Selection) would otherwise cover the marching ants (#6551).
        if let Some(wp) = doc.work_path.as_ref().filter(|_| work_path_shown_in(app, doc))
            && (anchors || app.ui.selected_path.as_deref() == Some("work"))
        {
            add(PathRef::Work, moved(MovedPath::Work).unwrap_or(wp));
        }
        // The saved path selected in the Paths panel is the target path (#6550).
        if active && let Some((i, p)) = selected_saved_path_index(app) {
            add(PathRef::Saved(i), moved(MovedPath::Saved).unwrap_or(&p));
        }
        let st = app.session.documents().iter().find(|s| s.doc.id == doc.id);
        if let Some(l) = st.and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
            && let LayerContent::Shape(sh) = &l.content
        {
            // Hide the stale shape path while its transform preview is active.
            let transforming_this_shape =
                app.ui.transform.as_ref().is_some_and(|t| t.layer == l.id.0 && !t.selection && t.target.is_none() && t.path.is_none());
            if !transforming_this_shape {
                add(PathRef::Layer(l.id.0), &sh.path);
            }
        }
        if active {
            // A targeted vector mask shows its path with any tool (#196).
            if let Some((id, p)) = targeted_vector_mask(app) {
                add(PathRef::Layer(id), &p);
            }
            // So does type on a path's Type Path, which the Pen's Auto Add/Delete edits (#9114).
            if let Some((id, p)) = active_type_path(app) {
                add(PathRef::TypePath(id), &p);
            }
        }
    }
    // The Pen path in progress, only on the document it is drawn on: a view of another document never shows it (#5373).
    if let Some(pen) = app.ui.pen.as_ref().filter(|p| !p.draft.knots.is_empty()).filter(|_| app.ui.pen_doc.is_none_or(|d| d == doc.id)) {
        out.push((PathRef::Pen, EditPath::new(PathRef::Pen.key(), vec![pen.draft.clone()]).copies(false)));
    }
    out
}

/// What [`show_paths`] shows for the canvas of `doc` with `mods` held: the overlay tool and each
/// path with its `PathRef` and whether its anchors show. The one builder of the overlay's paths.
fn plan(app: &PhotocraftApp, doc: DocId, mods: Mods) -> (BezierTool, Vec<(PathRef, EditPath, AnchorsShown)>) {
    let Some(state) = app.session.documents().iter().find(|s| s.doc.id == doc) else { return (BezierTool::Anchors, Vec::new()) };
    let active = app.session.active().is_some_and(|st| st.doc.id == doc);
    if active
        && paths_shown(app)
        && let Some(bezier) = editing_tool(app, mods)
    {
        let (paths, refs) = crate::direct_select::edit_paths(app, bezier);
        return (bezier, refs.into_iter().zip(paths).map(|(r, p)| (r, p, AnchorsShown::All)).collect());
    }
    let anchors = if shows_anchors(app.ui.tool) { AnchorsShown::All } else { AnchorsShown::Hidden };
    (BezierTool::Anchors, shown_paths(app, &state.doc).into_iter().map(|(r, p)| (r, p, anchors)).collect())
}

/// Shows the paths of the canvas of `doc` as one `BezierOverlay` over `at` and returns
/// its report with the path of each `EditPath`. `None` when no path shows; the editing state is
/// then told so (`edits::not_shown`).
pub(crate) fn show_paths(app: &mut PhotocraftApp, ui: &mut egui::Ui, at: egui::Rect, doc: DocId) -> Option<BezierShown> {
    let mods = craft::ui::runtime::pointer::frame(ui.ctx()).mods;
    let (tool, shown) = plan(app, doc, mods);
    if shown.is_empty() {
        crate::direct_select::edits::not_shown(app);
        return None;
    }
    let anchors = shown.first().map_or(AnchorsShown::Hidden, |(_, _, a)| *a);
    let (refs, paths): (Vec<PathRef>, Vec<EditPath>) = shown.into_iter().map(|(r, p, _)| (r, p)).unzip();
    let selection = crate::direct_select::selection(app, tool);
    // Given, not bound: the paths come from the document each pass and only an edit's end (`Commit`, `Set`) changes
    // the document, so a drag's own updates are no outside change that cancels it.
    let output =
        BezierOverlay::new("Paths", Bind::given(paths), &selection).tool(tool).anchors(anchors).path_options(&app.ui.tool_options.path_options).show_at(ui, at);
    Some(BezierShown { output, refs })
}

/// What [`show_paths`] would show in the canvas of `doc` with no modifier held, without a pass.
#[cfg(test)]
pub(crate) fn overlay_paths(app: &PhotocraftApp, doc: DocId) -> Vec<(PathRef, EditPath, AnchorsShown)> {
    plan(app, doc, Mods::default()).1
}
