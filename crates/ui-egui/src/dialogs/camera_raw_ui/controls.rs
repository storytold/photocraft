use super::edit_panel::{self, FileAction};
use super::open::{Target, cancel, raw_stage};
use super::sidecar::{load_settings, report_file_error, save_settings};
use super::{Align2, CameraRawDialog, Color32, ERect, FontId, PANEL_W, PhotocraftApp, Sense, Stroke, Tokens, Value, json, pos2, vec2, widgets};

const RAW_UNDO_LIMIT: usize = 40;
/// Width of the tool strip at the dialog's right edge.
const TOOL_STRIP_W: f32 = 44.0;

pub(super) fn remember_edit(d: &mut CameraRawDialog) {
    if d.undo.len() == RAW_UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.undo.push(d.params.clone());
    d.redo.clear();
}

/// Whether any of the four point curves (master, red, green, blue) differs: replacing the one
/// the curve editor shows invalidates its selected or dragged point.
pub(super) fn curves_changed(a: &super::CameraRaw, b: &super::CameraRaw) -> bool {
    a.point_curve != b.point_curve
        || a.point_curve_red != b.point_curve_red
        || a.point_curve_green != b.point_curve_green
        || a.point_curve_blue != b.point_curve_blue
}

pub(super) fn undo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(prev) = d.undo.pop() else { return false };
    d.redo.push(d.params.clone());
    if curves_changed(&d.params, &prev) {
        d.curve_state = Default::default();
    }
    d.params = prev;
    d.dirty = true;
    d.edit_origin = None;
    true
}

pub(super) fn redo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(next) = d.redo.pop() else { return false };
    d.undo.push(d.params.clone());
    if curves_changed(&d.params, &next) {
        d.curve_state = Default::default();
    }
    d.params = next;
    d.dirty = true;
    d.edit_origin = None;
    true
}

/// The settings before this frame's keyboard input, when a key (an arrow on a focused slider, a
/// typed value) may edit without the pointer: [`commit_key_edit`] makes that edit one undo step of
/// its own. Pointer edits have their own origin (one drag, one step).
fn key_edit_origin(d: &CameraRawDialog, ctx: &egui::Context, pointer: bool) -> Option<super::CameraRaw> {
    let typed = !pointer
        && d.edit_origin.is_none()
        && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) | egui::Event::Paste(_))));
    typed.then(|| d.params.clone())
}

/// Records `origin` (from [`key_edit_origin`]) in the dialog's history when the keys changed it.
fn commit_key_edit(d: &mut CameraRawDialog, origin: Option<super::CameraRaw>) {
    if let Some(origin) = origin
        && origin != d.params
    {
        let edited = std::mem::replace(&mut d.params, origin);
        remember_edit(d);
        d.params = edited;
    }
}

fn history_keys(d: &mut CameraRawDialog, ctx: &egui::Context) {
    let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
        redo_edit(d);
    } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        undo_edit(d);
    }
}

pub(super) fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("RAW Develop isn't open"))?;
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            // Panels hidden by their eye button apply nothing, as in Camera Raw.
            let effective = d.effective_params();
            effective.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&effective).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
        Target::OpenRaw => {
            let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
            let (params, original, panels) = (d.effective_params(), d.original.clone(), d.panels.clone());
            super::filmstrip::open_all(app, &params, &raw, &original, &panels)?
        }
    };
    app.camera_raw = None;
    Ok(result)
}

/// The open-time demosaic choice (at the top of Detail). Photoshop's Camera Raw picks one method
/// for every file; here it is the user's. Open re-develops with it; the preview keeps the method
/// the file was opened with (the differences are finer than the preview's pixels).
pub(super) fn demosaic_row(ui: &mut egui::Ui, m: &mut photocraft_io::raw::Demosaic) {
    use photocraft_io::raw::Demosaic;
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", "Demosaic")).on_hover_text(tl!(
            "How the camera's color filter mosaic is filled in. It is applied when the file opens; the preview shows the method it was opened with."
        ));
        let options = [
            (Demosaic::Ahd, "AHD (adaptive homogeneity)", "Hirakawa–Parks: the cleanest edges and the least color fringing, the slowest."),
            (Demosaic::Mhc, "Malvar–He–Cutler", "Gradient-corrected interpolation: sharp and fast, with some color fringing on fine detail."),
            (Demosaic::Bilinear, "Bilinear", "The fastest and the softest, with color fringing on fine detail."),
        ];
        widgets::dropdown_with_tooltips(ui, "camera-raw-demosaic", tl!("Demosaic"), m, &options, 180.0);
    });
    ui.add_space(4.0);
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.camera_raw.is_none()
        && !app.raw_shelf.is_empty()
        && let Err(e) = super::filmstrip::open_held(app, ctx, 0)
    {
        // The document stays open, developed with the defaults.
        app.raw_shelf.remove(0);
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let Some(d) = app.camera_raw.as_ref() else { return };
    if draws_over_main_window(ctx, app.services.is_wayland) {
        draw(app, ctx, false);
        return;
    }
    let window_title = d.window_title();
    // Camera Raw is modal, as in Photoshop: the main window is dimmed and takes no input while it
    // is open (in a separate window, or drawn over it: see `draws_over_main_window`).
    let main = ctx.content_rect();
    let scrim = egui::Area::new(egui::Id::unique("camera-raw-modal")).order(egui::Order::Foreground).fixed_pos(main.min).show(ctx, |ui| {
        let (r, response) = ui.allocate_exact_size(main.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, 0.0, Tokens::get(ui.ctx()).scrim);
        response
    });
    let window = egui::ViewportId::from_hash_of("camera-raw-window");
    // Coming back to PhotoCraft (⌘Tab, the Dock) can bring the main window over Camera Raw,
    // leaving only the dimmed window that takes no input: the main window gaining focus, or a
    // click on it, brings Camera Raw back to the front.
    let main_focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    if let Some(d) = app.camera_raw.as_mut() {
        if raise_camera_raw(d.main_focused, main_focused, scrim.inner.clicked() || scrim.inner.drag_started()) {
            for command in raise_commands() {
                ctx.send_viewport_cmd_to(window, command);
            }
        }
        d.main_focused = main_focused;
    }
    // A window of its own (title bar, moved and resized like any window), like Adobe Camera Raw.
    let builder = crate::os_window::builder().with_title(window_title).with_inner_size([1360.0, 900.0]).with_min_inner_size([900.0, 600.0]);
    let mut close = false;
    ctx.show_viewport_immediate(window, builder, |ui, class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            close = true;
        }
        draw(app, ui.ctx(), class != egui::ViewportClass::EmbeddedWindow);
    });
    if close && app.camera_raw.is_some() {
        cancel(app);
    }
}

/// Whether the dialog is drawn over the main window instead of in a window of its own: where the
/// platform has no extra windows (the web build), while the main window is full screen, and on
/// Wayland while it is maximized.
///
/// The dialog's window is an immediate viewport, so it takes input and redraws only while the
/// main window draws a frame. A window opened over a full-screen or maximized one can cover it
/// (Hyprland hands the new window the full screen or the maximized state), the compositor then
/// sends the hidden main window no frame callbacks, and the dialog freezes: its sliders don't
/// move and nothing else responds. Elsewhere a maximized main window keeps Camera Raw in its own
/// window, as in Photoshop.
fn draws_over_main_window(ctx: &egui::Context, wayland: bool) -> bool {
    let (fullscreen, maximized) = ctx.input(|i| (i.viewport().fullscreen == Some(true), i.viewport().maximized == Some(true)));
    ctx.embed_viewports() || fullscreen || (wayland && maximized)
}

/// Whether to bring Camera Raw's window to the front: when the main window has just gained focus
/// (it came forward over Camera Raw's) or is clicked.
fn raise_camera_raw(was_focused: bool, focused: bool, clicked: bool) -> bool {
    (focused && !was_focused) || clicked
}

/// Brings Camera Raw's window to the front, restoring it first if it was minimized (a minimized
/// window ignores Focus).
fn raise_commands() -> [egui::ViewportCommand; 2] {
    [egui::ViewportCommand::Minimized(false), egui::ViewportCommand::Focus]
}

impl CameraRawDialog {
    /// "RAW Develop (name)" for the open-time dialog, "RAW Develop Filter (layer)" otherwise.
    fn window_title(&self) -> String {
        if self.target == Target::OpenRaw {
            crate::i18n::fmt(tl!("RAW Develop ({name})"), &[("name", &self.layer_name)])
        } else {
            crate::i18n::fmt(tl!("RAW Develop Filter ({layer})"), &[("layer", &self.layer_name)])
        }
    }
}

/// The dialog's contents over the whole of `ctx`'s window. `own_window`: the OS window already
/// shows the title, so none is painted.
/// The filmstrip column at the left of `body` when several raws are being opened: its width
/// (0 when none shows) and the raw the user clicked, if any.
fn filmstrip_column(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    body: ERect,
    t: &Tokens,
    opening: bool,
    shelf: &mut [super::filmstrip::HeldRaw],
    current: Option<photocraft_doc::DocId>,
) -> (f32, Option<usize>) {
    if !opening || shelf.len() <= 1 {
        return (0.0, None);
    }
    let film_w = super::filmstrip::FILM_W;
    let film = ERect::from_min_max(body.min, pos2(body.left() + film_w - 1.0, body.bottom()));
    painter.rect_filled(film, 0.0, t.dock);
    painter.line_segment([film.right_top(), film.right_bottom()], Stroke::new(1.0, t.separator));
    (film_w, super::filmstrip::filmstrip(ui, film, shelf, current, t))
}

fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, own_window: bool) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let current_display = app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok());
    let mut action: Option<&str> = None;
    let mut show_held: Option<usize> = None;
    egui::Area::new(egui::Id::unique("camera-raw-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.camera_raw.as_mut() else { return };
        history_keys(d, ctx);
        let pointer = ctx.input(|i| i.pointer.primary_down());
        if pointer && d.edit_origin.is_none() {
            d.edit_origin = Some(d.params.clone());
        }
        let key_origin = key_edit_origin(d, ctx, pointer);
        if let Some(display) = &current_display
            && display.key != d.display.key
        {
            d.display = display.clone();
            d.dirty = true;
        }
        d.curve_rect = None;
        d.scope.vectorscope_rect = None;
        if let Err(e) = d.poll_render(ctx) {
            d.preview_error = Some((e.clone(), d.preview_revision));
            app.ui.status = e;
            app.ui.status_error = true;
        }
        d.request_render(ctx);
        d.clear_recovered_error(&mut app.ui);
        let (full, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        let title = ERect::from_min_size(full.min, vec2(full.width(), if own_window { 0.0 } else { 30.0 }));
        if !own_window {
            painter.rect_filled(title, 0.0, t.dock);
            painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
            painter.text(title.center(), Align2::CENTER_CENTER, d.window_title(), FontId::proportional(13.0), t.text);
        }
        let ok_label = if d.target == Target::OpenRaw { tl!("Open") } else { tl!("OK") };
        let footer_h = 44.0;
        let body = ERect::from_min_max(pos2(full.left(), title.bottom()), pos2(full.right(), full.bottom() - footer_h));
        let animated_zoom = app.session.prefs().general.animated_zoom;
        let scope = &mut app.ui.camera_raw_scope;
        // Camera Raw's columns: the preview, the Edit panels, the tool strip at the far right.
        let strip = ERect::from_min_max(pos2((body.right() - TOOL_STRIP_W).max(body.left()), body.top()), body.max);
        let right = ERect::from_min_max(pos2((strip.left() - PANEL_W).max(body.left()), body.top()), pos2(strip.left(), body.bottom()));
        // Clip both pixels and scope overlays before painting the settings dock/footer.
        // Several raws being opened: the filmstrip on the left.
        let current = d.raw_open.as_ref().map(|r| r.document);
        let (film_w, held) = filmstrip_column(ui, &painter, body, &t, d.target == Target::OpenRaw, &mut app.raw_shelf, current);
        show_held = held;
        let preview_body = ERect::from_min_max(pos2(body.left() + film_w, body.top()), pos2(right.left().max(body.left() + film_w + 1.0), body.bottom()));
        let view = ERect::from_min_max(
            preview_body.min + vec2(12.0, 12.0),
            pos2((preview_body.right() - 12.0).max(preview_body.left() + 13.0), (preview_body.bottom() - 40.0).max(preview_body.top() + 13.0)),
        );
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(view));
        preview.set_clip_rect(view.intersect(ui.clip_rect()));
        preview.painter().rect_filled(view, 0.0, t.canvas);
        // A mask brush, gradient or Upright guide takes the primary drag, as the Color Sampler
        // does, so the zoom tool must not claim it.
        // Guides follow the geometry the preview shows (none with Geometry hidden or Before shown).
        let drawing = super::masks_panel::captures_pointer(ctx, &d.effective_params(), d.show_before);
        // The White Balance tool and the Color Sampler are one at a time, as in Camera Raw.
        let wb_tool = d.panels.wb_tool && !drawing;
        if wb_tool {
            scope.sampler_tool = false;
        }
        let mut sampling = scope.sampler_tool || drawing || wb_tool;
        // Before / After side by side: Before at the left, the After the tools act on at the right,
        // both zoomed and panned together; Split shows Before over the left half of one image.
        use super::panels::Compare;
        let compare = if d.show_before { Compare::Single } else { d.panels.compare };
        let half_w = ((view.width() - 4.0) / 2.0).max(1.0);
        let after_view = if compare == Compare::SideBySide { ERect::from_min_max(pos2(view.right() - half_w, view.top()), view.max) } else { view };
        if compare == Compare::SideBySide {
            preview.set_clip_rect(after_view.intersect(ui.clip_rect()));
        }
        // The rectangle the image is fitted to (the After half side by side): `zoom` reads it.
        d.viewport = Some(after_view);
        let (r, response, panning, zoom_box) = crate::camera_raw_preview_ui::interact(&mut preview, navigation, after_view, size, &mut sampling, animated_zoom);
        if !drawing && !wb_tool {
            scope.sampler_tool = sampling;
        }
        let tex = if d.show_before { d.before_tex.as_ref() } else { d.tex.as_ref() };
        if let Some(tex) = tex {
            // Never tile a checker across the offscreen extent of a highly zoomed image.
            widgets::checker(preview.painter(), r.intersect(view), 8.0);
            preview.painter().image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let effective = d.effective_params();
            let raw_tone = d.raw_curve.is_some() && !raw_stage(&effective).is_neutral();
            let detail_needed = !raw_tone && (r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32);
            d.detail.paint(&preview, r, d.preview_revision, d.show_before, &effective, detail_needed, &d.display);
            // Masks: the overlay, pins and Guided Upright's guides over the image (and over the
            // full-resolution detail), under Split's Before half; their drags edit the masks.
            // Edits start only on the After image: Split's Before half covers the left of it.
            super::masks_panel::preview(&mut preview, d, r, &response, panning, compare.after_area(view, after_view));
            if wb_tool && !panning {
                super::wb_picker::preview(&preview, d, r, &response);
            }
            if let Some(before) = d.before_tex.as_ref() {
                paint_before(ui, &t, before, compare, view, after_view, r);
            }
            if d.detail.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            d.scope.preview_rect = Some(r);
            // Split's line (the view's centre) in image coordinates: readouts left of it read Before.
            d.scope.split = (compare == Compare::Split).then(|| (view.center().x - r.left()) / r.width().max(1e-3)).filter(|s| s.is_finite());
            crate::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
        }
        if let Some(region) = zoom_box {
            preview.painter().rect_stroke(region, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        // Under the preview: the zoom menu at the left, Before / After at the right.
        let toolbar = ERect::from_min_max(pos2(preview_body.left() + 12.0, view.bottom() + 6.0), preview_body.right_bottom() - vec2(12.0, 4.0));
        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(toolbar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        nav_ui.set_clip_rect(preview_body.intersect(ui.clip_rect()));
        crate::camera_raw_preview_ui::toolbar(&mut nav_ui, navigation, after_view, size);
        if d.detail.pending() {
            nav_ui.spinner();
        }
        if let Some(error) = &d.detail.error {
            nav_ui.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(t.text_faint)).on_hover_text(error);
        }
        nav_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Camera Raw's Before/After views button: After alone, side by side, split.
            let compare = d.panels.compare;
            let (rect, cycle) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
            let ink = if cycle.hovered() || compare != Compare::Single { t.text } else { t.text_dim };
            let icon = ERect::from_center_size(rect.center(), vec2(16.0, 12.0));
            ui.painter().rect_stroke(icon, 1.0, Stroke::new(1.0, ink), egui::StrokeKind::Inside);
            match compare {
                Compare::Single => {}
                Compare::SideBySide => {
                    ui.painter().line_segment([icon.center_top(), icon.center_bottom()], Stroke::new(1.0, ink));
                }
                Compare::Split => {
                    ui.painter().rect_filled(ERect::from_min_max(icon.min, pos2(icon.center().x, icon.bottom())), 0.0, ink);
                }
            }
            let name = tl!("Before/After Views");
            cycle.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            if cycle.on_hover_text(name).clicked() {
                d.panels.compare = compare.next();
                d.show_before = false;
            }
            let before = widgets::pill_tab(ui, tl!("Before (Y)"), d.show_before);
            before.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, d.show_before, tl!("Before (Y)")));
            if before.on_hover_text(tl!("Show the image without these settings")).clicked() {
                d.show_before = !d.show_before;
            }
            ui.label(egui::RichText::new(format!("{:.0} ms", d.render_ms)).color(t.text_faint));
        });
        // The tool strip.
        painter.rect_filled(strip, 0.0, t.dock);
        painter.line_segment([strip.left_top(), strip.left_bottom()], Stroke::new(1.0, t.separator));
        let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(6.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Center)));
        let masking = super::masks_panel::is_open(ctx);
        if crate::icons::button(&mut strip_ui, "sliders-horizontal", 30.0, !masking, tl!("Edit")).clicked() && masking {
            let mut s = super::masks_panel::load(ctx);
            s.open = false;
            super::masks_panel::store(ctx, s);
        }
        super::masks_panel::strip_button(&mut strip_ui);
        // Picking the Color Sampler (its button or S) turns the White Balance tool off; the
        // other way round happens above, when the White Balance tool is on.
        let sampler_was = scope.sampler_tool;
        strip_ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            crate::camera_raw_preview_ui::tools(ui, navigation, &mut scope.sampler_tool);
        });
        // The Edit panels.
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(18.0, 10.0))));
        // This header is outside the settings scroll area and follows the same Before/After
        // selector as the texture. Both derive from the same completed preview revision.
        crate::camera_raw_scope_ui::header(&mut props, d, scope);
        d.scope.alt_tone = None;
        let mut dirty = false;
        if super::masks_panel::is_open(ctx) {
            // The Masking tool's column has its own title, without Edit's Auto and menu.
            props.label(egui::RichText::new(tl!("Masking")).font(crate::theme::semibold(12.5)).color(t.text));
            props.add_space(4.0);
        } else {
            match edit_panel::header(&mut props, d, &mut dirty) {
                Some(FileAction::Load) => action = Some("load"),
                Some(FileAction::Save) => action = Some("save"),
                None => {}
            }
        }
        egui::ScrollArea::vertical().id_salt("camera-raw-props").show(&mut props, |ui| {
            if super::masks_panel::is_open(ctx) { super::masks_panel::panel(ui, d, &mut dirty) } else { edit_panel::panels(ui, d, &mut dirty) }
        });
        crate::camera_raw_scope_ui::floating(ctx, d, scope, right);
        crate::camera_raw_scope_ui::shortcuts(ctx, scope);
        if scope.sampler_tool && !sampler_was {
            d.panels.wb_tool = false;
        }
        d.dirty |= dirty;
        if d.dirty {
            ctx.request_repaint();
        }
        if d.render_pending() {
            // The worker asks for a repaint when it is done; this catches a missed wake-up.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Footer: Cancel and OK at the bottom right.
        let foot = ERect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);
        painter.rect_filled(foot, 0.0, t.dock);
        painter.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 8.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if let Some(role) = widgets::dialog_buttons(
            &mut fu,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, ok_label, 90.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 90.0),
            ],
        ) {
            action = Some(if role == widgets::ButtonRole::Default { "ok" } else { "cancel" });
        }
        // Done (open-time dialog): keep the settings in the raws' sidecars, open nothing.
        if d.target == Target::OpenRaw
            && fu
                .add_sized([90.0, 28.0], egui::Button::new(tl!("Done")))
                .on_hover_text(tl!("Save the settings to each raw's sidecar without opening it"))
                .clicked()
        {
            action = Some("done");
        }
        if !ctx.input(|i| i.pointer.primary_down())
            && let Some(origin) = d.edit_origin.take()
            && origin != d.params
        {
            let edited = std::mem::replace(&mut d.params, origin);
            remember_edit(d);
            d.params = edited;
        }
        commit_key_edit(d, key_origin);
    });
    crate::camera_raw_scope_ui::persist(app, ctx);
    if let Some(i) = show_held
        && let Err(e) = super::filmstrip::show_raw(app, ctx, i)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Y))
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.show_before = !d.show_before;
    }
    // I: the White Balance tool, as in Camera Raw (not while typing in a field).
    if !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::I))
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.panels.wb_tool = !d.panels.wb_tool;
        if d.panels.wb_tool {
            app.ui.camera_raw_scope.sampler_tool = false;
        }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some("cancel");
    }
    match action {
        Some("ok") => {
            if let Err(e) = commit(app) {
                app.ui.status = e;
            }
        }
        Some("cancel") => cancel(app),
        Some("done") => {
            super::filmstrip::done(app);
        }
        Some("save") => {
            let r = save_settings(app);
            report_file_error(app, r);
        }
        Some("load") => {
            let r = load_settings(app);
            report_file_error(app, r);
        }
        _ => {}
    }
}

/// The Before image beside (side by side) or over the left half of (split) the After image `r`,
/// with the Before / After labels and the dividing line. Nothing for the single view.
fn paint_before(ui: &egui::Ui, t: &Tokens, before: &egui::TextureHandle, compare: super::panels::Compare, view: ERect, after_view: ERect, r: ERect) {
    use super::panels::Compare;
    let (left, at) = match compare {
        Compare::Single => return,
        Compare::SideBySide => {
            (ERect::from_min_max(view.min, pos2(after_view.left() - 4.0, view.bottom())), r.translate(vec2(-(after_view.left() - view.left()), 0.0)))
        }
        Compare::Split => (ERect::from_min_max(view.min, pos2(view.center().x, view.bottom())), r),
    };
    let painter = ui.painter().with_clip_rect(left.intersect(ui.clip_rect()));
    painter.rect_filled(left, 0.0, t.canvas);
    widgets::checker(&painter, at.intersect(left), 8.0);
    painter.image(before.id(), at, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    let edge = if compare == Compare::Split { left.right() } else { after_view.left() - 2.0 };
    ui.painter()
        .with_clip_rect(view.intersect(ui.clip_rect()))
        .line_segment([pos2(edge, view.top()), pos2(edge, view.bottom())], Stroke::new(1.0, t.text_faint));
    for (text, x, align) in [(tl!("Before"), left.left() + 8.0, Align2::LEFT_TOP), (tl!("After"), view.right() - 8.0, Align2::RIGHT_TOP)] {
        let label = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.0), t.text);
        let at = align.anchor_size(pos2(x, view.top() + 8.0), label.size());
        ui.painter().rect_filled(at.expand2(vec2(6.0, 3.0)), t.radius_sm, Color32::from_black_alpha(150));
        ui.painter().galley(at.min, label, t.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coming back to PhotoCraft can put the main window over Camera Raw's: the main window
    /// gaining focus brings Camera Raw's window to the front, once per activation, and so does a
    /// click on the dimmed main window.
    #[test]
    fn the_main_window_gaining_focus_brings_camera_raw_back_to_the_front() {
        assert!(raise_camera_raw(false, true, false), "the main window came forward");
        assert!(!raise_camera_raw(true, true, false), "only once, so Camera Raw's own focus isn't fought");
        assert!(!raise_camera_raw(true, false, false) && !raise_camera_raw(false, false, false));
        assert!(raise_camera_raw(true, true, true), "a click on the dimmed main window");
    }

    /// Focus does nothing for a minimized window, which would leave only the dimmed main window
    /// that takes no input: Camera Raw's is restored first.
    #[test]
    fn raising_camera_raw_restores_it_when_minimized() {
        assert_eq!(raise_commands(), [egui::ViewportCommand::Minimized(false), egui::ViewportCommand::Focus]);
    }
}
