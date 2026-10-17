//! Camera Raw's Edit tool: the Edit header (Auto, B&W, HDR), Profile, and the panels in Camera
//! Raw 16.1.1's order (Light, Color, Effects, Curve, Color Mixer, Color Grading, Detail, Optics,
//! Geometry, Calibration; its Early Access Lens Blur is left out), laid out as measured in
//! Photoshop 25.4. Controls
//! whose processing PhotoCraft does not have yet are shown, stored and round-tripped like the
//! rest; `photocraft_algo::camera_raw` documents which ones are placeholders.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, Wheel, curve_lut};

use crate::camera_raw_scope_ui::ToneZone;
use crate::camera_raw_style::{self as st, Acr, Eye, Spec};
use crate::camera_raw_ui::CameraRawDialog;
use crate::state::CameraRawLayoutState;

/// Panel ids (layout state, control channel) and their titles.
pub(crate) const PANELS: [(&str, &str); 10] = [
    ("light", "Light"),
    ("color", "Color"),
    ("effects", "Effects"),
    ("curve", "Curve"),
    ("mixer", "Color Mixer"),
    ("grading", "Color Grading"),
    ("detail", "Detail"),
    ("optics", "Optics"),
    ("geometry", "Geometry"),
    ("calibration", "Calibration"),
];

/// Sub-groups that open with a triangle (layout state ids).
pub(crate) const GROUPS: [&str; 8] =
    ["vignette", "grain", "sharpening", "noiseReduction", "colorNoiseReduction", "lensVignette", "defringe", "manualTransformations"];

const BANDS: [&str; 8] = ["Reds", "Oranges", "Yellows", "Greens", "Aquas", "Blues", "Purples", "Magentas"];

fn tr(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

/// `p` with the settings of panel `id` back at their defaults: what the preview shows while that
/// panel's eye is off, and the test for "this panel has edits".
pub(crate) fn without(p: &CameraRaw, id: &str) -> CameraRaw {
    let d = CameraRaw::default();
    let mut q = p.clone();
    match id {
        "light" => {
            (q.exposure, q.contrast, q.highlights, q.shadows, q.whites, q.blacks) = (d.exposure, d.contrast, d.highlights, d.shadows, d.whites, d.blacks);
        }
        "color" => {
            (q.temperature, q.tint, q.vibrance, q.saturation) = (d.temperature, d.tint, d.vibrance, d.saturation);
            q.white_balance = d.white_balance;
        }
        "effects" => {
            (q.texture, q.clarity, q.dehaze) = (d.texture, d.clarity, d.dehaze);
            (q.vignette_amount, q.vignette_midpoint, q.vignette_roundness, q.vignette_feather, q.vignette_highlights) =
                (d.vignette_amount, d.vignette_midpoint, d.vignette_roundness, d.vignette_feather, d.vignette_highlights);
            q.vignette_style = d.vignette_style;
            (q.grain_amount, q.grain_size, q.grain_roughness) = (d.grain_amount, d.grain_size, d.grain_roughness);
        }
        "curve" => {
            (q.curve_highlights, q.curve_lights, q.curve_darks, q.curve_shadows, q.curve_splits) =
                (d.curve_highlights, d.curve_lights, d.curve_darks, d.curve_shadows, d.curve_splits);
            (q.point_curve, q.point_curve_red, q.point_curve_green, q.point_curve_blue) =
                (d.point_curve, d.point_curve_red, d.point_curve_green, d.point_curve_blue);
            q.curve_refine_saturation = d.curve_refine_saturation;
        }
        "mixer" => {
            (q.hsl_hue, q.hsl_sat, q.hsl_lum, q.gray_mixer) = (d.hsl_hue, d.hsl_sat, d.hsl_lum, d.gray_mixer);
        }
        "grading" => {
            (q.grade_shadows, q.grade_midtones, q.grade_highlights, q.grade_global) = (d.grade_shadows, d.grade_midtones, d.grade_highlights, d.grade_global);
            (q.grade_blending, q.grade_balance) = (d.grade_blending, d.grade_balance);
        }
        "detail" => {
            (q.sharpen_amount, q.sharpen_radius, q.sharpen_detail, q.sharpen_masking) =
                (d.sharpen_amount, d.sharpen_radius, d.sharpen_detail, d.sharpen_masking);
            (q.noise_luminance, q.noise_luminance_detail, q.noise_luminance_contrast) =
                (d.noise_luminance, d.noise_luminance_detail, d.noise_luminance_contrast);
            (q.noise_color, q.noise_color_detail, q.noise_color_smoothness) = (d.noise_color, d.noise_color_detail, d.noise_color_smoothness);
        }
        "optics" => {
            (q.lens_distortion, q.lens_vignette, q.lens_vignette_midpoint) = (d.lens_distortion, d.lens_vignette, d.lens_vignette_midpoint);
            (q.defringe_purple_amount, q.defringe_purple_hue, q.defringe_green_amount, q.defringe_green_hue) =
                (d.defringe_purple_amount, d.defringe_purple_hue, d.defringe_green_amount, d.defringe_green_hue);
        }
        "geometry" => {
            q.upright = d.upright;
            (q.transform_vertical, q.transform_horizontal, q.transform_rotate, q.transform_aspect) =
                (d.transform_vertical, d.transform_horizontal, d.transform_rotate, d.transform_aspect);
            (q.transform_scale, q.transform_offset_x, q.transform_offset_y) = (d.transform_scale, d.transform_offset_x, d.transform_offset_y);
        }
        "calibration" => {
            q.calibration_shadows_tint = d.calibration_shadows_tint;
            (q.calibration_red_hue, q.calibration_red_sat, q.calibration_green_hue, q.calibration_green_sat, q.calibration_blue_hue, q.calibration_blue_sat) = (
                d.calibration_red_hue,
                d.calibration_red_sat,
                d.calibration_green_hue,
                d.calibration_green_sat,
                d.calibration_blue_hue,
                d.calibration_blue_sat,
            );
        }
        _ => {}
    }
    q
}

pub(crate) fn edited(p: &CameraRaw, id: &str) -> bool {
    without(p, id) != *p
}

/// The settings the preview renders: `p` with every hidden panel's edits removed.
pub(crate) fn visible(p: &CameraRaw, hidden: &[String]) -> CameraRaw {
    hidden.iter().fold(p.clone(), |q, id| without(&q, id))
}

/// State of the Edit tool this frame, gathered from the dialog.
struct Edit<'a> {
    p: &'a mut CameraRaw,
    layout: &'a mut CameraRawLayoutState,
    dirty: bool,
    /// White balance › Auto was chosen: the dialog measures the preview.
    auto_wb: bool,
}

impl Edit<'_> {
    fn group_open(&self, id: &str) -> bool {
        self.layout.groups.iter().any(|g| g == id)
    }
    fn set_group(&mut self, id: &str, open: bool) {
        self.layout.groups.retain(|g| g != id);
        if open {
            self.layout.groups.push(id.to_owned());
        }
    }
}

fn slider(ui: &mut Ui, dirty: &mut bool, id: &str, label: &'static str, v: &mut f32, spec: &Spec) -> st::SliderResponse {
    let r = st::slider(ui, id, tr(label), v, spec);
    *dirty |= r.changed;
    r
}

/// The Edit tool: header, Profile and the panels. Returns whether settings changed.
pub(crate) fn edit(ui: &mut Ui, d: &mut CameraRawDialog, layout: &mut CameraRawLayoutState) -> bool {
    let mut dirty = false;
    if d.profile_browser {
        profile_browser(ui, d, &mut dirty);
        return dirty;
    }
    header(ui, d, &mut dirty);
    st::rule(ui);
    profile_row(ui, d, &mut dirty);
    st::rule(ui);
    for (id, title) in PANELS {
        let open = layout.open.iter().any(|o| o == id);
        let eye = if d.hidden.iter().any(|h| h == id) { Eye::Hidden } else { Eye::Shown { edited: edited(&d.params, id) } };
        let h = st::panel_header(ui, id, tr(title), open, None, eye);
        if h.eye_clicked {
            if eye == Eye::Hidden {
                d.hidden.retain(|x| x != id);
            } else {
                d.hidden.push(id.to_owned());
            }
            d.dirty = true;
        }
        if h.solo {
            layout.open = vec![id.to_owned()];
        } else if h.toggled {
            if open {
                layout.open.retain(|o| o != id);
            } else if layout.single_panel {
                layout.open = vec![id.to_owned()];
            } else {
                layout.open.push(id.to_owned());
            }
        }
        if layout.open.iter().any(|o| o == id) {
            let mut e = Edit { p: &mut d.params, layout, dirty: false, auto_wb: false };
            match id {
                "light" => light(ui, &mut e, &mut d.scope.alt_tone),
                "color" => color(ui, &mut e, &mut d.wb_picker),
                "effects" => effects(ui, &mut e),
                "curve" => curve(ui, &mut e, &d.after_histogram, &mut d.curve_state, &mut d.curve_rect),
                "mixer" => mixer(ui, &mut e),
                "grading" => grading(ui, &mut e),
                "detail" => detail(ui, &mut e),
                "optics" => optics(ui, &mut e),
                "geometry" => geometry(ui, &mut e),
                "calibration" => calibration(ui, &mut e),
                _ => {}
            }
            dirty |= e.dirty;
            if e.auto_wb {
                d.auto_white_balance();
            }
            if id == "detail"
                && let Some(raw) = d.raw_open.as_mut()
            {
                crate::camera_raw_ui::demosaic_row(ui, &mut raw.demosaic);
            }
            ui.add_space(8.0);
        }
        st::rule(ui);
    }
    ui.add_space(24.0);
    dirty
}

fn header(ui: &mut Ui, d: &mut CameraRawDialog, dirty: &mut bool) {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::ROW), Sense::hover());
    ui.painter().text(pos2(row.left() + st::INSET_L, row.center().y), Align2::LEFT_CENTER, tr("Edit"), crate::theme::semibold(14.0), c.text);
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(row.left(), row.top() + 8.0), pos2(row.right() - 14.0, row.bottom() - 8.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = 4.0;
    // HDR needs a high-dynamic-range display and document; never available here.
    st::small_button(&mut buttons, tr("HDR"), false, false, 28.0).on_disabled_hover_text(tl!("HDR editing needs an HDR display"));
    buttons.add_space(4.0);
    let (sep, _) = buttons.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
    buttons.painter().rect_filled(sep, 0.0, c.rule);
    buttons.add_space(4.0);
    let bw = st::small_button(&mut buttons, tr("B&W"), true, d.params.monochrome, 30.0).on_hover_text(tl!("Convert to black and white (V)"));
    if bw.clicked() {
        d.params.monochrome = !d.params.monochrome;
        *dirty = true;
    }
    let auto = st::small_button(&mut buttons, tr("Auto"), true, false, 28.0).on_hover_text(tl!("Auto tone and color (Ctrl+U)"));
    if auto.clicked() {
        d.auto_tone();
        *dirty = true;
    }
}

fn profile_row(ui: &mut Ui, d: &mut CameraRawDialog, dirty: &mut bool) {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::ROW), Sense::hover());
    ui.painter().text(pos2(row.left() + st::INSET_L, row.center().y), Align2::LEFT_CENTER, tr("Profile"), st::label_font(), c.label);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(row.left() + 67.0, row.top() + 8.0), pos2(row.right() - 14.0, row.bottom() - 8.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let mut choice = if d.params.monochrome { 1 } else { 0 };
    if crate::widgets::dropdown(&mut inner, "cr-profile", &mut choice, &[(0, tr("Color")), (1, tr("Monochrome")), (2, tr("Browse..."))], 128.0) {
        match choice {
            2 => d.profile_browser = true,
            n => {
                d.params.monochrome = n == 1;
                *dirty = true;
            }
        }
    }
    inner.add_space(10.0);
    let browse = crate::icons::button(&mut inner, "grid-2x2", 22.0, false, tl!("Browse profiles"));
    if browse.clicked() {
        d.profile_browser = true;
    }
}

/// Profile browser: Back, the Basic profiles as thumbnails of the preview, and the other groups
/// (no creative profiles ship with PhotoCraft yet).
fn profile_browser(ui: &mut Ui, d: &mut CameraRawDialog, dirty: &mut bool) {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::ROW), Sense::hover());
    let back = ui.interact(Rect::from_min_size(row.min, vec2(80.0, row.height())), ui.id().with("cr-profile-back"), Sense::click());
    crate::icons::paint(ui, Rect::from_center_size(pos2(row.left() + 26.0, row.center().y), vec2(16.0, 16.0)), "arrow-left", 15.0, c.text);
    ui.painter().text(pos2(row.left() + 44.0, row.center().y), Align2::LEFT_CENTER, tr("Back"), st::label_font(), c.text);
    ui.painter().text(row.center(), Align2::CENTER_CENTER, tr("Profile"), st::label_font(), c.text);
    if back.clicked() {
        d.profile_browser = false;
    }
    st::rule(ui);
    let current = if d.params.monochrome { tr("Monochrome") } else { tr("Color") };
    st::caption(ui, &format!("{current} - {}", tr("Basic")));
    st::rule(ui);
    let mut favorites = false;
    group_line(ui, tr("Favorites"), &mut favorites, true);
    let mut basic = true;
    group_line(ui, tr("Basic"), &mut basic, false);
    let w = ((ui.available_width() - st::INSET_L * 2.0 - 10.0) / 2.0).max(40.0);
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 6.0);
        for (mono, label) in [(false, tr("Color")), (true, tr("Monochrome"))] {
            let (r, resp) = ui.allocate_exact_size(vec2(w, w), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, d.params.monochrome == mono, label));
            if let Some(tex) = if mono { d.mono_tex.as_ref() } else { d.tex.as_ref() } {
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            let strip = Rect::from_min_max(pos2(r.left(), r.bottom() - 16.0), r.max);
            ui.painter().rect_filled(strip, 0.0, if d.params.monochrome == mono { c.accent } else { Color32::from_black_alpha(160) });
            ui.painter().text(strip.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, label, st::small_font(), Color32::WHITE);
            if resp.clicked() && d.params.monochrome != mono {
                d.params.monochrome = mono;
                *dirty = true;
            }
            ui.add_space(6.0);
        }
    });
    ui.add_space(8.0);
    st::rule(ui);
    for g in ["Artistic", "B&W", "Modern", "Vintage"] {
        let mut open = false;
        group_line(ui, tr(g), &mut open, false);
    }
    st::note(ui, tl!("Creative profiles are not available in PhotoCraft yet."), true);
}

fn group_line(ui: &mut Ui, label: &str, open: &mut bool, star: bool) {
    let c = Acr::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.clicked() {
        *open = !*open;
    }
    let tri = pos2(r.left() + st::INSET_L - 6.0, r.center().y);
    let pts = if *open {
        vec![tri + vec2(-4.0, -3.0), tri + vec2(4.0, -3.0), tri + vec2(0.0, 3.0)]
    } else {
        vec![tri + vec2(-3.0, -4.0), tri + vec2(-3.0, 4.0), tri + vec2(3.0, 0.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, c.text, Stroke::NONE));
    let mut x = r.left() + st::INSET_L + 6.0;
    if star {
        crate::icons::paint(ui, Rect::from_center_size(pos2(x + 6.0, r.center().y), vec2(12.0, 12.0)), "star-filled", 11.0, c.label);
        x += 16.0;
    }
    ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, st::label_font(), c.label);
}

fn light(ui: &mut Ui, e: &mut Edit, alt: &mut Option<ToneZone>) {
    let rows: [(ToneZone, Spec); 6] = [
        (ToneZone::Exposure, Spec::centred(5.0).decimals(2)),
        (ToneZone::Exposure, Spec::centred(100.0)),
        (ToneZone::Highlights, Spec::centred(100.0)),
        (ToneZone::Shadows, Spec::centred(100.0)),
        (ToneZone::Whites, Spec::centred(100.0)),
        (ToneZone::Blacks, Spec::centred(100.0)),
    ];
    for (i, (zone, spec)) in rows.into_iter().enumerate() {
        let (label, v) = match i {
            0 => ("Exposure", &mut e.p.exposure),
            1 => ("Contrast", &mut e.p.contrast),
            2 => ("Highlights", &mut e.p.highlights),
            3 => ("Shadows", &mut e.p.shadows),
            4 => ("Whites", &mut e.p.whites),
            _ => ("Blacks", &mut e.p.blacks),
        };
        let r = slider(ui, &mut e.dirty, label, label, v, &spec);
        // Alt-dragging a tone slider shows that zone's clipping on the preview.
        if r.alt_drag && i != 1 {
            *alt = Some(zone);
        }
    }
}

fn color(ui: &mut Ui, e: &mut Edit, picker: &mut bool) {
    let c = Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(tr("White balance")).font(st::label_font()).color(c.label));
        ui.add_space(8.0);
        let mut wb = e.p.white_balance.clone();
        if !matches!(wb.as_str(), "asShot" | "auto" | "custom") {
            wb = "custom".into();
        }
        let options = [("asShot".to_string(), tr("As Shot")), ("auto".to_string(), tr("Auto")), ("custom".to_string(), tr("Custom"))];
        if crate::widgets::dropdown(ui, "cr-wb", &mut wb, &options, 86.0) {
            match wb.as_str() {
                "asShot" => (e.p.temperature, e.p.tint) = (0.0, 0.0),
                // Filled in by the dialog from the preview (gray world).
                "auto" => e.auto_wb = true,
                _ => {}
            }
            e.p.white_balance = wb;
            e.dirty = true;
        }
        ui.add_space(6.0);
        let pick = crate::icons::button(ui, "pipette", 22.0, *picker, tl!("White Balance tool (I): click a neutral area"));
        if pick.clicked() {
            *picker = !*picker;
        }
    });
    let before = (e.p.temperature, e.p.tint);
    slider(ui, &mut e.dirty, "temperature", "Temperature", &mut e.p.temperature, &Spec::centred(100.0).colors(st::temperature_stops()));
    slider(ui, &mut e.dirty, "tint", "Tint", &mut e.p.tint, &Spec::centred(100.0).colors(st::tint_stops()));
    if before != (e.p.temperature, e.p.tint) {
        e.p.white_balance = "custom".into();
    }
    slider(ui, &mut e.dirty, "vibrance", "Vibrance", &mut e.p.vibrance, &Spec::centred(100.0).colors(st::saturation_stops()));
    slider(ui, &mut e.dirty, "saturation", "Saturation", &mut e.p.saturation, &Spec::centred(100.0).colors(st::saturation_stops()));
}

fn effects(ui: &mut Ui, e: &mut Edit) {
    slider(ui, &mut e.dirty, "texture", "Texture", &mut e.p.texture, &Spec::centred(100.0));
    slider(ui, &mut e.dirty, "clarity", "Clarity", &mut e.p.clarity, &Spec::centred(100.0));
    slider(ui, &mut e.dirty, "dehaze", "Dehaze", &mut e.p.dehaze, &Spec::centred(100.0));
    let mut open = e.group_open("vignette");
    let p = &mut *e.p;
    let active = p.vignette_amount != 0.0;
    let changed = st::group(
        ui,
        "vignette",
        &mut open,
        |ui| st::slider(ui, "vignette", tr("Vignette"), &mut p.vignette_amount, &Spec::centred(100.0)).changed,
        |ui| {
            let mut ch = false;
            ui.horizontal(|ui| {
                ui.add_space(st::INSET_L);
                ui.set_min_height(28.0);
                ui.add_enabled_ui(active, |ui| {
                    ui.label(egui::RichText::new(tr("Style")).font(st::label_font()));
                    ui.add_space(8.0);
                    let options = [
                        ("highlightPriority".to_string(), tr("Highlight Priority")),
                        ("colorPriority".to_string(), tr("Color Priority")),
                        ("paintOverlay".to_string(), tr("Paint Overlay")),
                    ];
                    ch |= crate::widgets::dropdown(ui, "cr-vig-style", &mut p.vignette_style, &options, 150.0);
                });
            });
            ch |= st::slider(ui, "vignetteMidpoint", tr("Midpoint"), &mut p.vignette_midpoint, &Spec::amount(100.0, 50.0).enabled(active)).changed;
            ch |= st::slider(ui, "vignetteRoundness", tr("Roundness"), &mut p.vignette_roundness, &Spec::centred(100.0).enabled(active)).changed;
            ch |= st::slider(ui, "vignetteFeather", tr("Feather"), &mut p.vignette_feather, &Spec::amount(100.0, 50.0).enabled(active)).changed;
            ch |= st::slider(ui, "vignetteHighlights", tr("Highlights"), &mut p.vignette_highlights, &Spec::amount(100.0, 0.0).enabled(active)).changed;
            ch
        },
    );
    e.dirty |= changed;
    e.set_group("vignette", open);
    let mut open = e.group_open("grain");
    let p = &mut *e.p;
    let active = p.grain_amount != 0.0;
    let changed = st::group(
        ui,
        "grain",
        &mut open,
        |ui| st::slider(ui, "grain", tr("Grain"), &mut p.grain_amount, &Spec::amount(100.0, 0.0)).changed,
        |ui| {
            let mut ch = st::slider(ui, "grainSize", tr("Size"), &mut p.grain_size, &Spec::amount(100.0, 25.0).enabled(active)).changed;
            ch |= st::slider(ui, "grainRoughness", tr("Roughness"), &mut p.grain_roughness, &Spec::amount(100.0, 50.0).enabled(active)).changed;
            ch
        },
    );
    e.dirty |= changed;
    e.set_group("grain", open);
}

/// The curve being edited: parametric, or the point curve of the master, red, green or blue
/// channel.
fn point_of<'a>(p: &'a mut CameraRaw, mode: &str) -> &'a mut Vec<[f32; 2]> {
    match mode {
        "red" => &mut p.point_curve_red,
        "green" => &mut p.point_curve_green,
        "blue" => &mut p.point_curve_blue,
        _ => &mut p.point_curve,
    }
}

fn curve(
    ui: &mut Ui,
    e: &mut Edit,
    histogram: &photocraft_algo::histogram::RgbHistogram,
    state: &mut crate::state::PointCurveState,
    curve_rect: &mut Option<Rect>,
) {
    let c = Acr::get(ui.ctx());
    const MODES: [&str; 5] = ["parametric", "point", "red", "green", "blue"];
    let mode = MODES.iter().position(|m| *m == e.layout.curve).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 6.0);
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(tr("Adjust")).font(st::label_font()).color(c.label));
        let tips = [tr("Parametric Curve"), tr("Point Curve"), tr("Red Channel"), tr("Green Channel"), tr("Blue Channel")];
        let rings = [c.label, Color32::from_gray(200), Color32::from_rgb(220, 50, 50), Color32::from_rgb(50, 170, 60), Color32::from_rgb(40, 110, 230)];
        for (i, tip) in tips.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            let resp = resp.on_hover_text(*tip);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, mode == i, *tip));
            let p = ui.painter();
            if i == 0 {
                // A swash: the parametric icon.
                let pts: Vec<Pos2> = (0..=12)
                    .map(|k| {
                        let t = k as f32 / 12.0;
                        pos2(r.left() + 5.0 + t * 14.0, r.center().y + 6.0 - (t * std::f32::consts::PI).sin() * 3.0 - t * 9.0)
                    })
                    .collect();
                p.add(egui::Shape::line(pts, Stroke::new(if mode == i { 2.2 } else { 1.5 }, if mode == i { c.text } else { c.label })));
            } else {
                p.circle_stroke(r.center(), 7.0, Stroke::new(1.6, rings[i]));
                if mode == i {
                    p.circle_filled(r.center(), 4.5, rings[i]);
                }
            }
            if resp.clicked() {
                e.layout.curve = MODES[i].to_owned();
            }
        }
    });
    let side = (ui.available_width() - st::INSET_L * 2.0).clamp(100.0, 230.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side + 4.0), Sense::hover());
    let rect = Rect::from_min_size(pos2(row.left() + st::INSET_L - 2.0, row.top() + 2.0), vec2(side, side));
    let resp = ui.interact(rect, ui.id().with("cr-curve"), Sense::click_and_drag());
    *curve_rect = Some(rect);
    let p = ui.painter_at(rect);
    // Background: a ramp in the channel's colour, the histogram, a 4 × 4 grid.
    let tint = match MODES[mode] {
        "red" => Some(Color32::from_rgb(190, 60, 50)),
        "green" => Some(Color32::from_rgb(60, 170, 70)),
        "blue" => Some(Color32::from_rgb(50, 90, 210)),
        _ => None,
    };
    let mut mesh = egui::Mesh::default();
    let (tl, br) = match tint {
        Some(t) => (t, Color32::from_rgb(255 - t.r(), 255 - t.g(), 255 - t.b()).gamma_multiply(0.85)),
        None => (Color32::from_gray(120), Color32::from_gray(52)),
    };
    mesh.colored_vertex(rect.left_top(), tl);
    mesh.colored_vertex(rect.right_top(), Color32::from_gray(96));
    mesh.colored_vertex(rect.right_bottom(), br);
    mesh.colored_vertex(rect.left_bottom(), Color32::from_gray(80));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
    paint_luma_histogram(&p, rect, histogram);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        p.line_segment(
            [pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())],
            Stroke::new(1.0, Color32::from_black_alpha(140)),
        );
        p.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], Stroke::new(1.0, Color32::from_black_alpha(140)));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    if mode == 0 {
        let lut = parametric_preview(e.p);
        let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, v)| to_screen(i as f32, v * 255.0)).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6, Color32::WHITE)));
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, c.rule), egui::StrokeKind::Outside);
        splits(ui, rect, e);
        slider(ui, &mut e.dirty, "curveHighlights", "Highlights", &mut e.p.curve_highlights, &Spec::centred(100.0));
        slider(ui, &mut e.dirty, "curveLights", "Lights", &mut e.p.curve_lights, &Spec::centred(100.0));
        slider(ui, &mut e.dirty, "curveDarks", "Darks", &mut e.p.curve_darks, &Spec::centred(100.0));
        slider(ui, &mut e.dirty, "curveShadows", "Shadows", &mut e.p.curve_shadows, &Spec::centred(100.0));
        return;
    }
    let curve = point_of(e.p, MODES[mode]);
    let mut points = if curve.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { curve.clone() };
    if crate::point_curve::interact(ui, &resp, rect, &mut points, state).changed {
        *curve = points.clone();
        e.dirty = true;
    }
    let lut = curve_lut(&points, 256);
    let line: Vec<Pos2> = lut.iter().enumerate().map(|(i, v)| to_screen(i as f32, v * 255.0)).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.6, Color32::WHITE)));
    for (i, q) in points.iter().enumerate() {
        let center = to_screen(q[0], q[1]);
        p.circle_filled(center, 4.0, if state.selected == Some(i) { Color32::WHITE } else { Color32::from_gray(30) });
        p.circle_stroke(center, 4.0, Stroke::new(1.4, Color32::WHITE));
    }
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, c.rule), egui::StrokeKind::Outside);
    if mode == 1 {
        let mut refine = e.p.curve_refine_saturation;
        if st::slider(
            ui,
            "refineSaturation",
            tr("Refine Saturation"),
            &mut refine,
            &Spec::amount(100.0, 100.0).enabled(!crate::camera_raw_ui::is_linear(&e.p.point_curve)),
        )
        .changed
        {
            e.p.curve_refine_saturation = refine;
            e.dirty = true;
        }
    }
    // Input / Output of the selected point.
    let sel = state.selected.and_then(|i| points.get(i).copied());
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.set_min_height(28.0);
        for (k, label) in [tr("Input:"), tr("Output:")].iter().enumerate() {
            ui.label(egui::RichText::new(*label).font(st::label_font()).color(c.label));
            let mut v = sel.map_or(0.0, |q| q[k]);
            let r = ui.add_enabled(sel.is_some(), egui::DragValue::new(&mut v).range(0.0..=255.0).speed(1.0).fixed_decimals(0));
            if r.changed()
                && let Some(i) = state.selected
                && let Some(mut q) = points.get(i).copied()
            {
                q[k] = v;
                if crate::point_curve::move_to(&mut points, i, q) {
                    *point_of(e.p, MODES[mode]) = points.clone();
                    e.dirty = true;
                }
            }
            ui.add_space(14.0);
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L + 20.0);
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(tr("Point Curve")).font(st::label_font()).color(c.label));
        ui.add_space(6.0);
        let presets: [(usize, &str); 4] = [(0, tr("Linear")), (1, tr("Medium Contrast")), (2, tr("Strong Contrast")), (3, tr("Custom"))];
        let shapes: [&[[f32; 2]]; 3] = [
            &[],
            &[[0.0, 0.0], [64.0, 56.0], [128.0, 128.0], [192.0, 196.0], [255.0, 255.0]],
            &[[0.0, 0.0], [64.0, 48.0], [128.0, 128.0], [192.0, 206.0], [255.0, 255.0]],
        ];
        let current = point_of(e.p, MODES[mode]).clone();
        let mut chosen = shapes.iter().position(|s| (s.is_empty() && crate::camera_raw_ui::is_linear(&current)) || **s == current[..]).unwrap_or(3);
        if crate::widgets::dropdown(ui, "cr-point-preset", &mut chosen, &presets, 120.0)
            && let Some(shape) = shapes.get(chosen)
        {
            *point_of(e.p, MODES[mode]) = shape.to_vec();
            *state = Default::default();
            e.dirty = true;
        }
    });
}

/// The parametric tone curve the pipeline applies, 256 entries over 0..=1.
fn parametric_preview(p: &CameraRaw) -> Vec<f32> {
    photocraft_algo::camera_raw::parametric_lut(p, 256)
}

/// The strip under the parametric curve: a gray ramp with the three region split handles.
fn splits(ui: &mut Ui, rect: Rect, e: &mut Edit) {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let strip = Rect::from_min_max(pos2(rect.left(), row.top() + 2.0), pos2(rect.right(), row.top() + 12.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(strip.left_top(), Color32::BLACK);
    mesh.colored_vertex(strip.right_top(), Color32::from_gray(230));
    mesh.colored_vertex(strip.right_bottom(), Color32::from_gray(230));
    mesh.colored_vertex(strip.left_bottom(), Color32::BLACK);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
    for k in 0..3 {
        let x = strip.left() + e.p.curve_splits[k] / 100.0 * strip.width();
        let hit = Rect::from_center_size(pos2(x, strip.center().y), vec2(12.0, 16.0));
        let r = ui.interact(hit, ui.id().with(("cr-split", k)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            let lo = if k == 0 { 5.0 } else { e.p.curve_splits[k - 1] + 5.0 };
            let hi = if k == 2 { 95.0 } else { e.p.curve_splits[k + 1] - 5.0 };
            let v = ((p.x - strip.left()) / strip.width().max(1.0) * 100.0).round().clamp(lo, hi.max(lo));
            if v != e.p.curve_splits[k] {
                e.p.curve_splits[k] = v;
                e.dirty = true;
            }
        }
        if r.double_clicked() {
            e.p.curve_splits[k] = [25.0, 50.0, 75.0][k];
            e.dirty = true;
        }
        let center = pos2(x, strip.center().y);
        ui.painter().circle_filled(center, 4.5, Color32::WHITE);
        ui.painter().circle_stroke(center, 4.5, Stroke::new(1.5, c.knob_edge));
    }
}

/// Luminance histogram in the curve's background.
fn paint_luma_histogram(p: &egui::Painter, rect: Rect, h: &photocraft_algo::histogram::RgbHistogram) {
    let bins = h.channels[1].len().max(1);
    let lum: Vec<f32> = (0..bins)
        .map(|i| {
            let g = |c: usize| h.channels.get(c).and_then(|v| v.get(i)).copied().unwrap_or(0) as f32;
            0.3 * g(0) + 0.59 * g(1) + 0.11 * g(2)
        })
        .collect();
    let max = lum.iter().copied().fold(0.0f32, f32::max).max(1.0);
    let mut pts = vec![rect.left_bottom()];
    for (i, v) in lum.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) / bins as f32 * rect.width();
        pts.push(pos2(x, rect.bottom() - (v / max).sqrt() * rect.height() * 0.62));
    }
    pts.push(rect.right_bottom());
    // Columns rather than a polygon: the outline is concave.
    for w in pts.windows(2) {
        let r = Rect::from_min_max(pos2(w[0].x, w[1].y), pos2(w[1].x, rect.bottom()));
        p.rect_filled(r, 0.0, Color32::from_black_alpha(70));
    }
}

fn mixer(ui: &mut Ui, e: &mut Edit) {
    let tabs = [tr("Mixer"), tr("Point Color")];
    let sel = usize::from(e.layout.mixer == "pointColor");
    if let Some(i) = st::text_tabs(ui, "cr-mixer-tabs", &tabs, sel, !e.p.monochrome) {
        e.layout.mixer = if i == 1 { "pointColor".into() } else { "mixer".into() };
    }
    ui.add_space(4.0);
    if e.p.monochrome {
        st::caption(ui, tr("B&W Mix"));
        for (k, name) in BANDS.iter().enumerate() {
            slider(ui, &mut e.dirty, &format!("gray{k}"), name, &mut e.p.gray_mixer[k], &Spec::centred(100.0).colors(st::band_stops(k)));
        }
        return;
    }
    if e.layout.mixer == "pointColor" {
        point_color(ui, e);
        return;
    }
    let c = Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(tr("Adjust")).font(st::label_font()).color(c.label));
        ui.add_space(6.0);
        let mut adjust = e.layout.mixer_adjust.clone();
        if crate::widgets::dropdown(ui, "cr-mixer-adjust", &mut adjust, &[("hsl".to_string(), tr("HSL")), ("color".to_string(), tr("Color"))], 120.0) {
            e.layout.mixer_adjust = adjust;
        }
        let tat = crate::icons::button(ui, "scan-eye", 22.0, false, tl!("Targeted Adjustment Tool: drag in the image to adjust the colour under the pointer"));
        tat.on_disabled_hover_text(tl!("Not available yet"));
    });
    if e.layout.mixer_adjust == "color" {
        // One colour at a time: eight swatches, then its hue, saturation and luminance.
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let w = (row.width() - st::INSET_L * 2.0) / 8.0;
        for (k, band) in BANDS.iter().enumerate() {
            let r = Rect::from_center_size(pos2(row.left() + st::INSET_L + w * (k as f32 + 0.5), row.center().y), vec2(16.0, 16.0));
            let resp = ui.interact(r, ui.id().with(("cr-swatch", k)), Sense::click()).on_hover_text(tr(band));
            let col = st::hsv(photocraft_algo::camera_raw::HSL_BANDS[k], 0.85, 0.85);
            ui.painter().circle_filled(r.center(), 7.0, col);
            if e.layout.mixer_color == k {
                ui.painter().circle_stroke(r.center(), 9.5, Stroke::new(1.5, c.text));
            }
            if resp.clicked() {
                e.layout.mixer_color = k;
            }
        }
        let k = e.layout.mixer_color.min(7);
        slider(ui, &mut e.dirty, "colorHue", "Hue", &mut e.p.hsl_hue[k], &Spec::centred(100.0).colors(st::band_stops(k)));
        let base = st::hsv(photocraft_algo::camera_raw::HSL_BANDS[k], 0.85, 0.85);
        slider(ui, &mut e.dirty, "colorSat", "Saturation", &mut e.p.hsl_sat[k], &Spec::centred(100.0).colors(vec![Color32::from_gray(130), base]));
        slider(ui, &mut e.dirty, "colorLum", "Luminance", &mut e.p.hsl_lum[k], &Spec::centred(100.0).colors(vec![Color32::BLACK, base, Color32::WHITE]));
        return;
    }
    const TABS: [&str; 4] = ["hue", "saturation", "luminance", "all"];
    let labels = [tr("Hue"), tr("Saturation"), tr("Luminance"), tr("All")];
    let tab = TABS.iter().position(|t| *t == e.layout.mixer_tab).unwrap_or(0);
    if let Some(i) = st::text_tabs(ui, "cr-hsl-tabs", &labels, tab, true) {
        e.layout.mixer_tab = TABS[i].into();
    }
    ui.add_space(4.0);
    let which: Vec<usize> = if tab == 3 { vec![0, 1, 2] } else { vec![tab] };
    for t in which {
        if tab == 3 {
            st::caption(ui, labels[t]);
        }
        for (k, name) in BANDS.iter().enumerate() {
            let base = st::hsv(photocraft_algo::camera_raw::HSL_BANDS[k], 0.62, 0.78);
            let (v, stops) = match t {
                0 => (&mut e.p.hsl_hue[k], st::band_stops(k)),
                1 => (&mut e.p.hsl_sat[k], vec![Color32::from_gray(140), base]),
                _ => (&mut e.p.hsl_lum[k], vec![Color32::from_gray(40), base, Color32::from_gray(235)]),
            };
            slider(ui, &mut e.dirty, &format!("hsl{t}{k}"), name, v, &Spec::centred(100.0).colors(stops));
        }
    }
}

/// Point Color (Camera Raw 15+): samples picked from the image get their own hue, saturation and
/// luminance shifts. Not available yet: the layout is in place, the controls are disabled.
fn point_color(ui: &mut Ui, e: &mut Edit) {
    let c = Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 4.0);
        ui.add_enabled(false, egui::Button::image(crate::icons::image("pipette", 15.0, c.label)).frame(false));
    });
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 108.0), Sense::hover());
    let well = Rect::from_min_max(pos2(row.left() + st::INSET_L - 4.0, row.top()), pos2(row.right() - st::INSET_R - 18.0, row.bottom()));
    ui.painter().rect_filled(well, 0.0, Color32::from_gray(96));
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(well.right() + 4.0, well.top()), pos2(row.right() - st::INSET_R, well.bottom())),
        0.0,
        Color32::from_gray(96),
    );
    ui.painter().text(well.center(), Align2::CENTER_CENTER, tl!("Use the Point Color Dropper\nto add samples."), st::small_font(), c.text);
    ui.add_space(18.0);
    let mut zero = [0.0f32; 3];
    for (k, label) in ["Hue", "Saturation", "Luminance"].iter().enumerate() {
        st::slider(ui, &format!("pointColor{k}"), tr(label), &mut zero[k], &Spec::centred(100.0).enabled(false));
    }
    let mut range = 50.0;
    st::slider(ui, "pointColorRange", tr("Range"), &mut range, &Spec::amount(100.0, 50.0).enabled(false));
    let _ = e;
}

fn grading(ui: &mut Ui, e: &mut Edit) {
    let c = Acr::get(ui.ctx());
    const VIEWS: [&str; 5] = ["threeWay", "shadows", "midtones", "highlights", "global"];
    let view = VIEWS.iter().position(|v| *v == e.layout.grading).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 6.0);
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(tr("Adjust")).font(st::label_font()).color(c.label));
        let tips = [tr("3-Way"), tr("Shadows"), tr("Midtones"), tr("Highlights"), tr("Global")];
        for (i, tip) in tips.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            let resp = resp.on_hover_text(*tip);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, view == i, *tip));
            let p = ui.painter();
            let col = if view == i { c.text } else { c.label };
            match i {
                0 => {
                    for (dx, dy) in [(-3.5, -3.0), (3.5, -3.0), (0.0, 3.5)] {
                        p.circle_filled(r.center() + vec2(dx, dy), 3.3, col);
                    }
                }
                1 => {
                    p.circle_filled(r.center(), 7.0, Color32::from_gray(20));
                    p.circle_stroke(r.center(), 7.0, Stroke::new(1.3, col));
                }
                2 => {
                    p.circle_filled(r.center(), 7.0, Color32::from_gray(140));
                }
                3 => {
                    p.circle_filled(r.center(), 7.0, Color32::from_gray(235));
                }
                _ => {
                    p.circle_stroke(r.center(), 7.0, Stroke::new(1.3, col));
                    let half = Rect::from_min_max(r.center() - vec2(7.0, 7.0), r.center() + vec2(0.0, 7.0));
                    p.with_clip_rect(half).circle_filled(r.center(), 7.0, col);
                }
            }
            if view == i {
                p.rect_stroke(r, 3.0, Stroke::new(1.0, c.field_border), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                e.layout.grading = VIEWS[i].into();
            }
            if i == 0 || i == 3 {
                ui.add_space(4.0);
            }
        }
    });
    let p = &mut *e.p;
    let mut ch = false;
    let w = ui.available_width();
    match view {
        0 => {
            st::caption(ui, tr("Midtones"));
            ch |= wheel(ui, "midtones", &mut p.grade_midtones, w * 0.19, false);
            ui.columns(2, |cols| {
                st::caption(&mut cols[0], tr("Shadows"));
                ch |= wheel(&mut cols[0], "shadows", &mut p.grade_shadows, w * 0.19, false);
                st::caption(&mut cols[1], tr("Highlights"));
                ch |= wheel(&mut cols[1], "highlights", &mut p.grade_highlights, w * 0.19, false);
            });
        }
        k => {
            let (title, wh) = match k {
                1 => ("Shadows", &mut p.grade_shadows),
                2 => ("Midtones", &mut p.grade_midtones),
                3 => ("Highlights", &mut p.grade_highlights),
                _ => ("Global", &mut p.grade_global),
            };
            st::caption(ui, tr(title));
            ch |= wheel(ui, title, wh, w * 0.33, true);
            ch |= st::slider(
                ui,
                &format!("{title}Hue"),
                tr("Hue"),
                &mut wh.hue,
                &Spec::range(0.0, 359.0, 0.0).colors((0..7).map(|i| st::hsv(i as f32 * 60.0, 0.6, 0.8)).collect()),
            )
            .changed;
            ch |= st::slider(ui, &format!("{title}Sat"), tr("Saturation"), &mut wh.sat, &Spec::amount(100.0, 0.0)).changed;
        }
    }
    e.dirty |= ch;
    st::rule(ui);
    ui.add_space(4.0);
    slider(ui, &mut e.dirty, "gradeBlending", "Blending", &mut e.p.grade_blending, &Spec::amount(100.0, 50.0));
    slider(ui, &mut e.dirty, "gradeBalance", "Balance", &mut e.p.grade_balance, &Spec::centred(100.0));
}

/// A colour wheel with its hue/saturation handle and the luminance slider below. The handle's
/// angle is the hue, its distance from the centre the saturation; double-click the wheel to
/// reset hue and saturation.
fn wheel(ui: &mut Ui, id: &str, w: &mut Wheel, radius: f32, labelled: bool) -> bool {
    let c = Acr::get(ui.ctx());
    let radius = radius.clamp(30.0, 100.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), radius * 2.0 + 10.0), Sense::hover());
    let center = row.center();
    let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    let resp = ui.interact(rect.expand(6.0), ui.id().with(("cr-wheel", id)), Sense::click_and_drag());
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let v = p - center;
        let sat = (v.length() / radius * 100.0).clamp(0.0, 100.0);
        // Hue 0 (red) at 3 o'clock, counter-clockwise.
        let hue = (-v.y).atan2(v.x).to_degrees().rem_euclid(360.0);
        if (sat - w.sat).abs() > 0.01 || (hue - w.hue).abs() > 0.01 {
            w.sat = sat.round();
            w.hue = hue.round().rem_euclid(360.0);
            changed = true;
        }
    }
    if resp.double_clicked() {
        w.sat = 0.0;
        w.hue = 0.0;
        changed = true;
    }
    let p = ui.painter();
    let mut mesh = egui::Mesh::default();
    const SEG: usize = 72;
    // Camera Raw's wheels are muted: a mid-gray centre to moderately saturated rims.
    mesh.colored_vertex(center, Color32::from_gray(96));
    for i in 0..=SEG {
        let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + vec2(a.cos(), -a.sin()) * radius, st::hsv(a.to_degrees(), 0.72, 0.62));
    }
    for i in 0..SEG as u32 {
        mesh.add_triangle(0, i + 1, i + 2);
    }
    p.add(mesh);
    let a = w.hue.to_radians();
    let handle = center + vec2(a.cos(), -a.sin()) * (w.sat.clamp(0.0, 100.0) / 100.0 * radius);
    p.circle_stroke(center, 4.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
    p.circle_filled(handle, 4.0, Color32::from_gray(40));
    p.circle_stroke(handle, 4.0, Stroke::new(1.5, Color32::WHITE));
    // The hue handle on the rim (sat shown by the inner handle).
    p.circle_filled(center + vec2(a.cos(), -a.sin()) * (radius + 6.0), 3.0, c.label);
    let mut lum = w.lum;
    let r = if labelled {
        st::slider(ui, &format!("{id}Lum"), tr("Luminance"), &mut lum, &Spec::centred(100.0))
    } else {
        // Under the 3-way wheels Camera Raw shows the luminance track alone.
        st::bare_slider(ui, &format!("{id}Lum"), tr("Luminance"), &mut lum, &Spec::centred(100.0))
    };
    if r.changed {
        w.lum = lum;
        changed = true;
    }
    changed
}

fn detail(ui: &mut Ui, e: &mut Edit) {
    let pairs: [(&str, &str); 3] = [("sharpening", "Sharpening"), ("noiseReduction", "Noise Reduction"), ("colorNoiseReduction", "Color Noise Reduction")];
    for (gid, title) in pairs {
        let mut open = e.group_open(gid);
        let p = &mut *e.p;
        let changed = match gid {
            "sharpening" => {
                let active = p.sharpen_amount > 0.0;
                st::group(
                    ui,
                    gid,
                    &mut open,
                    |ui| st::slider(ui, "sharpenAmount", tr(title), &mut p.sharpen_amount, &Spec::amount(150.0, 0.0)).changed,
                    |ui| {
                        let mut ch =
                            st::slider(ui, "sharpenRadius", tr("Radius"), &mut p.sharpen_radius, &Spec::range(0.5, 3.0, 1.0).decimals(1).enabled(active))
                                .changed;
                        ch |= st::slider(ui, "sharpenDetail", tr("Detail"), &mut p.sharpen_detail, &Spec::amount(100.0, 25.0).enabled(active)).changed;
                        ch |= st::slider(ui, "sharpenMasking", tr("Masking"), &mut p.sharpen_masking, &Spec::amount(100.0, 0.0).enabled(active)).changed;
                        ch
                    },
                )
            }
            "noiseReduction" => {
                let active = p.noise_luminance > 0.0;
                st::group(
                    ui,
                    gid,
                    &mut open,
                    |ui| st::slider(ui, "noiseLuminance", tr(title), &mut p.noise_luminance, &Spec::amount(100.0, 0.0)).changed,
                    |ui| {
                        let mut ch =
                            st::slider(ui, "noiseLuminanceDetail", tr("Detail"), &mut p.noise_luminance_detail, &Spec::amount(100.0, 50.0).enabled(active))
                                .changed;
                        ch |= st::slider(
                            ui,
                            "noiseLuminanceContrast",
                            tr("Contrast"),
                            &mut p.noise_luminance_contrast,
                            &Spec::amount(100.0, 0.0).enabled(active),
                        )
                        .changed;
                        ch
                    },
                )
            }
            _ => {
                let active = p.noise_color > 0.0;
                st::group(
                    ui,
                    gid,
                    &mut open,
                    |ui| st::slider(ui, "noiseColor", tr(title), &mut p.noise_color, &Spec::amount(100.0, 0.0)).changed,
                    |ui| {
                        let mut ch =
                            st::slider(ui, "noiseColorDetail", tr("Detail"), &mut p.noise_color_detail, &Spec::amount(100.0, 50.0).enabled(active)).changed;
                        ch |=
                            st::slider(ui, "noiseColorSmoothness", tr("Smoothness"), &mut p.noise_color_smoothness, &Spec::amount(100.0, 50.0).enabled(active))
                                .changed;
                        ch
                    },
                )
            }
        };
        e.dirty |= changed;
        e.set_group(gid, open);
    }
    ui.add_space(4.0);
    st::note(ui, tl!("For a more accurate preview, zoom the preview size to 100% or larger when adjusting the controls in this panel."), false);
}

fn optics(ui: &mut Ui, e: &mut Edit) {
    slider(ui, &mut e.dirty, "lensDistortion", "Distortion", &mut e.p.lens_distortion, &Spec::centred(100.0));
    let mut open = e.group_open("lensVignette");
    let p = &mut *e.p;
    let active = p.lens_vignette != 0.0;
    e.dirty |= st::group(
        ui,
        "lensVignette",
        &mut open,
        |ui| st::slider(ui, "lensVignette", tr("Vignette"), &mut p.lens_vignette, &Spec::centred(100.0)).changed,
        |ui| st::slider(ui, "lensVignetteMidpoint", tr("Midpoint"), &mut p.lens_vignette_midpoint, &Spec::amount(100.0, 50.0).enabled(active)).changed,
    );
    e.set_group("lensVignette", open);
    let mut open = e.group_open("defringe");
    let p = &mut *e.p;
    e.dirty |= st::boxed_group(ui, "defringe", "droplet", tr("Defringe"), &mut open, |ui| {
        let c = Acr::get(ui.ctx());
        ui.horizontal(|ui| {
            ui.add_space(st::INSET_L);
            ui.set_min_height(26.0);
            ui.label(egui::RichText::new(tr("Sample fringe")).font(st::label_font()).color(c.label));
            ui.add_enabled(false, egui::Button::image(crate::icons::image("pipette", 14.0, c.label)).frame(false))
                .on_disabled_hover_text(tl!("Not available yet"));
        });
        st::note(ui, tl!("Use the eyedropper to sample purple and green areas of fringing that need correction."), true);
        let mut ch = st::slider(ui, "defringePurple", tr("Purple Amount"), &mut p.defringe_purple_amount, &Spec::amount(20.0, 0.0)).changed;
        ch |= hue_range(ui, "defringePurpleHue", tr("Purple Hue"), &mut p.defringe_purple_hue, [260.0, 340.0], 10.0);
        ch |= st::slider(ui, "defringeGreen", tr("Green Amount"), &mut p.defringe_green_amount, &Spec::amount(20.0, 0.0)).changed;
        ch |= hue_range(ui, "defringeGreenHue", tr("Green Hue"), &mut p.defringe_green_hue, [60.0, 180.0], 10.0);
        ch
    });
    e.set_group("defringe", open);
}

/// A two-knob range slider (Defringe hue windows): `lo / hi` in 0..=100 at least `gap` apart,
/// over a hue ramp spanning `hues` (degrees).
fn hue_range(ui: &mut Ui, id: &str, label: &str, v: &mut [f32; 2], hues: [f32; 2], gap: f32) -> bool {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::SLIDER_ROW + 2.0), Sense::hover());
    let p = ui.painter().clone();
    p.text(pos2(row.left() + st::INSET_L + 2.0, row.top() + 9.0), Align2::LEFT_CENTER, label, st::label_font(), c.label);
    p.text(pos2(row.right() - st::INSET_R, row.top() + 9.0), Align2::RIGHT_CENTER, format!("{:.0} / {:.0}", v[0], v[1]), st::label_font(), c.text);
    let track = Rect::from_min_max(pos2(row.left() + st::INSET_L, row.top() + 21.0), pos2(row.right() - st::INSET_R, row.top() + 24.0));
    let stops: Vec<Color32> = (0..7).map(|i| st::hsv(hues[0] + (hues[1] - hues[0]) * i as f32 / 6.0, 0.7, 0.85)).collect();
    let mut mesh = egui::Mesh::default();
    for (i, s) in stops.iter().enumerate() {
        let x = track.left() + track.width() * i as f32 / 6.0;
        mesh.colored_vertex(pos2(x, track.top()), *s);
        mesh.colored_vertex(pos2(x, track.bottom()), *s);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    p.add(mesh);
    let mut changed = false;
    for k in 0..2 {
        let x = track.left() + v[k].clamp(0.0, 100.0) / 100.0 * track.width();
        let r = ui.interact(Rect::from_center_size(pos2(x, track.center().y), vec2(12.0, 16.0)), ui.id().with((id, k)), Sense::drag());
        if r.dragged()
            && let Some(q) = r.interact_pointer_pos()
        {
            let nv = ((q.x - track.left()) / track.width().max(1.0) * 100.0).round();
            let nv = if k == 0 { nv.clamp(0.0, v[1] - gap) } else { nv.clamp(v[0] + gap, 100.0) };
            if nv != v[k] {
                v[k] = nv;
                changed = true;
            }
        }
        st::knob(&p, pos2(x, track.top() - 3.0), c.knob, c.knob_edge);
    }
    p.line_segment(
        [pos2(track.left() + v[0] / 100.0 * track.width(), track.top() - 4.0), pos2(track.left() + v[1] / 100.0 * track.width(), track.top() - 4.0)],
        Stroke::new(1.0, c.text),
    );
    changed
}

fn geometry(ui: &mut Ui, e: &mut Edit) {
    st::caption(ui, tr("Upright"));
    const MODES: [&str; 6] = ["off", "auto", "level", "vertical", "full", "guided"];
    let tips = [
        tr("Disable Upright"),
        tr("Auto: apply balanced perspective corrections"),
        tr("Level: apply only level correction"),
        tr("Vertical: apply level and vertical perspective correction"),
        tr("Full: apply level, horizontal, and vertical perspective corrections"),
        tr("Guided: draw two or more guides to customize perspective corrections"),
    ];
    let sel = MODES.iter().position(|m| *m == e.p.upright);
    let clicked = st::icon_strip(ui, "cr-upright", 6, sel, 28.0, &tips, |p, r, i, col| {
        let s = Stroke::new(1.4, col);
        let b = r.shrink(7.0);
        match i {
            0 => {
                p.circle_stroke(r.center(), 7.0, s);
                p.line_segment([r.center() + vec2(-5.0, -5.0), r.center() + vec2(5.0, 5.0)], s);
            }
            1 => {
                p.text(r.center(), Align2::CENTER_CENTER, "A", crate::theme::semibold(15.0), col);
            }
            2 => {
                p.rect_stroke(b, 0.0, s, egui::StrokeKind::Inside);
                p.line_segment([b.left_center(), b.right_center()], s);
            }
            3 => {
                p.rect_stroke(b, 0.0, s, egui::StrokeKind::Inside);
                for f in [0.33, 0.66] {
                    p.line_segment([pos2(b.left() + b.width() * f, b.top()), pos2(b.left() + b.width() * f, b.bottom())], s);
                }
            }
            4 => {
                p.rect_stroke(b, 0.0, s, egui::StrokeKind::Inside);
                for f in [0.33, 0.66] {
                    p.line_segment([pos2(b.left() + b.width() * f, b.top()), pos2(b.left() + b.width() * f, b.bottom())], s);
                    p.line_segment([pos2(b.left(), b.top() + b.height() * f), pos2(b.right(), b.top() + b.height() * f)], s);
                }
            }
            _ => {
                p.line_segment([pos2(b.left() + 3.0, b.top()), pos2(b.left() + 3.0, b.bottom())], s);
                p.line_segment([pos2(b.left(), b.bottom() - 3.0), pos2(b.right(), b.bottom() - 3.0)], s);
                p.line_segment([pos2(b.right() - 3.0, b.top()), pos2(b.right() - 3.0, b.bottom() - 3.0)], s);
            }
        }
    });
    if let Some(i) = clicked {
        let mode = if sel == Some(i) && i != 0 { "off" } else { MODES[i] };
        if e.p.upright != mode {
            e.p.upright = mode.into();
            e.dirty = true;
        }
    }
    st::note(ui, tl!("For best results, apply lens corrections first."), true);
    let mut open = e.group_open("manualTransformations");
    let p = &mut *e.p;
    e.dirty |= st::boxed_group(ui, "manualTransformations", "scaling", tr("Manual Transformations"), &mut open, |ui| {
        let mut ch = st::slider(ui, "transformVertical", tr("Vertical"), &mut p.transform_vertical, &Spec::centred(100.0)).changed;
        ch |= st::slider(ui, "transformHorizontal", tr("Horizontal"), &mut p.transform_horizontal, &Spec::centred(100.0)).changed;
        ch |= st::slider(ui, "transformRotate", tr("Rotate"), &mut p.transform_rotate, &Spec::centred(10.0).decimals(1)).changed;
        ch |= st::slider(ui, "transformAspect", tr("Aspect"), &mut p.transform_aspect, &Spec::centred(100.0)).changed;
        ch |= st::slider(ui, "transformScale", tr("Scale"), &mut p.transform_scale, &Spec::range(50.0, 150.0, 100.0)).changed;
        ch |= st::slider(ui, "transformOffsetX", tr("Offset X"), &mut p.transform_offset_x, &Spec::centred(100.0).decimals(1)).changed;
        ch |= st::slider(ui, "transformOffsetY", tr("Offset Y"), &mut p.transform_offset_y, &Spec::centred(100.0).decimals(1)).changed;
        ch
    });
    e.set_group("manualTransformations", open);
}

fn calibration(ui: &mut Ui, e: &mut Edit) {
    let mut process = e.p.process_version;
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.set_min_height(30.0);
        let c = Acr::get(ui.ctx());
        ui.label(egui::RichText::new(tr("Process")).font(st::label_font()).color(c.label));
        ui.add_space(8.0);
        let options =
            [(6u32, tr("Version 6 (Current)")), (5, tr("Version 5")), (4, tr("Version 4")), (3, tr("Version 3")), (2, tr("Version 2")), (1, tr("Version 1"))];
        if crate::widgets::dropdown(ui, "cr-process", &mut process, &options, 150.0) {
            e.p.process_version = process;
            e.dirty = true;
        }
    });
    st::caption(ui, tr("Shadows"));
    slider(ui, &mut e.dirty, "shadowsTint", "Tint", &mut e.p.calibration_shadows_tint, &Spec::centred(100.0).colors(st::tint_stops()));
    let p = &mut *e.p;
    let rows: [(&str, f32, &mut f32, &mut f32); 3] = [
        ("Red Primary", 0.0, &mut p.calibration_red_hue, &mut p.calibration_red_sat),
        ("Green Primary", 120.0, &mut p.calibration_green_hue, &mut p.calibration_green_sat),
        ("Blue Primary", 240.0, &mut p.calibration_blue_hue, &mut p.calibration_blue_sat),
    ];
    let mut ch = false;
    for (title, hue, h, s) in rows {
        ui.add_space(4.0);
        st::caption(ui, tr(title));
        let hues = vec![st::hsv(hue - 35.0, 0.6, 0.62), st::hsv(hue, 0.6, 0.62), st::hsv(hue + 35.0, 0.6, 0.62)];
        ch |= st::slider(ui, &format!("{title}Hue"), tr("Hue"), h, &Spec::centred(100.0).colors(hues)).changed;
        let sats = vec![st::hsv(hue, 0.15, 0.58), st::hsv(hue, 0.65, 0.58)];
        ch |= st::slider(ui, &format!("{title}Sat"), tr("Saturation"), s, &Spec::centred(100.0).colors(sats)).changed;
    }
    e.dirty |= ch;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_panel_resets_only_its_own_settings() {
        let edited_all = CameraRaw {
            exposure: 1.0,
            temperature: 10.0,
            clarity: 20.0,
            curve_lights: 10.0,
            hsl_sat: [5.0; 8],
            grade_balance: 20.0,
            sharpen_amount: 40.0,
            lens_distortion: 10.0,
            transform_rotate: 2.0,
            calibration_red_hue: 10.0,
            ..Default::default()
        };
        let mut all_reset = edited_all.clone();
        for (id, _) in PANELS {
            assert!(edited(&edited_all, id), "{id} has an edit in the fixture");
            let without_one = without(&edited_all, id);
            assert!(!edited(&without_one, id));
            // Every other panel keeps its edit.
            for (other, _) in PANELS.iter().filter(|(o, _)| *o != id) {
                assert!(edited(&without_one, other), "hiding {id} must not reset {other}");
            }
            all_reset = without(&all_reset, id);
        }
        assert_eq!(all_reset, CameraRaw::default());
        assert!(!edited(&CameraRaw::default(), "light"));
        // The pipeline's own bookkeeping is never part of a panel.
        let scaled = CameraRaw { pixel_scale: 0.25, encoding_gamma: 1.8, ..Default::default() };
        assert_eq!(visible(&scaled, &PANELS.map(|(id, _)| id.to_string())), scaled);
    }

    #[test]
    fn the_parametric_preview_is_the_pipeline_curve() {
        let flat = parametric_preview(&CameraRaw::default());
        assert!(flat.iter().enumerate().all(|(i, v)| (v - i as f32 / 255.0).abs() < 1e-4));
        let lighter = parametric_preview(&CameraRaw { curve_lights: 80.0, ..Default::default() });
        assert!(lighter[160] > flat[160]);
    }
}
