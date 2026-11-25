//! Document canvas: display texture cache, view transform, tool dispatch, extra document windows.
//!
//! Rendering is CPU (`photocraft-compose`) for now, uploaded into an egui texture. Brush strokes
//! update only their damage rectangle (`set_partial`). The wgpu compositor (M5) will replace this
//! with direct GPU rendering behind the same `CanvasCache` interface.

use craft::behavior::PressPhase;
use egui::Rect;
use photocraft_doc::Document;

use crate::PhotocraftApp;
use crate::state::View;

#[cfg(test)]
use egui::{Color32, Pos2, Stroke, TextureOptions, Vec2, pos2, vec2};
#[cfg(test)]
use photocraft_doc::LayerContent;
#[cfg(test)]
use photocraft_geom::Rect as DRect;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::state::Tool;

pub mod artboard_ui;
pub(crate) mod background_redraw;
pub(crate) mod cam_preview;
pub mod canvas_tool_menu;
pub mod channel_view;
pub(crate) mod cursor_policy;
mod display;
pub(crate) mod document_area;
pub(crate) mod document_display;
pub(crate) mod document_windows;
mod eyedropper_tool;
pub(crate) mod filter_preview;
pub mod gpu_canvas;
mod gpu_picture;
pub mod guide_select;
pub mod layer_pick_ui;
pub mod move_ui;
mod navigator;
pub mod outline;
pub mod paint_mouse;
mod paint_thread;
#[cfg(test)]
mod paint_thread_tests;
pub(crate) mod paint_tool;
pub mod patch_preview;
pub mod quick_pick;
mod raster_source;
pub mod rulers;
pub mod sampling;
pub mod scroll_all_windows;
pub mod select_mask_view;
pub mod snap_ui;
pub(crate) mod start_screen;
pub(crate) mod strokes;
pub(crate) use strokes::ClonePreviewCache;
pub(crate) mod tabs;
#[cfg(test)]
pub(crate) mod testing;
mod textures;
pub(super) mod tile_overlay;
pub mod tool_feedback;
pub(crate) mod tools;
mod transform_controls;
pub(crate) mod viewport_pages;
mod window_drop;

pub(crate) use cursor_policy::alt_samples;
pub(crate) use display::GPU_OUTPUT;
#[cfg(test)]
pub(crate) use display::cache_key;
#[cfg(test)]
use display::sync_display_lut;
use display::texture_key;
pub(crate) use display::{canvas_display, display_doc, display_image};
pub(crate) use document_area::arranged_cells;
pub use document_area::document_area;
pub(crate) use document_display::view_target;
pub use document_display::{CanvasHost, active_view};
pub use document_windows::extra_windows;
#[cfg(test)]
pub(crate) use filter_preview::committed_filter_preview;
#[cfg(test)]
pub(crate) use filter_preview::ensure_filter_preview;
#[cfg(test)]
use filter_preview::filter_channel_preview;
#[cfg(test)]
use filter_preview::filter_preview_display_doc;
pub(crate) use filter_preview::filter_preview_factor;
pub(crate) use navigator::NavigatorCache;
pub use navigator::{NAVIGATOR_SIDE, navigator_gpu_view, navigator_texture, navigator_waits_for_gpu};
#[cfg(test)]
pub(crate) use raster_source::CompositePrep;
pub(crate) use raster_source::SourceLogs;
pub(crate) use sampling::composite_color;
#[cfg(test)]
pub(crate) use strokes::live_retouch_points;
pub(crate) use strokes::multiple_color_target;
pub use strokes::paint_target;
#[cfg(test)]
pub(crate) use strokes::strokes_live;
pub(crate) use strokes::{CommittedStroke, LiveStroke, unsmoothed_line};
#[cfg(test)]
use tabs::{main_canvas_document, move_target_after_filter};
#[cfg(test)]
pub(crate) use testing::{ToolEvent, tool_event, tool_event_response};
pub use textures::MAX_TEXTURE;
pub(crate) use textures::visible_doc_rect;
pub(crate) use textures::{damage_since, effect_reach, retain_gpu_documents, shown_as_document};
#[cfg(test)]
use textures::{ensure_gpu, redraw_focus};
#[cfg(test)]
pub(crate) use tools::selection::floating_outline;
#[cfg(test)]
pub(crate) use transform_controls::layer_edges;

/// One document view's cached display state.
#[derive(Default)]
pub struct CanvasCache {
    pub revision: u64,
    /// Hash of the live-adjust preview used for this render (0 = none).
    pub preview_key: u64,
    /// The current image lives in the GPU canvas (`gpu_canvas`).
    pub on_gpu: bool,
}

/// Photoshop-style zoom label: "33.3", "100", "12.5".
pub fn fmt_zoom(pct: f32) -> String {
    if (pct - pct.round()).abs() < 0.05 { format!("{}", pct.round() as i64) } else { format!("{pct:.1}") }
}

pub fn mode_label(doc: &Document) -> &'static str {
    match doc.mode {
        photocraft_doc::ColorMode::Rgb => "RGB",
        photocraft_doc::ColorMode::Grayscale => "Gray",
        photocraft_doc::ColorMode::Cmyk => "CMYK",
        photocraft_doc::ColorMode::Lab => "Lab",
        photocraft_doc::ColorMode::Indexed => "Indexed",
        photocraft_doc::ColorMode::Bitmap => "Bitmap",
        photocraft_doc::ColorMode::Duotone => "Duotone",
        photocraft_doc::ColorMode::Multichannel => "Multichannel",
    }
}

/// Per-frame reconciliation of the camera with its document: a resize re-centres it. Returns the pixel aspect the view
/// is drawn with.
fn sync_view_to_document(app: &PhotocraftApp, view: &mut View, doc: &Document) -> f32 {
    let size = [doc.size.width, doc.size.height];
    if view.doc_size != size {
        // Like Photoshop: keep the zoom level, re-centre the resized document.
        if view.doc_size != [0, 0] {
            view.center = [size[0] as f32 / 2.0, size[1] as f32 / 2.0];
        }
        view.doc_size = size;
    }
    // View › Pixel Aspect Ratio Correction: document pixels are shown this much wider.
    app.ui.view.display_aspect()
}

/// Draws one document's canvas into `rect` and runs its tools. `host` names the canvas (document, viewport and document
/// window); `primary` is the main window's, where the tools are active. Returns the view after the pass and the
/// canvas's output, whose press the caller reads (a press activates its document).
pub fn canvas_view(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    host: document_display::CanvasHost,
    rect: Rect,
    mut view: View,
    primary: bool,
) -> (View, craft::ui::canvas::CanvasOutput) {
    let ctx = ui.ctx().clone();
    let idx = host.idx;
    // The primary canvas keeps its view in `app.ui.views`: the pass starts from the stored view and stores the one it
    // ends with, unless something stored another view while the pass ran (a tool's commit that moved it), which then
    // stands; view commands asked of it meanwhile are kept for the next pass.
    // A New Window for Document keeps a view of its own (`window_views`), which is not the document's.
    let own_window = host.window.is_some_and(|window| app.ui.window_views.contains_key(&window));
    let stored = (primary && !own_window).then(|| app.ui.views.get(idx).cloned()).flatten();
    // `View::zoom` is device pixels per document pixel; this canvas's geometry is in its own viewport's egui points. A
    // document window can sit on another display with a different scale, so only the primary canvas records `app.ppp`
    // for the frame's shared helpers (`point_zoom`, the navigator, control-channel coordinate mapping).
    // `pixels_per_point` folds in the display scale and Interface › UI Scale alike, so dividing by it keeps 100% one
    // document pixel per physical display pixel at any UI scale (#2121).
    let ppp = ctx.pixels_per_point();
    if primary {
        app.ppp = ppp;
    }
    // The display this window is on: its monitor profile (#569). A document window on a display the last reading
    // didn't know asks for a new one.
    let output = crate::monitor_status::view_display(app, &ctx);
    if !primary {
        crate::monitor_status::check_window(app, &ctx);
    }
    let doc = app.session.documents()[idx].doc.clone();
    let aspect = sync_view_to_document(app, &mut view, &doc);
    let active = primary && app.session.active_index() == Some(idx);
    if active {
        // A document switched while a button is down ends the gesture of the document it began on.
        tools::end_foreign_gesture(app, &ctx, doc.id.0);
    }
    // Pen pressure/tilt for this frame's tool events (mouse = 1.0), unless Preferences › Tools › Use Tablet Pressure
    // is off; the pen's eraser end selects the Eraser.
    app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
    let curve = app.session.prefs().tools.pressure_curve.clone();
    app.stylus.set_pressure_curve(&curve);
    crate::stylus::Stylus::sync_eraser_tool(app);
    // A Properties Levels/Curves eyedropper left behind by a tool or layer switch is dropped.
    crate::layer_picker::prune(app);
    let flip = app.ui.view.flip_horizontal;
    let pass = document_display::CanvasPass { ctx: &ctx, host, doc: &doc, output, rect, ppp, flip, aspect };
    let shown = document_display::show(app, ui, &mut view, &pass);
    canvas_tool_menu::run(ui, app, &shown.output, &shown.response, shown.guides.as_ref(), shown.overlay_context);
    // A press on the canvas while a tool letter is held makes that tool temporary (#2672).
    if active && !app.dialog_open() && matches!(shown.output.press.phase, PressPhase::Down { .. }) {
        crate::spring_tools::note_canvas_press(app);
    }
    if let Some(stored) = stored
        && let Some(current) = app.ui.views.get(idx)
        && let Some(asked) = current.pending.strip_prefix(&stored.pending[..])
        && (View { pending: stored.pending.clone(), ..current.clone() }) == stored
    {
        // View commands asked while the pass ran (a canvas menu row) wait for the next pass.
        let mut next = view.clone();
        next.pending.extend_from_slice(asked);
        app.ui.views[idx] = next;
    }
    (view, shown.output)
}

#[cfg(test)]
mod pattern_drag_tests;

#[cfg(test)]
mod tabs_tests;

#[cfg(test)]
mod doc_window_dialog_tests;

#[cfg(test)]
mod document_window_title_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transform_controls_tests;

#[cfg(test)]
mod layer_edges_tests;

#[cfg(test)]
mod live_frame_tests;

#[cfg(test)]
#[path = "canvas/art_history_shift_click_tests.rs"]
mod art_history_shift_click_tests;

#[cfg(test)]
mod rotation_preview_isolation_tests {
    use craft::ui::dialog::DialogId;
    use photocraft_engine::channel_cmds::ChannelTarget;
    use photocraft_engine::color_cmds::ColorExt;

    use super::*;

    /// Opens the filter dialog of `command` with its Preview box and returns its id. Rotate Canvas has no Preview
    /// (#14462); its size-changing preview (#1436) is reached through `open_previewing`.
    fn open(app: &mut PhotocraftApp, command: &str) -> DialogId {
        let id = crate::filter_dialog::open_previewing(app, command).expect("the dialog opens");
        // The Preview box is on from the dialog's first drawn pass.
        for _ in 0..2 {
            crate::smart_filter_dialog::tests::pass(app);
        }
        id
    }

    /// A harness running the app's dialogs, for the Preview box's clicks.
    fn dialogs_harness(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .with_context(app.ctx().clone())
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
        h.run_steps(2);
        h
    }

    /// Clicks the open dialog's Preview box.
    fn toggle_preview(h: &mut egui_kittest::Harness<'static, Box<PhotocraftApp>>) {
        use egui_kittest::kittest::Queryable;
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Preview").click();
        h.run_steps(2);
    }

    /// Writes a field of the hosted dialog `id`, as the form's control would.
    fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: serde_json::Value) {
        app.framework.dialogs.set_json_field(ctx, id, field, value).unwrap_or_else(|e| panic!("the dialog has a `{field}` field: {e}"));
    }

    #[test]
    fn channel_overlay_shows_filter_preview_and_restores_on_cancel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        app.run("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "filter.blur.gaussianBlur");
        set(&mut app, &ctx, dialog, "radius", json!(2.0));
        ensure_filter_preview(&mut app, 0, &ctx).unwrap();
        let st = app.session.active().unwrap();
        let original =
            crate::channel_view::render(&before, &st.channel_view, before.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
        assert_ne!(overlay_pixels(&mut app, &ctx), original, "a nontrivial filter must be visible before OK");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        let mut h = dialogs_harness(app);
        toggle_preview(&mut h);
        assert_eq!(overlay_pixels(h.state_mut(), &ctx), original, "Preview off restores the channel");
        toggle_preview(&mut h);
        let app = h.state_mut();
        crate::dialogs::cancel(app, &ctx, dialog).unwrap();
        assert_eq!(overlay_pixels(app, &ctx), original, "Cancel restores the channel");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }

    /// The whole of document 0's channel view as the canvas draws it over the composite.
    fn overlay_pixels(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<Color32> {
        use craft::ui::canvas::{PixelRect, RasterSource};
        let source = raster_source::OverlayPrep::new(app, ctx, 0, None, None).expect("a channel view").source(app);
        let size = source.size();
        let mut out = egui::ColorImage::filled([size.width as usize, size.height as usize], Color32::TRANSPARENT);
        source.read(PixelRect::new(0, 0, size.width as i32, size.height as i32), 1, &mut out);
        out.pixels
    }

    fn assert_filter_preview_target(target: ChannelTarget) {
        for depth in [8, 16, 32] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width":16,"height":16,"depth":depth})).unwrap();
            session.execute("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
            session
                .edit("Fixture", |doc, active| {
                    let layer = doc.layer_mut(active.unwrap()).unwrap();
                    layer.mask = Some(photocraft_doc::LayerMask::hide_all());
                    for y in 0..16 {
                        for x in 0..8 {
                            layer.surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            layer.mask.as_mut().unwrap().surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    Ok(())
                })
                .unwrap();
            session.active_mut().unwrap().channel_view.target = target;
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.background_jobs = false;
            let ctx = app.ctx().clone();
            let dialog = open(&mut app, "filter.blur.gaussianBlur");
            set(&mut app, &ctx, dialog, "radius", json!(2.0));
            ensure_filter_preview(&mut app, 0, &ctx).unwrap();
            let preview = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap().clone();
            crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
            let final_doc = &app.session.active().unwrap().doc;
            let bounds = final_doc.bounds();
            assert_eq!(
                preview.layers[0].surface().unwrap().read_region(bounds),
                final_doc.layers[0].surface().unwrap().read_region(bounds),
                "RGB: {depth} {target:?}"
            );
            assert_eq!(preview.channels[0].surface.read_region(bounds), final_doc.channels[0].surface.read_region(bounds), "alpha: {depth} {target:?}");
            assert_eq!(
                preview.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                final_doc.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                "mask: {depth} {target:?}"
            );
        }
    }

    #[test]
    fn filter_preview_matches_the_applied_result_for_every_channel_target() {
        for target in [ChannelTarget::Alpha(0), ChannelTarget::Color(0), ChannelTarget::LayerMask] {
            assert_filter_preview_target(target);
        }
    }

    #[test]
    fn filter_preview_refreshes_when_channel_target_changes() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        for name in ["First", "Second"] {
            app.run("channel.new", json!({"name":name,"fill":"black"})).unwrap();
        }
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[1].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let revision = app.session.active().unwrap().revision;
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "filter.blur.gaussianBlur");
        set(&mut app, &ctx, dialog, "radius", json!(2.0));
        let (first, first_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
        let (second, second_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_ne!(first_key, second_key);
        assert_ne!(first.channels[1].surface.read_region(first.bounds()), second.channels[1].surface.read_region(second.bounds()));
        let (unchanged, unchanged_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert!(std::sync::Arc::ptr_eq(&second, &unchanged));
        assert_eq!(second_key, unchanged_key);
        crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
        let applied = &app.session.active().unwrap().doc;
        assert_eq!(second.channels[1].surface.read_region(second.bounds()), applied.channels[1].surface.read_region(applied.bounds()));
    }

    #[test]
    fn filter_preview_overlay_matches_quick_mask_and_mixed_color_views() {
        for quick_mask in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.background_jobs = false;
            app.run("file.new", json!({"width":16,"height":16})).unwrap();
            app.session
                .edit("Fixture", |doc, active| {
                    if quick_mask {
                        doc.quick_mask =
                            Some(photocraft_doc::AlphaChannel::new("Quick Mask", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8)));
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            if let Some(mask) = &mut doc.quick_mask {
                                mask.surface.write_pixel(x, y, &[1.0]);
                            } else {
                                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            }
                        }
                    }
                    Ok(())
                })
                .unwrap();
            if !quick_mask {
                let view = &mut app.session.active_mut().unwrap().channel_view;
                view.target = ChannelTarget::Color(0);
                view.set_color_visible(2, false);
            }
            let ctx = app.ctx().clone();
            let dialog = open(&mut app, "filter.blur.gaussianBlur");
            set(&mut app, &ctx, dialog, "radius", json!(2.0));
            let shown = overlay_pixels(&mut app, &ctx);
            crate::dialogs::confirm(&mut app, &ctx, dialog).unwrap();
            let st = app.session.active().unwrap();
            let expected =
                crate::channel_view::render(&st.doc, &st.channel_view, st.doc.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
            assert_eq!(shown, expected, "quick_mask={quick_mask}");
        }
    }

    /// Assign and Convert to Profile have Photoshop's Preview. Assign changes only the profile, so
    /// its preview must be shown through the profile it assigns (through the document's own it
    /// looked unchanged); Convert keeps the look of in-gamut colours. Their `working` is the
    /// working space from Color Settings, not the built-in default.
    #[test]
    fn profile_dialogs_preview_through_the_profile_they_choose() {
        let mut session = photocraft_engine::Session::new();
        session.color_mut().settings.monitor_profile = "srgb".into();
        session.color_mut().settings.working_rgb = "display-p3".into();
        session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        session.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
        session
            .edit("Fixture", |doc, _| {
                let layer = doc.layers.last_mut().unwrap();
                for y in 0..4 {
                    for x in 0..4 {
                        layer.surface_mut().unwrap().write_pixel(x, y, &[0.6, 0.35, 0.25, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let shown_pixels = |app: &PhotocraftApp, doc: &Document| {
            let (display, _) = canvas_display(app, doc, None);
            display_image(display.as_deref(), &photocraft_compose::flatten(doc)).pixels[0]
        };
        let original = app.session.active().unwrap().doc.clone();
        let before = shown_pixels(&app, &original);
        let preview = |app: &mut PhotocraftApp, command: &str, profile: &str| {
            let ctx = app.ctx().clone();
            let dialog = open(app, command);
            set(app, &ctx, dialog, "profile", json!(profile));
            ensure_filter_preview(app, 0, &ctx).expect("a preview");
            let result = app.filter_preview.as_ref().unwrap().result.clone().unwrap();
            let key = original.id.0 ^ (1u64 << 61);
            assert_eq!(filter_preview_display_doc(app, &original, key).icc_profile, result.icc_profile, "the canvas shows it through its own profile");
            assert_eq!(app.session.active().unwrap().doc.icc_profile, original.icc_profile, "previewing changes nothing");
            crate::dialogs::cancel(app, &ctx, dialog).unwrap();
            app.filter_preview = None;
            result
        };
        let description = |doc: &Document| photocraft_engine::color_cmds::profile_from_bytes(doc.icc_profile.as_ref().unwrap()).unwrap().description.clone();

        let assigned = preview(&mut app, "edit.assignProfile", "prophoto-compat");
        assert_eq!(description(&assigned), craft::cms::Builtin::ProPhotoCompat.description());
        let shown = shown_pixels(&app, &assigned);
        assert!(shown.r().abs_diff(before.r()) > 10 || shown.b().abs_diff(before.b()) > 10, "the assigned profile shows: {before:?} → {shown:?}");

        let converted = preview(&mut app, "edit.convertToProfile", "working");
        assert_eq!(description(&converted), craft::cms::Builtin::DisplayP3.description(), "`working` is Color Settings' working space");
        let shown = shown_pixels(&app, &converted);
        for (a, b) in [(shown.r(), before.r()), (shown.g(), before.g()), (shown.b(), before.b())] {
            assert!(a.abs_diff(b) <= 2, "converting keeps the look: {before:?} → {shown:?}");
        }
    }

    /// A CPU-only app must be able to generate the actual rotation preview without a
    /// GPU adapter; checking the result also verifies Preview off and document integrity.
    #[test]
    fn rotation_preview_is_computed_headlessly_without_mutating_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        assert!(app.gpu.is_none());
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "image.rotation.arbitrary");
        set(&mut app, &ctx, dialog, "angle", json!(90.0));
        set(&mut app, &ctx, dialog, "direction", json!("cw"));

        let (factor, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("CPU preview computed");
        assert_eq!(factor, 1);
        assert_eq!(size, [48, 96]);
        let original = &app.session.active().unwrap().doc;
        assert_eq!([original.size.width, original.size.height], [96, 48]);

        let mut h = dialogs_harness(app);
        toggle_preview(&mut h);
        let app = h.state_mut();
        assert!(ensure_filter_preview(app, 0, &ctx).is_none(), "disabling Preview must hide the preview");
        assert_eq!([app.session.active().unwrap().doc.size.width, app.session.active().unwrap().doc.size.height], [96, 48]);
    }

    /// A dialog operates on the active document and cannot render a preview into a
    /// background tab, even if that tab has a valid canvas index.
    #[test]
    fn filter_preview_is_not_returned_for_inactive_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let ctx = app.ctx().clone();
        let dialog = open(&mut app, "image.rotation.arbitrary");
        set(&mut app, &ctx, dialog, "angle", json!(90.0));
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some());

        app.session.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        assert_eq!(app.session.active_index(), Some(1));
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "an inactive tab must never inherit the command preview");
    }
}

#[cfg(test)]
mod zoom_animation_tests {
    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point;

    use super::*;

    /// A view of a 1000×700 canvas at `zoom` on `center`, turned by `rotation` degrees.
    fn camera(zoom: f64, center: Point, rotation: f64) -> CanvasView {
        CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0)), center, zoom).rotation(rotation)
    }

    /// Deferred uploads prioritize both visible areas throughout a recentering zoom.
    #[test]
    fn deferred_redraw_focus_covers_target_and_displayed_cameras() {
        for rotation in [0.0, 37.0] {
            let initial = camera(1.0, Point::new(200.0, 150.0), rotation);
            let target = camera(2.0, Point::new(1600.0, 900.0), rotation);
            let target_visible = visible_doc_rect(&target);
            // The cameras a zoom ease shows on its way from `initial` to `target`.
            for t in [0.0f64, 0.5, 1.0] {
                let lerp = |a: f64, b: f64| a + (b - a) * t;
                let center = Point::new(lerp(initial.center.x, target.center.x), lerp(initial.center.y, target.center.y));
                let displayed = camera(lerp(initial.zoom, target.zoom), center, rotation);
                let displayed_visible = visible_doc_rect(&displayed);
                let focus = redraw_focus(&target, &displayed);
                assert_eq!(focus.intersect(&target_visible), target_visible);
                assert_eq!(focus.intersect(&displayed_visible), displayed_visible);
                if t == 0.0 {
                    assert_ne!(focus, target_visible, "the old view must also receive priority");
                }
            }
            assert_eq!(redraw_focus(&target, &target), target_visible, "settled views keep the same focus");
        }
    }
}
#[cfg(test)]
mod display_memo_tests;

#[cfg(test)]
mod cpu_background_gpu_tests;

#[cfg(test)]
#[path = "canvas/blend_viewport_gpu_tests.rs"]
mod blend_viewport_gpu_tests;

#[cfg(test)]
#[path = "canvas/pixel_alignment_tests.rs"]
mod pixel_alignment_tests;

#[cfg(test)]
mod multi_channel_ui_tests {
    use super::*;
    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width":64,"height":64})).unwrap();
        app.sync_views();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app
    }
    #[test]
    fn pixel_tools_capture_the_whole_channel_set_and_gradients_stay_on_pixels() {
        let mut app = app();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}));
        app.ui.tool = Tool::Gradient;
        app.ui.tool_options.gradient_classic = false;
        assert!(!crate::gradient_ui::live_mode(&app));
    }
    #[test]
    fn viewing_a_layer_mask_overrides_colour_targets_for_live_brush_strokes() {
        for depth in [8, 16, 32] {
            for view in ["gray", "overlay"] {
                for colors in [vec![0], vec![0, 2]] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.run("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    app.run("view.layerMask", json!({"mode":view})).unwrap();
                    assert_eq!(paint_target(&app), json!("mask"), "{depth}-bit {view} {colors:?}");
                    assert!(app.layer_mask_targeted());
                    app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
                    app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
                    app.ui.tool = Tool::Brush;
                    let st = app.session.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let original = st.doc.layer(layer).unwrap().surface().unwrap().clone();
                    let steps = st.history.past_len();
                    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    let shown = display_doc(&mut app, 0).0;
                    let shown_layer = shown.layer(layer).unwrap();
                    assert_eq!(shown_layer.surface().unwrap(), &original);
                    assert!(shown_layer.mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1);
                    tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
                    let st = app.session.active().unwrap();
                    let committed = st.doc.layer(layer).unwrap();
                    assert_eq!(committed.surface().unwrap(), &original);
                    assert_eq!(committed.mask.as_ref().unwrap().surface, shown_layer.mask.as_ref().unwrap().surface);
                    assert_eq!(st.history.past_len(), steps + 1);
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    assert_eq!(paint_target(&app), json!({"colors":colors}));
                    assert!(!app.layer_mask_targeted());
                }
            }
        }
    }
    /// The colour rows stay selected when another layer becomes active. An adjustment layer has no
    /// pixels, so the Brush paints its mask, as it does without colour rows selected.
    #[test]
    fn colour_rows_left_selected_still_paint_an_adjustment_layers_mask() {
        for colors in [vec![0], vec![0, 2]] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":64,"height":64})).unwrap();
            app.run("channel.target", json!({"colors":colors})).unwrap();
            app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
            app.sync_views();
            assert_eq!(paint_target(&app), json!("mask"), "{colors:?}");
            app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
            app.ui.tool = Tool::Brush;
            let st = app.session.active().unwrap();
            let layer = st.active_layer.unwrap();
            let steps = st.history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), steps + 1, "{colors:?}: one stroke");
            assert!(st.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1, "{colors:?}: the mask was painted");
        }
    }

    /// Targeting a layer mask with colour rows selected (its thumbnail, or adding the mask) moves
    /// the Channels panel onto the mask, as in Photoshop: tools, dialogs and menu commands then
    /// all edit the mask. Several rows still stop the Move tool on a layer that needs
    /// rasterizing, which paints only after the prompt.
    #[test]
    fn targeting_a_mask_over_colour_rows_edits_the_mask_everywhere() {
        let set = |app: &mut PhotocraftApp, p: serde_json::Value| {
            let (req, _) = crate::control::ControlRequest::new("ui.set", p);
            let ctx = app.ctx().clone();
            assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Reply::Done(v) if v["ok"] != false));
        };
        for colors in [vec![0], vec![0, 2]] {
            for add in [false, true] {
                let mut app = self::app();
                if !add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    set(&mut app, json!({"maskTarget": false}));
                }
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?}: the rows are the target first");
                let journal = app.session.journal.len();
                if add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                } else {
                    set(&mut app, json!({"maskTarget": true}));
                }
                app.sync_views();
                assert_eq!(app.session.journal.len(), journal + usize::from(add), "{colors:?} add={add}: no extra recorded step");
                assert_eq!(paint_target(&app), json!("mask"), "{colors:?} add={add}");
                assert!(app.layer_mask_targeted(), "{colors:?} add={add}");
                assert_eq!(app.with_mask_target("filter.blur.gaussianBlur", json!({}))["target"], "mask");
                assert!(crate::distort_ui::colour_set_target(&app).is_null());
                assert!(!multiple_color_target(&app));
                // Clicking a colour row again takes the rows back.
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?} add={add}: the rows again");
            }
        }
        // A mask added to another layer, or a fill layer's mask, leaves the rows targeted.
        let mut app = self::app();
        let active = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer": active.0})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("layer.layerMask.revealAll", json!({"layer": other.0})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}), "a mask on another layer");
        app.run("layer.newFillLayer.solidColor", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        set(&mut app, json!({"maskTarget": true}));
        app.sync_views();
        assert!(matches!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_)), "a fill layer");
        let mut app = self::app();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!("pixels"));
        assert!(crate::rasterize_prompt::needed(&app).is_some());
        assert!(multiple_color_target(&app));
        let ctx = app.ctx().clone();
        let refused = crate::liquify_ui::open(&mut app, &ctx).unwrap_err();
        assert!(refused.contains("Rasterize"), "{refused}");
    }

    #[test]
    fn subset_does_not_block_vector_path_transforms() {
        let mut app = app();
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[10,10],[30,10],[30,20],[10,20]]}]}})).unwrap();
        app.ui.tool = Tool::PathSelection;
        assert!(crate::menus::is_enabled(&app, "edit.freeTransform"));
        let ctx = app.ctx().clone();
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        assert!(app.ui.transform.as_ref().unwrap().path.is_some());
    }
}

#[cfg(test)]
#[path = "pen_gesture_owner_tests.rs"]
mod pen_gesture_owner_tests;

#[cfg(test)]
#[path = "path_selection_cursor_tests.rs"]
mod path_selection_cursor_tests;
