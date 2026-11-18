//! The Type tool's controls over the canvas while a text edit is open: the Type Mask tint, the text frame of a
//! paragraph box (or the outline of an area, or the baselines of point and path text), the text editor, and the type
//! transform's box. They are shown in the canvas body, before the tool host runs, in the order that decides who takes a
//! press: the move zone of the transform sits under the editor, which sits under the frame's handles, and the shown
//! transform box is above them all.
//!
//! Each control reports this pass's own events only (a second pass of a frame gets none), so nothing here applies an
//! input twice.

use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{self, BezPath};
use craft::behavior::{DocKey, GesturePhase, PointerButton};
use craft::draw::ui::transform::TransformBox;
use craft::imaging::selection::{ColorIndicates, PreviewMode, QuickMaskLook};
use craft::imaging::trace::{MaskRect, MaskSource};
use craft::imaging::ui::selection::SelectionOverlay;
use craft::text::quotes::QuoteStyle;
use craft::text::ui::editor::{EditorEnd, TextEditor, TextSelection};
use craft::theme::Srgba;
use craft::ui::canvas::canvas_space;
use craft::ui::canvas_controls::SelectionOutline;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::runtime::env::{Env, Label};
use photocraft_raster::Surface;
use serde_json::json;

use super::editing::{cancel, commit_with, edit_home, end_if_left};
use super::host::LayerHost;
use super::layout::{doc_point, layout, path_warp};
use super::{Affine, LayerId, PhotocraftApp, text_layer};

/// The document key of the text editor. One editing session is open at a time, so one key is enough; the session
/// start paths request the editor's focus by it.
pub(crate) const TYPE_EDITOR: DocKey = DocKey::new("photocraft.type-editor");

/// How far past point text's laid-out bounds a press still belongs to the editor, in screen points: a click just
/// beside the glyphs places the caret there.
const EDITOR_SLOP_PT: f32 = 6.0;

/// The coverage of a type layer's glyphs (their alpha), served to the Quick Mask preview of the Type Mask tools.
struct GlyphMask<'a> {
    surface: Option<&'a Surface>,
    revision: u64,
    size: [u32; 2],
}

impl MaskSource for GlyphMask<'_> {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    /// The allocated tiles: every glyph pixel lies in one, and the union of their rects is cheap where the exact
    /// bounds scan the pixels; the overlay asks every pass.
    fn content_bounds(&self) -> MaskRect {
        let r = self.surface.map_or(photocraft_geom::Rect::EMPTY, Surface::tile_bounds);
        if r.is_empty() { MaskRect::default() } else { MaskRect::new(r.x0, r.y0, r.x1, r.y1) }
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let Some(surface) = self.surface else {
            out.fill(0.0);
            return;
        };
        let step = step.max(1) as usize;
        let span = out.len().saturating_sub(1) * step + 1;
        let mut row = vec![[0.0f32; 4]; span];
        surface.read_rgba_into(photocraft_geom::Rect { x0, y0: y, x1: x0 + i32::try_from(span).unwrap_or(i32::MAX), y1: y + 1 }, &mut row);
        for (value, pixel) in out.iter_mut().zip(row.iter().step_by(step)) {
            *value = pixel[3];
        }
    }
}

/// The controls of the open text edit, over the canvas `canvas`. Does nothing without an edit in this document.
pub(crate) fn canvas_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect) {
    app.ui.type_box_overflow = false;
    crate::type_transform::cancel_stale(app);
    // A window that lost the focus ends a transform drag whose release it will never see.
    if !ui.input(|i| i.focused) {
        crate::type_transform::cancel_drag(app);
    }
    end_if_left(app);
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let id = LayerId(ed.layer);
    // Until it commits, an edit in a document no longer active shows nothing over this one.
    if edit_home(app, &ed) != app.session.active_index() {
        return;
    }
    // The session ends with its layer.
    if app.session.active().and_then(|st| text_layer(&st.doc, id)).is_none() {
        app.ui.text_edit = None;
        app.ui.text_edit_return_tool = None;
        return;
    }
    let ctx = ui.ctx().clone();
    crate::type_box_preview::wake(app, &ctx);
    let Some(view) = crate::type_transform::view_of(ui) else { return };
    let transform_shown = crate::type_transform::visible(app, craft::ui::runtime::pointer::frame(&ctx).mods);
    if ed.mask {
        mask_tint(app, ui, canvas, id);
    }
    if !transform_shown {
        outline(app, ui, canvas, id);
    }
    crate::type_transform::show(app, ui, canvas, false);
    editor(app, ui, &view, id);
    // The editor may have ended the session.
    if app.ui.text_edit.is_none() {
        return;
    }
    if !transform_shown {
        frame_box(app, ui, canvas, id);
    }
    crate::type_transform::show(app, ui, canvas, true);
}

/// The Type Mask tools while typing: Quick Mask's colour over the document with the glyphs clear, as Photoshop shows
/// the selection-to-be. The glyphs are the hidden working layer's pixels.
fn mask_tint(app: &PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, id: LayerId) {
    let Some(st) = app.session.active() else { return };
    let surface = text_layer(&st.doc, id).and_then(|t| t.cache.as_ref());
    let mask = GlyphMask { surface, revision: st.revision, size: [st.doc.size.width, st.doc.size.height] };
    let options = app.session.quick_mask_options;
    let rgb = options.color.to_rgb();
    let look = QuickMaskLook { color: Srgba::rgb(rgb[0], rgb[1], rgb[2]), opacity: options.opacity, indicates: ColorIndicates::MaskedAreas };
    SelectionOverlay::new(Label::from(&*tl!("Type mask")), &mask).visible(false).preview(PreviewMode::QuickMask).quick_mask(look).show_at(ui, canvas);
}

fn doc_pt(p: photocraft_geom::Point) -> kurbo::Point {
    kurbo::Point::new(p.x, p.y)
}

/// The outline of what a text edit is laid out in, when it has no frame box: the shape an area wraps inside, or a
/// line under each line of point or path text (the line bends with the path).
fn outline(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, id: LayerId) {
    let Some(shape) = app.session.active().and_then(|st| text_layer(&st.doc, id).map(|t| t.shape.clone())) else { return };
    let Some((l, aff, _)) = layout(app, id) else { return };
    let mut paths: Vec<BezPath> = Vec::new();
    match shape {
        // A paragraph box shows its frame, with handles.
        photocraft_doc::text::TextShape::Box { .. } => return,
        photocraft_doc::text::TextShape::Area { path } => {
            for poly in photocraft_vector::flatten::flatten_path(&path, 0.5) {
                let mut line = BezPath::new();
                for (i, &(x, y)) in poly.pts.iter().enumerate() {
                    let p = doc_pt(aff.apply(photocraft_geom::Point::new(x, y)));
                    if i == 0 { line.move_to(p) } else { line.line_to(p) }
                }
                if poly.pts.len() >= 2 {
                    line.close_path();
                    paths.push(line);
                }
            }
        }
        _ => {
            let warp = path_warp(app, id, &l);
            for ln in &l.lines {
                // Horizontal: just under the baseline; vertical: the column's centre line.
                let y = if l.vertical { ln.baseline } else { ln.baseline + ln.descent * 0.25 };
                let (x0, x1) = (ln.x0.min(0.0), ln.x1.max(ln.x0 + 1.0));
                // Sampled, so it bends with path type.
                let pieces = if warp.is_some() { ((x1 - x0) / 4.0).ceil().clamp(1.0, 512.0) as usize } else { 1 };
                let mut line = BezPath::new();
                for i in 0..=pieces {
                    let p = doc_pt(doc_point(&l, &aff, warp.as_ref(), x0 + (x1 - x0) * i as f32 / pieces as f32, y));
                    if i == 0 { line.move_to(p) } else { line.line_to(p) }
                }
                paths.push(line);
            }
        }
    }
    if !paths.is_empty() {
        SelectionOutline::solid(Label::from(&*tl!("Text outline")), &paths).show_at(ui, canvas);
    }
}

/// The screen rect the editor takes presses in: a paragraph box's own rect, so a press inside it off the glyphs
/// places the caret (#7606); for other text its laid-out bounds, grown by `EDITOR_SLOP_PT`.
fn editor_rect(app: &mut PhotocraftApp, view: &craft::behavior::CanvasView, id: LayerId) -> Option<egui::Rect> {
    let (rect, aff) = crate::type_transform::source_rect(app, id)?;
    let quad = crate::type_transform::quad_of(rect, &aff)?;
    let mut bounds = egui::Rect::NOTHING;
    for corner in quad {
        bounds.extend_with(view.to_screen(corner));
    }
    let boxed = super::box_shape(app, id).is_some();
    Some(if boxed { bounds } else { bounds.expand(EDITOR_SLOP_PT) })
}

/// The text editor over the open edit: typing, the caret and selection, clicks and the input method. Its commit and
/// cancel end the session; its selection is written back to `TextEdit` in the same pass.
fn editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &craft::behavior::CanvasView, id: LayerId) {
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let mut selection = TextSelection::new(ed.anchor, ed.caret);
    let Some(rect) = editor_rect(app, view, id) else { return };
    let (smart_quotes, esc_commits) = {
        let prefs = &app.session.prefs().type_;
        (prefs.smart_quotes, prefs.use_esc_to_commit)
    };
    let mask = app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask);
    let out = {
        let mut host = LayerHost::new(app, id);
        let mut editor = TextEditor::new(Label::from(&*tl!("Type editor")), &mut host, Bind::bound(&mut selection))
            // A Type Mask commits on Escape whatever "Use ESC to commit" says; the options bar's Cancel still throws it away (#12070).
            .esc_commits(mask || esc_commits)
            .doc_key(TYPE_EDITOR);
        if smart_quotes {
            editor = editor.smart_quotes(QuoteStyle::ENGLISH);
        }
        editor.show_at(ui, rect)
    };
    super::editing::take_editor_focus(ui.ctx());
    if let Some(ed) = app.ui.text_edit.as_mut() {
        (ed.anchor, ed.caret) = (selection.anchor, selection.caret);
    }
    // A caret move that edits no text (arrows, Home/End, a click) drops the pending font, even when the caret comes
    // back to where the font was chosen. Typing keeps it (`insertion_style::replace` moves it with the caret).
    if out.selection_changed && !out.changed {
        app.ui.type_insertion_style = None;
    }
    if out.character_panel {
        let _ = crate::menus::invoke(app, ui.ctx(), "window.panel.character", json!({}));
    }
    // A primary press on the canvas background is what makes the editor commit (editor.rs: `background().pressed`);
    // the same press reaches the tool, which then starts no new text.
    let background_press = canvas_space(&Env::of(ui).view()).is_some_and(|space| space.background().pressed == Some(PointerButton::Primary));
    let held = craft::ui::runtime::pointer::frame(ui.ctx()).mods;
    if let Some(at) = out.context {
        app.canvas_tools.type_tool.context = Some((at, held));
    }
    let mods = egui::Modifiers { alt: held.alt, ctrl: held.ctrl, shift: held.shift, mac_cmd: false, command: held.cmd };
    // The editor's end is the session's, whether or not its text changed: an untouched placeholder or an empty mask
    // ends on Escape too. Every other end of the gesture (the frame drag's `cancel_all`, an undo, a removed editor)
    // leaves the text.
    match out.ended {
        // A Type Mask replaces the selection with ⇧ or ⌥ on the press that commits (#12073).
        Some(EditorEnd::Commit) => {
            if background_press {
                app.canvas_tools.type_tool.began_with_session = true;
            }
            commit_with(app, mods);
        }
        Some(EditorEnd::Cancel) => cancel(app),
        _ => {}
    }
}

/// The box of `state`, as `type.edit`'s `box` takes it: the top-left corner in document units, then the width and
/// height in text space, through the layer's transform `aff`.
fn box_of(state: &BoxState, aff: &Affine) -> Option<[f64; 4]> {
    let inverse = aff.inverse()?;
    let (top_left, bottom_right) = (
        inverse.apply(photocraft_geom::Point::new(state.quad[0].x, state.quad[0].y)),
        inverse.apply(photocraft_geom::Point::new(state.quad[2].x, state.quad[2].y)),
    );
    let boxed = [state.quad[0].x, state.quad[0].y, bottom_right.x - top_left.x, bottom_right.y - top_left.y];
    (boxed.iter().all(|v| v.is_finite()) && boxed[2] > 0.0 && boxed[3] > 0.0).then_some(boxed)
}

/// Applies the box a frame drag has reached: in the background render when one runs, else with `type.edit` at once.
fn apply_box(app: &mut PhotocraftApp, id: LayerId, boxed: [f64; 4]) {
    if crate::type_box_preview::want(app, id, boxed) {
        return;
    }
    let Some(session) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()) else { return };
    let _ = app.run("type.edit", json!({"layer": id.0, "box": boxed, "coalesce": session}));
}

/// The text frame of a paragraph box: its outline with handles, which resize the box. A drag is recorded by the
/// preview (`begin`, `want`, `end_drag`, `discard`) and applies one `type.edit` per move, or one on release when the
/// preview renders it in the background. A click on a handle with no drag changes nothing.
fn frame_box(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, id: LayerId) {
    if super::box_shape(app, id).is_none() {
        return;
    }
    let Some((rect, aff)) = crate::type_transform::source_rect(app, id) else { return };
    // A drag rendered in the background has not changed the document: the box stands where the pointer has it.
    let dragged = crate::type_box_preview::wanted_box(app, id);
    let (start, shown_aff) = match dragged {
        Some(b) => (kurbo::Rect::new(0.0, 0.0, b[2].max(1.0), b[3].max(1.0)), crate::type_box_preview::overlay_affine(aff, b)),
        None => (rect, aff),
    };
    let Some(quad) = crate::type_transform::quad_of(start, &shown_aff) else { return };
    // Text the box can't hold: as in the reference application, the lower-right handle grows and shows a black "+".
    // Automation reads `ui.type_box_overflow` instead.
    let overflow = layout(app, id).is_some_and(|(l, _, text)| l.overflows(&text));
    app.ui.type_box_overflow = overflow;
    // The canvas's snap reaches the box through the canvas (`Canvas::snap`, document_display.rs).
    let out = TransformBox::text_frame(Label::from(&*tl!("Text frame")), Bind::given(BoxState::new(quad, quad[0].midpoint(quad[2]))), start)
        .overflow(overflow)
        .show_at(ui, canvas);
    for event in out.edit.events.iter() {
        match event.phase() {
            Some(GesturePhase::Begin) => {
                app.ui.type_insertion_style = None;
                if let Some(session) = app.ui.text_edit.as_ref().map(|ed| ed.session.clone()) {
                    crate::type_box_preview::begin(app, id, session);
                }
            }
            // The frame's phases of a drag that has no record (it ended already, or a command ended it) are ignored.
            _ if !crate::type_box_preview::holds_drag(app) => {}
            Some(GesturePhase::Update) => {
                if let Some(boxed) = event.value().and_then(|state| box_of(state, &aff)) {
                    apply_box(app, id, boxed);
                }
            }
            // The pass that commits carries the final box (a pass folds the `Update` before a `Commit` into it).
            Some(GesturePhase::Commit) => {
                if let Some(boxed) = event.value().and_then(|state| box_of(state, &aff)) {
                    apply_box(app, id, boxed);
                }
                crate::type_box_preview::end_drag(app);
            }
            Some(GesturePhase::Cancel) => {
                // A background drag never touched the document; a drag applied as it came goes back to where it began.
                let untouched = crate::type_box_preview::discard(app);
                if !untouched && let Some(boxed) = event.landed().and_then(|state| box_of(state, &aff)) {
                    apply_box(app, id, boxed);
                }
            }
            _ => {}
        }
        // An arrow key moves a focused handle by one step: a value that arrives as one edit.
        if event.phase().is_none()
            && let Some(boxed) = event.value().and_then(|state| box_of(state, &aff))
        {
            apply_box(app, id, boxed);
        }
    }
}
