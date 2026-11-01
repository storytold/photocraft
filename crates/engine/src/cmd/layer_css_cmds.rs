//! Layer › Copy CSS (#9372): CSS for the selected layers, in the form Photoshop writes. The
//! command returns the text; the shell puts it on the clipboard.
//!
//! Each layer is a class named after it (punctuation and spaces become `_`, and a leading digit
//! gets a `_` in front). What is written, in this order:
//! - a type layer: `font-size`, `font-family`, `color`, `line-height` and `text-align`, from its
//!   first run and first paragraph;
//! - a live rounded rectangle: `border-radius`; a live ellipse: `border-radius: 50%`;
//! - a shape or fill layer with a solid colour: `background-color`; any other layer (pixels,
//!   gradient or pattern fills, smart objects): `background-image`, an image named after it;
//! - `opacity` when it isn't 100 %;
//! - `position: absolute`, `left` and `top`, and `width` and `height` except for point type;
//! - `z-index`, its place in the stack counted from the bottom layer as 1.
//!
//! Not written: strokes (`border`), layer effects (`box-shadow`, `text-shadow`), gradients as
//! CSS gradients, blend modes (`mix-blend-mode`), fill opacity, the styles of later runs and
//! paragraphs, the corners of a rounded rectangle scaled unequally (elliptical, while it keeps
//! one radius per corner), and the geometry of other shapes (live polygons, stars and lines, and
//! custom paths), which get the rule of their box. Groups and adjustment layers have no rule.

use photocraft_doc::text::{TextAlign, TextShape};
use photocraft_doc::{Document, Fill, Layer, LayerContent, LayerId, LiveShape, Path};
use photocraft_geom::Point;
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

/// The class name for a layer: its name with every character other than a letter, digit or `_`
/// as `_` (a hyphen too, as Photoshop does), and a `_` in front when it starts with a digit,
/// which a class selector can't.
pub fn class_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.is_empty() {
        "layer".into()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{s}")
    } else {
        s
    }
}

/// `s` as a double-quoted CSS string, escaped as CSSOM serializes one: `"` and `\` get a
/// backslash, control characters (line breaks among them) become hex escapes, NUL becomes U+FFFD.
pub fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{1F}' | '\u{7F}' => out.push_str(&format!("\\{:x} ", u32::from(c))),
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The image Photoshop names after a layer, as a `url()`.
fn image(l: &Layer) -> String {
    format!("url({})", quoted(&format!("{}.png", l.name)))
}

fn rgb(c: &photocraft_color::Color) -> String {
    let [r, g, b] = c.to_rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("rgb({r}, {g}, {b})")
}

/// Whether `path` is the outline `live` draws: every anchor of one is an anchor of the other,
/// within a hundredth of a pixel, in any order or direction (a flipped shape runs the other way).
/// A live rectangle scaled unequally keeps one radius per corner, but its path keeps the corners
/// it was drawn with, now elliptical, so the two differ.
fn draws(live: &LiveShape, path: &Path) -> bool {
    let anchors = |p: &Path| p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor)).collect::<Vec<_>>();
    let (have, want) = (anchors(path), anchors(&photocraft_vector::shapes::live_path(live)));
    let among = |p: &Point, q: &[Point]| q.iter().any(|q| (p.x - q.x).abs() < 0.01 && (p.y - q.y).abs() < 0.01);
    have.iter().all(|p| among(p, &want)) && want.iter().all(|p| among(p, &have))
}

/// The rule for one layer, `z` its place in the stack from the bottom (1-based); `None` for a
/// layer CSS can't describe (groups, adjustments) or one with nothing to place.
pub fn rule(doc: &Document, z: usize, l: &Layer) -> Option<String> {
    let px_per_pt = f64::from(doc.resolution_dpi.max(1.0)) / 72.0;
    let mut props: Vec<(&str, String)> = Vec::new();
    let mut sized = true;
    let mut bounds = photocraft_doc::comps::position_bounds(l);
    // Where the rule starts when it isn't where the layer's pixels start.
    let mut origin: Option<Point> = None;
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
            props.push(("font-family", quoted(family)));
            props.push(("color", rgb(&color)));
            props.push(("line-height", number(line_height)));
            props.push(("text-align", align.into()));
            match t.shape {
                // Photoshop places paragraph text at its box, moved down by an explicit leading,
                // and sizes it by its text, not the box.
                TextShape::Box { x, y, .. } => {
                    let at = t.transform.apply(Point::new(f64::from(x), f64::from(y)));
                    let leading = style.and_then(|s| s.leading_pt).map_or(0.0, |v| f64::from(v) * px_per_pt);
                    origin = Some(Point::new(at.x, at.y + leading));
                }
                // Point text has no box of its own: only its position.
                _ => sized = false,
            }
        }
        LayerContent::Shape(sh) => {
            // A live rounded rectangle's corners, before the fill as Photoshop writes them: one
            // value when all four are equal, else top-left, top-right, bottom-right, bottom-left.
            // Scaled unequally, its corners are elliptical though it keeps one radius each: none.
            // A live ellipse fills its box, which half the box's size on each axis rounds to.
            match &sh.live {
                Some(live @ LiveShape::Rect { radii, .. }) if radii.iter().any(|&r| r > 0.0) && draws(live, &sh.path) => {
                    let px = |r: f64| format!("{}px", number(r.max(0.0)));
                    let all = if radii.iter().all(|&r| r == radii[0]) { px(radii[0]) } else { radii.map(px).join(" ") };
                    props.push(("border-radius", all));
                }
                Some(LiveShape::Ellipse { .. }) => props.push(("border-radius", "50%".into())),
                _ => {}
            }
            match &sh.fill {
                Some(Fill::Solid(c)) => props.push(("background-color", rgb(c))),
                _ => props.push(("background-image", image(l))),
            }
        }
        LayerContent::Fill(f) => {
            // A fill layer without a shape covers the canvas.
            if bounds.is_empty() {
                bounds = doc.bounds();
            }
            match f {
                Fill::Solid(c) => props.push(("background-color", rgb(c))),
                _ => props.push(("background-image", image(l))),
            }
        }
        _ => props.push(("background-image", image(l))),
    }
    if bounds.is_empty() {
        return None;
    }
    if l.opacity < 1.0 {
        props.push(("opacity", number(f64::from(l.opacity.max(0.0)))));
    }
    props.push(("position", "absolute".into()));
    let at = origin.unwrap_or(Point::new(f64::from(bounds.x0), f64::from(bounds.y0)));
    props.push(("left", format!("{}px", number(at.x))));
    props.push(("top", format!("{}px", number(at.y))));
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
