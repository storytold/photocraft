//! The Hue Curves editor (#6241): Curves' point editor (`point_curve`) over the hue circle. The
//! graph's x axis is the pixel's hue (0° to 360°, red at both ends), its y axis the chosen curve's
//! value with neutral in the middle (saturation: factor 1). The point at 0° and its mirror at 360°
//! are the same hue, so moving either moves both, and neither can be removed. An empty curve starts
//! from neutral anchors at the six primaries and secondaries, as in DaVinci Resolve, so a moved
//! point bends only its own colours.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::adjust::{CurvePoint, HUE_CURVE_NEUTRAL};
use serde_json::{Value, json};

use super::{CURVE_HANDLE_SIZE, Edit, EditorCx, adjusted_spectrum, channel_picker_opts, curve_graph, hue_color, label, spectrum_bar};
use crate::state::PointCurveState;
use crate::theme::Tokens;

/// The curves (hue vs hue, saturation, luminance): parameter key, picker label, value at the
/// graph's bottom and top.
const CURVES: [(&str, &str, f32, f32); 3] = [("hue", "Hue", -180.0, 180.0), ("saturation", "Saturation", 0.0, 2.0), ("luminance", "Luminance", -100.0, 100.0)];

/// Graph level (0..255) of curve value `v`.
fn level(c: usize, v: f32) -> f32 {
    let (_, _, lo, hi) = CURVES.get(c).copied().unwrap_or(CURVES[0]);
    ((v - lo) / (hi - lo) * 255.0).clamp(0.0, 255.0)
}

/// Curve value at graph level `y` (0..255).
fn value(c: usize, y: f32) -> f32 {
    let (_, _, lo, hi) = CURVES.get(c).copied().unwrap_or(CURVES[0]);
    lo + y.clamp(0.0, 255.0) / 255.0 * (hi - lo)
}

fn neutral(c: usize) -> f32 {
    HUE_CURVE_NEUTRAL.get(c).copied().unwrap_or(0.0)
}

/// The curve's `[hue degrees, value]` points.
fn read(v: &Value, key: &str) -> Vec<[f32; 2]> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|q| {
                    let q = q.as_array()?;
                    Some([q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32])
                })
                .filter(|q| q.iter().all(|x| x.is_finite()))
                .collect()
        })
        .unwrap_or_default()
}

fn curve_points(pts: &[[f32; 2]]) -> Vec<CurvePoint> {
    pts.iter().map(|q| CurvePoint { input: q[0], output: q[1] }).collect()
}

/// The graph points (levels 0..255 on both axes) of curve `c`: its points, one at 0° (the curve's
/// value there when it has none), and the mirror of that one at 360° last.
pub(super) fn graph_points(c: usize, pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut deg: Vec<[f32; 2]> = if pts.is_empty() {
        (0..6).map(|i| [i as f32 * 60.0, neutral(c)]).collect()
    } else {
        pts.iter().map(|q| [q[0].rem_euclid(360.0), q[1]]).filter(|q| q[0] < 360.0).collect()
    };
    deg.sort_by(|a, b| a[0].total_cmp(&b[0]));
    if deg.first().is_none_or(|q| q[0] * 255.0 / 360.0 >= 0.5) {
        let at0 = photocraft_compose::hue_curves::curve_table(&curve_points(&deg), neutral(c)).first().copied().unwrap_or(neutral(c));
        deg.insert(0, [0.0, at0]);
    }
    let mut out: Vec<[f32; 2]> = deg.iter().map(|q| [q[0] / 360.0 * 255.0, level(c, q[1])]).collect();
    if let Some(first) = out.first_mut() {
        first[0] = 0.0;
    }
    if let Some(first) = out.first().copied() {
        out.push([255.0, first[1]]);
    }
    out
}

/// Curve `c`'s `[hue degrees, value]` points from graph points (mirror dropped).
pub(super) fn from_graph(c: usize, g: &[[f32; 2]]) -> Vec<[f32; 2]> {
    // Whole steps of 1/10 (1/1000 for the factor), in f64 so 1.75 stays 1.75.
    let round = |x: f32, per: f64| ((f64::from(x) * per).round() / per) as f32;
    let step = if c == 1 { 1000.0 } else { 10.0 };
    g.iter().take(g.len().saturating_sub(1)).map(|q| [round(q[0] / 255.0 * 360.0, 10.0).min(359.9), round(value(c, q[1]), step)]).collect()
}

/// Keeps the 0° point and its 360° mirror together after an edit (`before`: their level then).
pub(super) fn tie_ends(g: &mut [[f32; 2]], before: f32) {
    let n = g.len();
    if n < 2 {
        return;
    }
    let last = g[n - 1][1];
    let first = g[0][1];
    let y = if last != before { last } else { first };
    g[0] = [0.0, y];
    g[n - 1] = [255.0, y];
}

pub(super) fn editor(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let opts: Vec<(usize, &str)> = CURVES.iter().enumerate().map(|(i, c)| (i, c.1)).collect();
    let c = channel_picker_opts(ui, cx.mem.with("hue-curves-ch"), &opts, tl!("Curve:"), true);
    let key = CURVES.get(c).map_or("hue", |k| k.0);
    let state_id = cx.mem.with(("hue-curves", c));
    let mut st: PointCurveState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let mut g = graph_points(c, &read(v, key));
    let before = g.first().map_or(0.0, |q| q[1]);

    let side = ui.available_width().clamp(120.0, 300.0);
    ui.add_space(4.0);
    let (full, resp) = ui.allocate_exact_size(vec2(side, side + 14.0), Sense::click_and_drag());
    let graph = curve_graph(full, side);
    ui.data_mut(|d| d.insert_temp(cx.mem.with("hue-curves-graph"), graph));
    let interaction = crate::point_curve::interact(ui, &resp, graph, &mut g, &mut st);
    tie_ends(&mut g, before);
    let mut e = Edit { changed: false, commit: interaction.commit, ..Default::default() };
    if interaction.changed {
        v[key] = json!(from_graph(c, &g));
        e.changed = true;
    }

    // Draw: the hues as columns behind the graph, grid at every 60°, the neutral line, the curve.
    let to_scr = |q: [f32; 2]| pos2(graph.left() + q[0] / 255.0 * graph.width(), graph.bottom() - q[1] / 255.0 * graph.height());
    let p = ui.painter_at(full.expand(2.0));
    p.rect_filled(graph, 0.0, t.field);
    for i in 1..6 {
        let x = graph.left() + i as f32 / 6.0 * graph.width();
        p.line_segment([pos2(x, graph.top()), pos2(x, graph.bottom())], Stroke::new(1.0, t.separator));
    }
    let mid = to_scr([0.0, level(c, neutral(c))]).y;
    p.line_segment([pos2(graph.left(), mid), pos2(graph.right(), mid)], Stroke::new(1.0, t.separator.gamma_multiply(1.6)));
    p.rect_stroke(
        graph,
        0.0,
        Stroke::new(if resp.has_focus() { 1.5 } else { 1.0 }, if resp.has_focus() { t.accent } else { t.field_border }),
        StrokeKind::Outside,
    );
    let table = photocraft_compose::hue_curves::curve_table(&curve_points(&from_graph(c, &g)), neutral(c));
    let n = table.len().max(2);
    let line: Vec<Pos2> =
        (0..n).step_by((n / 256).max(1)).map(|i| to_scr([i as f32 / (n - 1) as f32 * 255.0, level(c, table.get(i).copied().unwrap_or(0.0))])).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.5, t.text)));
    for (i, q) in g.iter().enumerate() {
        let r = Rect::from_center_size(to_scr(*q), vec2(CURVE_HANDLE_SIZE, CURVE_HANDLE_SIZE));
        let fill = hue_color(q[0] / 255.0 * 360.0);
        p.rect_filled(r, 0.0, fill);
        let stroke = if Some(i) == st.selected { Stroke::new(1.5, t.text) } else { Stroke::new(1.0, Color32::BLACK) };
        p.rect_stroke(r, 0.0, stroke, StrokeKind::Outside);
    }
    // The hues in, and what they become (Hue/Saturation's two bars).
    let bar = |y: f32| Rect::from_min_max(pos2(graph.left(), graph.bottom() + y), pos2(graph.right(), graph.bottom() + y + 4.0));
    let hues: Vec<Color32> = (0..=36).map(|i| hue_color(i as f32 * 10.0)).collect();
    spectrum_bar(&p, bar(4.0), &hues);
    spectrum_bar(&p, bar(8.0), &adjusted_spectrum("hueCurves", v, 37));

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let sel = st.selected.and_then(|i| from_graph(c, &[g.get(i).copied().unwrap_or_default(), [0.0, 0.0]]).first().copied());
        label(ui, tl!("Input:"));
        ui.label(sel.map_or_else(|| "–".into(), |q| format!("{:.0}°", q[0])));
        label(ui, tl!("Output:"));
        let unit = match c {
            0 => "°",
            2 => "%",
            _ => "×",
        };
        ui.label(sel.map_or_else(|| "–".into(), |q| if c == 1 { format!("{:.2}{unit}", q[1]) } else { format!("{:.0}{unit}", q[1]) }));
    });
    let hint = crate::i18n::fmt(tl!("Click to add a point · drag off or {key}-click or Delete to remove"), &[("key", &crate::shortcuts::pretty("Cmd"))]);
    ui.label(RichText::new(hint).color(t.text_faint).size(11.0));
    ui.data_mut(|d| d.insert_temp(state_id, st));
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_curves_start_from_six_neutral_anchors_and_a_mirror() {
        for c in 0..3 {
            let g = graph_points(c, &[]);
            assert_eq!(g.len(), 7);
            assert_eq!(g[0][0], 0.0);
            assert_eq!(g[6], [255.0, g[0][1]]);
            assert!(g.iter().all(|q| (q[1] - level(c, neutral(c))).abs() < 1e-4));
            assert_eq!(from_graph(c, &g).len(), 6);
            assert!(from_graph(c, &g).iter().all(|q| (q[1] - neutral(c)).abs() < 1e-3));
        }
    }

    #[test]
    fn graph_points_round_trip() {
        let pts = vec![[0.0, 30.0], [120.0, -45.0], [250.5, 0.0]];
        assert_eq!(from_graph(0, &graph_points(0, &pts)), pts);
        let sat = vec![[0.0, 1.0], [90.0, 0.25], [300.0, 1.75]];
        assert_eq!(from_graph(1, &graph_points(1, &sat)), sat);
        // A curve without a 0° point gets one at its value there.
        let g = graph_points(2, &[[60.0, 40.0], [240.0, 40.0]]);
        assert_eq!(g.len(), 4);
        assert!((value(2, g[0][1]) - 40.0).abs() < 0.5);
    }

    #[test]
    fn the_ends_move_together() {
        let mut g = graph_points(0, &[]);
        let before = g[0][1];
        g[6][1] = 200.0;
        tie_ends(&mut g, before);
        assert_eq!((g[0], g[6]), ([0.0, 200.0], [255.0, 200.0]));
        let before = g[0][1];
        g[0] = [3.0, 40.0];
        tie_ends(&mut g, before);
        assert_eq!((g[0], g[6]), ([0.0, 40.0], [255.0, 40.0]));
    }
}
