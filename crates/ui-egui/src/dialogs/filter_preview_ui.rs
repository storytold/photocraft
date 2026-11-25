//! The picture the filter dialogs' preview pane shows (#3769).
//!
//! Photoshop shows a preview of the filtered result *inside* its filter dialogs. The pane, its zoom, pan and
//! press-and-hold compare belong to the framework's `ParamDialog`; this module gives each open dialog the
//! picture: the canvas's filtered proxy (`filter_dialog::preview_document`) while its Preview box is on, else the
//! unfiltered proxy ([`feed`]). The Preview checkbox also keeps toggling the on-canvas live result
//! (`canvas::ensure_filter_preview`). While such a dialog is on top, the image under it is the preview's locator:
//! the pointer is a small square there, and a press centres the pane on that document point ([`centre_on`], #9648).

use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::{HostedParamDialog, PreviewState};
use egui::load::SizedTexture;
use egui::{Pos2, pos2};
use photocraft_doc::Document;
use serde_json::Value;

use crate::PhotocraftApp;
use crate::filter_dialog::FilterPreviewKey;

/// What a dialog's preview shows: the command it edits, that command's params as the dialog holds them now,
/// and the command the canvas previews for it (an edited smart filter previews as `layer.smartFilter.setParams`,
/// #6324).
struct Subject {
    command: String,
    params: Value,
    previewed: String,
}

/// What `dialog` previews, or `None` when it has no preview picture: a filter dialog, or the Blending Options
/// dialog of a smart filter.
fn subject(app: &PhotocraftApp, dialog: &HostedParamDialog) -> Option<Subject> {
    let key = dialog.key()?;
    // The smart filter an open edits: the open's captured value, not a dialog field.
    let captured = crate::dialogs::params::captured(app, key);
    if crate::smart_filter_dialog::owns(key) {
        let (command, params) = crate::smart_filter_dialog::blend_command(app, &captured, dialog.fields()).ok()?;
        return Some(Subject { previewed: command.clone(), command, params });
    }
    if !shows(key) {
        return None;
    }
    let params = app.with_mask_target(key, crate::filter_dialog::command_params(key, dialog.fields()));
    Some(Subject { command: key.to_owned(), params, previewed: crate::smart_ui::preview_command(key, key, &captured).to_owned() })
}

/// The filtered proxy the canvas already computed for `id`, when there is one. Matching the dialog keeps a
/// stacked dialog from showing another's preview; the Preview checkbox decides whether the filtered result
/// (rather than the original) is shown.
fn filtered_preview(
    app: &PhotocraftApp,
    id: DialogId,
    doc: photocraft_doc::DocId,
    previewed: &str,
    previewing: bool,
) -> Option<(FilterPreviewKey, std::sync::Arc<Document>)> {
    let preview = app.filter_preview.as_ref()?;
    let key = &preview.key;
    (previewing && key.dialog == id && key.doc == doc && key.command == previewed).then(|| preview.result.clone().map(|result| (key.clone(), result))).flatten()
}

/// The proxy factor of the picture the dialog shows: the filtered key's when it exists, else the request's.
/// The fallback must be the factor the canvas will ask for, not the document's own: since #6326 the canvas caps it
/// with the view's zoom, and a pane that sized its textures from the uncapped factor built a proxy the canvas never
/// asked for, then rebuilt it the moment the first result arrived, shifting the pane's mapping for a frame.
fn picture_factor(app: &PhotocraftApp, id: DialogId, dialog: &HostedParamDialog, doc: &Document, subject: &Subject) -> u32 {
    filtered_preview(app, id, doc.id, &subject.previewed, dialog.is_previewing())
        .map(|(key, _)| key.k)
        .unwrap_or_else(|| crate::canvas::filter_preview_factor(app, doc, &subject.command, &subject.params))
}

/// Gives every open dialog that has a preview pane its picture: [`HostedParamDialog::set_preview`] each pass.
///
/// A second pass in one frame hands the same picture: nothing here reads the frame's input or drains a queue, and
/// the texture cache uploads nothing for a picture that did not change.
pub(crate) fn feed(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let ids: Vec<DialogId> = app.framework.dialogs.stack().ids().collect();
    for id in ids {
        let Some(picture) = picture(app, ctx, id) else { continue };
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
            dialog.set_preview(picture.map(PreviewState::Ready));
        }
    }
}

/// The picture of dialog `id`: `None` when the dialog has no preview pane, `Some(None)` when it has one and there
/// is nothing to show.
fn picture(app: &PhotocraftApp, ctx: &egui::Context, id: DialogId) -> Option<Option<SizedTexture>> {
    let dialog = app.framework.dialogs.get_by_id::<HostedParamDialog>(id)?;
    let subject = subject(app, dialog)?;
    let Some(st) = app.session.active() else { return Some(None) };
    let (doc_id, revision) = (st.doc.id, st.revision);
    let filtered = filtered_preview(app, id, doc_id, &subject.previewed, dialog.is_previewing());
    let base_k = picture_factor(app, id, dialog, &st.doc, &subject);

    let cache_id = egui::Id::unique(("filter-preview-ui", id.value()));
    let mut cache: Cache = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<Cache>(cache_id)));
    // Channel visibility and the active mask can change without a document revision, and so can a tile that fails
    // to read and recovers (`crate::scratch_reads`). Both the original (press-and-hold) and filtered textures must
    // follow the canvas's current view.
    let view = (st.channel_view.clone(), app.session.prefs().interface.show_channels_in_color, st.active_layer, crate::scratch_reads::generation());
    if cache.view.as_ref() != Some(&view) {
        cache = Cache { view: Some(view), ..Default::default() };
    }

    // Unfiltered proxy: built once per document revision and factor (cheap at the proxy factor).
    let orig_key = TexKey::Original(doc_id, revision, base_k);
    if cache.original.as_ref().map(|c| &c.key) != Some(&orig_key) {
        let proxy = crate::proxy::proxy_document(&st.doc, base_k);
        let tex = upload(ctx, app, "filter-preview-original", &proxy);
        cache.original = Some(CachedTex { key: orig_key, tex, size: [proxy.size.width, proxy.size.height] });
    }
    // Filtered proxy: rebuilt whenever the worker publishes a new result.
    match &filtered {
        Some((key, filtered_doc)) => {
            let fkey = TexKey::Filtered(key.clone());
            if cache.filtered.as_ref().map(|c| &c.key) != Some(&fkey) {
                let tex = upload(ctx, app, "filter-preview-filtered", filtered_doc);
                cache.filtered = Some(CachedTex { key: fkey, tex, size: [filtered_doc.size.width, filtered_doc.size.height] });
            }
        }
        None => cache.filtered = None,
    }
    // Press-and-hold shows the unfiltered image, as Photoshop's preview does.
    let shown = if dialog.is_comparing() { cache.original.as_ref() } else { cache.filtered.as_ref().or(cache.original.as_ref()) };
    let picture = shown.filter(|c| c.size[0] > 0 && c.size[1] > 0).map(|c| SizedTexture::from_handle(&c.tex));
    ctx.data_mut(|d| d.insert_temp(cache_id, cache));
    Some(picture)
}

/// The top dialog, when it has the preview pane: the image under it is then the preview's locator (another dialog
/// opened over it takes the input). Not while the image is something else of the dialog's: the Blur Gallery's pins
/// (`blur_pins`) or a dropper armed on the canvas.
pub(crate) fn locator(app: &PhotocraftApp) -> Option<DialogId> {
    if app.dialog_dropper.is_some() {
        return None;
    }
    let id = app.framework.dialogs.stack().top()?;
    let key = app.framework.dialogs.get_by_id::<HostedParamDialog>(id)?.key()?;
    (shows(key) && !crate::blur_pins::handles(key)).then_some(id)
}

/// Centre the top dialog's preview pane on document point `point`, clamped to the image, as a click on the image
/// does in Photoshop's filter dialogs (#9648). Only the pane's view moves: no command runs, the document and its
/// history are untouched. `false` when no dialog with a preview pane is on top or the point is not finite.
pub(crate) fn centre_on(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(id) = locator(app) else { return false };
    if !point.iter().all(|v| v.is_finite()) {
        return false;
    }
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return false };
    let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id) else { return false };
    let Some(subject) = subject(app, dialog) else { return false };
    let k = picture_factor(app, id, dialog, &doc, &subject);
    // Clamped before narrowing: a huge finite point (1e40 from `ui.pointer`) would overflow `f32` to infinity.
    let clamped = [point[0].clamp(0.0, f64::from(doc.size.width)), point[1].clamp(0.0, f64::from(doc.size.height))];
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return false };
    dialog.centre_preview(picture_point(clamped, k));
    true
}

/// The point of a picture reduced `k` times that shows document point `document`.
fn picture_point(document: [f64; 2], k: u32) -> Pos2 {
    let k = f64::from(k.max(1));
    pos2((document[0] / k) as f32, (document[1] / k) as f32)
}

/// The document point under `picture`, a point of a picture reduced `k` times: the inverse of [`picture_point`].
#[cfg(test)]
fn document_point(picture: Pos2, k: u32) -> [f64; 2] {
    let k = f64::from(k.max(1));
    [f64::from(picture.x) * k, f64::from(picture.y) * k]
}

/// Whether the dialog of `command` gets the preview pane: Filter menu dialogs, as in Photoshop. Image › Rotation ›
/// Arbitrary and the adjustment dialogs keep only the on-canvas preview, and Select and Mask is a workspace.
pub(crate) fn shows(command: &str) -> bool {
    command.starts_with("filter.")
}

/// A composite as monitor pixels, matching the canvas (`canvas::display_image`).
fn upload(ctx: &egui::Context, app: &PhotocraftApp, name: &str, doc: &Document) -> egui::TextureHandle {
    // Reduce only the display texture when a full-resolution result exceeds the device limit.
    // The cached document and the pane's document-pixel coordinates stay at full size.
    let factor = texture_factor(doc.size.width, doc.size.height, ctx.input(|i| i.max_texture_side));
    let (width, height) = (doc.size.width.div_ceil(factor), doc.size.height.div_ceil(factor));
    if let Some(st) = app.session.active()
        && let Some(pixels) =
            crate::channel_view::render(doc, &st.channel_view, doc.bounds(), factor, crate::channel_view::Look::of(&app.session), st.active_layer)
    {
        let image = egui::ColorImage::new([width as usize, height as usize], pixels);
        return ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
    }
    let buf = if factor == 1 { photocraft_compose::flatten(doc) } else { photocraft_compose::render_reduced(doc, width, height) };
    let (display, _) = crate::canvas::canvas_display(app, doc, None);
    let img = crate::canvas::display_image(display.as_deref(), &buf);
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

/// The reduction that fits a `w`×`h` image inside a `max_side` texture (1: it already fits).
pub(crate) fn texture_factor(w: u32, h: u32, max_side: usize) -> u32 {
    let max_side = u32::try_from(max_side).unwrap_or(u32::MAX).max(1);
    w.max(h).div_ceil(max_side).max(1)
}

/// What a cached texture was built from, so it is rebuilt only when its source changes.
#[derive(Clone, PartialEq)]
enum TexKey {
    Filtered(FilterPreviewKey),
    Original(photocraft_doc::DocId, u64, u32),
}

#[derive(Clone)]
struct CachedTex {
    key: TexKey,
    tex: egui::TextureHandle,
    /// The image's size in pixels.
    size: [u32; 2],
}

/// The dialog's live textures: the filtered proxy and the unfiltered proxy.
#[derive(Clone, Default)]
struct Cache {
    filtered: Option<CachedTex>,
    original: Option<CachedTex>,
    view: Option<(photocraft_engine::channel_cmds::ChannelView, bool, Option<photocraft_doc::LayerId>, u64)>,
}

#[cfg(test)]
mod tests {
    use egui::Color32;
    use serde_json::json;

    use super::*;

    /// The id of the dialog on top of the host's stack: the one a test just opened.
    fn top_dialog(app: &PhotocraftApp) -> DialogId {
        app.framework.dialogs.stack().top().expect("a dialog is open")
    }

    /// Cancels the dialog on top.
    fn close_top(app: &mut PhotocraftApp) {
        let (ctx, id) = (app.ctx().clone(), top_dialog(app));
        assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    }

    /// One frame of the dialogs: the picture fed first, as `dialogs::run` does, then the host's pass.
    fn frame(app: &mut PhotocraftApp) -> egui::FullOutput {
        let ctx = app.ctx().clone();
        ctx.run_ui(Default::default(), |ui| {
            feed(app, ui.ctx());
            app.framework.dialogs.run(ui);
        })
    }

    #[test]
    fn full_resolution_preview_textures_respect_the_device_limit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        let limit = ctx.input(|i| i.max_texture_side) as u32;
        let width = limit + 2;
        app.run("file.new", json!({"width": width, "height": 2})).unwrap();
        for alpha in [false, true] {
            if alpha {
                app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
                app.session
                    .edit("Fixture", |doc, _| {
                        doc.channels[0].surface.fill_rect(photocraft_geom::Rect::new(0, 0, (width / 2) as i32, 2), &[1.0]);
                        Ok(())
                    })
                    .unwrap();
            }
            let doc = &app.session.active().unwrap().doc;
            let texture = upload(&ctx, &app, "full-resolution-preview", doc);
            let mut delta = ctx.tex_manager().write().take_delta();
            let egui::ImageData::Color(image) = &delta.set.get(&texture.id()).unwrap()[0].image;
            let pixels = image.pixels.clone();
            delta.clear();
            assert_eq!(texture.size(), [width.div_ceil(2) as usize, 1]);
            assert!(texture.size().into_iter().all(|side| side <= limit as usize));
            assert_eq!(doc.size.width, width, "display reduction must not resize the filter result");
            assert_eq!(pixels[0], Color32::WHITE);
            assert_eq!(*pixels.last().unwrap(), if alpha { Color32::BLACK } else { Color32::WHITE });
        }
    }

    #[test]
    fn uploaded_preview_shows_the_filtered_alpha_at_every_depth_and_proxy_scale() {
        use photocraft_engine::channel_cmds::ChannelTarget;
        for depth in [8, 16, 32] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
            app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
            app.session
                .edit("Fixture", |doc, _| {
                    doc.channels[0].surface.fill_rect(photocraft_geom::Rect::new(0, 0, 8, 16), &[1.0]);
                    Ok(())
                })
                .unwrap();
            let st = app.session.active().unwrap();
            for k in [1, 2, 4] {
                let preview = crate::filter_dialog::preview_document_for_target(
                    &st.doc,
                    st.active_layer,
                    "filter.blur.gaussianBlur",
                    &json!({"radius": 2.0}),
                    k,
                    None,
                    ChannelTarget::Alpha(0),
                    &[],
                    &Default::default(),
                    Some((app.session.tools.foreground, app.session.tools.background)),
                )
                .unwrap();
                let ctx = app.ctx().clone();
                let texture = upload(&ctx, &app, "alpha-preview", &preview);
                let mut delta = ctx.tex_manager().write().take_delta();
                let egui::ImageData::Color(image) = &delta.set.get(&texture.id()).unwrap()[0].image;
                let pixels = image.pixels.clone();
                delta.clear();
                let expected =
                    crate::channel_view::render(&preview, &st.channel_view, preview.bounds(), 1, crate::channel_view::Look::default(), st.active_layer)
                        .unwrap();
                assert_eq!(pixels, expected, "depth {depth}, factor {k}");
                assert_ne!(pixels[0], pixels[pixels.len() - 1], "the alpha edge is visible, not the white RGB layer");
                assert!(pixels.iter().any(|p| p.r() > 0 && p.r() < 255), "the blurred edge is visible");
            }
        }
    }

    #[test]
    fn original_texture_refreshes_when_the_channel_view_changes_without_an_edit() {
        use photocraft_engine::channel_cmds::ChannelTarget;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("channel.new", json!({"name": "White", "fill": "white"})).unwrap();
        app.run("channel.new", json!({"name": "Black", "fill": "black"})).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let revision = app.session.active().unwrap().revision;
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let id = top_dialog(&app);
        let ctx = app.ctx().clone();
        let cache_id = egui::Id::unique(("filter-preview-ui", id.value()));
        let original = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<Cache>(cache_id).unwrap().original.unwrap().tex.id());
        ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx())).textures_delta.clear();
        let before = original(&ctx);
        // Fed again with nothing changed: the same texture, nothing uploaded.
        ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(original(&ctx), before, "an unchanged picture is not uploaded again");
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
        assert_eq!(app.session.active().unwrap().revision, revision);
        let mut output = ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx()));
        let after = original(&ctx);
        let pixels = output.textures_delta.set.get(&after).map(|delta| {
            let egui::ImageData::Color(image) = &delta[0].image;
            image.pixels.clone()
        });
        output.textures_delta.clear();
        assert_ne!(before, after, "press-and-hold must not reuse the old channel");
        assert!(pixels.unwrap().iter().all(|p| *p == Color32::BLACK));
    }

    /// A tile that fails to read and recovers moves no revision; the unfiltered texture still draws again then, so it
    /// does not keep the zeros it read during the error.
    #[test]
    fn original_texture_refreshes_after_a_scratch_read_error_transition() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let id = top_dialog(&app);
        let ctx = app.ctx().clone();
        let cache_id = egui::Id::unique(("filter-preview-ui", id.value()));
        let original = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<Cache>(cache_id).unwrap().original.unwrap().tex.id());
        ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx())).textures_delta.clear();
        let before = original(&ctx);
        ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(original(&ctx), before, "an unchanged picture is not uploaded again");
        crate::scratch_reads::tests::simulate_read_error_transition();
        ctx.run_ui(Default::default(), |ui| feed(&mut app, ui.ctx())).textures_delta.clear();
        assert_ne!(original(&ctx), before, "the picture draws again after the transition");
    }

    #[test]
    fn only_filter_dialogs_get_the_preview_pane() {
        for (command, pane) in [
            ("filter.blur.gaussianBlur", true),
            ("filter.sharpen.smartSharpen", true),
            // Photoshop's Rotate Canvas and the adjustment dialogs have no preview pane; Select and Mask is a workspace.
            ("image.rotation.arbitrary", false),
            ("image.adjustments.brightnessContrast", false),
            ("image.adjustments.replaceColor", false),
            (crate::select_mask_view::COMMAND, false),
            ("", false),
        ] {
            assert_eq!(shows(command), pane, "{command}");
        }
    }

    #[test]
    fn textures_are_reduced_to_fit_the_gpu() {
        assert_eq!(texture_factor(3000, 2000, 2048), 2);
        assert_eq!(texture_factor(2048, 100, 2048), 1);
        assert_eq!(texture_factor(9000, 9000, 2048), 5);
        // A zero or absurd limit never divides by zero.
        assert_eq!(texture_factor(3000, 2000, 0), 3000);
        assert_eq!(texture_factor(0, 0, 2048), 1);
    }

    #[test]
    fn a_click_maps_to_the_document_point_under_it() {
        // The pane reports a click as a point of the picture; the picture is the document reduced `k` times.
        for (picture, k, document) in
            [(pos2(10.5, 40.0), 1, [10.5, 40.0]), (pos2(10.5, 40.0), 4, [42.0, 160.0]), (pos2(0.0, 0.0), 4, [0.0, 0.0]), (pos2(7.0, 3.0), 0, [7.0, 3.0])]
        {
            assert_eq!(document_point(picture, k), document, "k {k}");
            assert_eq!(picture_point(document, k), picture, "k {k}: centring is the inverse");
        }
    }

    #[test]
    fn a_point_on_the_image_centres_the_top_preview_pane() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 4000, "height": 3000})).unwrap();
        // No dialog: nothing to centre.
        assert!(!centre_on(&mut app, [10.0, 10.0]));
        // A dialog without a preview pane takes no centring.
        crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
        assert_eq!(locator(&app), None, "Rotate Canvas has no preview pane");
        close_top(&mut app);
        // Every filter dialog with the pane shares the locator, Unsharp Mask and Reduce Noise too.
        for command in ["filter.sharpen.unsharpMask", "filter.noise.reduceNoise", "filter.sharpen.smartSharpen"] {
            crate::filter_dialog::open(&mut app, command).unwrap();
            assert_eq!(locator(&app), Some(top_dialog(&app)), "{command}");
            close_top(&mut app);
        }
        crate::filter_dialog::open(&mut app, "filter.sharpen.unsharpMask").unwrap();
        let id = top_dialog(&app);
        let (revision, history) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
        let params =
            crate::filter_dialog::command_params("filter.sharpen.unsharpMask", app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields());
        let k = crate::canvas::filter_preview_factor(&app, &app.session.active().unwrap().doc, "filter.sharpen.unsharpMask", &params);
        // The pane shows its picture from the second pass on.
        frame(&mut app).textures_delta.clear();
        let centre = |app: &PhotocraftApp| app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().preview_centre();
        let near = |at: Option<Pos2>, document: [f64; 2]| {
            let want = picture_point(document, k);
            at.is_some_and(|at| (at.x - want.x).abs() < 1.0 && (at.y - want.y).abs() < 1.0)
        };
        assert!(centre_on(&mut app, [1200.0, 900.0]));
        frame(&mut app).textures_delta.clear();
        assert!(near(centre(&app), [1200.0, 900.0]), "{:?}", centre(&app));
        // Off the image it clamps to the nearest edge, and the pane then holds a point of the picture.
        assert!(centre_on(&mut app, [1e300, -1e300]));
        frame(&mut app).textures_delta.clear();
        let at = centre(&app).expect("the pane has a centre");
        assert!(at.x > 0.0 && at.y >= 0.0, "a huge point clamps to the far edge, not to 0: {at:?}");
        // A non-finite point changes nothing.
        let held = centre(&app);
        assert!(!centre_on(&mut app, [f64::NAN, 5.0]));
        assert!(!centre_on(&mut app, [5.0, f64::INFINITY]));
        frame(&mut app).textures_delta.clear();
        assert_eq!(centre(&app), held);
        let st = app.session.active().unwrap();
        assert_eq!((st.revision, st.history.past_len()), (revision, history), "the document and its history are untouched");
        // A dialog opened over it takes the press.
        crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
        assert_eq!(locator(&app), None);
        assert!(!centre_on(&mut app, [20.0, 20.0]));
    }
}
