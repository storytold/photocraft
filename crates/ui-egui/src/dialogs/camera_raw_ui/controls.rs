use super::open::{Target, cancel, raw_stage};
use super::sidecar::{load_settings, report_file_error, save_settings};
use super::{
    Align2, BANDS, CameraRaw, CameraRawDialog, Color32, ERect, FontId, PANEL_W, PhotocraftApp, Pos2, Sense, Stroke, Tokens, ToneZone, Value, Wheel, curve_lut,
    json, pos2, vec2, widgets,
};

const RAW_UNDO_LIMIT: usize = 40;

/// The dialog's settings at one moment: the Camera Raw settings and, for a raw the shared engine
/// develops, the develop settings they mirror (an undo step must bring back both).
pub(super) type Snapshot = (CameraRaw, Option<photocraft_io::develop_bridge::DevelopEdit>);

pub(super) fn snapshot(d: &CameraRawDialog) -> Snapshot {
    (d.params.clone(), d.develop.clone())
}

fn restore(d: &mut CameraRawDialog, s: Snapshot) {
    (d.params, d.develop) = s;
}

/// Undo or redo to `s`: a curve that changes loses its selected point.
fn step_to(d: &mut CameraRawDialog, s: Snapshot) {
    let curve = |s: &Snapshot| (s.0.point_curve.clone(), s.1.as_ref().map(|e| e.settings.curve.clone()));
    if curve(&snapshot(d)) != curve(&s) {
        d.curve_state = Default::default();
        d.develop_panel.curve_state = Default::default();
    }
    restore(d, s);
}

pub(super) fn remember_edit(d: &mut CameraRawDialog) {
    if d.undo.len() == RAW_UNDO_LIMIT {
        d.undo.remove(0);
    }
    d.undo.push(snapshot(d));
    d.redo.clear();
}

pub(super) fn undo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(prev) = d.undo.pop() else { return false };
    d.redo.push(snapshot(d));
    step_to(d, prev);
    d.dirty = true;
    d.edit_origin = None;
    true
}

pub(super) fn redo_edit(d: &mut CameraRawDialog) -> bool {
    let Some(next) = d.redo.pop() else { return false };
    d.undo.push(snapshot(d));
    step_to(d, next);
    d.dirty = true;
    d.edit_origin = None;
    true
}

pub(super) fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    if let Some(d) = app.camera_raw.as_mut() {
        d.sync_develop();
    }
    let d = app.camera_raw.as_ref().ok_or(tl!("RAW Develop isn't open"))?;
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            d.params.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&d.params).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
        Target::OpenRaw => {
            let raw = d.raw_open.clone().ok_or("no raw file is being opened")?;
            let (params, original) = (d.params.clone(), d.original.clone());
            let develop = d.develop.as_ref().filter(|e| e.edited(&e.settings)).map(|e| e.settings.clone());
            super::filmstrip::open_all(app, &params, develop.as_ref(), &raw, &original)?
        }
    };
    app.camera_raw = None;
    Ok(result)
}

/// The open-time demosaic choice (at the top of Detail). Photoshop's Camera Raw picks one method
/// for every file; here it is the user's. Open re-develops with it; the preview keeps the method
/// the file was opened with (the differences are finer than the preview's pixels).
fn demosaic_row(ui: &mut egui::Ui, m: &mut photocraft_io::raw::Demosaic) {
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

pub(super) fn temp_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(200, 200, 200), Color32::from_rgb(235, 200, 60)]
}

pub(super) fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 190, 80), Color32::from_rgb(200, 200, 200), Color32::from_rgb(210, 80, 200)]
}

/// One labelled slider; marks the dialog dirty when it moves.
/// The track of Color Mixer band `k` on `tab` (0 Hue, 1 Saturation, 2 Luminance). A Hue slider
/// turns its band towards the neighbouring bands, so its track runs from the previous band's hue
/// through its own to the next one's (Reds: magenta, red, orange; #9550); Saturation and Luminance
/// run from grey to the band's colour.
pub(super) fn mixer_gradient(tab: usize, k: usize) -> Vec<Color32> {
    let bands = &photocraft_algo::camera_raw::HSL_BANDS;
    let hue = |i: usize| widgets::hue_color(bands.get(i % bands.len()).copied().unwrap_or(0.0));
    if tab == 0 { vec![hue(k + bands.len() - 1), hue(k), hue(k + 1)] } else { vec![Color32::from_gray(128), hue(k)] }
}

fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) {
    let _ = slider(ui, dirty, label, v, range, grad);
}

fn slider(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) -> egui::Response {
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", label), v, range, "", grad);
    if r.changed() {
        *dirty = true;
    }
    if r.double_clicked() {
        *v = 0.0;
        *dirty = true;
    }
    r
}

/// A Light tone slider; Alt-dragging it shows that zone's channel clipping on the preview.
fn tone_row(ui: &mut egui::Ui, dirty: &mut bool, alt: &mut Option<ToneZone>, zone: ToneZone, v: &mut f32) {
    let r = slider(ui, dirty, zone.name(), v, -zone.limit()..=zone.limit(), None);
    if r.dragged() && ui.input(|i| i.modifiers.alt) {
        *alt = Some(zone);
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title)).font(FontId::proportional(13.0)))
        .id_salt(("camera-raw-section", title))
        .default_open(open)
        .show(ui, body);
}

/// Geometry: Upright (Off, Auto, Level, Vertical, Full, Guided) and the Manual Transformations.
/// `sampler`: the Color Sampler tool, which the Guided button puts down (the newest tool wins).
fn geometry(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool, guides: &mut crate::camera_raw_guides_ui::GuideTool, sampler: &mut bool) {
    // Shift+T opens the section. Through the header itself: its id is not
    // `make_persistent_id` of the same salt, so a state stored under that never reached it.
    let reveal = std::mem::take(&mut guides.reveal);
    let response =
        egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", "Geometry")).font(FontId::proportional(13.0)))
            .id_salt(("camera-raw-section", "Geometry"))
            .default_open(false)
            .open(reveal.then_some(true))
            .show(ui, |ui| {
                widgets::section_label(ui, tl!("Upright"));
                const MODES: [(&str, &str, &str); 6] = [
                    ("off", "Off", "Disable Upright"),
                    ("auto", "Auto", "Auto: apply balanced perspective corrections"),
                    ("level", "Level", "Level: apply only level correction"),
                    ("vertical", "Vertical", "Vertical: apply level and vertical perspective correction"),
                    ("full", "Full", "Full: apply level, horizontal, and vertical perspective corrections"),
                    ("guided", "Guided", "Guided: draw two or more guides to customize perspective corrections"),
                ];
                ui.horizontal_wrapped(|ui| {
                    for (mode, name, tip) in MODES {
                        let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", name);
                        let on = p.upright == mode;
                        let response = widgets::pill_tab(ui, name, on).on_hover_text(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", tip));
                        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, name));
                        if response.clicked() {
                            if p.upright != mode {
                                p.upright = mode.into();
                                // Guides belong to Guided; another mode starts without them.
                                p.upright_guides.clear();
                                *dirty = true;
                            }
                            guides.active = mode == "guided";
                            if guides.active {
                                *sampler = false;
                            }
                        }
                    }
                });
                if p.upright == "guided" {
                    let n = p.upright_guides.len();
                    let text = if n < 2 {
                        tl!("Draw two or more guides on the image along lines that should be vertical or horizontal (Shift+T).")
                    } else {
                        tl!("Drag a guide's end to move it; select a guide and press Delete to remove it. Up to four guides.")
                    };
                    ui.label(egui::RichText::new(text).small().weak());
                    if n > 0 && ui.button(tl!("Clear Guides")).clicked() {
                        p.upright_guides.clear();
                        guides.selected = None;
                        *dirty = true;
                    }
                }
                widgets::section_label(ui, tl!("Manual Transformations"));
                row(ui, dirty, "Vertical", &mut p.transform_vertical, -100.0..=100.0, None);
                row(ui, dirty, "Horizontal", &mut p.transform_horizontal, -100.0..=100.0, None);
                row(ui, dirty, "Rotate", &mut p.transform_rotate, -10.0..=10.0, None);
                row(ui, dirty, "Aspect", &mut p.transform_aspect, -100.0..=100.0, None);
                let scale = slider(ui, dirty, "Scale", &mut p.transform_scale, 50.0..=150.0, None);
                if scale.double_clicked() {
                    // Scale's neutral is 100 %, not the 0 every other slider resets to.
                    p.transform_scale = 100.0;
                }
                row(ui, dirty, "Offset X", &mut p.transform_offset_x, -100.0..=100.0, None);
                row(ui, dirty, "Offset Y", &mut p.transform_offset_y, -100.0..=100.0, None);
            });
    if reveal {
        response.header_response.scroll_to_me(Some(egui::Align::TOP));
    }
}

/// The pointer is up: the edit since its gesture began (or, without one, since `frame_start`) is
/// one undo step.
fn record_edit(d: &mut CameraRawDialog, frame_start: Snapshot) {
    d.sync_develop();
    let origin = d.edit_origin.take().unwrap_or(frame_start);
    if origin != snapshot(d) {
        let edited = snapshot(d);
        restore(d, origin);
        remember_edit(d);
        restore(d, edited);
    }
}

/// A shortcut changed the settings from `was`: its own undo step. A gesture the pointer is still
/// holding ends here with a step of its own, so its release doesn't record `was` a second time.
pub(super) fn remember_shortcut_edit(d: &mut CameraRawDialog, was: Snapshot) {
    let edited = snapshot(d);
    restore(d, was);
    if let Some(origin) = d.edit_origin.take()
        && origin != snapshot(d)
    {
        let was = snapshot(d);
        restore(d, origin);
        remember_edit(d);
        restore(d, was);
    }
    remember_edit(d);
    restore(d, edited);
    d.dirty = true;
}

fn wheel(ui: &mut egui::Ui, dirty: &mut bool, title: &str, w: &mut Wheel) {
    widgets::section_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title));
    let hs = widgets::hue_stops();
    row(ui, dirty, "Hue", &mut w.hue, 0.0..=360.0, Some(&hs));
    row(ui, dirty, "Saturation", &mut w.sat, 0.0..=100.0, None);
    row(ui, dirty, "Luminance", &mut w.lum, -100.0..=100.0, None);
}

/// Small point-curve editor (master channel): click to add, drag to move, right-click to delete.
fn curve_editor(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool, state: &mut crate::state::PointCurveState) -> ERect {
    // The identity handles are presentation until a gesture actually edits them.
    let mut points = if p.point_curve.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { p.point_curve.clone() };
    let (rect, changed) = curve_points_editor(ui, &mut points, state);
    if changed {
        p.point_curve = points;
        *dirty = true;
    }
    rect
}

/// The point-curve editor on `points` (0..255 in and out). Returns its rect and whether a gesture
/// changed the points.
pub(super) fn curve_points_editor(ui: &mut egui::Ui, points: &mut Vec<[f32; 2]>, state: &mut crate::state::PointCurveState) -> (ERect, bool) {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment([pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], Stroke::new(1.0, t.separator));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    let changed = crate::point_curve::interact(ui, &resp, rect, points, state).changed;
    // Curve (point curve after the parametric one).
    let lut = curve_lut(points, 256);
    let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, value)| to_screen(i as f32, value * 255.0)).collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, t.text)));
    for (i, c) in points.iter().enumerate() {
        let center = to_screen(c[0], c[1]);
        if state.selected == Some(i) {
            painter.circle_filled(center, 3.0, t.accent);
        }
        painter.circle_stroke(center, 4.0, Stroke::new(1.5, t.accent));
    }
    (rect, changed)
}

/// The develop panel ([`super::develop_panel`]) of a raw the shared engine develops, and whether it
/// edited the settings; `None` for the classic panel.
fn develop_props(ui: &mut egui::Ui, d: &mut CameraRawDialog, sampler: &mut bool) -> Option<bool> {
    let edit = d.develop.as_mut()?;
    let before = edit.settings.clone();
    match d.tool {
        super::rail::Tool::Edit => {
            let tools = super::develop_panel::Tools { guides: &mut d.guides, sampler, alt: &mut d.scope.alt_tone, keymap: &d.keymap };
            let out = super::develop_panel::panel(ui, edit, d.develop_source.as_deref(), &mut d.develop_panel, tools);
            d.curve_rect = out.curve_rect;
            d.auto_all |= out.auto;
        }
        tool => {
            super::lc::style(ui);
            ui.spacing_mut().item_spacing.y = 0.0;
            let overlay_was = d.local.overlay_mask(tool, &edit.settings);
            match tool {
                super::rail::Tool::Masking => super::local_panels::masking(ui, &mut d.local, &mut edit.settings),
                super::rail::Tool::Heal => super::local_panels::heal(ui, &mut d.local, &mut edit.settings),
                _ => super::local_panels::red_eye(ui, &mut d.local, &mut edit.settings),
            }
            // another mask selected, or the overlay switched: the preview renders its alpha
            if d.local.overlay_mask(tool, &edit.settings) != overlay_was {
                d.dirty = true;
            }
        }
    }
    let edited = edit.settings != before;
    if edited {
        d.params = edit.camera_raw(&d.params);
    }
    Some(edited)
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.camera_raw.is_none() && !app.raw_shelf.is_empty() {
        super::filmstrip::clear_selection(&mut app.raw_shelf);
    }
    if app.camera_raw.is_none()
        && !app.raw_shelf.is_empty()
        && let Err(e) = super::filmstrip::open_held(app, ctx, 0)
    {
        // It opens as developed with the defaults rather than vanish.
        let held = app.raw_shelf.remove(0);
        let _ = super::open::commit_open_raw(app, &Default::default(), None, &held.raw, None);
        app.ui.status = e;
        app.ui.status_error = true;
    }
    let Some(d) = app.camera_raw.as_ref() else { return };
    // Camera Raw's window is drawn from the main window's frame: while a filmstrip thumbnail waits
    // to be redrawn, the main window keeps frames coming, whatever reaches the dialog's own window.
    if super::filmstrip::thumbnails_pending(app) {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    // Where the platform has no extra windows (the web build), draw over the main window.
    if ctx.embed_viewports() {
        draw(app, ctx, false);
        return;
    }
    let window_title = d.window_title();
    // Camera Raw is modal, as in Photoshop: the main window is dimmed and takes no input while it
    // is open (in a separate window, or drawn over it where there are no extra windows).
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
    let changed = has_changes(app);
    // Keys that reach the main window while the dialog is open are the dialog's: a native menu's
    // key equivalent comes back as a key press there (AppKit takes it before the dialog's window
    // sees it; `native_menu::NativeMenu::raw_input`), and so does the control channel's `ui.key`
    // (eframe's `raw_input_hook` feeds the main window only, never an immediate viewport's). The
    // main window runs no shortcut under the dialog (`shortcuts::preflight_keys`).
    let forwarded = forwarded_keys(ctx);
    ctx.show_viewport_immediate(window, builder, |ui, class| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            close = true;
            // closing the window is Cancel: with changes it asks first, the window staying
            if changed {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if !forwarded.is_empty() {
            ui.ctx().input_mut(|i| i.events.extend(forwarded.iter().cloned()));
        }
        draw(app, ui.ctx(), class != egui::ViewportClass::EmbeddedWindow);
    });
    if close && app.camera_raw.is_some() {
        request_cancel(app);
    }
}

/// Whether Cancel would throw anything away: the shown raw's settings (masks, spots and red eyes
/// among them) changed since the dialog first showed them, or another raw of the filmstrip was
/// edited.
pub(super) fn has_changes(app: &PhotocraftApp) -> bool {
    let Some(d) = app.camera_raw.as_ref() else { return false };
    let shown = d.opened.as_ref().is_some_and(|o| *o != snapshot(d));
    let held = app.raw_shelf.iter().any(|h| h.kept.as_ref().is_some_and(|(now, from)| now != from));
    shown || held
}

/// Cancel (its button, Esc, closing the window): at once when nothing changed, else after
/// "Discard changes to …?" (Keep Editing is the default).
pub(super) fn request_cancel(app: &mut PhotocraftApp) {
    if has_changes(app) {
        if let Some(d) = app.camera_raw.as_mut() {
            d.confirm_discard = true;
        }
    } else {
        cancel(app);
    }
}

/// The "Discard changes?" prompt over the dialog, while Cancel waits for it: Discard cancels, Keep
/// Editing (the default: Enter, and Esc) goes back to the dialog with everything as it was.
fn discard_prompt(app: &mut PhotocraftApp, ctx: &egui::Context) {
    use crate::widgets::{ButtonRole, DialogButton};
    let Some(d) = app.camera_raw.as_ref().filter(|d| d.confirm_discard) else { return };
    let tr = |s| crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s);
    let message = crate::i18n::fmt(tr("Discard changes to “{name}”?"), &[("name", &d.layer_name)]);
    let (discard, keep) = (tr("Discard"), tr("Keep Editing"));
    let row = [DialogButton::new(ButtonRole::Alternate, discard, 84.0), DialogButton::new(ButtonRole::Default, keep, 110.0).focus(true)];
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("camera-raw-discard")).show(ctx, |ui| {
        ui.set_max_width(400.0);
        ui.label(egui::RichText::new(tr("Discard changes?")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        widgets::hairline(ui);
        ui.add_space(8.0);
        ui.label(message);
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            answer = widgets::dialog_buttons(ui, &row);
        });
    });
    // Esc (the modal closing) and Enter keep editing, unless Tab put the focus on Discard: that
    // button took the Enter as its click
    if modal.should_close() || answer.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        answer = Some(ButtonRole::Default);
    }
    match answer {
        Some(ButtonRole::Alternate) => cancel(app),
        Some(_) => {
            if let Some(d) = app.camera_raw.as_mut() {
                d.confirm_discard = false;
            }
        }
        None => {}
    }
}

/// The dialog's shortcuts this frame, all from its keymap: undo, zoom, Hand and Zoom, before /
/// after, clipping, the sampler, ⌘A, Esc, ⇧T, Delete, and the develop view's tools, brush and
/// section keys. `filmstrip`: several raws are open.
fn dialog_keys(app: &mut PhotocraftApp, ctx: &egui::Context, filmstrip: bool) -> super::keys::Outcome {
    let Some(d) = app.camera_raw.as_mut() else { return Default::default() };
    let guides_were_active = d.guides.active;
    let view =
        super::keys::View { nav: &mut app.ui.camera_raw_preview, scope: &mut app.ui.camera_raw_scope, filmstrip: filmstrip && d.target == Target::OpenRaw };
    let keys = super::keys::handle(d, view, ctx);
    // Shift+T took the Guided Upright tool up: it puts the Color Sampler down (the newest tool
    // wins), rather than the sampler putting the new tool straight down again.
    if d.guides.active && !guides_were_active {
        app.ui.camera_raw_scope.sampler_tool = false;
    }
    keys
}

/// The key and text events of the main window's input this frame, for the dialog's own window.
fn forwarded_keys(ctx: &egui::Context) -> Vec<egui::Event> {
    ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))).cloned().collect())
}

/// Keeps the mask brush's Auto Mask and its preference (`rawDefaults.brushAutoMask`) together: the
/// dialog starts from the preference, a change in the dialog (its checkbox, the A key, the control
/// channel) is saved there, and a change of the preference elsewhere reaches the dialog.
pub(super) fn sync_auto_mask(app: &mut PhotocraftApp) {
    let pref = app.session.prefs().raw_defaults.brush_auto_mask;
    let Some(d) = app.camera_raw.as_mut() else { return };
    let st = &mut d.local;
    match st.auto_mask_synced {
        Some(was) if st.brush.auto_mask != was => {
            let on = st.brush.auto_mask;
            st.auto_mask_synced = Some(on);
            app.session.edit_prefs(|p| p.raw_defaults.brush_auto_mask = on);
        }
        Some(was) if was == pref => {}
        _ => {
            st.brush.auto_mask = pref;
            st.auto_mask_synced = Some(pref);
        }
    }
}

/// The develop view's Heal, Masking and Red Eye on the preview's image rect `r`: the mask
/// overlay, the tool's outlines and handles, and its gestures.
fn local_preview(preview: &mut egui::Ui, d: &mut CameraRawDialog, r: ERect, response: &egui::Response, panning: bool) {
    if d.show_before || d.tool == super::rail::Tool::Edit {
        return;
    }
    let Some(e) = d.develop.as_mut() else { return };
    super::local::paint_overlay(preview, &d.local, r, d.tool, &e.settings);
    let overlay_was = d.local.overlay_mask(d.tool, &e.settings);
    let source = d.develop_source.clone();
    let (pw, ph) = (d.pw, d.ph);
    let sample = move |s: &craft_develop_model::DevelopSettings, n: craft_geom::Point| source.as_ref().and_then(|src| src.color_range_sample(s, n, pw, ph));
    let out = super::local::preview(preview, &mut d.local, d.tool, &mut e.settings, r, response, panning, &sample);
    if out.edited {
        d.params = e.camera_raw(&d.params);
        d.dirty = true;
    }
    // a press on the mask's geometry brought its overlay back: the preview renders its alpha
    if d.local.overlay_mask(d.tool, &e.settings) != overlay_was {
        d.dirty = true;
    }
}

/// The dialog's colours and panel width: PhotoCraft's, or LightCraft's in the develop view.
struct Look {
    chrome: Color32,
    /// Round the preview.
    body: Color32,
    canvas: Color32,
    dock: Color32,
    separator: Color32,
    panel_w: f32,
}

impl Look {
    fn of(ctx: &egui::Context, lc_view: bool) -> Look {
        let t = Tokens::get(ctx);
        if lc_view {
            let l = super::lc::Lc::get(ctx);
            Look { chrome: l.chrome, body: l.canvas, canvas: l.canvas, dock: l.chrome, separator: l.divider, panel_w: super::lc::PANEL_W + super::lc::RAIL_W }
        } else {
            Look { chrome: t.chrome, body: t.chrome, canvas: t.canvas, dock: t.dock, separator: t.separator, panel_w: PANEL_W }
        }
    }
}

/// The settings panel's area in `right` (with the develop view's rail at its right edge), and a
/// `Ui` inside its margins to draw the panel in.
fn panels(ui: &mut egui::Ui, painter: &egui::Painter, d: &mut CameraRawDialog, right: ERect) -> (ERect, egui::Ui) {
    if d.develop.is_some() {
        let right = rail_and_panel(ui, painter, d, right);
        let mut props = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(0.0, 8.0))));
        super::lc::style(&mut props);
        return (right, props);
    }
    let t = Tokens::get(ui.ctx());
    painter.rect_filled(right, 0.0, t.dock);
    painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
    (right, ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(14.0, 10.0)))))
}

/// The develop view's icon rail at the right edge of `right`, and the panel's background beside
/// it; the panel's rect.
fn rail_and_panel(ui: &mut egui::Ui, painter: &egui::Painter, d: &mut CameraRawDialog, mut right: ERect) -> ERect {
    let lc_t = super::lc::Lc::get(ui.ctx());
    // the rail at the far right, the panel beside it, as LightCraft lays them out
    let rail = ERect::from_min_max(pos2(right.right() - super::lc::RAIL_W, right.top()), right.max);
    right.max.x = rail.left();
    let was = d.tool;
    super::rail::rail(ui, rail, &mut d.tool, &d.keymap);
    if d.tool != was {
        d.tool_changed(was);
    }
    painter.rect_filled(right, 0.0, lc_t.chrome);
    painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, lc_t.divider));
    right
}

/// The dialog's footer buttons (LightCraft's in the develop view); the action clicked.
fn footer(fu: &mut egui::Ui, d: &mut CameraRawDialog, lc_view: bool, ok_label: &'static str) -> Option<&'static str> {
    let t = Tokens::get(fu.ctx());
    let lc_t = super::lc::Lc::get(fu.ctx());
    let mut action = None;
    if lc_view {
        super::lc::style(fu);
    }
    if lc_view {
        // LightCraft's footer: a filled Open, rounded outline buttons for the rest
        if super::lc::widgets::accent_button(fu, ok_label, 90.0).clicked() {
            action = Some("ok");
        }
        if super::lc::widgets::pill_button(fu, tl!("Cancel"), Some(90.0)).clicked() {
            action = Some("cancel");
        }
        if d.target == Target::OpenRaw
            && super::lc::widgets::pill_button(fu, tl!("Done"), Some(90.0))
                .on_hover_text(tl!("Save the settings to each raw's sidecar without opening it"))
                .clicked()
        {
            action = Some("done");
        }
        fu.add_space(12.0);
        if super::lc::widgets::text_button(fu, tl!("Before (Y)"), d.show_before).clicked() {
            d.show_before = !d.show_before;
        }
        fu.label(egui::RichText::new(format!("{:.0} ms", d.render_ms)).color(lc_t.text_dim));
        fu.add_space(12.0);
        if super::lc::widgets::pill_button(fu, tl!("Save Settings…"), None).on_hover_text(tl!("Save these settings as a Camera Raw .xmp file")).clicked() {
            action = Some("save");
        }
        if super::lc::widgets::pill_button(fu, tl!("Load Settings…"), None).on_hover_text(tl!("Load settings from a Camera Raw .xmp file")).clicked() {
            action = Some("load");
        }
    } else {
        if let Some(role) = widgets::dialog_buttons(
            fu,
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
        fu.add_space(12.0);
        widgets::checkbox(fu, &mut d.show_before, tl!("Before (Y)"));
        fu.label(egui::RichText::new(format!("{:.0} ms", d.render_ms)).color(t.text_faint));
        fu.add_space(12.0);
        if fu.button(tl!("Save Settings…")).on_hover_text(tl!("Save these settings as a Camera Raw .xmp file")).clicked() {
            action = Some("save");
        }
        if fu.button(tl!("Load Settings…")).on_hover_text(tl!("Load settings from a Camera Raw .xmp file")).clicked() {
            action = Some("load");
        }
    }
    action
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
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context, own_window: bool) {
    sync_auto_mask(app);
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // A raw being opened is not in the session: its own document says how it displays.
    let held = app.camera_raw.as_ref().and_then(|d| d.raw_open.as_ref()).and_then(|r| r.held.clone());
    let current_display = match held {
        Some(doc) => app.session.color.canvas_display(&doc).ok(),
        None => app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok()),
    };
    let filmstrip = app.raw_shelf.len() > 1;
    let keys = dialog_keys(app, ctx, filmstrip);
    let mut action: Option<&str> = keys.cancel.then_some("cancel");
    let mut film_click: Option<super::filmstrip::FilmClick> = keys.select_all.then_some(super::filmstrip::FilmClick::SelectAll);
    egui::Area::new(egui::Id::unique("camera-raw-dialog")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let Some(d) = app.camera_raw.as_mut() else { return };
        let pointer = ctx.input(|i| i.pointer.primary_down());
        // Edits made elsewhere (the guides, a sidecar) reach the develop settings first.
        d.sync_develop();
        // what Cancel would throw away is measured from here: the dialog as it first shows
        if d.opened.is_none() {
            d.opened = Some(snapshot(d));
        }
        if pointer && d.edit_origin.is_none() {
            d.edit_origin = Some(snapshot(d));
        }
        // What this frame's controls start from, for an edit no pointer made (below).
        let frame_start = snapshot(d);
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
        // A raw the shared engine develops: LightCraft's develop view (its palette, panel and rail).
        let lc_view = d.develop.is_some();
        let look = Look::of(ctx, lc_view);
        painter.rect_filled(full, 0.0, look.chrome);
        let title = ERect::from_min_size(full.min, vec2(full.width(), if own_window { 0.0 } else { 30.0 }));
        if !own_window {
            painter.rect_filled(title, 0.0, t.dock);
            painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
            painter.text(title.center(), Align2::CENTER_CENTER, d.window_title(), FontId::proportional(13.0), t.text);
        }
        let ok_label = if d.target == Target::OpenRaw { tl!("Open") } else { tl!("OK") };
        let footer_h = 48.0;
        let body = ERect::from_min_max(pos2(full.left(), title.bottom()), pos2(full.right(), full.bottom() - footer_h));
        let animated_zoom = app.session.prefs().general.animated_zoom;
        let scope = &mut app.ui.camera_raw_scope;
        // Clip both pixels and scope overlays before painting the settings dock/footer.
        // Several raws being opened: the filmstrip on the left.
        let film_w = if d.target == Target::OpenRaw && app.raw_shelf.len() > 1 { super::filmstrip::FILM_W } else { 0.0 };
        if film_w > 0.0 {
            let film = ERect::from_min_max(body.min, pos2(body.left() + film_w - 1.0, body.bottom()));
            painter.rect_filled(film, 0.0, t.dock);
            painter.line_segment([film.right_top(), film.right_bottom()], Stroke::new(1.0, t.separator));
            // (⌘A, every raw, is the keymap's: application shortcuts don't run under the dialog,
            // `shortcuts::preflight_keys`, so it never selects the document behind it)
            film_click = super::filmstrip::filmstrip(ui, film, &mut app.raw_shelf, d.raw_open.as_ref().map(|r| r.document), &t).or(film_click.take());
        }
        let panel_w = look.panel_w;
        let preview_body =
            ERect::from_min_max(pos2(body.left() + film_w, body.top()), pos2((body.right() - panel_w).max(body.left() + film_w + 1.0), body.bottom()));
        painter.rect_filled(preview_body, 0.0, look.body);
        let view = ERect::from_min_max(
            preview_body.min + vec2(16.0, 16.0),
            pos2((preview_body.right() - 16.0).max(preview_body.left() + 17.0), (preview_body.bottom() - 48.0).max(preview_body.top() + 17.0)),
        );
        d.viewport = Some(view);
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(view));
        preview.set_clip_rect(view.intersect(ui.clip_rect()));
        preview.painter().rect_filled(view, 0.0, look.canvas);
        // The Guided Upright tool takes the preview's clicks as the sampler does (the Hand and
        // Zoom tools put it down, and so does choosing the sampler with S).
        if d.params.upright != "guided" || scope.sampler_tool {
            d.guides.active = false;
        }
        let guided = d.guides.active;
        // the develop view's Heal, Masking and Red Eye draw on the preview
        let local_tool = lc_view && d.tool != super::rail::Tool::Edit && !d.local.paused && !d.show_before;
        let mut clicks_taken = scope.sampler_tool || guided || local_tool;
        let (r, response, panning, zoom_box) = crate::camera_raw_preview_ui::interact(&mut preview, navigation, view, size, &mut clicks_taken, animated_zoom);
        if !clicks_taken {
            scope.sampler_tool = false;
            d.guides.active = false;
            if local_tool {
                // the Hand or Zoom took the preview: the tool waits until it is picked again
                d.local.paused = true;
            }
        }
        let tex = if d.show_before { d.before_tex.as_ref() } else { d.tex.as_ref() };
        if let Some(tex) = tex {
            // Never tile a checker across the offscreen extent of a highly zoomed image.
            widgets::checker(preview.painter(), r.intersect(view), 8.0);
            preview.painter().image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            // Without the sensor data, the full-size preview can't show the raw-stage sliders.
            let raw_tone = d.raw_source.is_none() && d.raw_curve.is_some() && !raw_stage(&d.params).is_neutral();
            let detail_needed = !raw_tone && (r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32);
            d.detail.settings = d.develop.as_ref().map(|e| (e.settings.clone(), !e.edited(&e.settings)));
            d.detail.paint(&preview, r, d.preview_revision, d.show_before, &d.params, detail_needed, &d.display);
            if d.detail.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            d.scope.preview_rect = Some(r);
            crate::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
            if lc_view {
                local_preview(&mut preview, d, r, &response, panning);
            }
            if guided && d.guides.active && !d.show_before {
                let shown = d.params.clone();
                let (fw, fh) = (d.full_w, d.full_h);
                if crate::camera_raw_guides_ui::preview(&mut preview, &mut d.guides, &mut d.params, &shown, r, fw, fh, &response, panning, d.tex.as_ref()) {
                    d.dirty = true;
                }
            }
        }
        if let Some(region) = zoom_box {
            preview.painter().rect_stroke(region, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        let toolbar = ERect::from_min_max(pos2(preview_body.left() + 16.0, view.bottom() + 8.0), preview_body.right_bottom() - vec2(16.0, 4.0));
        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(toolbar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        nav_ui.set_clip_rect(preview_body.intersect(ui.clip_rect()));
        let mut tool_taken = scope.sampler_tool || d.guides.active || local_tool;
        crate::camera_raw_preview_ui::toolbar(&mut nav_ui, navigation, view, size, &mut tool_taken);
        if !tool_taken {
            scope.sampler_tool = false;
            d.guides.active = false;
            if local_tool {
                // the Hand or Zoom button took the preview: the tool waits until it is picked again
                d.local.paused = true;
                d.local.end_gesture();
            }
        }
        if d.detail.pending() {
            nav_ui.spinner();
        }
        if let Some(error) = &d.detail.error {
            nav_ui.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(t.text_faint)).on_hover_text(error);
        }
        // Panels.
        let (right, mut props) = panels(ui, &painter, d, ERect::from_min_max(pos2(body.right() - panel_w, body.top()), body.max));
        // This header is outside the settings scroll area and follows the same Before/After
        // selector as the texture. Both derive from the same completed preview revision.
        if !lc_view || d.tool == super::rail::Tool::Edit {
            crate::camera_raw_scope_ui::header(&mut props, d, scope);
        }
        d.scope.alt_tone = None;
        let mut dirty = false;
        egui::ScrollArea::vertical().id_salt("camera-raw-props").show(&mut props, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            // A raw the shared engine develops: LightCraft's Edit panel, from the control registry.
            if let Some(edited) = develop_props(ui, d, &mut scope.sampler_tool) {
                dirty |= edited;
                return;
            }
            let p = &mut d.params;
            let proxy = &d.proxy;
            let auto_all = &mut d.auto_all;
            let alt = &mut d.scope.alt_tone;
            let (ts, tn) = (temp_stops(), tint_stops());
            section(ui, "Light", true, |ui| {
                // Auto: Camera Raw's one-click tone from the histogram; the sliders show what it
                // chose, so each can be adjusted afterwards.
                if ui.button(tl!("Auto")).on_hover_text(tl!("Set the tone sliders from the image")).clicked() {
                    photocraft_algo::camera_raw::auto_tone(proxy, p);
                    *auto_all = true;
                    dirty = true;
                }
                tone_row(ui, &mut dirty, alt, ToneZone::Exposure, &mut p.exposure);
                row(ui, &mut dirty, "Contrast", &mut p.contrast, -100.0..=100.0, None);
                tone_row(ui, &mut dirty, alt, ToneZone::Highlights, &mut p.highlights);
                tone_row(ui, &mut dirty, alt, ToneZone::Shadows, &mut p.shadows);
                tone_row(ui, &mut dirty, alt, ToneZone::Whites, &mut p.whites);
                tone_row(ui, &mut dirty, alt, ToneZone::Blacks, &mut p.blacks);
            });
            section(ui, "Color", false, |ui| {
                widgets::section_label(ui, tl!("White Balance: As Shot"));
                row(ui, &mut dirty, "Temperature", &mut p.temperature, -100.0..=100.0, Some(&ts));
                row(ui, &mut dirty, "Tint", &mut p.tint, -100.0..=100.0, Some(&tn));
                row(ui, &mut dirty, "Vibrance", &mut p.vibrance, -100.0..=100.0, None);
                row(ui, &mut dirty, "Saturation", &mut p.saturation, -100.0..=100.0, None);
            });
            section(ui, "Effects", false, |ui| {
                row(ui, &mut dirty, "Texture", &mut p.texture, -100.0..=100.0, None);
                row(ui, &mut dirty, "Clarity", &mut p.clarity, -100.0..=100.0, None);
                row(ui, &mut dirty, "Dehaze", &mut p.dehaze, -100.0..=100.0, None);
                widgets::section_label(ui, tl!("Vignetting"));
                let mut style = p.vignette_style.clone();
                if widgets::dropdown(
                    ui,
                    "cr-vig-style",
                    tl!("Vignetting"),
                    &mut style,
                    &[
                        ("highlightPriority".to_string(), tl!("Highlight Priority")),
                        ("colorPriority".to_string(), tl!("Color Priority")),
                        ("paintOverlay".to_string(), tl!("Paint Overlay")),
                    ],
                    200.0,
                ) {
                    p.vignette_style = style;
                    dirty = true;
                }
                row(ui, &mut dirty, "Amount", &mut p.vignette_amount, -100.0..=100.0, None);
                row(ui, &mut dirty, "Midpoint", &mut p.vignette_midpoint, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roundness", &mut p.vignette_roundness, -100.0..=100.0, None);
                row(ui, &mut dirty, "Feather", &mut p.vignette_feather, 0.0..=100.0, None);
                row(ui, &mut dirty, "Highlights", &mut p.vignette_highlights, 0.0..=100.0, None);
                widgets::section_label(ui, tl!("Grain"));
                row(ui, &mut dirty, "Amount", &mut p.grain_amount, 0.0..=100.0, None);
                row(ui, &mut dirty, "Size", &mut p.grain_size, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roughness", &mut p.grain_roughness, 0.0..=100.0, None);
            });
            section(ui, "Curve", false, |ui| {
                d.curve_rect = Some(curve_editor(ui, p, &mut dirty, &mut d.curve_state));
                row(ui, &mut dirty, "Highlights", &mut p.curve_highlights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Lights", &mut p.curve_lights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Darks", &mut p.curve_darks, -100.0..=100.0, None);
                row(ui, &mut dirty, "Shadows", &mut p.curve_shadows, -100.0..=100.0, None);
            });
            section(ui, "Color Mixer", false, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, source) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
                        let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", source);
                        let response = widgets::pill_tab(ui, name, d.mixer_tab == i);
                        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), d.mixer_tab == i, name));
                        if response.clicked() {
                            d.mixer_tab = i;
                        }
                    }
                });
                let arr = match d.mixer_tab {
                    0 => &mut p.hsl_hue,
                    1 => &mut p.hsl_sat,
                    _ => &mut p.hsl_lum,
                };
                for (k, name) in BANDS.iter().enumerate() {
                    let grad = mixer_gradient(d.mixer_tab, k);
                    row(ui, &mut dirty, name, &mut arr[k], -100.0..=100.0, Some(&grad));
                }
            });
            section(ui, "Color Grading", false, |ui| {
                wheel(ui, &mut dirty, "Shadows", &mut p.grade_shadows);
                wheel(ui, &mut dirty, "Midtones", &mut p.grade_midtones);
                wheel(ui, &mut dirty, "Highlights", &mut p.grade_highlights);
                wheel(ui, &mut dirty, "Global", &mut p.grade_global);
                row(ui, &mut dirty, "Blending", &mut p.grade_blending, 0.0..=100.0, None);
                row(ui, &mut dirty, "Balance", &mut p.grade_balance, -100.0..=100.0, None);
            });
            geometry(ui, p, &mut dirty, &mut d.guides, &mut scope.sampler_tool);
            section(ui, "Detail", false, |ui| {
                if let Some(raw) = d.raw_open.as_mut() {
                    demosaic_row(ui, &mut raw.demosaic);
                }
                widgets::section_label(ui, tl!("Sharpening"));
                row(ui, &mut dirty, "Amount", &mut p.sharpen_amount, 0.0..=150.0, None);
                row(ui, &mut dirty, "Radius", &mut p.sharpen_radius, 0.5..=3.0, None);
                row(ui, &mut dirty, "Detail", &mut p.sharpen_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Masking", &mut p.sharpen_masking, 0.0..=100.0, None);
                widgets::section_label(ui, tl!("Noise Reduction"));
                row(ui, &mut dirty, "Luminance", &mut p.noise_luminance, 0.0..=100.0, None);
                row(ui, &mut dirty, "Luminance Detail", &mut p.noise_luminance_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color", &mut p.noise_color, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color Detail", &mut p.noise_color_detail, 0.0..=100.0, None);
            });
            // Camera Raw's Calibration panel: a tint for the shadows and each primary's hue and
            // saturation (Process Version is implied: 2012 and later).
            section(ui, "Calibration", false, |ui| {
                widgets::section_label(ui, tl!("Shadows"));
                row(ui, &mut dirty, "Tint", &mut p.shadow_tint, -100.0..=100.0, Some(&tn));
                for (i, name) in ["Red Primary", "Green Primary", "Blue Primary"].iter().enumerate() {
                    widgets::section_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", name));
                    ui.push_id(name, |ui| {
                        row(ui, &mut dirty, "Hue", &mut p.calibration_hue[i], -100.0..=100.0, None);
                        row(ui, &mut dirty, "Saturation", &mut p.calibration_sat[i], -100.0..=100.0, None);
                    });
                }
            });
        });
        crate::camera_raw_scope_ui::floating(ctx, d, scope, right);
        d.typing = ctx.text_edit_focused();
        d.dirty |= dirty;
        if d.dirty {
            ctx.request_repaint();
        }
        if d.render_pending() {
            // The worker asks for a repaint when it is done; this catches a missed wake-up.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Footer.
        let foot = ERect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);
        painter.rect_filled(foot, 0.0, look.dock);
        painter.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, look.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 9.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if let Some(a) = footer(&mut fu, d, lc_view, ok_label) {
            action = Some(a);
        }
        // One undo step per finished edit: a pointer gesture from where it began, and an edit made
        // without the pointer (a button from the keyboard or a screen reader) from this frame's start.
        if !ctx.input(|i| i.pointer.primary_down()) {
            record_edit(d, frame_start);
        }
    });
    crate::camera_raw_scope_ui::persist(app, ctx);
    // This frame's edit reaches the other selected raws before another raw is shown or Open runs.
    super::filmstrip::sync_selected(app);
    let clicked = match film_click {
        Some(super::filmstrip::FilmClick::Tile(i, shift, command)) => super::filmstrip::click(app, ctx, i, shift, command),
        Some(super::filmstrip::FilmClick::SelectAll) => {
            super::filmstrip::select_all(app);
            Ok(())
        }
        None => Ok(()),
    };
    if let Err(e) = clicked {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    // Thumbnails of raws an edit synced to, redrawn on workers once the drag ends. The dialog
    // polls them: a worker can't wake an immediate viewport's window by itself.
    if !ctx.input(|i| i.pointer.primary_down()) {
        if super::filmstrip::refresh_thumbnails(app, false) {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // The shown raw's own tile, from the preview once it has landed.
        if super::filmstrip::refresh_shown_thumbnail(app) {
            ctx.request_repaint();
        }
    }
    discard_prompt(app, ctx);
    match action {
        Some("ok") => {
            if let Err(e) = commit(app) {
                app.ui.status = e;
            }
        }
        Some("cancel") => request_cancel(app),
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
