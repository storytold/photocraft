//! The icons beside the warp styles in Edit › Transform › Warp's options bar and in Type › Warp
//! Text, and the separators between their groups, as in Photoshop's Warp menu (#3656). Each icon
//! is a small square with a cross through its middle, bent by that style's own warp at the
//! default 50 % bend, so it shows what the style does to a layer. It is drawn from the warp itself, not
//! taken from Photoshop's artwork.

use egui::{Color32, Painter, Pos2, Rect, Stroke};
use photocraft_geom::warp::{StyleWarp, WarpStyle};

/// The box an icon bends: square, so the icons fill their square in the list.
const BOX: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
/// Points per edge or line.
const STEPS: usize = 16;

/// Whether `style` starts a group in Photoshop's Warp menu: the arcs, the arches and shells, the
/// waves, and the lens shapes.
pub fn starts_group(style: &WarpStyle) -> bool {
    matches!(style, WarpStyle::Arc | WarpStyle::Arch | WarpStyle::Flag | WarpStyle::Fisheye)
}

/// The icon of `style` as polylines in box units: the outline first (closed), then the middle
/// line across and the one down. Empty for None and Custom, which have no shape of their own.
pub fn lines(style: WarpStyle) -> Vec<Vec<[f64; 2]>> {
    let Some(w) = StyleWarp::new(style, 50.0, 0.0, 0.0, false, BOX) else { return Vec::new() };
    let along = |a: [f64; 2], b: [f64; 2]| -> Vec<[f64; 2]> {
        (0..=STEPS)
            .map(|i| {
                let t = i as f64 / STEPS as f64;
                let (x, y) = w.apply(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                [x, y]
            })
            .collect()
    };
    let [x0, y0, x1, y1] = BOX;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
    let outline = corners.windows(2).flat_map(|e| along(e[0], e[1])).collect();
    let mut out = vec![outline];
    let (x, y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    out.push(along([x0, y], [x1, y]));
    out.push(along([x, y0], [x, y1]));
    out
}

/// Paints `style`'s icon into `rect` in `color`: the outline at full strength, the inner lines
/// fainter. Nothing for a style without a shape.
pub fn paint(painter: &Painter, rect: Rect, style: &WarpStyle, color: Color32) {
    let lines = lines(*style);
    let points = || lines.iter().flatten();
    let (lo, hi) = points().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    if lines.is_empty() || !(w > 0.0 && h > 0.0) {
        return;
    }
    // Fitted into the square with a pixel to spare for the stroke, centred, same scale both ways.
    let room = rect.shrink(1.0);
    let k = (f64::from(room.width()) / w).min(f64::from(room.height()) / h);
    let at = |p: &[f64; 2]| {
        let c = room.center();
        Pos2::new(c.x + ((p[0] - (lo[0] + w / 2.0)) * k) as f32, c.y + ((p[1] - (lo[1] + h / 2.0)) * k) as f32)
    };
    for (i, line) in lines.iter().enumerate() {
        let stroke = if i == 0 { Stroke::new(1.0, color) } else { Stroke::new(1.0, color.gamma_multiply(0.45)) };
        painter.add(egui::Shape::line(line.iter().map(at).collect(), stroke));
    }
}

/// The decoration for a dropdown of warp styles.
pub fn decor() -> crate::widgets::Decor<'static, WarpStyle> {
    crate::widgets::Decor { icon: &paint, starts_group: &starts_group }
}

// `&String`: the callbacks of a `Decor<String>` take the option's own type.
#[allow(clippy::ptr_arg)]
fn paint_id(painter: &Painter, rect: Rect, id: &String, color: Color32) {
    if let Some(style) = WarpStyle::parse(id) {
        paint(painter, rect, &style, color);
    }
}

#[allow(clippy::ptr_arg)]
fn starts_group_id(id: &String) -> bool {
    WarpStyle::parse(id).is_some_and(|s| starts_group(&s))
}

/// [`decor`] for a dropdown of style ids (`arcLower`), as Type › Warp Text's dialog lists them.
pub fn decor_ids() -> crate::widgets::Decor<'static, String> {
    crate::widgets::Decor { icon: &paint_id, starts_group: &starts_group_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_an_icon_of_its_own_shape() {
        let all: Vec<_> = WarpStyle::PRESETS.iter().map(|s| lines(*s)).collect();
        for (s, l) in WarpStyle::PRESETS.iter().zip(&all) {
            // The outline closes, then a line across and one down.
            assert_eq!(l.len(), 3, "{s:?}");
            let o = &l[0];
            assert!(o.len() > 4 * STEPS, "{s:?}");
            let (first, last) = (o[0], o[o.len() - 1]);
            assert!((first[0] - last[0]).abs() < 1e-9 && (first[1] - last[1]).abs() < 1e-9, "{s:?}: the outline closes");
            assert!(l.iter().flatten().all(|p| p[0].is_finite() && p[1].is_finite()), "{s:?}");
        }
        // No two presets draw the same icon.
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                let same = all[i].iter().flatten().zip(all[j].iter().flatten()).all(|(a, b)| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
                assert!(!same, "{:?} and {:?} look alike", WarpStyle::PRESETS[i], WarpStyle::PRESETS[j]);
            }
        }
        // None and Custom have no shape of their own.
        assert!(lines(WarpStyle::None).is_empty() && lines(WarpStyle::Custom).is_empty());
    }

    #[test]
    fn the_icons_bend_the_way_their_styles_do() {
        let outline = |s| lines(s).swap_remove(0);
        let top_middle = |s| outline(s)[STEPS / 2];
        // Arc and Arc Upper raise the top edge's middle; Arc Lower leaves the top straight.
        assert!(top_middle(WarpStyle::Arc)[1] < -0.05);
        assert!(top_middle(WarpStyle::ArcUpper)[1] < -0.05);
        assert!(top_middle(WarpStyle::ArcLower)[1].abs() < 1e-6);
        // Squeeze pinches the sides in: the left edge's middle moves right.
        let left_middle = |s| outline(s)[3 * (STEPS + 1) + STEPS / 2];
        assert!(left_middle(WarpStyle::Squeeze)[0] > 0.05, "{:?}", left_middle(WarpStyle::Squeeze));
    }

    #[test]
    fn groups_start_where_photoshops_separators_are() {
        let starts: Vec<_> = WarpStyle::PRESETS.iter().filter(|s| starts_group(s)).collect();
        assert_eq!(starts, [&WarpStyle::Arc, &WarpStyle::Arch, &WarpStyle::Flag, &WarpStyle::Fisheye]);
        // Type › Warp Text lists the styles by id.
        let id = |s: &str| s.to_string();
        assert!(starts_group_id(&id("arc")) && starts_group_id(&id("fisheye")));
        assert!(!starts_group_id(&id("arcLower")) && !starts_group_id(&id("none")) && !starts_group_id(&id("nonsense")));
    }
}

/// Through egui on the GPU: the open list draws each style's icon in the slot before its label,
/// and leaves the slot empty for None and Custom.
#[cfg(test)]
mod drawn {
    use egui_kittest::kittest::Queryable;
    use photocraft_geom::warp::WarpStyle;

    #[test]
    fn the_open_list_draws_an_icon_before_each_style() {
        let mut style = WarpStyle::Arc;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(260.0, 560.0)).with_pixels_per_point(2.0).wgpu().build_ui(move |ui| {
            crate::theme::install_fonts(ui.ctx());
            crate::theme::apply(ui.ctx(), crate::theme::ThemeKind::default());
            let opts: Vec<(WarpStyle, &str)> = WarpStyle::all().map(|s| (s, s.label())).collect();
            crate::widgets::dropdown_decorated(ui, "styles", &mut style, &opts, 120.0, &super::decor());
        });
        h.run_steps(8);
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run_steps(8);
        let image = h.render().expect("rendered");
        // Light (label-coloured) pixels in the square before a row's label, at 2 px per point.
        let lit = |label: &str| {
            let r = h.get_by_label(label).rect();
            let (x0, x1) = (((r.left() + 2.0) * 2.0) as u32, ((r.left() + r.height()) * 2.0) as u32);
            let (y0, y1) = ((r.top() * 2.0) as u32, (r.bottom() * 2.0) as u32);
            (x0..x1).flat_map(|x| (y0..y1).map(move |y| (x, y))).filter(|&(x, y)| image.get_pixel(x, y).0[..3].iter().all(|&c| c > 150)).count()
        };
        for label in ["Arc Lower", "Arch", "Flag"] {
            assert!(lit(label) > 10, "{label}: {}", lit(label));
        }
        for label in ["None", "Custom"] {
            assert_eq!(lit(label), 0, "{label}");
        }
    }
}
