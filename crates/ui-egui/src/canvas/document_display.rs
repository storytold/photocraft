//! The document canvas on the framework's `Canvas`: the view binding, the navigation options from Preferences, the
//! pasteboard, the canvas controls of the body in their order, the document's picture (a GPU picture or the CPU
//! composite, and the Channels panel's view over it), and the tool host that runs between the controls and the picture.

use craft::behavior::geometry::crop::normalized_angle;
use craft::behavior::geometry::quad::Homography;
use craft::behavior::kurbo::{Point, Rect as Rect64, Size};
use craft::behavior::{CanvasView, PixelSize};
use craft::imaging::ui::SelectionOverlay;
use craft::ui::canvas::{
    Canvas, CanvasOutput, CanvasScrollbars, CanvasSpace, Extent, NavigationOptions, PageBorder, PageFrame, Pasteboard, PasteboardColor, PixelRect,
    RasterDisplay, RasterSource, RotateView, ToolHost, ToolHostResponse, ViewTarget, canvas_space, easing_view, ruler_guide, tool_pass,
};
use craft::ui::canvas_controls::{GuidesOutput, SelectionOutline};
use craft::ui::contract::{Bind, Props, Show, ShowContainer};
use craft::ui::primitives::Checker;
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use egui::{Color32, Pos2, Rect};

use super::gpu_picture::{GpuPicture, GpuRequest, gpu_picture};
use super::raster_source::{CompositePrep, OverlayPrep};
use super::{Document, PhotocraftApp, View, tools};
use photocraft_doc::LayerId;
use photocraft_geom::Rect as DRect;

impl View {
    /// This view as the framework's canvas view of `rect` at `ppp` device pixels per point, mirrored by View › Flip
    /// Horizontal and stretched by View › Pixel Aspect Ratio Correction.
    pub(crate) fn canvas_view(&self, rect: Rect, ppp: f32, flip: bool, aspect: f32) -> CanvasView {
        let center = Point::new(f64::from(self.center[0]), f64::from(self.center[1]));
        CanvasView::new(rect, center, CanvasView::points_per_unit(f64::from(self.zoom), ppp))
            .rotation(f64::from(self.rotation))
            .flip_x(flip)
            .aspect(f64::from(aspect))
    }

    /// Takes the zoom, centre and rotation of `view`, shown at `ppp` device pixels per point.
    pub(crate) fn set_from(&mut self, view: &CanvasView, ppp: f32) {
        self.zoom = (view.zoom * f64::from(ppp)) as f32;
        self.center = [view.center.x as f32, view.center.y as f32];
        self.rotation = normalized_angle(view.rotation) as f32;
    }
}

/// Where a document canvas is shown: document `idx` in document window `window` (Window › New Window, a floating
/// document) or in the main document area (`None`). A window id names one canvas wherever the window draws (its own
/// viewport or the main one), and one document can show in both the main area and a window, so the window is the
/// canvas's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CanvasHost {
    pub idx: usize,
    pub window: Option<u64>,
}

impl CanvasHost {
    /// Document `idx`'s canvas in the main window's document area.
    pub fn main(idx: usize) -> Self {
        Self { idx, window: None }
    }

    /// The canvas that shows document `idx`'s own view, the one the Navigator, the status bar's zoom, the tools and
    /// the dialogs' canvas picks work on: the floating document window that draws it, else the main document area.
    /// A New Window for Document keeps a view of its own, so it is never the document's own canvas.
    pub(crate) fn primary(app: &PhotocraftApp, idx: usize) -> Self {
        let window = app
            .ui
            .document_windows
            .floating
            .iter()
            .filter(|(g, _)| !app.ui.window_views.contains_key(&g.id))
            .find(|(g, _)| g.shown.or_else(|| g.documents.first().copied()).and_then(|d| super::tabs::document_index(app, d)) == Some(idx))
            .map(|(g, _)| g.id);
        Self { idx, window }
    }

    /// Whether this canvas shows document `idx`'s own view ([`primary`](Self::primary)).
    pub(crate) fn is_primary(self, app: &PhotocraftApp) -> bool {
        self == Self::primary(app, self.idx)
    }

    /// The canvas's id, under which it keeps its zoom ease and its tool host's state.
    pub(crate) fn canvas_id(self) -> egui::Id {
        egui::Id::unique(("pc-canvas", self))
    }
}

#[cfg(test)]
/// The zoom (device pixels per document pixel at `ppp`) and centre the document view shows on `host`'s canvas: the
/// view on screen of the canvas's zoom ease while one runs, else `view`'s own. For the panels that read the view
/// outside the canvas's pass.
pub(crate) fn shown_camera(ctx: &egui::Context, host: CanvasHost, view: &View, ppp: f32) -> (f32, [f32; 2]) {
    match easing_view(ctx, host.canvas_id()) {
        Some(eased) => ((eased.zoom * f64::from(ppp)) as f32, [eased.center.x as f32, eased.center.y as f32]),
        None => (view.zoom, view.center),
    }
}

/// The active document's own canvas ([`CanvasHost::primary`]) as its last pass showed it, rect included: the tool
/// host keeps the space it ran in, so no view is stored beside it. `None` before the first pass and with no document.
pub fn active_view(app: &PhotocraftApp) -> Option<CanvasView> {
    let idx = app.session.active_index()?;
    ToolHost::shown_space(app.ctx(), CanvasHost::primary(app, idx).canvas_id()).map(|space| space.view)
}

/// The document point under the screen point `pos` of the active document's main canvas; `None` when `pos` is outside
/// the canvas.
pub(crate) fn doc_point_at(app: &PhotocraftApp, pos: Pos2) -> Option<Point> {
    active_view(app).filter(|view| view.rect.contains(pos)).map(|view| view.to_doc(pos))
}

/// Whether the selection's marching ants show on document `idx`, and where: `Some(offset)` is the whole-pixel offset
/// a floating piece or a selection drag shows them at, `(0, 0)` otherwise; `None` hides them. The one answer the body
/// reads for `SelectionOverlay::visible` and its offset. Four conditions hide them: Selection Edges off, a Polygonal
/// or Magnetic Lasso outline that is open (it replaces the selection when made), a Patch or Content-Aware Move drag
/// (which replaces it on the active document, never a floating piece), and Content-Aware Move's Transform On Drop
/// box, which draws them where it puts them.
pub(crate) fn ants_shown(app: &PhotocraftApp, idx: usize) -> Option<(i32, i32)> {
    if !app.ui.view.shows(app.ui.view.show.selection_edges) || app.canvas_tools.selection.replaces_selection() || crate::cam_transform::active(app) {
        return None;
    }
    let active = app.session.active_index() == Some(idx);
    let floating = super::tools::selection::floating_outline(app, idx).is_some();
    if active && !floating && app.canvas_tools.retouch.replaces_selection() {
        return None;
    }
    Some(if active { super::tools::selection::shown_offset(app).unwrap_or((0, 0)) } else { (0, 0) })
}

/// Whether PhotoCraft's tool moves the view with the pointer this pass: the Crop tool in its default mode slides the
/// view under a fixed frame. The canvas then shows the view unaligned.
pub(crate) fn view_follows_pointer(app: &PhotocraftApp) -> bool {
    crate::crop_ui::moves_view(app)
}

/// The user's navigation choices from Preferences (General, Tools, Enhanced Controls).
pub(crate) fn navigation(app: &PhotocraftApp) -> NavigationOptions {
    let p = app.session.prefs();
    let mut options = NavigationOptions::default();
    options.zoom_with_wheel = p.general.zoom_with_scroll_wheel;
    options.animated_zoom = p.general.animated_zoom;
    options.zoom_clicked_point_to_center = p.tools.zoom_clicked_point_to_center;
    options.flick_panning = p.tools.enable_flick_panning;
    options.overscroll = p.tools.overscroll;
    options.pinch_zooms = p.enhanced_controls.zoom_with_trackpad_pinch;
    options.trackpad_rotate = p.enhanced_controls.rotate_view_with_trackpad;
    options.touch_gestures = p.enhanced_controls.touch_gestures;
    options
}

/// The transparency checkerboard of Preferences › Transparency & Gamut, `None` when it is off.
pub(crate) fn checkerboard(app: &PhotocraftApp) -> Option<Checker> {
    let prefs = app.session.prefs();
    prefs.transparency_and_gamut.square().map(|square| {
        let [light, dark] = prefs.transparency_and_gamut.colors();
        let color = |c: [u8; 3]| DataColor::new(Color32::from_rgb(c[0], c[1], c[2]));
        Checker::new().size(square).colors(color(light), color(dark))
    })
}

/// What the view commands fit and fill: the document, its artboard (the active board, else the last, production's
/// Fit Artboard), and the resolutions Print Size and Actual Pixels read. Fit Layers asks [`layer_bounds`] itself.
pub(crate) fn view_target(app: &PhotocraftApp, idx: usize) -> ViewTarget {
    let doc = &app.session.documents()[idx].doc;
    let pages = super::artboard_ui::pages(app, idx);
    let mut target = ViewTarget::new(Size::new(f64::from(doc.size.width), f64::from(doc.size.height)))
        .ppi(f64::from(doc.resolution_dpi))
        .screen_ppi(app.session.prefs().units_and_rulers.screen_resolution);
    if let Some(page) = pages.iter().find(|p| p.active == Some(true)).or_else(|| pages.last()) {
        target = target.artboard(page.rect);
    }
    target
}

/// The bounds Fit Layers shows in `doc`, whose active layer is `active` and selection `selected`: the selected layers'
/// content, else the document's. It walks the document and the layers' tiles, so the canvas asks for it only in the
/// pass that runs the command.
fn layer_bounds(doc: &Document, active: Option<LayerId>, selected: &[LayerId]) -> Rect64 {
    let mut layers = DRect::EMPTY;
    for id in photocraft_engine::selected_in(doc, active, selected) {
        if let Some(bounds) = doc.layer(id).and_then(|l| l.surface()).map(|s| s.content_bounds()) {
            layers = if layers.is_empty() { bounds } else { layers.union(&bounds) };
        }
    }
    if layers.is_empty() {
        layers = doc.bounds();
    }
    Rect64::new(f64::from(layers.x0), f64::from(layers.y0), f64::from(layers.x1), f64::from(layers.y1))
}

/// One document view's canvas this pass.
pub(crate) struct CanvasPass<'a> {
    pub ctx: &'a egui::Context,
    pub host: CanvasHost,
    pub doc: &'a std::sync::Arc<Document>,
    /// The display the canvas is on (`None`: the main window's).
    pub output: Option<u32>,
    pub rect: Rect,
    pub ppp: f32,
    pub flip: bool,
    pub aspect: f32,
}

/// What a canvas pass leaves for the caller.
pub(crate) struct Shown {
    /// The canvas's output, for the canvas menu.
    pub output: CanvasOutput,
    /// What the tool host did this pass.
    pub response: ToolHostResponse,
    /// The guides' output (their context request), when the body showed them.
    pub guides: Option<GuidesOutput>,
    /// Where an overlay (the Crop frame's parts, the text editor) was asked for its menu by a secondary press, and the
    /// modifiers held then: the overlay takes that press, so the tool host never reports it.
    pub overlay_context: Option<(Pos2, craft::keymap::Mods)>,
}

/// The document's picture as prepared for this pass, before the app is borrowed shared.
enum Prepared {
    Gpu(GpuPicture),
    Cpu(CompositePrep),
}

/// The pasteboard colour: Preferences › Interface's, else black in Full Screen Mode (the backdrop there) and the
/// theme's canvas otherwise.
fn pasteboard_color(app: &PhotocraftApp) -> PasteboardColor {
    match crate::prefs_ui::pasteboard_color(app) {
        Some(color) => PasteboardColor::Custom(DataColor::new(color)),
        None if app.ui.view.hides_chrome() => PasteboardColor::Black,
        None => PasteboardColor::Default,
    }
}

/// Shows document `pass.host.idx` in a framework `Canvas` over `pass.rect` through `view`: the canvas folds the wheel,
/// pinch, page keys, view commands and the Overscroll clamp into the view, eases PhotoCraft's own eased zooms, and its
/// body shows the pasteboard and the canvas controls, runs the tool host on the canvas's tool input, and last shows
/// the document's picture into the slot the canvas reserved under the controls, so the picture shows what this pass's
/// press and moves did. `view` is the view after the canvas's pass.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &mut View, pass: &CanvasPass<'_>) -> Shown {
    let (ctx, host, idx, doc, output, rect, ppp) = (pass.ctx, pass.host, pass.host.idx, pass.doc, pass.output, pass.rect, pass.ppp);
    let given = view.canvas_view(rect, ppp, pass.flip, pass.aspect);
    let id = host.canvas_id();
    // A discrete zoom eases from the view it was asked for in. A measuring run starts no ease, so the request waits for
    // the pass that shows the canvas.
    let measuring = craft::ui::runtime::measure::active(ctx);
    let from = view.zoom_from.map_or(given, |(zoom, center)| View { zoom, center, ..view.clone() }.canvas_view(rect, ppp, pass.flip, pass.aspect));
    let ease = (view.zoom_animating && !measuring)
        .then(|| (from, view.zoom_anchor.map(|(doc, screen)| (Point::new(doc[0], doc[1]), Pos2::new(screen[0], screen[1])))));
    if !measuring {
        view.zoom_animating = false;
        view.zoom_anchor = None;
        view.zoom_from = None;
    }
    // What is on screen until the canvas runs: the view of a zoom ease running, else the view given.
    let drawn = easing_view(ctx, id).map_or(given, |mut eased| {
        eased.rect = rect;
        eased
    });

    let border = match app.session.prefs().interface.canvas_border {
        photocraft_engine::prefs::CanvasBorder::DropShadow => PageBorder::DropShadow,
        photocraft_engine::prefs::CanvasBorder::Line => PageBorder::Line,
        _ => PageBorder::None,
    };
    let pattern = app.ui.view.pattern_preview;
    // A flipped or pixel-aspect-corrected view draws through the CPU path (the GPU canvas shader has no mirroring or
    // non-square pixels). Rotation stays on the GPU: it is a view uniform.
    let gpu_ok = crate::gpu_canvas::gpu_view_ok(pass.flip, view.rotation, pass.aspect);
    if app.gpu.is_some() {
        if gpu_ok {
            crate::render_path::view_ok(app, doc.id.0);
        } else {
            crate::render_path::view_cpu(app, doc.id.0);
        }
    }
    let label = super::tabs::tab_title_at(app, idx, view.zoom * 100.0);
    let navigation = navigation(app);
    let follows_pointer = view_follows_pointer(app);
    let checker = checkerboard(app);
    let snap = super::snap_ui::snap(app, idx);
    let target = view_target(app, idx);
    let state = &app.session.documents()[idx];
    let (fit_doc, active, selected) = (std::sync::Arc::clone(&state.doc), state.active_layer, state.selected_layers.clone());
    let fit_layers = move || Some(layer_bounds(&fit_doc, active, &selected));
    let pasteboard = pasteboard_color(app);
    let primary = host.is_primary(app);
    // Where the tool takes a press over the canvas controls this pass, asked before the canvas shows.
    let regions = tools::claimed_regions(app, ctx, host);

    // The view commands asked of this view (a menu row, an action step, a new document's fit) run in the canvas's pass.
    let queued = view.pending.clone();
    let mut bound = given;
    let mut ruler_zero = Point::ZERO;
    let mut canvas = Canvas::new(label.clone())
        .id(id)
        .view(Bind::bound(&mut bound))
        .document(target)
        .layer_bounds(&fit_layers)
        .queued(&queued)
        .snap(snap)
        .pasteboard(pasteboard)
        .navigation(navigation)
        // The canvas shows the rulers along its top and left edges, and its view lies inside the strips.
        .with_rulers(super::rulers::strips(app, host, &mut ruler_zero))
        .view_follows_pointer(follows_pointer)
        .precise_cursors(app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise)
        .tool_regions(regions);
    if let Some((from, anchor)) = ease {
        canvas = canvas.ease_from(from, anchor);
    }
    let mut guides = None;
    let mut turned = None;
    // What the CPU display read from the composite this pass, and how long its pass took.
    let mut cpu_read: Option<(PixelRect, f64)> = None;
    // What the tool host did; none in a measuring run, where it does not run.
    let mut hosted: Option<ToolHostResponse> = None;
    let mut overlay_context = None;
    let shown = canvas.show_at(ui, rect, |ui, body| {
        let Some(space) = canvas_space(&body.env) else { return };
        pasteboards(app, ui, body.rect, idx, border, label.clone());
        guides = super::rulers::show(app, ui, host, ruler_guide(&body.env));
        selection_controls(app, ui, body.rect, idx, &label);
        // The Levels clipping display, over the picture and under the controls: it joins the canvas's picture slot
        // after the picture.
        let clipping = if primary { crate::levels_clipping::shapes(app, ctx, &space.view, idx) } else { Vec::new() };
        // Color Range's Selection Preview: the dialog's proxy mask, drawn by the selection overlay without ants.
        if let Some(preview) = crate::color_range_ui::selection_preview(app, ctx) {
            SelectionOverlay::new(format!("{label} Selection Preview"), &*preview.mask)
                .visible(false)
                .reduction(preview.reduction)
                .preview(preview.mode)
                .show_at(ui, body.rect);
        }
        if app.ui.view.tile_grid {
            super::tile_overlay::show(ui, app, idx);
        }
        // What the controls report for the host's pass.
        let mut body_shown = tools::BodyShown::default();
        let live = canvas_controls(app, ui, body.rect, host, &space, &mut turned, &mut body_shown);
        // The Free Transform preview joins the picture slot after the document, which is prepared after the tools.
        let transform_picture = std::mem::take(&mut body_shown.transform_picture);
        // The body can run more than once in a frame; a report from any of its runs counts.
        if let Some(context) = body_shown.crop.as_mut().and_then(craft::draw::ui::CropShown::take_context) {
            overlay_context = Some(context);
        }
        // A secondary press the text editor took, for the type edit menu.
        if let Some(context) = app.canvas_tools.type_tool.take_context() {
            overlay_context = Some(context);
        }
        // The tools run after the controls, which take a press first, and before the picture, which shows the stroke
        // or edit they made from this pass's input.
        if let Some(input) = tool_pass(&body.env) {
            // An ⌥-click the window manager took never arrives; say so (alt_grab.rs). Tools run on the document's own canvas only.
            if primary {
                let (tool, active) = (app.ui.tool, !app.dialog_open());
                crate::alt_grab::watch(app, ui.ctx(), egui::Id::unique(("pc-alt-grab", host.idx)), input.rect, tool, active);
            }
            hosted = Some(tools::run(app, ui, host, &input, body_shown));
        }
        // The picture shows the document as the tools left it: a commit (a crop) can replace it.
        let current = app.session.documents().get(idx).map(|state| std::sync::Arc::clone(&state.doc));
        let doc = current.as_ref().unwrap_or(doc);
        let size = PixelSize::new(doc.size.width, doc.size.height);
        // The picture is prepared after the controls, so a gradient's live proposal shows in the pass that made it.
        let picture = gpu_ok
            .then(|| {
                let (xf, target) = (&drawn, &given);
                // PLAN: `live` is the field canvas-widgets-w32 adds to `GpuRequest` (cw.P19); its name follows w32's decision.
                gpu_picture(app, &GpuRequest { ctx, idx, doc, output, view: xf, target, zoom: view.zoom, render_zoom: view.zoom, live: live.as_ref() })
            })
            .flatten();
        match &picture {
            Some(picture) if picture.shows_document() && app.navigator_plain.contains(&doc.id) => {
                app.navigator_gpu.insert(doc.id, picture.params(&given));
            }
            _ => {
                app.navigator_gpu.remove(&doc.id);
            }
        }
        let prepared = match picture {
            Some(picture) => Some(Prepared::Gpu(picture)),
            None => CompositePrep::new(app, ctx, idx, output, live.as_ref()).map(Prepared::Cpu),
        };
        // The grid reads the committed composite; with the GPU picture on there is no CPU one yet.
        let grid_prep = (app.ui.view.shows(app.ui.view.show.pixel_grid) && matches!(prepared, Some(Prepared::Gpu(_))))
            .then(|| CompositePrep::new(app, ctx, idx, output, None))
            .flatten();
        let overlay = OverlayPrep::new(app, ctx, idx, output, live.as_ref());
        // The canvas edge isn't outlined while the pixels past it show (Photoshop's crop preview), nor while the Crop
        // tool hides everything outside its box (Show Cropped Area off).
        let edge = border == PageBorder::Line && !crate::crop_ui::shows_beyond_canvas(app) && !crate::crop_ui::hides_canvas_edge(app);
        let region = crate::crop_ui::shows_beyond_canvas(app).then(|| crate::crop_ui::frame(app)).flatten().map(|frame| {
            let r = frame.turned_bounds();
            PixelRect::new(r.x0.floor() as i32, r.y0.floor() as i32, r.x1.ceil() as i32, r.y1.ceil() as i32)
        });
        // What a crop that grows the canvas adds to a Background previews in its fill colour, as on the monitor.
        let region_fill = region.and_then(|_| crate::crop_ui::grown_fill(app, doc)).and_then(|c| {
            let (display, _) = super::display::canvas_display(app, doc, output);
            let one = photocraft_compose::Buffer::filled(DRect::from_xywh(0, 0, 1, 1), [c[0], c[1], c[2], 1.0]);
            super::display::display_image(display.as_deref(), &one).pixels.first().copied()
        });
        let app: &PhotocraftApp = app;
        let (gpu, composite) = match prepared {
            Some(Prepared::Gpu(picture)) => (Some(picture), None),
            Some(Prepared::Cpu(prep)) => (None, Some(prep.source(app))),
            None => (None, None),
        };
        let overlay = overlay.map(|prep| prep.source(app));
        let bounds = gpu.as_ref().map(GpuPicture::bounds).or_else(|| composite.as_ref().map(|c| c.bounds())).unwrap_or_else(|| doc.bounds());
        let mapping = (bounds != doc.bounds()).then(|| placement(bounds));
        let frame = Frame { checker: checker.as_ref(), edge, mapping, pattern, region, region_fill };
        if let Some(picture) = &gpu {
            frame.apply(RasterDisplay::painted(label.clone(), size, picture.clone().into_paint())).picture().show_at(ui, body.rect);
        } else if let Some(composite) = &composite {
            let started = crate::gpu_canvas::now_ms();
            let out = frame.apply(RasterDisplay::new(label.clone(), composite as &dyn RasterSource)).picture().show_at(ui, body.rect);
            cpu_read = out.read.map(|read| (read, crate::gpu_canvas::now_ms() - started));
        }
        if let Some(overlay) = &overlay {
            let display = RasterDisplay::new(Label::Empty, overlay as &dyn RasterSource).id_salt("pc-channel-view").picture();
            let display = match mapping {
                Some(frame) => display.mapped(frame, 1),
                None => display,
            };
            display.show_at(ui, body.rect);
        }
        // The moving pixels over the document, which hides the layer they come from, and under the controls.
        space.paint_picture(ctx, transform_picture);
        space.paint_picture(ctx, clipping);
        // The grid is drawn over the picture.
        if app.ui.view.shows(app.ui.view.show.pixel_grid) {
            let own = grid_prep.map(|prep| prep.source(app));
            let source: Option<&dyn RasterSource> = composite.as_ref().map(|c| c as &dyn RasterSource).or(own.as_ref().map(|c| c as &dyn RasterSource));
            if let Some(source) = source {
                craft::ui::canvas_controls::PixelGrid::new(label.clone(), source).show_at(ui, body.rect);
            }
        }
    });
    // The refresh the CPU display made is the app's to record: the display reports what it read.
    if let Some((read, ms)) = cpu_read.filter(|(read, _)| !read.is_empty()) {
        let px = u64::from(read.width().unsigned_abs()) * u64::from(read.height().unsigned_abs());
        let whole = u64::from(doc.size.width) * u64::from(doc.size.height);
        app.perf.record(if px >= whole { "full" } else { "rect" }, px, ms, 0.0);
    }
    // A measuring run ran none of them; the real run did, and commands asked while it ran wait for the next pass.
    if !measuring {
        view.pending.drain(..queued.len().min(view.pending.len()));
    }
    if let Some(rulers) = &shown.output.rulers {
        super::rulers::run_rulers(app, rulers);
    }
    let canvas_output = shown.output;
    let response = hosted.unwrap_or_default();
    // The host also echoes the canvas's view (for example when rulers inset its rect). Only an actual tool
    // edit takes precedence over the scroll bars or compass.
    let tool_view = response.view.filter(|next| Some(*next) != canvas_output.view);
    if let Some(next) = tool_view.or(turned).or(canvas_output.view) {
        view.set_from(&next, ppp);
    }
    Shown { output: canvas_output, response, guides, overlay_context }
}

/// The document's pasteboard, and for an artboard document a second one over the boards that leaves the shown layers
/// uncovered (the layers' pixels past a board show through).
fn pasteboards(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, idx: usize, border: PageBorder, label: String) {
    let Some(doc) = app.session.documents().get(idx).map(|state| state.doc.clone()) else { return };
    let page = [PageFrame::new(Rect64::new(0.0, 0.0, f64::from(doc.size.width), f64::from(doc.size.height)))];
    let pages = super::artboard_ui::pages(app, idx);
    if pages.is_empty() {
        Pasteboard::new(label, &page).border(border).show_at(ui, rect);
        return;
    }
    let (shown, revision) = super::display_doc(app, idx);
    let Some(st) = app.session.documents().get(idx) else { return };
    let uncovered = app.doc_queries.uncovered(&app.session, st, revision, || super::artboard_ui::uncovered(&shown));
    Pasteboard::new(label, &pages).border(border).uncovered(&uncovered).show_at(ui, rect);
}

/// The selection's marching ants, the Layer Edges outline and the pixel grid.
fn selection_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect, idx: usize, label: &str) {
    let ants = ants_shown(app, idx);
    if let Some(state) = app.session.documents().get(idx) {
        let doc = state.doc.clone();
        if let Some(selection) = doc.selection.as_ref() {
            let mask = super::outline::SurfaceMask::new(selection, selection.generation(), [doc.size.width, doc.size.height]);
            // The map is `transform_tool::selection_map` while a Free Transform carries the selection.
            let mut overlay = SelectionOverlay::new(label.to_owned(), &mask).visible(ants.is_some()).offset(ants.map_or([0, 0], |(x, y)| [x, y]));
            if let Some(map) = crate::transform_tool::selection_map(app, doc.id) {
                overlay = overlay.mapped(map);
            }
            overlay.show_at(ui, rect);
        }
    }
    if let Some(edges) = super::transform_controls::layer_edges(app, idx) {
        SelectionOutline::solid(label.to_owned(), std::slice::from_ref(&edges)).show_at(ui, rect);
    }
}

/// The controls the tools show over the document, in the order that decides who takes a press: a control shown later
/// is above the earlier ones. Returns the gradient's live proposal, which the picture of the same pass shows, and puts
/// what the Crop tool's frame and the Free Transform box reported in `reported`, for the host's pass.
fn canvas_controls(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    rect: Rect,
    host: CanvasHost,
    space: &CanvasSpace,
    turned: &mut Option<CanvasView>,
    reported: &mut tools::BodyShown,
) -> Option<(photocraft_doc::LayerId, photocraft_doc::Fill)> {
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let now = ui.input(|i| i.time);
    crate::distort_ui::show_in_canvas(app, ui, rect);
    crate::blur_pins::overlay(app, ui, rect);
    let doc = app.session.documents()[host.idx].doc.id;
    if let Some(shown) = crate::vector_ui::overlay::show_paths(app, ui, rect, doc) {
        let mods = craft::ui::runtime::pointer::frame(ui.ctx()).mods;
        crate::direct_select::edits::apply(app, &shown.output, &shown.refs, app.ui.tool, mods);
    }
    crate::type_tool::canvas_controls(app, ui, rect);
    crate::type_path_placement::show(app, ui, rect);
    // Only the document's own canvas shows and drives the open Free Transform (production's `paint_primary_overlays`):
    // a New Window copy of the document shows no box, and its tool keeps its presses.
    let primary = host.is_primary(app);
    let session = if primary { crate::transform_tool::show_in_canvas(app, ui, rect, now) } else { crate::transform_tool::SessionShown::default() };
    let session_box_shown = session.shown;
    reported.transform_snapped = session.snapped;
    reported.transform_picture = session.picture;
    crate::cam_transform::show(app, ui, rect);
    super::transform_controls::show(app, ui, rect, session_box_shown);
    crate::move_box::show(app, ui, rect);
    crate::move_hover::show(app, ui, rect, pointer);
    crate::move_distance::show(app, ui, rect, pointer);
    crate::artboard_tool::show_overlay(app, ui, rect);
    crate::frame_tool::show(app, ui, rect);
    crate::shape_controls::show(app, ui);
    let live = crate::gradient_ui::live_overlay::show(app, ui, rect);
    crate::slice_ui::canvas_controls(ui, app, rect);
    crate::symmetry_ui::canvas_controls(ui, app, rect);
    crate::perspective_crop_ui::body(app, ui, rect);
    crate::analysis::canvas::body(app, ui, rect);
    if primary {
        reported.crop = super::tools::crop::show_frame(app, ui, rect);
    }
    // The scroll bars (Standard Screen Mode only) and the rotation compass change the view they are bound to. They are
    // window chrome that works with every tool, so they are shown after the tools' controls: a control shown later is
    // above the earlier ones, and the Crop frame covers the whole canvas (its outside turns the frame; #16231). They
    // declare parts only over the bars and the compass, so a press elsewhere still reaches the controls below.
    let mut shown = space.view;
    if !app.ui.view.hides_chrome() {
        let size = app.session.documents()[host.idx].doc.size;
        let bounds = Rect64::from_origin_size(Point::ZERO, Size::new(f64::from(size.width), f64::from(size.height)));
        CanvasScrollbars::new("Scroll bars", Bind::bound(&mut shown), bounds)
            .overscroll(app.session.prefs().tools.overscroll)
            .document_chrome()
            .show_at(ui, rect);
    }
    if RotateView::visible(shown.rotation) {
        let mut rotation = shown.rotation;
        RotateView::new("Rotate View", Bind::bound(&mut rotation), shown.flip_x).show_at(ui, RotateView::compass_rect(ui, rect));
        shown.rotation = rotation;
    }
    // One contextual task bar per window: the active document's tile carries it, not the tile drawn last.
    if app.session.active_index() == Some(host.idx) {
        crate::crop_task_bar::show(app, ui, rect);
        if primary {
            crate::transform_task_bar::show(app, ui, rect);
        }
        crate::frame_tool::task_bar::show(ui, app, rect);
    }
    crate::crop_ui::each_pass(app);
    crate::vector_ui::shapes::show_preview(app, ui, rect, &space.view);
    crate::tools::brush_feedback::show(ui, app, rect, pointer);
    if shown != space.view {
        *turned = Some(shown);
    }
    live
}

/// How the document's picture sits on the canvas: the transparency checkerboard under it, its edge, where a reduced or
/// size-changing picture lies, the pixels past it Crop previews, and View › Pattern Preview's copies around it.
struct Frame<'c> {
    checker: Option<&'c Checker>,
    edge: bool,
    mapping: Option<Homography>,
    pattern: bool,
    region: Option<PixelRect>,
    /// The colour of the region past the document (a crop's grown Background), over the checkerboard.
    region_fill: Option<Color32>,
}

impl Frame<'_> {
    fn apply<'d>(&self, display: RasterDisplay<'d>) -> RasterDisplay<'d> {
        let display = match self.checker {
            Some(checker) => display.checkerboard(*checker),
            None => display,
        };
        let display = if self.edge { display.outlined() } else { display };
        let display = match self.mapping {
            Some(frame) => display.mapped(frame, 1),
            None => display,
        };
        let display = match self.region {
            Some(region) => display.extent(Extent::Region(region)),
            None => display,
        };
        let display = match self.region_fill {
            Some(fill) => display.region_fill(fill),
            None => display,
        };
        if self.pattern { display.extent(Extent::Tiled) } else { display }
    }
}

/// The unit square onto `bounds`, the document area a reduced or size-changing picture covers.
fn placement(bounds: DRect) -> Homography {
    let (x0, y0) = (f64::from(bounds.x0), f64::from(bounds.y0));
    let (w, h) = (f64::from(bounds.width()), f64::from(bounds.height()));
    Homography::from_rows([[w, 0.0, x0], [0.0, h, y0], [0.0, 0.0, 1.0]])
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;
    use serde_json::json;

    use super::*;
    use crate::canvas::testing;
    use crate::state::Tool;

    /// A 64×64 document with a 16×16 selection, shown by a first pass.
    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        testing::frame(&mut app);
        app
    }

    /// #15529: a fitting document can still move through its scroll bars when Overscroll is on.
    #[test]
    fn canvas_scrollbars_follow_overscroll_for_a_fitting_document() {
        check_canvas_scrollbar_overscroll(false);
    }

    #[test]
    fn canvas_scrollbars_follow_overscroll_with_rulers() {
        check_canvas_scrollbar_overscroll(true);
    }

    fn check_canvas_scrollbar_overscroll(rulers: bool) {
        use egui::accesskit::Role;
        use egui::{Event, PointerButton, pos2, vec2};
        use egui_kittest::kittest::{NodeT, Queryable};

        let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(1.0).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.ui.extras.rulers = rulers;
            app
        });
        h.run_steps(6);
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.clear_pending();
        h.run_steps(3);

        for overscroll in [false, true] {
            h.state_mut().session.prefs.edit(|prefs| prefs.tools.overscroll = overscroll);
            h.run_steps(3);
            let view = testing::view(h.state()).expect("the document is shown");
            assert_eq!(view.zoom, 1.0, "100% at one pixel per point");
            assert_eq!(view.center, Point::new(200.0, 150.0));
            assert!(view.rect.width() > 600.0 && view.rect.height() > 450.0, "the 400 × 300 document fits");

            let horizontal = h.get_by_role_and_label(Role::ScrollBar, "Horizontal scroll bar");
            let vertical = h.get_by_role_and_label(Role::ScrollBar, "Vertical scroll bar");
            let track = horizontal.rect();
            let thumb_center = vertical.rect().center();
            if rulers {
                // As in 0.23.1, the scroll bars run along the view inside the rulers; the document chrome puts a step arrow,
                // as long as the bar is thick, before each track.
                let arrow = vertical.rect().width();
                let top = vertical.rect().top() - arrow;
                assert!(
                    (top - view.rect.top()).abs() < 0.5,
                    "the vertical bar's top arrow starts at the view's top, below the ruler: {top} vs {}",
                    view.rect.top()
                );
            }
            // With Overscroll the track reaches into the pasteboard until 100 screen points of the document (all of a
            // shorter one) stay in view at the opposite edge (#16328): the document plus, on each side, the visible
            // length less those points. The scroll room is that less the visible length.
            let room = |length: f32, document: f64| if overscroll { f64::from(length) + document - 2.0 * document.min(100.0) } else { 0.0 };
            let x_room = room(view.rect.width(), 400.0);
            let y_room = room(view.rect.height(), 300.0);
            assert_eq!(horizontal.accesskit_node().max_numeric_value(), Some(x_room), "horizontal scroll room with Overscroll {overscroll}");
            assert_eq!(vertical.accesskit_node().max_numeric_value(), Some(y_room), "vertical scroll room with Overscroll {overscroll}");

            // On a fitting document a page press reaches the end of the scrollable range.
            let at = pos2(track.right() - 3.0, track.center().y);
            h.hover_at(at);
            h.run_steps(2);
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                h.run_steps(2);
            }
            let paged = testing::view(h.state()).expect("the document is shown");
            assert!((paged.center.x - (view.center.x + x_room / 2.0)).abs() < 0.5, "track page position with Overscroll {overscroll}: {paged:?}");
            assert_eq!(paged.center.y, view.center.y);

            // The vertical thumb is centred and its drag can move only its own axis.
            h.hover_at(thumb_center);
            h.run_steps(2);
            h.drag_at(thumb_center);
            h.run_steps(1);
            for distance in [8.0, 24.0, 40.0] {
                h.hover_at(thumb_center + vec2(0.0, distance));
                h.run_steps(1);
            }
            h.event(Event::PointerButton { pos: thumb_center + vec2(0.0, 40.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            let dragged = testing::view(h.state()).expect("the document is shown");
            assert_eq!(dragged.center.x, paged.center.x);
            if overscroll {
                assert!(dragged.center.y > view.center.y + 10.0 && dragged.center.y <= view.center.y + y_room / 2.0, "{dragged:?}");
            } else {
                assert_eq!(dragged.center.y, view.center.y);
            }
        }

        // Turning Overscroll off after both axes have moved restores the centred view.
        h.state_mut().session.prefs.edit(|prefs| prefs.tools.overscroll = false);
        h.run_steps(3);
        assert_eq!(testing::view(h.state()).expect("the document is shown").center, Point::new(200.0, 150.0));
        for label in ["Horizontal scroll bar", "Vertical scroll bar"] {
            let bar = h.get_by_role_and_label(Role::ScrollBar, label);
            assert_eq!(bar.accesskit_node().max_numeric_value(), Some(0.0), "{label} after turning Overscroll off");
        }
    }

    /// #16231: with the Crop tool active the scroll bars still scroll the view. The crop frame is shown after them and
    /// covers the whole canvas (its outside turns the frame), so it took every press on them before.
    #[test]
    fn canvas_scrollbars_scroll_while_the_crop_tool_is_active() {
        use egui::accesskit::Role;
        use egui::{Event, PointerButton, vec2};
        use egui_kittest::kittest::Queryable;

        let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(1.0).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.ui.tool = Tool::Crop;
            app
        });
        h.run_steps(6);
        // 400 % makes the document larger than the window, so both bars have room to scroll.
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 4.0;
        view.center = [200.0, 150.0];
        view.clear_pending();
        h.run_steps(4);
        let frame = h.state().canvas_tools.crop.tool.frame();
        assert!(frame.is_some(), "the Crop tool shows its default frame");

        for (label, along) in [("Horizontal scroll bar", vec2(40.0, 0.0)), ("Vertical scroll bar", vec2(0.0, 40.0))] {
            let before = testing::view(h.state()).expect("the document is shown");
            // The view is centred, so the thumb lies over the middle of its track.
            let thumb = h.get_by_role_and_label(Role::ScrollBar, label).rect().center();
            h.hover_at(thumb);
            h.run_steps(2);
            h.drag_at(thumb);
            h.run_steps(1);
            for step in [0.25, 0.5, 1.0] {
                h.hover_at(thumb + along * step);
                h.run_steps(1);
            }
            h.event(Event::PointerButton { pos: thumb + along, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            let after = testing::view(h.state()).expect("the document is shown");
            if along.x > 0.0 {
                assert!(after.center.x > before.center.x + 10.0, "{label} drag scrolls the view: {before:?} -> {after:?}");
                assert_eq!(after.center.y, before.center.y, "{label} drag keeps the other axis");
            } else {
                assert!(after.center.y > before.center.y + 10.0, "{label} drag scrolls the view: {before:?} -> {after:?}");
                assert_eq!(after.center.x, before.center.x, "{label} drag keeps the other axis");
            }
            let tool = &h.state().canvas_tools.crop.tool;
            assert_eq!(tool.frame(), frame, "{label} drag leaves the crop frame alone");
            assert!(!tool.modified(), "{label} drag starts no crop edit");
        }
    }

    /// Four conditions decide whether the selection's ants show; each hides them alone, and Escape on an open polygon
    /// outline brings them back.
    #[test]
    fn selection_ants_visibility() {
        let mut app = app();
        let idx = app.session.active_index().unwrap();
        assert_eq!(ants_shown(&app, idx), Some((0, 0)), "a selection shows its ants");

        app.ui.view.show.selection_edges = false;
        assert_eq!(ants_shown(&app, idx), None, "Selection Edges off hides them");
        app.ui.view.show.selection_edges = true;

        // A Polygonal or Magnetic Lasso outline left open replaces the selection when made: its ants are hidden until
        // it ends.
        for tool in [Tool::PolygonLasso, Tool::MagneticLasso] {
            app.ui.tool = tool;
            testing::click(&mut app, [30.0, 30.0], Modifiers::NONE);
            testing::click(&mut app, [50.0, 30.0], Modifiers::NONE);
            assert!(app.canvas_tools.selection.outline_open(), "{tool:?}");
            assert_eq!(ants_shown(&app, idx), None, "an open {tool:?} outline hides them");
            testing::key(&mut app, egui::Key::Escape, Modifiers::NONE);
            assert!(!app.canvas_tools.selection.outline_open(), "{tool:?}");
            assert_eq!(ants_shown(&app, idx), Some((0, 0)), "Escape ends the {tool:?} outline and shows them again");
        }

        // A Patch or Content-Aware Move drag that lassos a new selection replaces the old one while it runs.
        app.ui.tool_options.cam_transform_on_drop = false;
        for tool in [Tool::Patch, Tool::ContentAwareMove] {
            app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
            app.ui.tool = tool;
            testing::press(&mut app, [30.0, 30.0], Modifiers::NONE);
            testing::move_to(&mut app, [45.0, 40.0], Modifiers::NONE);
            assert_eq!(ants_shown(&app, idx), None, "a {tool:?} drag hides them");
            testing::release(&mut app, [45.0, 40.0], Modifiers::NONE);
        }

        // Content-Aware Move's Transform On Drop box draws the ants where it puts them.
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.ui.tool_options.cam_transform_on_drop = true;
        assert!(crate::cam_transform::begin(&mut app, [20, 8]), "the dropped patch gets a box");
        assert_eq!(ants_shown(&app, idx), None, "the Transform On Drop box hides them");
        crate::cam_transform::cancel(&mut app);
        assert_eq!(ants_shown(&app, idx), Some((0, 0)), "cancelling the box shows them again");

        // A floating piece shows them at its offset.
        app.session.execute("select.float", json!({"dx": 4, "dy": 3})).unwrap();
        assert_eq!(ants_shown(&app, idx), Some((4, 3)), "a floating piece's ants follow its offset");
    }
}
