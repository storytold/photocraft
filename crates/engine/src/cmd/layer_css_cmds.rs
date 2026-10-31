//! Layer › Copy CSS (#9372): CSS for the selected layers, written as Photoshop writes it. The
//! command returns the text; the shell puts it on the clipboard.
//!
//! Each layer is a class named after it (characters CSS doesn't allow in a name become `_`):
//! a type layer gives its font, colour, line height and alignment, a solid shape or fill layer
//! its colour, any other layer an image named after it; then the opacity when it isn't 100 %,
//! the absolute position (and size, except for type), and its place in the stack as `z-index`,
//! counted from the bottom layer as 1.

use photocraft_doc::{Document, Fill, Layer, LayerContent, LayerId, text::TextAlign};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const ID: &str = "layer.copyCSS";

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selected_layers().is_empty() { Err("no layer selected".into()) } else { Ok(()) }
}

/// A number as Photoshop writes it in CSS: at most three decimals, no trailing zeros.
pub fn number(v: f64) -> String {
    let v = if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 };
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The class name for a layer: its name with every character CSS doesn't allow in a name as `_`.
pub fn class_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "layer".into() } else { s }
}

fn rgb(c: &photocraft_color::Color) -> String {
    let [r, g, b] = c.to_rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("rgb({r}, {g}, {b})")
}

/// The rule for one layer, `z` its place in the stack from the bottom (1-based); `None` for a
/// layer CSS can't describe (groups, adjustments) or one with nothing to place.
pub fn rule(doc: &Document, z: usize, l: &Layer) -> Option<String> {
    let px_per_pt = f64::from(doc.resolution_dpi.max(1.0)) / 72.0;
    let mut props: Vec<(&str, String)> = Vec::new();
    let mut sized = true;
    let mut bounds = photocraft_doc::comps::position_bounds(l);
    match &l.content {
        LayerContent::Group(_) | LayerContent::Adjustment(_) => return None,
        LayerContent::Text(t) => {
            let style = t.runs.first().map(|r| &r.style);
            let size_pt = style.map_or(t.size_pt, |s| s.size_pt);
            let family = style.map(|s| s.font_family.as_str()).filter(|f| !f.is_empty()).unwrap_or(&t.font_family);
            let color = style.map_or(t.color, |s| s.color);
            let paragraph = t.paragraphs.first().map(|p| p.style.clone()).unwrap_or_default();
            // Auto leading is the paragraph's multiple of the size (1.2 by default).
            let line_height = match style.and_then(|s| s.leading_pt) {
                Some(leading) if size_pt > 0.0 => f64::from(leading / size_pt),
                _ => f64::from(paragraph.auto_leading),
            };
            let align = match paragraph.align {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
                _ => "justify",
            };
            props.push(("font-size", format!("{}px", number(f64::from(size_pt) * px_per_pt))));
            props.push(("font-family", format!("\"{}\"", family.replace('"', "'"))));
            props.push(("color", rgb(&color)));
            props.push(("line-height", number(line_height)));
            props.push(("text-align", align.into()));
            // Point text has no box of its own: only its position.
            sized = false;
        }
        LayerContent::Shape(sh) => match &sh.fill {
            Some(Fill::Solid(c)) => props.push(("background-color", rgb(c))),
            _ => props.push(("background-image", format!("url(\"{}.png\")", l.name))),
        },
        LayerContent::Fill(f) => {
            // A fill layer without a shape covers the canvas.
            if bounds.is_empty() {
                bounds = doc.bounds();
            }
            match f {
                Fill::Solid(c) => props.push(("background-color", rgb(c))),
                _ => props.push(("background-image", format!("url(\"{}.png\")", l.name))),
            }
        }
        _ => props.push(("background-image", format!("url(\"{}.png\")", l.name))),
    }
    if bounds.is_empty() {
        return None;
    }
    if l.opacity < 1.0 {
        props.push(("opacity", number(f64::from(l.opacity.max(0.0)))));
    }
    props.push(("position", "absolute".into()));
    props.push(("left", format!("{}px", number(f64::from(bounds.x0)))));
    props.push(("top", format!("{}px", number(f64::from(bounds.y0)))));
    if sized {
        props.push(("width", format!("{}px", number(f64::from(bounds.width())))));
        props.push(("height", format!("{}px", number(f64::from(bounds.height())))));
    }
    props.push(("z-index", z.to_string()));
    let mut out = format!(".{} {{\n", class_name(&l.name));
    for (k, v) in props {
        out.push_str(&format!("  {k}: {v};\n"));
    }
    out.push_str("}\n");
    Some(out)
}

fn copy_css(s: &mut Session, p: &Value) -> Result<Value> {
    let p = p.as_object().ok_or_else(|| bad("expected an object"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let ids = match p.get("layer") {
        None | Some(Value::Null) => st.selected_layers(),
        Some(v) => vec![LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be an unsigned integer"))?)],
    };
    if ids.is_empty() {
        return Err(bad("no layer selected"));
    }
    let walk = st.doc.walk();
    for id in &ids {
        if !walk.iter().any(|(_, _, l)| l.id == *id) {
            return Err(EngineError::NoLayer(*id));
        }
    }
    // In Layers panel order, top first.
    let rules: Vec<(u64, String)> = walk
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, (_, _, l))| ids.contains(&l.id))
        .filter_map(|(i, (_, _, l))| rule(&st.doc, i + 1, l).map(|r| (l.id.0, r)))
        .collect();
    if rules.is_empty() {
        return Err(EngineError::Other("Copy CSS needs a pixel, type, shape or fill layer with content".into()));
    }
    let css = rules.iter().map(|(_, r)| r.as_str()).collect::<Vec<_>>().join("\n");
    Ok(json!({"css": css, "layers": rules.iter().map(|(id, _)| id).collect::<Vec<_>>()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Copy CSS",
        menu: &["Layer"],
        shortcut: None,
        params: r#"{"layer":id?} (no layer: the selected layers) → {"css": text, "layers": [id…]}; the app puts the text on the clipboard"#,
        enabled,
        run: copy_css,
        journal: false,
    }]
}

#[cfg(test)]
mod tests;
