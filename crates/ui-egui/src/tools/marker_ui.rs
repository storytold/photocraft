//! Marker tool (U): click to stamp a highlight marker (star, asterisk, circle or triangle) on the
//! image, the way findings are flagged on scans next to the arrows. A marker is an ordinary shape
//! layer centred on the click, sized in points at the document's resolution (like the Arrow tool),
//! with an optional contrasting outline so it reads on dark and bright tissue alike.

use photocraft_doc::ShapeLayer;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;

/// The marker shapes: (id, label).
const SHAPES: [(&str, &str); 4] = [("star", "Star"), ("asterisk", "Asterisk"), ("circle", "Circle"), ("triangle", "Triangle")];

/// Marker options bar state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkerOptions {
    /// "star" | "asterisk" | "circle" | "triangle".
    pub shape: String,
    /// Size across, in points.
    pub size_pt: f32,
    /// Straight RGBA. White by default, outlined in black, so a marker reads on dark and bright
    /// tissue alike and prints in black and white. (Stored as `tone`: an earlier build kept a
    /// yellow default as `color`, which is ignored now.)
    #[serde(rename = "tone")]
    pub color: [f32; 4],
    pub outline: bool,
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

impl Default for MarkerOptions {
    fn default() -> Self {
        MarkerOptions { shape: "star".into(), size_pt: 12.0, color: WHITE, outline: true }
    }
}

/// `shape.create` params for a marker centred on `at` (fill, stroke and name included).
pub fn params(app: &PhotocraftApp, at: [f64; 2]) -> Value {
    let o = &app.ui.tool_options.marker;
    let d = f64::from(o.size_pt * crate::arrow_ui::px_per_pt(app)).max(4.0);
    let r = d / 2.0;
    let rect = [at[0] - r, at[1] - r, d, d];
    let line = (d * 0.14).max(1.0);
    let (name, mut p) = match o.shape.as_str() {
        "asterisk" => {
            // Three strokes through the centre: six spokes.
            let subpaths: Vec<Value> = [90.0_f64, 30.0, -30.0]
                .iter()
                .map(|deg| {
                    let (s, c) = deg.to_radians().sin_cos();
                    let a = [at[0] - c * r, at[1] - s * r];
                    let b = [at[0] + c * r, at[1] + s * r];
                    json!({"closed": false, "knots": [a, b]})
                })
                .collect();
            ("Asterisk", json!({"kind": "path", "path": {"subpaths": subpaths}, "fill": null, "stroke": {"width": line, "color": o.color, "cap": "round"}}))
        }
        "circle" => ("Circle", json!({"kind": "ellipse", "rect": rect, "fill": null, "stroke": {"width": line, "color": o.color}})),
        "triangle" => {
            // Equilateral, pointing up, its box centred on the click (a regular polygon centres
            // its circumscribed circle instead, which leaves a triangle sitting low).
            let h = d * 3f64.sqrt() / 2.0;
            let knots = [[at[0], at[1] - h / 2.0], [at[0] + r, at[1] + h / 2.0], [at[0] - r, at[1] + h / 2.0]];
            ("Triangle", json!({"kind": "path", "path": {"subpaths": [{"closed": true, "knots": knots}]}, "fill": o.color, "stroke": null}))
        }
        _ => ("Star", json!({"kind": "star", "rect": rect, "sides": 5, "starRatio": 0.45, "fill": o.color, "stroke": null})),
    };
    p["name"] = json!(app.session.active().map_or_else(|| name.to_string(), |st| st.doc.next_layer_name(name)));
    p
}

/// What a click at `at` would stamp, for the canvas preview.
pub fn preview(app: &PhotocraftApp, at: [f64; 2]) -> Option<ShapeLayer> {
    let p = params(app, at);
    let path =
        if p["kind"] == "path" { photocraft_engine::vector_cmds::parse_path(&p["path"]).ok()? } else { photocraft_engine::vector_cmds::shape_path(&p).ok()? };
    let c = app.ui.tool_options.marker.color;
    let paint = photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(c[0], c[1], c[2], c[3]));
    let stroke = p["stroke"]["width"].as_f64().map(|w| photocraft_doc::ShapeStroke {
        width: w as f32,
        paint: paint.clone(),
        cap: photocraft_doc::LineCap::Round,
        ..Default::default()
    });
    Some(ShapeLayer { path, fill: p["fill"].is_array().then_some(paint), stroke, ..Default::default() })
}

/// Stamps a marker centred on `at`, with its outline, as one history step.
pub fn place(app: &mut PhotocraftApp, at: [f64; 2]) {
    let key = format!("marker-{}", app.ui.alloc_id());
    let mut p = params(app, at);
    p["coalesce"] = json!(key);
    let o = app.ui.tool_options.marker.clone();
    let r = app.run("shape.create", p).and_then(|info| match info["layer"].as_u64() {
        Some(layer) if o.outline => {
            let [r, g, b, _] = o.color;
            let light = 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.45;
            let size = (o.size_pt * crate::arrow_ui::px_per_pt(app) * 0.06).max(1.0).round();
            app.run(
                "layer.layerStyle.stroke",
                json!({"layer": layer, "size": size, "position": "outside", "color": if light { "#000000" } else { "#ffffff" }, "coalesce": key}),
            )
        }
        _ => Ok(info),
    });
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Options bar for the Marker tool; false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Marker {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(t.text_dim).size(12.0));
    };
    let o = &mut app.ui.tool_options.marker;
    lbl(ui, tl!("Marker:"));
    let shapes: Vec<(String, &str)> = SHAPES.iter().map(|(id, l)| (id.to_string(), *l)).collect();
    crate::widgets::dropdown(ui, "marker-shape", tl!("Marker:"), &mut o.shape, &shapes, 100.0);
    lbl(ui, tl!("Size:"));
    crate::widgets::value_field(ui, &mut o.size_pt, 2.0..=144.0, "pt", 58.0);
    lbl(ui, tl!("Color:"));
    // One click for the usual black-and-white markers (each gets the opposite outline).
    for (c, name) in [(WHITE, "White"), (BLACK, "Black")] {
        if ui.selectable_label(o.color == c, tl!(name)).clicked() {
            o.color = c;
        }
    }
    let color = o.color;
    if let Some(c) = color_swatch(app, ui, color, tl!("Marker color")) {
        app.ui.tool_options.marker.color = c;
    }
    crate::widgets::checkbox(ui, &mut app.ui.tool_options.marker.outline, tl!("Outline"));
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Click to place a marker"));
    true
}

/// The shared shape paint swatch for a solid colour (Arrow, Marker and Cut Out options): only a
/// Solid pick changes it; No Color and Gradient don't apply to these tools.
pub(crate) fn color_swatch(app: &mut PhotocraftApp, ui: &mut egui::Ui, color: [f32; 4], tip: &str) -> Option<[f32; 4]> {
    let paint = photocraft_doc::Fill::Solid(photocraft_doc::Color::rgba(color[0], color[1], color[2], color[3]));
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let lib = crate::paint_picker::library(&app.session, ui.ctx());
    let mut gradients = crate::shape_gradient_ui::GradientChoices {
        presets: &app.session.presets.gradients,
        current: &app.session.presets.gradient,
        recent: &mut app.ui.recent_gradients,
        fg,
        bg,
    };
    match crate::shape_paint_ui::swatch(ui, &lib, Some(&paint), color, tip, &mut gradients) {
        Some(crate::shape_paint_ui::PaintChange::Color(c)) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use photocraft_doc::{Effect, LayerContent};

    use super::*;

    #[test]
    fn markers_default_to_white_with_a_black_outline() {
        let o = MarkerOptions::default();
        assert_eq!((o.color, o.outline), (WHITE, true));
        // A saved yellow from an earlier build (stored as `color`) no longer applies.
        let old: MarkerOptions = serde_json::from_str(r#"{"shape":"star","color":[1.0,0.8,0.0,1.0]}"#).unwrap();
        assert_eq!(old.color, WHITE);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200, "background": "black", "resolution": 300})).unwrap();
        place(&mut app, [100.0, 100.0]);
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let Some(Effect::Stroke(fx)) = l.effects.items.iter().find(|e| matches!(e, Effect::Stroke(_))) else { panic!("outlined") };
        assert!(format!("{:?}", fx.paint).contains("c: [0.0, 0.0, 0.0"), "black outline on a white marker: {:?}", fx.paint);
    }

    #[test]
    fn each_marker_is_one_outlined_shape_layer_at_print_size() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 1000, "height": 800, "background": "black", "resolution": 300})).unwrap();
        for (shape, _) in SHAPES {
            app.ui.tool_options.marker.shape = shape.into();
            assert!(preview(&app, [500.0, 400.0]).is_some(), "{shape}");
            let before = app.session.active().unwrap().doc.layers.len();
            place(&mut app, [500.0, 400.0]);
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layers.len(), before + 1, "{shape}: {}", app.ui.status);
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
            let LayerContent::Shape(sh) = &l.content else { panic!("{shape} is a shape layer") };
            // 12 pt at 300 dpi = 50 px across, centred on the click.
            let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
            assert!(((x0 + x1) / 2.0 - 500.0).abs() < 3.0 && ((y0 + y1) / 2.0 - 400.0).abs() < 3.0, "{shape} centred");
            assert!((x1 - x0 - 50.0).abs() < 3.0 || (y1 - y0 - 50.0).abs() < 3.0, "{shape} is 12 pt: {}", x1 - x0);
            assert!(l.effects.items.iter().any(|e| matches!(e, Effect::Stroke(_))), "{shape} outlined");
        }
    }
}
