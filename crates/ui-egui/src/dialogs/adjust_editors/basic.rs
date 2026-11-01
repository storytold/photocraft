use super::hue_saturation::RANGE_LABELS;
use super::{
    Color32, Edit, EditorCx, HUE_RANGES, PHOTO_FILTERS, RichText, Sense, Stroke, StrokeKind, Tokens, Value, adjust_params, color_button, flag, json, label,
    num, nums, pos2, rgb_of, round, tone, vec2, widgets,
};

/// Labelled slider rows over top-level keys: (key, label, min, max, default, unit).
/// A row of [`sliders`]: params key, label, range, default, unit, and its decimal places (None:
/// two for a range of 10 or less, else whole numbers).
type SliderRow<'a> = (&'a str, &'a str, f32, f32, f32, &'a str, Option<u32>);

/// The value a [`sliders`] row stores for `x`: rounded to `decimals` places, or by its range.
pub(crate) fn row_value(x: f32, min: f32, max: f32, decimals: Option<u32>) -> f32 {
    match decimals {
        Some(d) => {
            let scale = 10f32.powi(d.min(6) as i32);
            (x * scale).round() / scale
        }
        None if max - min <= 10.0 => round(x, 0.01),
        None => x.round(),
    }
}

pub(super) fn sliders(ui: &mut egui::Ui, v: &mut Value, rows: &[SliderRow]) -> Edit {
    // Drawn inside the caller's mirrored body scope: the editors below wrap themselves, and the
    // plain kinds (Exposure/Vibrance/Posterize) are hosted by the dialog/Properties body scopes.
    slider_rows(ui, v, rows)
}

/// The rows themselves, drawn inside the mirrored body scope.
fn slider_rows(ui: &mut egui::Ui, v: &mut Value, rows: &[SliderRow]) -> Edit {
    let mut e = Edit::default();
    for &(key, text, min, max, default, unit, decimals) in rows {
        let mut x = num(v, key, default).clamp(min, max);
        // A double-click on the slider puts it back to its default (Photoshop).
        let gestures = widgets::RowGestures { reset: Some(default), wheel_step: None, ..Default::default() };
        let r = widgets::slider_row_prec(ui, text, &mut x, min..=max, unit, None, gestures, decimals);
        if r.changed() {
            v[key] = json!(row_value(x, min, max, decimals));
        }
        e.add(Edit::of(&r));
    }
    e
}

fn gradient_slider(ui: &mut egui::Ui, text: &str, x: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str, stops: [Color32; 2], dynamic: bool) -> Edit {
    let r = widgets::slider_row(ui, text, x, range, unit, dynamic.then_some(&stops));
    Edit::of(&r)
}

/// Preferences ▸ Interface ▸ Dynamic Color Sliders: the colour track for a slider, or the plain
/// track when it is off.
pub(super) fn track_stops<'a>(cx: &EditorCx, stops: &'a [Color32]) -> Option<&'a [Color32]> {
    cx.dynamic_color_sliders.then_some(stops)
}

// ---------------------------------------------------------------------------------------------
// Brightness/Contrast

pub(super) fn brightness_contrast(ui: &mut egui::Ui, v: &mut Value) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| brightness_contrast_body(ui, v)).inner
}

fn brightness_contrast_body(ui: &mut egui::Ui, v: &mut Value) -> Edit {
    let legacy = flag(v, "legacy", false);
    let lo = if legacy { -100.0 } else { -50.0 };
    let mut e = sliders(ui, v, &[("brightness", tl!("Brightness"), -150.0, 150.0, 0.0, "", None), ("contrast", tl!("Contrast"), lo, 100.0, 0.0, "", None)]);
    let mut l = legacy;
    if widgets::checkbox(ui, &mut l, tl!("Use Legacy")).changed() {
        v["legacy"] = json!(l);
        if !l {
            v["contrast"] = json!(num(v, "contrast", 0.0).max(-50.0));
        }
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Threshold

pub(super) fn threshold(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| threshold_body(ui, v, cx)).inner
}

fn threshold_body(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width().min(300.0);
    let (hr, _) = ui.allocate_exact_size(vec2(w, 90.0), Sense::hover());
    let hr = hr.shrink2(vec2(6.0, 0.0));
    let p = ui.painter_at(hr.expand(1.0));
    // The histogram is a tonal graph: it and the level marker on it keep their physical
    // dark-to-light order whatever the UI direction.
    p.rect_filled(hr, 0.0, t.field);
    if let Some(h) = &cx.hist {
        tone::draw_histogram(&p, hr, &h[0], Color32::from_gray(if t.pro { 150 } else { 110 }));
    }
    let level = num(v, "level", 128.0);
    let x = hr.left() + level / 255.0 * hr.width();
    p.line_segment([pos2(x, hr.top()), pos2(x, hr.bottom())], Stroke::new(1.0, t.accent));
    p.rect_stroke(hr, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    ui.add_space(4.0);
    sliders(ui, v, &[("level", tl!("Threshold Level"), 1.0, 255.0, 128.0, "", None)])
}

// ---------------------------------------------------------------------------------------------
// Color Balance

/// Where the Color Balance editor rooted at `mem` keeps its Tone choice.
fn color_balance_tone_id(mem: egui::Id) -> egui::Id {
    mem.with("cb-tone")
}

/// Photoshop's Color Balance reset to defaults (Alt+Cancel in the dialog; the Properties panel's
/// Reset once the layer is as the panel found it): every tone back to 0 and the Tone choice back
/// to Midtones; Preserve Luminosity stays as it is.
pub fn reset_color_balance(ctx: &egui::Context, mem: egui::Id, v: &mut Value) {
    for key in ["shadows", "midtones", "highlights"] {
        v[key] = json!([0.0, 0.0, 0.0]);
    }
    ctx.data_mut(|d| d.insert_temp(color_balance_tone_id(mem), 1usize));
}

/// Kind-specific editor memory after Alt+Reset (or Properties Reset to defaults). Color Balance
/// shows Midtones; other kinds have no dialog-local temp to clear yet.
pub fn reset_adjustment_editor(ctx: &egui::Context, kind: &str, mem: egui::Id, v: &mut Value) {
    if kind == "colorBalance" {
        reset_color_balance(ctx, mem, v);
    }
}

pub(super) fn color_balance(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| color_balance_body(ui, v, cx)).inner
}

fn color_balance_body(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let tone_id = color_balance_tone_id(cx.mem);
    let mut tone_ix: usize = ui.data(|d| d.get_temp(tone_id)).unwrap_or(1);
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Tone:"));
        for (i, l) in [tl!("Shadows"), tl!("Midtones"), tl!("Highlights")].iter().enumerate() {
            if ui.radio(tone_ix == i, *l).clicked() {
                tone_ix = i;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(tone_id, tone_ix));
    let key = ["shadows", "midtones", "highlights"][tone_ix.min(2)];
    let mut vals = nums(v, key, [0.0; 3]);
    let mut e = Edit::default();
    let rows = [
        (tl!("Cyan  ·  Red"), Color32::from_rgb(0, 190, 210), Color32::from_rgb(225, 40, 40)),
        (tl!("Magenta  ·  Green"), Color32::from_rgb(210, 40, 190), Color32::from_rgb(40, 190, 60)),
        (tl!("Yellow  ·  Blue"), Color32::from_rgb(230, 210, 30), Color32::from_rgb(40, 80, 230)),
    ];
    // Photoshop: a double-click on a slider zeroes it (this tone only); the dialog's sliders step
    // with the wheel; Up/Down step the fields; Shift makes those steps 10.
    let gestures = widgets::RowGestures { reset: Some(0.0), wheel_step: cx.dialog.then_some(1.0), ..Default::default() };
    let mut changed = false;
    for (i, (text, a, b)) in rows.iter().enumerate() {
        let r = Edit::of(&widgets::slider_row_with(ui, text, &mut vals[i], -100.0..=100.0, "", Some(&[*a, *b]), gestures));
        changed |= r.changed;
        e.add(r);
    }
    if changed {
        v[key] = json!(vals.map(f32::round));
    }
    let mut pl = flag(v, "preserveLuminosity", true);
    if widgets::checkbox(ui, &mut pl, tl!("Preserve Luminosity")).changed() {
        v["preserveLuminosity"] = json!(pl);
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Black & White

pub(super) fn black_white(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| black_white_body(ui, v, cx)).inner
}

fn black_white_body(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let mut e = Edit::default();
    let colors = [[230, 50, 50], [235, 215, 40], [50, 200, 70], [40, 200, 220], [50, 90, 235], [220, 50, 210]];
    for (i, key) in HUE_RANGES.iter().enumerate() {
        let mut x = num(v, key, adjust_params::BW_DEFAULTS[i]).clamp(-200.0, 300.0);
        let c = colors[i];
        let r =
            gradient_slider(ui, RANGE_LABELS[i], &mut x, -200.0..=300.0, "%", [Color32::BLACK, Color32::from_rgb(c[0], c[1], c[2])], cx.dynamic_color_sliders);
        if r.changed {
            v[*key] = json!(x.round());
        }
        e.add(r);
    }
    ui.add_space(2.0);
    crate::direction::dialog_row(ui, |ui| {
        let mut on = flag(v, "tint", false);
        if widgets::checkbox(ui, &mut on, tl!("Tint")).changed() {
            v["tint"] = json!(on);
            e.add(Edit::discrete(true));
        }
        let mut c = rgb_of(v, "tintColor", [0.882, 0.827, 0.702]);
        let r = ui.add_enabled_ui(on, |ui| color_button(ui, &mut c, tl!("Tint Color"))).inner;
        if r.changed {
            v["tintColor"] = json!(adjust_params::hex(c));
        }
        e.add(r);
        if widgets::secondary_button(ui, tl!("Default"), 0.0).clicked() {
            for (i, key) in HUE_RANGES.iter().enumerate() {
                v[*key] = json!(adjust_params::BW_DEFAULTS[i]);
            }
            e.add(Edit::discrete(true));
        }
    });
    e
}

// ---------------------------------------------------------------------------------------------
// Photo Filter

pub(super) fn photo_filter(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| photo_filter_body(ui, v, cx)).inner
}

fn photo_filter_body(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let mut e = Edit::default();
    let mut c = rgb_of(v, "color", [236.0 / 255.0, 138.0 / 255.0, 0.0]);
    let hex = adjust_params::hex(c);
    let preset = PHOTO_FILTERS.iter().find(|f| adjust_params::hex(f.2.map(|x| f32::from(x) / 255.0)) == hex).map(|f| f.0);
    let mode_id = cx.mem.with("pf-mode");
    let mut use_color: bool = ui.data(|d| d.get_temp(mode_id)).unwrap_or(preset.is_none());
    crate::direction::dialog_row(ui, |ui| {
        if ui.radio(!use_color, tl!("Filter:")).clicked() {
            use_color = false;
        }
        let mut sel = preset.unwrap_or("custom").to_string();
        let mut opts: Vec<(String, &str)> = PHOTO_FILTERS.iter().map(|f| (f.0.to_string(), f.1)).collect();
        if preset.is_none() {
            opts.insert(0, ("custom".into(), "Custom"));
        }
        let changed = ui.add_enabled_ui(!use_color, |ui| widgets::dropdown(ui, &format!("{mode_id:?}-filter"), tl!("Filter:"), &mut sel, &opts, 170.0)).inner;
        if changed && let Some(f) = PHOTO_FILTERS.iter().find(|f| f.0 == sel) {
            c = f.2.map(|x| f32::from(x) / 255.0);
            v["color"] = json!(adjust_params::hex(c));
            e.add(Edit::discrete(true));
        }
    });
    crate::direction::dialog_row(ui, |ui| {
        if ui.radio(use_color, tl!("Color:")).clicked() {
            use_color = true;
        }
        let r = ui.add_enabled_ui(use_color, |ui| color_button(ui, &mut c, tl!("Photo Filter"))).inner;
        if r.changed {
            v["color"] = json!(adjust_params::hex(c));
        }
        e.add(r);
    });
    ui.data_mut(|d| d.insert_temp(mode_id, use_color));
    e.add(sliders(ui, v, &[("density", tl!("Density"), 0.0, 100.0, 25.0, "%", None)]));
    let mut pl = flag(v, "preserveLuminosity", true);
    if widgets::checkbox(ui, &mut pl, tl!("Preserve Luminosity")).changed() {
        v["preserveLuminosity"] = json!(pl);
        e.add(Edit::discrete(true));
    }
    e
}

// ---------------------------------------------------------------------------------------------
// Channel Mixer

pub(super) fn channel_mixer(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    ui.with_layout(crate::direction::body_layout(), |ui| channel_mixer_body(ui, v, cx)).inner
}

fn channel_mixer_body(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let mono = flag(v, "monochrome", false);
    let out_id = cx.mem.with("cm-out");
    let mut out: usize = ui.data(|d| d.get_temp(out_id)).unwrap_or(0);
    let outs: &[(usize, &str)] = if mono { &[(0, tl!("Gray"))] } else { &[(0, tl!("Red")), (1, tl!("Green")), (2, tl!("Blue"))] };
    if out >= outs.len() {
        out = 0;
    }
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Output Channel:"));
        widgets::dropdown(ui, &format!("{out_id:?}"), tl!("Output Channel:"), &mut out, outs, 110.0);
    });
    ui.data_mut(|d| d.insert_temp(out_id, out));
    let key = if mono { "gray" } else { ["red", "green", "blue"][out] };
    let identity: [f32; 4] = if mono { [40.0, 40.0, 20.0, 0.0] } else { std::array::from_fn(|i| if i == out { 100.0 } else { 0.0 }) };
    let mut row = nums(v, key, if mono { nums(v, "red", identity) } else { identity });
    let mut e = Edit::default();
    let mut changed = false;
    for (i, (text, c)) in [(tl!("Red"), [225, 50, 50]), (tl!("Green"), [50, 190, 70]), (tl!("Blue"), [60, 100, 235])].iter().enumerate() {
        let r = gradient_slider(ui, text, &mut row[i], -200.0..=200.0, "%", [Color32::BLACK, Color32::from_rgb(c[0], c[1], c[2])], cx.dynamic_color_sliders);
        changed |= r.changed;
        e.add(r);
    }
    let total = row[0] + row[1] + row[2];
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Total:"));
        let warn = total > 100.0;
        ui.label(RichText::new(format!("{total:+.0}%")).font(crate::theme::mono(12.0)).color(if warn { t.warning } else { t.text }));
    });
    let r = widgets::slider_row(ui, tl!("Constant"), &mut row[3], -200.0..=200.0, "%", None);
    changed |= r.changed();
    e.add(Edit::of(&r));
    if changed {
        v[key] = json!(row.map(f32::round));
    }
    let mut m = mono;
    if widgets::checkbox(ui, &mut m, tl!("Monochrome")).changed() {
        v["monochrome"] = json!(m);
        if m {
            v["gray"] = json!([40.0, 40.0, 20.0, 0.0]);
        } else if let Some(o) = v.as_object_mut() {
            o.remove("gray");
        }
        e.add(Edit::discrete(true));
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{Queryable, by};

    /// The Brightness/Contrast editor drawn in `dir`.
    fn harness(dir: Dir) -> Harness<'static, Value> {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(egui::vec2(380.0, 220.0)).build_ui_state(
            |ui, v: &mut Value| {
                brightness_contrast(ui, v);
            },
            json!({}),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        h
    }

    /// The Brightness row's caption and its number field.
    fn brightness_row(h: &Harness<'_, Value>) -> (egui::Rect, egui::Rect) {
        let caption = h.query_all(by().role(egui::accesskit::Role::Label).label("Brightness").include_labels()).next().expect("the Brightness caption").rect();
        let field = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Brightness").rect();
        (caption, field)
    }

    /// The editor context the harnesses build.
    fn test_cx() -> EditorCx {
        EditorCx {
            mem: egui::Id::unique("editor"),
            hist: None,
            gray: false,
            swatches: [[0.0; 3]; 2],
            gradients: Vec::new(),
            dialog: true,
            dynamic_color_sliders: true,
            isolated_curve: None,
        }
    }

    /// An adjustment editor with hand-built rows, drawn in `dir`; the editor's own body scope
    /// mirrors them.
    fn editor_harness(dir: Dir, editor: fn(&mut egui::Ui, &mut Value, &EditorCx) -> Edit) -> Harness<'static, Value> {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(egui::vec2(420.0, 300.0)).build_ui_state(
            move |ui, v: &mut Value| {
                editor(ui, v, &test_cx());
            },
            json!({}),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        h
    }

    /// The rect of the widget of `role` named `label`.
    fn rect_of(h: &Harness<'_, Value>, role: egui::accesskit::Role, label: &str) -> egui::Rect {
        h.get_by_role_and_label(role, label).rect()
    }

    /// The editor's only dropdown.
    fn combo_rect(h: &Harness<'_, Value>) -> egui::Rect {
        h.query_all(by().role(egui::accesskit::Role::ComboBox)).next().expect("the dropdown").rect()
    }

    /// A hand-built row mirrors in a right-to-left UI: its first-added atom moves to the
    /// reading start, its last-added one to the reading end.
    fn assert_row_mirrors(editor: fn(&mut egui::Ui, &mut Value, &EditorCx) -> Edit, first: (egui::accesskit::Role, &str), last: (egui::accesskit::Role, &str)) {
        let ltr = editor_harness(Dir::Ltr, editor);
        let rtl = editor_harness(Dir::Rtl, editor);
        let (fl, ll) = (rect_of(&ltr, first.0, first.1), rect_of(&ltr, last.0, last.1));
        assert!(fl.left() < ll.left(), "LTR reads left to right: {fl:?} {ll:?}");
        let (fr, lr) = (rect_of(&rtl, first.0, first.1), rect_of(&rtl, last.0, last.1));
        assert!(fr.left() > lr.left(), "RTL reads right to left: {fr:?} {lr:?}");
    }

    /// The row mirrors in a right-to-left UI: the caption moves to the reading start and the
    /// number to the reading end.
    #[test]
    fn brightness_contrast_row_mirrors_under_rtl() {
        let (ltr_caption, ltr_field) = brightness_row(&harness(Dir::Ltr));
        let (rtl_caption, rtl_field) = brightness_row(&harness(Dir::Rtl));
        assert!(ltr_caption.left() < ltr_field.left(), "LTR reads left to right: {ltr_caption:?} {ltr_field:?}");
        assert!(rtl_caption.left() > rtl_field.left(), "RTL reads right to left: {rtl_caption:?} {rtl_field:?}");
    }

    /// The threshold histogram and the level marker on it, painted in `dir`.
    fn painted_threshold(dir: Dir) -> (egui::Rect, f32) {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, Default::default());
        let t = Tokens::get(&ctx);
        let mut v = json!({ "level": 64.0 });
        let cx = EditorCx {
            mem: egui::Id::unique("threshold"),
            hist: None,
            gray: false,
            swatches: [[0.0; 3]; 2],
            gradients: Vec::new(),
            dialog: true,
            dynamic_color_sliders: true,
            isolated_curve: None,
        };
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            threshold(ui, &mut v, &cx);
        });
        out.textures_delta.clear();
        let mut hist = None;
        let mut marker = None;
        for clipped in &out.shapes {
            match &clipped.shape {
                egui::Shape::Rect(r) if (r.rect.height() - 90.0).abs() < 0.5 => hist = Some(r.rect),
                egui::Shape::LineSegment { points, stroke } if stroke.color == t.accent => marker = Some(points[0].x),
                _ => {}
            }
        }
        (hist.expect("the histogram"), marker.expect("the level marker"))
    }

    /// The histogram is a tonal graph, so it and the level marker on it keep their physical
    /// dark-to-light order in a right-to-left UI.
    #[test]
    fn threshold_graph_stays_physical_under_rtl() {
        for dir in [Dir::Ltr, Dir::Rtl] {
            let (hist, marker) = painted_threshold(dir);
            let fraction = (marker - hist.left()) / hist.width();
            assert!((fraction - 64.0 / 255.0).abs() < 0.01, "{dir:?}: the marker sits at the level: {hist:?} {marker}");
        }
    }

    /// The Color Balance Tone radio row: Shadows first, Highlights last.
    #[test]
    fn color_balance_tone_row_mirrors_under_rtl() {
        assert_row_mirrors(color_balance, (egui::accesskit::Role::RadioButton, "Shadows"), (egui::accesskit::Role::RadioButton, "Highlights"));
    }

    /// The Black & White Tint row: the Tint checkbox first, the Default button last. The
    /// Default button draws in the "medium" family; frames where that family has no usable
    /// face skip the editor, and the test skips when no row rendered.
    #[test]
    fn black_white_tint_row_mirrors_under_rtl() {
        fn harness(dir: Dir) -> Harness<'static, Value> {
            let _dir = direction_scope(dir);
            let mut h = Harness::builder().with_size(egui::vec2(420.0, 300.0)).build_ui_state(
                move |ui, v: &mut Value| {
                    let faces = ui.ctx().fonts(|f| {
                        let d = f.definitions();
                        d.families.get(&egui::FontFamily::Name("medium".into())).is_some_and(|s| s.iter().any(|n| d.font_data.contains_key(n)))
                    });
                    if faces {
                        black_white(ui, v, &test_cx());
                    }
                },
                json!({}),
            );
            crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(4);
            h
        }
        fn find(h: &Harness<'_, Value>, role: egui::accesskit::Role, label: &str) -> Option<egui::Rect> {
            h.query_all(by().role(role).label(label)).next().map(|n| n.rect())
        }
        let ltr = harness(Dir::Ltr);
        let rtl = harness(Dir::Rtl);
        let check = egui::accesskit::Role::CheckBox;
        let button = egui::accesskit::Role::Button;
        let (Some(fl), Some(ll), Some(fr), Some(lr)) =
            (find(&ltr, check, "Tint"), find(&ltr, button, "Default"), find(&rtl, check, "Tint"), find(&rtl, button, "Default"))
        else {
            return;
        };
        assert!(fl.left() < ll.left(), "LTR reads left to right: {fl:?} {ll:?}");
        assert!(fr.left() > lr.left(), "RTL reads right to left: {fr:?} {lr:?}");
    }

    /// The Photo Filter row: the "Filter:" radio first, the dropdown last.
    #[test]
    fn photo_filter_row_mirrors_under_rtl() {
        let ltr = editor_harness(Dir::Ltr, photo_filter);
        let rtl = editor_harness(Dir::Rtl, photo_filter);
        let radio = egui::accesskit::Role::RadioButton;
        let (fl, cl) = (rect_of(&ltr, radio, "Filter:"), combo_rect(&ltr));
        assert!(fl.left() < cl.left(), "LTR reads left to right: {fl:?} {cl:?}");
        let (fr, cr) = (rect_of(&rtl, radio, "Filter:"), combo_rect(&rtl));
        assert!(fr.left() > cr.left(), "RTL reads right to left: {fr:?} {cr:?}");
    }

    /// The Channel Mixer Output Channel row: the caption first, the dropdown last.
    #[test]
    fn channel_mixer_output_row_mirrors_under_rtl() {
        let ltr = editor_harness(Dir::Ltr, channel_mixer);
        let rtl = editor_harness(Dir::Rtl, channel_mixer);
        let caption = egui::accesskit::Role::Label;
        let (fl, cl) = (rect_of(&ltr, caption, "Output Channel:"), combo_rect(&ltr));
        assert!(fl.left() < cl.left(), "LTR reads left to right: {fl:?} {cl:?}");
        let (fr, cr) = (rect_of(&rtl, caption, "Output Channel:"), combo_rect(&rtl));
        assert!(fr.left() > cr.left(), "RTL reads right to left: {fr:?} {cr:?}");
    }
}

#[cfg(test)]
mod rtl_row_twin {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// The Color Balance editor drawn in `dir`.
    fn harness(dir: Dir) -> Harness<'static, Value> {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(egui::vec2(380.0, 420.0)).build_ui_state(
            move |ui, v: &mut Value| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let cx = EditorCx {
                    mem: egui::Id::unique("cb-twin"),
                    hist: None,
                    gray: false,
                    swatches: [[0.0; 3]; 2],
                    gradients: Vec::new(),
                    dialog: true,
                    dynamic_color_sliders: true,
                    isolated_curve: None,
                };
                color_balance(ui, v, &cx);
            },
            json!({}),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        h
    }

    /// The migrated Tone row keeps its LTR order and gap, and mirrors both under RTL.
    #[test]
    fn the_tone_row_keeps_ltr_gap_and_mirrors_under_rtl() {
        let ltr = harness(Dir::Ltr);
        let rtl = harness(Dir::Rtl);
        let radio = egui::accesskit::Role::RadioButton;
        let ltr_sh = ltr.get_by_role_and_label(radio, "Shadows").rect();
        let ltr_hi = ltr.get_by_role_and_label(radio, "Highlights").rect();
        assert!(ltr_sh.left() < ltr_hi.left(), "LTR reads Shadows then Highlights: {ltr_sh:?} {ltr_hi:?}");
        let ltr_gap = ltr_hi.left() - ltr_sh.right();
        let rtl_sh = rtl.get_by_role_and_label(radio, "Shadows").rect();
        let rtl_hi = rtl.get_by_role_and_label(radio, "Highlights").rect();
        assert!(rtl_sh.left() > rtl_hi.left(), "RTL mirrors the row: {rtl_sh:?} {rtl_hi:?}");
        let rtl_gap = rtl_sh.left() - rtl_hi.right();
        assert!((ltr_gap - rtl_gap).abs() < 1.0, "the gap is mirrored, not resized: LTR {ltr_gap} RTL {rtl_gap}");
    }
}
