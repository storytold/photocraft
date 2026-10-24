//! Layer › Copy CSS (#8099): the CSS of a type or shape layer (or of every such layer in a group)
//! as text, for a designer to paste into a stylesheet. Shapes give their size, position, fill
//! (solid or gradient), border, corner radius and drop shadow; type gives the font, size, weight,
//! line height, spacing, decoration, alignment and colour. The command returns the CSS and the
//! text to put on the clipboard (`clipboardText`): the engine has no clipboard of its own, the
//! shell copies it.

use std::collections::HashMap;
use std::fmt::Write as _;

use photocraft_color::Color;
use photocraft_doc::effects::{Effect, GradientStyle, Shadow};
use photocraft_doc::text::{CharStyle, TextAlign};
use photocraft_doc::{Document, Fill, Layer, LayerContent, LayerId, LiveShape, MAX_GROUP_DEPTH, TextLayer};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// A number as CSS writes it: at most two decimals, no trailing zeros, never `-0` or NaN.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{:.2}", (v * 100.0).round() / 100.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
}

fn px(v: f64) -> String {
    format!("{}px", num(v))
}

/// `#rrggbb`, or `rgba(r, g, b, a)` when the colour (times `opacity`) isn't opaque.
fn css_color(c: &Color, opacity: f32) -> String {
    let [r, g, b, _] = c.to_rgba8();
    let a = (c.alpha * opacity).clamp(0.0, 1.0);
    if a >= 0.999 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("rgba({r}, {g}, {b}, {})", num(f64::from(a))) }
}

/// A class name from a layer name: lower case, runs of other characters become one hyphen, and a
/// name that doesn't start with a letter gets `layer-` in front. Repeats get `-2`, `-3`, …
fn class_name(name: &str, used: &mut HashMap<String, usize>) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-');
    let base = if s.chars().next().is_some_and(char::is_alphabetic) { s.to_string() } else { format!("layer-{s}").trim_end_matches('-').to_string() };
    let n = used.entry(base.clone()).or_insert(0);
    *n += 1;
    if *n == 1 { base } else { format!("{base}-{n}") }
}

fn gradient_css(stops: &[(f32, Color)], angle: f32, style: &GradientStyle, reverse: bool) -> Option<String> {
    if stops.len() < 2 {
        return None;
    }
    let mut list: Vec<(f32, Color)> = stops.to_vec();
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    if reverse {
        list = list.into_iter().rev().map(|(p, c)| (1.0 - p, c)).collect();
    }
    let parts: Vec<String> = list.iter().map(|(p, c)| format!("{} {}%", css_color(c, 1.0), num(f64::from(p.clamp(0.0, 1.0) * 100.0)))).collect();
    match style {
        // Photoshop measures the angle counter-clockwise from the right; CSS clockwise from the top.
        GradientStyle::Linear => Some(format!("linear-gradient({}deg, {})", num(f64::from(90.0 - angle).rem_euclid(360.0)), parts.join(", "))),
        GradientStyle::Radial => Some(format!("radial-gradient(circle, {})", parts.join(", "))),
        _ => None,
    }
}

/// `box-shadow` / `text-shadow` value of an enabled drop shadow: the offset runs opposite to the
/// light angle, the blur is the size and the spread a fraction of it.
fn shadow_value(s: &Shadow, text: bool) -> String {
    let a = f64::from(s.angle).to_radians();
    let d = f64::from(s.distance);
    let (dx, dy) = (-a.cos() * d, a.sin() * d);
    let blur = f64::from(s.size);
    let color = css_color(&s.color, s.common.opacity);
    if text {
        format!("{} {} {} {}", px(dx), px(dy), px(blur), color)
    } else {
        format!("{} {} {} {} {}", px(dx), px(dy), px(blur), px(f64::from(s.spread) * blur), color)
    }
}

fn drop_shadow(l: &Layer) -> Option<&Shadow> {
    if !l.effects.enabled {
        return None;
    }
    l.effects.items.iter().find_map(|e| match e {
        Effect::DropShadow(s) if s.common.enabled => Some(s),
        _ => None,
    })
}

/// Left, top, width and height of the layer in document pixels.
fn frame(l: &Layer) -> Option<[f64; 4]> {
    if let LayerContent::Shape(s) = &l.content {
        if let Some(LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect }) = &s.live {
            return Some(*rect);
        }
        if let Some((x0, y0, x1, y1)) = s.path.control_bounds() {
            return Some([x0, y0, x1 - x0, y1 - y0]);
        }
    }
    let r = crate::layer_multi_cmds::layer_bounds(l)?;
    Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.width()), f64::from(r.height())])
}

fn text_properties(t: &TextLayer, dpi: f32, out: &mut Vec<String>) {
    let k = f64::from(if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 });
    let runs = t.char_runs();
    let style: CharStyle = runs.first().map(|r| r.style.clone()).unwrap_or_default();
    if !style.font_family.is_empty() {
        let family = style.font_family.replace(['"', '\\', '\n', '\r'], "");
        out.push(format!("font-family: \"{family}\", sans-serif"));
    }
    let size = f64::from(style.size_pt) * k;
    out.push(format!("font-size: {}", px(size)));
    if style.weight != 400 && style.weight != 0 {
        out.push(format!("font-weight: {}", style.weight));
    } else if style.faux_bold {
        out.push("font-weight: 700".into());
    }
    if style.italic || style.faux_italic {
        out.push("font-style: italic".into());
    }
    let leading = style.leading_pt.map(|v| f64::from(v) * k).or_else(|| {
        let auto = t.paragraph_runs().first().map(|p| f64::from(p.style.auto_leading)).filter(|v| v.is_finite() && *v > 0.0)?;
        Some(size * auto)
    });
    if let Some(v) = leading {
        out.push(format!("line-height: {}", px(v)));
    }
    if style.tracking != 0.0 {
        out.push(format!("letter-spacing: {}em", num(f64::from(style.tracking) / 1000.0)));
    }
    let decoration = match (style.underline, style.strikethrough) {
        (true, true) => Some("underline line-through"),
        (true, false) => Some("underline"),
        (false, true) => Some("line-through"),
        _ => None,
    };
    if let Some(d) = decoration {
        out.push(format!("text-decoration: {d}"));
    }
    let align = t.paragraph_runs().first().map(|p| p.style.align).unwrap_or_default();
    out.push(
        match align {
            TextAlign::Left => "text-align: left",
            TextAlign::Center => "text-align: center",
            TextAlign::Right => "text-align: right",
            TextAlign::JustifyLeft | TextAlign::JustifyCenter | TextAlign::JustifyRight | TextAlign::JustifyAll => "text-align: justify",
        }
        .into(),
    );
    out.push(format!("color: {}", css_color(&style.color, 1.0)));
}

fn shape_properties(l: &Layer, out: &mut Vec<String>) {
    let LayerContent::Shape(s) = &l.content else { return };
    match &s.fill {
        Some(Fill::Solid(c)) => out.push(format!("background-color: {}", css_color(c, 1.0))),
        Some(Fill::Gradient { stops, angle, style, reverse, .. }) => match gradient_css(stops, *angle, style, *reverse) {
            Some(g) => out.push(format!("background-image: {g}")),
            None => out.push("/* gradient fill not expressible in CSS */".into()),
        },
        _ => {}
    }
    if let Some(st) = &s.stroke
        && st.width > 0.0
        && let Fill::Solid(c) = &st.paint
    {
        let kind = if st.dashes.is_empty() { "solid" } else { "dashed" };
        out.push(format!("border: {} {kind} {}", px(f64::from(st.width)), css_color(c, st.opacity)));
    }
    match &s.live {
        Some(LiveShape::Ellipse { .. }) => out.push("border-radius: 50%".into()),
        Some(LiveShape::Rect { radii, .. }) if radii.iter().any(|r| *r > 0.0) => {
            let [a, b, c, d] = *radii;
            if a == b && b == c && c == d {
                out.push(format!("border-radius: {}", px(a)));
            } else {
                out.push(format!("border-radius: {} {} {} {}", px(a), px(b), px(c), px(d)));
            }
        }
        _ => {}
    }
}

/// The CSS rule of one type or shape layer (`None` for other kinds).
fn rule(l: &Layer, class: &str, dpi: f32) -> Option<String> {
    let text = matches!(l.content, LayerContent::Text(_));
    if !text && !matches!(l.content, LayerContent::Shape(_)) {
        return None;
    }
    let mut props = vec!["position: absolute".to_string()];
    if let Some([x, y, w, h]) = frame(l) {
        props.push(format!("left: {}", px(x)));
        props.push(format!("top: {}", px(y)));
        props.push(format!("width: {}", px(w)));
        props.push(format!("height: {}", px(h)));
    }
    if l.opacity < 0.999 {
        props.push(format!("opacity: {}", num(f64::from(l.opacity.clamp(0.0, 1.0)))));
    }
    match &l.content {
        LayerContent::Text(t) => text_properties(t, dpi, &mut props),
        _ => shape_properties(l, &mut props),
    }
    if let Some(s) = drop_shadow(l) {
        props.push(format!("{}: {}", if text { "text-shadow" } else { "box-shadow" }, shadow_value(s, text)));
    }
    let mut css = format!(".{class} {{\n");
    for p in props {
        let _ = writeln!(css, "  {p};");
    }
    css.push('}');
    Some(css)
}

/// Appends the rules of `l`, or of the supported layers inside it when it is a group.
fn collect(l: &Layer, dpi: f32, used: &mut HashMap<String, usize>, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_GROUP_DEPTH {
        return;
    }
    if let LayerContent::Group(g) = &l.content {
        for c in &g.children {
            collect(c, dpi, used, depth + 1, out);
        }
        return;
    }
    let class = class_name(&l.name, used);
    if let Some(r) = rule(l, &class, dpi) {
        out.push(r);
    }
}

fn targets(d: &Document, selected: &[LayerId], active: Option<LayerId>, p: &Value) -> Result<Vec<LayerId>> {
    if let Some(id) = p.get("layer") {
        let id = LayerId(id.as_u64().ok_or_else(|| EngineError::Other("layer must be a layer id".into()))?);
        return Ok(vec![id]);
    }
    let ids: Vec<LayerId> = if selected.is_empty() { active.into_iter().collect() } else { selected.to_vec() };
    if ids.is_empty() {
        return Err(EngineError::Other("select a type or shape layer".into()));
    }
    let _ = d;
    Ok(ids)
}

fn copy_css(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let ids = targets(&st.doc, &st.selected_layers, st.active_layer, p)?;
    let dpi = st.doc.resolution_dpi;
    let mut used = HashMap::new();
    let mut rules = Vec::new();
    for id in ids {
        let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        collect(l, dpi, &mut used, 0, &mut rules);
    }
    if rules.is_empty() {
        return Err(EngineError::Other("Copy CSS needs a type or shape layer (or a group holding one)".into()));
    }
    let css = rules.join("\n\n");
    Ok(json!({"css": css, "layers": rules.len(), "clipboardText": css}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "layer.copyCss",
        label: "Copy CSS",
        menu: &[],
        shortcut: None,
        params: r#"{"layer":id?} → {css, layers, clipboardText}: CSS of the type or shape layer `layer` (default: the selected layers); a group gives one rule per type or shape layer inside. The app copies `clipboardText`"#,
        enabled: |s| match s.active() {
            Some(d) if d.active_layer.is_some() || !d.selected_layers.is_empty() => Ok(()),
            Some(_) => Err("select a type or shape layer".into()),
            None => Err("no document open".into()),
        },
        journal: false,
        run: copy_css,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::text::{CharStyle, TextRun};
    use photocraft_doc::vector::{Path, ShapeLayer, ShapeStroke, Subpath};

    fn shape(name: &str, fill: Option<Fill>, live: Option<LiveShape>) -> Layer {
        let path = Path::new(vec![Subpath::polygon(&[(10.0, 20.0), (110.0, 20.0), (110.0, 80.0), (10.0, 80.0)])]);
        Layer::new(name, LayerContent::Shape(ShapeLayer { path, fill, stroke: None, live, cache: None, psd_raw: None }))
    }

    fn session(layers: Vec<Layer>) -> (Session, Vec<LayerId>) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ids = s
            .edit("setup", |doc, _| {
                let ids = layers.iter().map(|l| l.id).collect();
                doc.layers.extend(layers);
                Ok(ids)
            })
            .unwrap();
        (s, ids)
    }

    #[test]
    fn numbers_and_colours_read_as_css() {
        assert_eq!((num(1.0), num(1.50), num(-0.001), num(f64::NAN), num(12.345)), ("1".into(), "1.5".into(), "0".into(), "0".into(), "12.35".into()));
        assert_eq!(css_color(&Color::rgb(1.0, 0.0, 0.5), 1.0), "#ff0080");
        assert_eq!(css_color(&Color::rgb(0.0, 0.0, 0.0), 0.5), "rgba(0, 0, 0, 0.5)");
        let mut used = HashMap::new();
        assert_eq!(class_name("Rectangle 1", &mut used), "rectangle-1");
        assert_eq!(class_name("Rectangle 1", &mut used), "rectangle-1-2");
        assert_eq!(class_name("42 copy!", &mut used), "layer-42-copy");
        assert_eq!(class_name("", &mut used), "layer");
    }

    #[test]
    fn a_shape_gives_its_box_fill_border_and_shadow() {
        let mut l = shape("Button", Some(Fill::Solid(Color::rgb(1.0, 0.0, 0.0))), Some(LiveShape::Rect { rect: [10.0, 20.0, 100.0, 60.0], radii: [8.0; 4] }));
        if let LayerContent::Shape(sh) = &mut l.content {
            sh.stroke = Some(ShapeStroke { width: 2.0, paint: Fill::Solid(Color::BLACK), ..Default::default() });
        }
        l.opacity = 0.5;
        let (mut s, ids) = session(vec![l]);
        let css = s.execute("layer.copyCss", json!({"layer": ids[0].0})).unwrap()["css"].as_str().unwrap().to_string();
        for line in [
            ".button {",
            "  position: absolute;",
            "  left: 10px;",
            "  top: 20px;",
            "  width: 100px;",
            "  height: 60px;",
            "  opacity: 0.5;",
            "  background-color: #ff0000;",
            "  border: 2px solid #000000;",
            "  border-radius: 8px;",
        ] {
            assert!(css.contains(line), "{line:?} in\n{css}");
        }
    }

    #[test]
    fn gradients_and_ellipses_and_the_selection() {
        let stops = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
        let g = Fill::Gradient {
            stops,
            angle: 90.0,
            scale: 1.0,
            style: GradientStyle::Linear,
            reverse: false,
            opacity_stops: vec![],
            midpoints: vec![],
            dither: false,
            align: true,
            offset: (0.0, 0.0),
        };
        let (mut s, ids) = session(vec![shape("Disc", Some(g), Some(LiveShape::Ellipse { rect: [0.0, 0.0, 50.0, 50.0] }))]);
        s.execute("layer.select", json!({"layer": ids[0].0, "mode": "replace"})).unwrap();
        let r = s.execute("layer.copyCss", json!({})).unwrap();
        let css = r["css"].as_str().unwrap();
        assert!(css.contains("background-image: linear-gradient(0deg, #ff0000 0%, #0000ff 100%)"), "{css}");
        assert!(css.contains("border-radius: 50%"), "{css}");
        assert_eq!((r["layers"].as_u64(), r["clipboardText"].as_str()), (Some(1), Some(css)));
    }

    #[test]
    fn a_type_layer_gives_its_font_and_a_group_one_rule_per_layer() {
        let style = CharStyle {
            font_family: "Inter".into(),
            size_pt: 24.0,
            weight: 700,
            italic: true,
            underline: true,
            tracking: 50.0,
            leading_pt: Some(30.0),
            color: Color::rgb(0.0, 0.0, 1.0),
            ..Default::default()
        };
        let text = "Hello".to_string();
        let t = Layer::new("Title", LayerContent::Text(TextLayer { runs: vec![TextRun { len: text.len(), style }], text, ..Default::default() }));
        let inner = shape("Box", Some(Fill::Solid(Color::WHITE)), None);
        let group = Layer::group("Card", vec![t, inner]);
        let (mut s, ids) = session(vec![group]);
        let r = s.execute("layer.copyCss", json!({"layer": ids[0].0})).unwrap();
        let css = r["css"].as_str().unwrap();
        for line in [
            ".title {",
            "  font-family: \"Inter\", sans-serif;",
            "  font-size: 24px;",
            "  font-weight: 700;",
            "  font-style: italic;",
            "  line-height: 30px;",
            "  letter-spacing: 0.05em;",
            "  text-decoration: underline;",
            "  color: #0000ff;",
            ".box {",
        ] {
            assert!(css.contains(line), "{line:?} in\n{css}");
        }
        assert_eq!(r["layers"].as_u64(), Some(2));
    }

    #[test]
    fn a_drop_shadow_becomes_box_and_text_shadow() {
        use photocraft_doc::effects::FxCommon;
        let mut l = shape("Card", Some(Fill::Solid(Color::WHITE)), None);
        let sh = Shadow {
            common: FxCommon::new(photocraft_doc::BlendMode::Multiply, 0.75),
            color: Color::BLACK,
            angle: 120.0,
            use_global_light: false,
            distance: 10.0,
            spread: 0.5,
            size: 8.0,
            contour: Default::default(),
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        };
        l.effects.enabled = true;
        l.effects.items.push(Effect::DropShadow(sh));
        let (mut s, ids) = session(vec![l]);
        let css = s.execute("layer.copyCss", json!({"layer": ids[0].0})).unwrap()["css"].as_str().unwrap().to_string();
        assert!(css.contains("box-shadow: 5px 8.66px 8px 4px rgba(0, 0, 0, 0.75);"), "{css}");
    }

    #[test]
    fn unsupported_and_bad_input_is_an_error_not_a_panic() {
        let (mut s, ids) = session(vec![Layer::raster("Pixels", photocraft_doc::PixelFormat::RGBA8)]);
        assert!(s.execute("layer.copyCss", json!({"layer": ids[0].0})).is_err(), "a pixel layer has no CSS");
        for bad in [json!({"layer": "x"}), json!({"layer": -1}), json!({"layer": 999_999}), json!({"layer": null})] {
            assert!(s.execute("layer.copyCss", bad).is_err());
        }
        assert!(Session::new().execute("layer.copyCss", json!({})).is_err(), "no document");
    }
}
