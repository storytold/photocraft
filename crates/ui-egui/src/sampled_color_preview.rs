//! Floating readout for the Eyedropper's uncommitted hover sample.
//!
//! The sampled value comes from `canvas::composite_color`, the same one-pixel composite used by
//! the committed Eyedropper and Color Picker. The cache key keeps pointer motion within one pixel
//! from repeatedly asking the compositor for the same value.

use egui::{Color32, Pos2, Stroke, vec2};
use photocraft_doc::{DocId, Document};
use std::sync::{Arc, Weak};

use crate::PhotocraftApp;
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleKey {
    doc: DocId,
    revision: u64,
    x: i32,
    y: i32,
}

/// Cached hover sample. This is UI-only state: it never changes a tool colour or document.
#[derive(Default)]
pub(crate) struct Preview {
    key: Option<SampleKey>,
    rgb: Option<[f32; 3]>,
    document: Option<Weak<Document>>,
}

impl Preview {
    pub(crate) fn clear(&mut self) {
        self.key = None;
        self.rgb = None;
        self.document = None;
    }

    fn cached(&self, key: SampleKey, document: &Arc<Document>) -> Option<Option<[f32; 3]>> {
        let same_document = self.document.as_ref().and_then(Weak::upgrade).is_some_and(|cached| Arc::ptr_eq(&cached, document));
        (same_document && self.key == Some(key)).then_some(self.rgb)
    }

    fn store(&mut self, key: SampleKey, document: &Arc<Document>, rgb: Option<[f32; 3]>) {
        self.key = Some(key);
        self.rgb = rgb;
        self.document = Some(Arc::downgrade(document));
    }
}

/// Return the composite colour at a finite in-bounds document point, caching by pixel and
/// revision. `None` means the cursor is outside the image or over fully transparent pixels.
pub(crate) fn sample(app: &mut PhotocraftApp, document: &Arc<Document>, revision: u64, size: [u32; 2], point: [f64; 2]) -> Option<[f32; 3]> {
    let Some([x, y]) = pixel_at(point, size) else {
        app.sampled_color_preview.clear();
        return None;
    };
    let key = SampleKey { doc: document.id, revision, x, y };
    if let Some(rgb) = app.sampled_color_preview.cached(key, document) {
        return rgb;
    }
    let rgb = crate::canvas::composite_color(app, f64::from(x), f64::from(y));
    app.sampled_color_preview.store(key, document, rgb);
    rgb
}

/// Convert a document-space point to an addressable pixel without casting hostile coordinates.
fn pixel_at(point: [f64; 2], size: [u32; 2]) -> Option<[i32; 2]> {
    if !(point[0].is_finite() && point[1].is_finite()) {
        return None;
    }
    let x = point[0].floor();
    let y = point[1].floor();
    if x < 0.0 || y < 0.0 || x >= f64::from(size[0]) || y >= f64::from(size[1]) || x > f64::from(i32::MAX) || y > f64::from(i32::MAX) {
        return None;
    }
    Some([x as i32, y as i32])
}

/// Where to put the readout around the sampled pixel. Pick the side with the most room so egui
/// does not have to clamp the area back across the sample at a canvas edge.
fn placement(pixel: egui::Rect, bounds: egui::Rect) -> (Pos2, egui::Align2) {
    const GAP: f32 = 12.0;
    let right_edge = pixel.right().min(bounds.right());
    let left_edge = pixel.left().max(bounds.left());
    let right = bounds.right() - right_edge >= left_edge - bounds.left();
    let bottom_edge = pixel.bottom().min(bounds.bottom());
    let top_edge = pixel.top().max(bounds.top());
    let below = bounds.bottom() - bottom_edge >= top_edge - bounds.top();
    let x = if right { right_edge + GAP } else { left_edge - GAP };
    let y = if below { bottom_edge + GAP } else { top_edge - GAP };
    let anchor = match (right, below) {
        (true, true) => egui::Align2::LEFT_TOP,
        (false, true) => egui::Align2::RIGHT_TOP,
        (true, false) => egui::Align2::LEFT_BOTTOM,
        (false, false) => egui::Align2::RIGHT_BOTTOM,
    };
    (egui::pos2(x, y), anchor)
}

/// Draw a compact, pointer-transparent swatch and its 8-bit display readouts outside the sampled
/// pixel, constrained to the canvas.
pub(crate) fn draw(ctx: &egui::Context, pixel: egui::Rect, bounds: egui::Rect, rgb: [f32; 3]) -> egui::Rect {
    let t = Tokens::get(ctx);
    let hex = crate::color_picker_ui::hex(rgb);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgb_text = format!("{}, {}, {}", byte(rgb[0]), byte(rgb[1]), byte(rgb[2]));
    let swatch = Color32::from_rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2]));
    let (pos, anchor) = placement(pixel, bounds);

    egui::Area::new(egui::Id::new("sampled-color-preview"))
        .order(egui::Order::Tooltip)
        .pivot(anchor)
        .fixed_pos(pos)
        .interactable(false)
        .constrain_to(bounds)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_sm)
                .inner_margin(egui::Margin::symmetric(6, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, t.radius_sm, swatch);
                        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
                        ui.add_space(3.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(egui::RichText::new(format!("{} {hex}", tl!("HEX:"))).size(11.0).color(t.text));
                            ui.label(egui::RichText::new(format!("{} {rgb_text}", tl!("RGB:"))).size(11.0).color(t.text));
                        });
                    });
                });
        })
        .response
        .rect
}

/// Other modal surfaces are drawn after the canvas. The hover readout uses Tooltip order, so it
/// must be suppressed while these surfaces are open rather than relying on draw order.
pub(crate) fn blocks_canvas_hover(app: &PhotocraftApp) -> bool {
    !app.ui.dialogs.is_empty()
        || app.ui.shell.dialog.is_some()
        || app.ui.palette_open
        || app.discard.is_some()
        || app.tiff_options.is_some()
        || app.camera_raw.is_some()
        || app.wide_angle.is_some()
        || app.distort.gallery.is_some()
        || app.distort.liquify.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 20, 10], "fill": "#ff0000"})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [20, 0, 20, 10], "fill": "#00ff00"})).unwrap();
        app
    }

    #[test]
    fn pixel_address_rejects_out_of_bounds_and_nonfinite_coordinates() {
        assert_eq!(pixel_at([4.9, 7.1], [40, 20]), Some([4, 7]));
        for point in [[-0.01, 0.0], [40.0, 1.0], [1.0, 20.0], [f64::NAN, 1.0], [1.0, f64::INFINITY], [f64::NEG_INFINITY, 0.0]] {
            assert_eq!(pixel_at(point, [40, 20]), None, "{point:?}");
        }
    }

    #[test]
    fn hover_sample_tracks_pixels_without_committing_colors_or_history() {
        let mut app = app();
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        let colors = (app.session.tools.foreground, app.session.tools.background);
        let history = app.session.active().map(|st| st.history.past_len()).unwrap();

        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [10.2, 5.8]), Some([1.0, 0.0, 0.0]));
        // Motion within one source pixel uses the cached composite.
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [10.9, 5.1]), Some([1.0, 0.0, 0.0]));
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [25.0, 5.0]), Some([0.0, 1.0, 0.0]));

        assert_eq!((app.session.tools.foreground, app.session.tools.background), colors);
        assert_eq!(app.session.active().map(|st| st.history.past_len()), Some(history));
        assert_eq!(app.session.active().map(|st| st.revision), Some(revision));
    }

    #[test]
    fn transparent_outside_and_nonfinite_hover_positions_hide_the_preview() {
        let mut app = app();
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [30.0, 15.0]), None);
        assert_eq!(app.sampled_color_preview.rgb, None, "fully transparent pixels clear the readout");
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [40.0, 10.0]), None);
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [f64::NAN, 0.0]), None);
        assert_eq!(app.sampled_color_preview.key, None, "invalid coordinates clear the cached readout");
    }

    #[test]
    fn display_readouts_preserve_depth_until_rgb_hex_formatting() {
        for depth in [8, 16, 32] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 4, "height": 4, "depth": depth, "background": "#336699"})).unwrap();
            let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
            let rgb = sample(&mut app, &doc, revision, [4, 4], [2.0, 2.0]).unwrap();
            assert!((rgb[0] - 0x33 as f32 / 255.0).abs() < 0.01, "depth {depth}: {rgb:?}");
            assert!((rgb[1] - 0x66 as f32 / 255.0).abs() < 0.01, "depth {depth}: {rgb:?}");
            assert!((rgb[2] - 0x99 as f32 / 255.0).abs() < 0.01, "depth {depth}: {rgb:?}");
            assert_eq!(crate::color_picker_ui::hex(rgb), "#336699");
        }
    }

    #[test]
    fn a_new_document_instance_cannot_reuse_a_persisted_id_and_revision_cache_entry() {
        let mut old = app();
        let (old_doc, old_revision) = old.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        assert_eq!(sample(&mut old, &old_doc, old_revision, [40, 20], [25.0, 5.0]), Some([0.0, 1.0, 0.0]));

        // Native documents preserve their DocId and may reopen at the same revision. Keep the
        // cached entry while replacing the active document to reproduce that lifecycle directly.
        let mut reopened = app();
        reopened.run("layer.new.layer", json!({})).unwrap();
        reopened.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let mut doc = (*reopened.session.active().unwrap().doc).clone();
        doc.id = old_doc.id;
        reopened.session.active_mut().unwrap().doc = Arc::new(doc);
        reopened.session.active_mut().unwrap().revision = old_revision;
        reopened.sampled_color_preview = std::mem::take(&mut old.sampled_color_preview);

        let reopened_doc = reopened.session.active().unwrap().doc.clone();
        assert_eq!(sample(&mut reopened, &reopened_doc, old_revision, [40, 20], [25.0, 5.0]), Some([0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_composite_edit_invalidates_the_sample_at_the_same_pixel() {
        let mut app = app();
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [25.0, 5.0]), Some([0.0, 1.0, 0.0]));

        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        assert_eq!(sample(&mut app, &doc, revision, [40, 20], [25.0, 5.0]), Some([0.0, 0.0, 1.0]));
    }

    #[test]
    fn readout_area_stays_outside_the_sampled_pixel_at_canvas_edges() {
        let bounds = egui::Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0));
        for pixel in [
            egui::Rect::from_min_size(egui::pos2(1.0, 1.0), vec2(1.0, 1.0)),
            egui::Rect::from_min_size(egui::pos2(599.0, 1.0), vec2(1.0, 1.0)),
            egui::Rect::from_min_size(egui::pos2(1.0, 399.0), vec2(1.0, 1.0)),
            egui::Rect::from_min_size(egui::pos2(599.0, 399.0), vec2(1.0, 1.0)),
        ] {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, Default::default());
            let mut area = egui::Rect::NOTHING;
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(bounds), ..Default::default() }, |ui| {
                    area = draw(ui.ctx(), pixel, bounds, [1.0, 0.0, 0.0]);
                });
                output.textures_delta.clear();
            }
            assert!(bounds.contains_rect(area), "area {area:?} must remain on the canvas");
            assert!(!area.intersects(pixel), "area {area:?} must not cover sampled pixel {pixel:?}");
        }
    }

    #[test]
    fn real_canvas_hover_shows_preview_and_eyedropper_drag_still_samples() {
        use crate::state::{Tool, View};
        use egui::{Event, PointerButton, pos2};
        use egui_kittest::Harness;

        let mut app = app();
        app.ui.tool = Tool::Eyedropper;
        app.ui.views = vec![View { zoom: 1.0, center: [20.0, 10.0], fit_pending: false, doc_size: [40, 20] }];
        let mut h = Harness::builder().with_size(vec2(600.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let Some(index) = app.session.active_index() else { return };
                let Some(view) = app.ui.views.get(index).cloned() else { return };
                let view = crate::canvas::canvas_view(app, ui, index, ui.max_rect(), view, true);
                if let Some(slot) = app.ui.views.get_mut(index) {
                    *slot = view;
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(2);

        let red = pos2(290.5, 195.5);
        let green = pos2(305.5, 195.5);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.event(Event::PointerMoved(red));
        h.step();
        assert_eq!(h.state().sampled_color_preview.rgb, Some([1.0, 0.0, 0.0]), "real canvas hover samples the image");

        h.event(button(red, true));
        h.step();
        h.event(Event::PointerMoved(green));
        h.step();
        assert_eq!(h.state().session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "drag keeps sampling as before");
        assert_eq!(h.state().sampled_color_preview.key, None, "no hover readout is drawn while sampling by drag");
        h.event(button(green, false));
        h.step();
        assert_eq!(h.state().sampled_color_preview.rgb, Some([0.0, 1.0, 0.0]));

        h.state_mut().ui.open_dialog(crate::state::DialogKind::Command, Default::default());
        h.step();
        assert_eq!(h.state().sampled_color_preview.key, None, "dialogs suppress the overlay");
        h.state_mut().ui.dialogs.clear();
        h.state_mut().ui.tool = Tool::Move;
        h.step();
        assert_eq!(h.state().sampled_color_preview.key, None, "switching tools clears the cached hover");

        h.state_mut().ui.tool = Tool::Eyedropper;
        h.state_mut().run("file.new", json!({"width": 40, "height": 20, "background": "#0000ff"})).unwrap();
        let view = View { zoom: 1.0, center: [20.0, 10.0], fit_pending: false, doc_size: [40, 20] };
        h.state_mut().ui.views = vec![view.clone(), view];
        h.state_mut().session.set_active(0);
        h.step();
        assert_eq!(h.state().sampled_color_preview.rgb, Some([0.0, 1.0, 0.0]), "the current canvas view samples its active document");
        h.state_mut().session.set_active(1);
        h.step();
        assert_eq!(h.state().sampled_color_preview.rgb, Some([0.0, 0.0, 1.0]), "switching documents refreshes the hover overlay");
    }

    #[test]
    fn document_area_clears_the_preview_when_no_primary_canvas_is_drawn() {
        let mut app = app();
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        let _ = sample(&mut app, &doc, revision, [40, 20], [10.0, 5.0]);
        assert!(app.sampled_color_preview.key.is_some());

        app.session.close(0);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut output = ctx.run_ui(Default::default(), |ui| crate::canvas::document_area(&mut app, ui));
        output.textures_delta.clear();
        assert_eq!(app.sampled_color_preview.key, None, "the empty/start screen path clears the primary-canvas cache");
    }
}
