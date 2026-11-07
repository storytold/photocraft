//! Camera Raw's Geometry panel: Upright (Off, Auto, Level, Vertical, Full, Guided), the manual
//! Transform sliders (Vertical, Horizontal, Rotate, Aspect, Scale, Offset X / Y) and Constrain
//! Crop. The maths is [`photocraft_algo::camera_raw::geometry`]; Guided's guides are drawn on the
//! preview (or set through `{"ui": {"set": {"uprightGuides": [[[x0, y0], [x1, y1]], …]}}}` in
//! 0 … 1 image coordinates).

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_algo::camera_raw::geometry::{MAX_GUIDES, Upright};

use super::widgets;

/// The Upright buttons in Camera Raw's order, with their tooltips.
pub(super) const UPRIGHT: [(Upright, &str, &str); 6] = [
    (Upright::Off, "Off", "Disable Upright"),
    (Upright::Auto, "A", "Auto: apply balanced perspective corrections"),
    (Upright::Level, "Level", "Level: apply only level correction"),
    (Upright::Vertical, "Vertical", "Vertical: apply level and vertical perspective correction"),
    (Upright::Full, "Full", "Full: apply level, horizontal and vertical perspective corrections"),
    (Upright::Guided, "Guided", "Guided: draw two or more guides to customize perspective corrections"),
];

fn trc(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

/// A Geometry slider, drawn like the other Edit panels' (Camera Raw's 30 px rows).
fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, reset: f32) {
    super::edit_panel::row(ui, dirty, label, v, range, None, reset);
}

/// The Geometry panel's body (the caller draws its section header).
pub(super) fn body(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool) {
    let current = Upright::parse(&p.perspective_upright).unwrap_or(Upright::Off);
    // The Edit column sets item spacing to zero and spaces rows by hand.
    ui.add_space(6.0);
    widgets::section_label(ui, trc("Upright"));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (mode, label, tip) in UPRIGHT {
            if widgets::pill_tab(ui, trc(label), current == mode).on_hover_text(trc(tip)).clicked() && current != mode {
                p.perspective_upright = mode.id().into();
                // Guided draws its guides on the preview straight away, as in Camera Raw.
                super::masks_panel::set_guide_tool(ui.ctx(), mode == Upright::Guided);
                *dirty = true;
            }
        }
    });
    if current == Upright::Guided {
        let n = p.upright_guides.len().min(MAX_GUIDES);
        ui.horizontal(|ui| {
            ui.label(format!("{} {n}/{MAX_GUIDES}", trc("Guides:")));
            let mut drawing = super::masks_panel::load(ui.ctx()).tool == super::masks_panel::DrawTool::Guide;
            if widgets::checkbox(ui, &mut drawing, trc("Draw guides")).changed() {
                super::masks_panel::set_guide_tool(ui.ctx(), drawing);
            }
            if n > 0 && ui.small_button(trc("Clear guides")).clicked() {
                p.upright_guides.clear();
                *dirty = true;
            }
        });
    }
    let d = CameraRaw::default();
    ui.add_space(8.0);
    row(ui, dirty, "Vertical", &mut p.perspective_vertical, -100.0..=100.0, d.perspective_vertical);
    row(ui, dirty, "Horizontal", &mut p.perspective_horizontal, -100.0..=100.0, d.perspective_horizontal);
    row(ui, dirty, "Rotate", &mut p.perspective_rotate, -10.0..=10.0, d.perspective_rotate);
    row(ui, dirty, "Aspect", &mut p.perspective_aspect, -100.0..=100.0, d.perspective_aspect);
    row(ui, dirty, "Scale", &mut p.perspective_scale, 50.0..=150.0, d.perspective_scale);
    row(ui, dirty, "Offset X", &mut p.perspective_x, -100.0..=100.0, d.perspective_x);
    row(ui, dirty, "Offset Y", &mut p.perspective_y, -100.0..=100.0, d.perspective_y);
    ui.add_space(6.0);
    if widgets::checkbox(ui, &mut p.constrain_crop, trc("Constrain crop")).changed() {
        *dirty = true;
    }
}

/// Puts the Geometry settings of `out` back to `d`'s (the panel hidden by its eye button).
pub(super) fn reset(out: &mut CameraRaw, d: &CameraRaw) {
    out.perspective_upright = d.perspective_upright.clone();
    out.upright_guides = d.upright_guides.clone();
    (out.perspective_vertical, out.perspective_horizontal, out.perspective_rotate, out.perspective_aspect) =
        (d.perspective_vertical, d.perspective_horizontal, d.perspective_rotate, d.perspective_aspect);
    (out.perspective_scale, out.perspective_x, out.perspective_y, out.constrain_crop) = (d.perspective_scale, d.perspective_x, d.perspective_y, d.constrain_crop);
}

/// Adds a guide (0 … 1 image coordinates) for Guided Upright; the oldest goes past four, as in
/// Camera Raw. Returns `false` for a guide too short to mean anything.
pub(super) fn add_guide(p: &mut CameraRaw, a: [f32; 2], b: [f32; 2]) -> bool {
    let ok = a.iter().chain(&b).all(|v| v.is_finite()) && (a[0] - b[0]).hypot(a[1] - b[1]) > 0.01;
    if !ok {
        return false;
    }
    let clamp = |v: [f32; 2]| v.map(|c| c.clamp(0.0, 1.0));
    if p.upright_guides.len() >= MAX_GUIDES {
        p.upright_guides.remove(0);
    }
    p.upright_guides.push([clamp(a), clamp(b)]);
    p.perspective_upright = Upright::Guided.id().into();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guides_keep_the_last_four_and_switch_to_guided() {
        let mut p = CameraRaw::default();
        for k in 0..6 {
            assert!(add_guide(&mut p, [0.1 * k as f32, 0.0], [0.1 * k as f32, 1.0]));
        }
        assert_eq!(p.upright_guides.len(), MAX_GUIDES);
        assert_eq!(p.upright_guides[0][0][0], 0.2);
        assert_eq!(p.perspective_upright, "guided");
        assert!(!add_guide(&mut p, [0.5, 0.5], [0.5, 0.5]));
        assert!(!add_guide(&mut p, [f32::NAN, 0.5], [0.5, 0.9]));
        assert!(add_guide(&mut p, [-3.0, 0.0], [9.0, 1.0]));
        assert_eq!(p.upright_guides.last().map(|g| g[0][0]), Some(0.0));
    }

    #[test]
    fn reset_restores_geometry_only() {
        let mut p = CameraRaw { perspective_vertical: 30.0, perspective_scale: 80.0, constrain_crop: true, exposure: 1.0, ..CameraRaw::default() };
        reset(&mut p, &CameraRaw::default());
        assert_eq!(p.perspective_vertical, 0.0);
        assert_eq!(p.perspective_scale, 100.0);
        assert!(!p.constrain_crop);
        assert_eq!(p.exposure, 1.0);
    }
}
