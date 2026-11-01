//! Faithful vector and selectable text export for PDF.

use std::fmt::Write as _;

use photocraft_color::ColorMode;
use photocraft_doc::TextLayer;

fn actual_text_hex(text: &str) -> String {
    let mut hex = String::from("FEFF");
    for code in text.encode_utf16() {
        let _ = write!(hex, "{code:04X}");
    }
    hex
}

fn write_fill_color(out: &mut String, color: &photocraft_color::Color, mode: ColorMode, channels: usize) -> Option<()> {
    for value in color.c.iter().take(channels) {
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return None;
        }
        write!(out, "{value} ").ok()?;
    }
    writeln!(
        out,
        "{}",
        match mode {
            ColorMode::Rgb => "rg",
            ColorMode::Grayscale => "g",
            ColorMode::Cmyk => "k",
            _ => return None,
        }
    )
    .ok()?;
    Some(())
}

/// Whether any glyph is thickened by faux bold or by a face lacking the weight: rendering
/// emboldens those glyphs, but [`photocraft_text::render::glyph_outlines`] gives the regular outline.
fn synthetic_bold(layout: &photocraft_text::TextLayout) -> bool {
    layout
        .glyphs
        .iter()
        .any(|g| layout.faces.get(g.face as usize).is_some_and(|f| f.embolden) || layout.styles.get(g.style as usize).is_some_and(|s| s.faux_bold))
}

/// Whether `layer` is laid out in straight horizontal lines with a finite transform and `dpi`.
/// Path text is bent along its path when rendered, and the outlines written here are not.
fn straight_horizontal(layer: &TextLayer, dpi: f32) -> bool {
    layer.warp.is_none()
        && !matches!(layer.shape, photocraft_doc::text::TextShape::Path { .. })
        && layer.orientation == photocraft_doc::text::Orientation::Horizontal
        && dpi.is_finite()
        && dpi > 0.0
        && layer.transform.m.iter().all(|v| v.is_finite())
}

/// Return PDF vector and invisible text operators for a directly representable text layer.
/// Coordinates are in document pixels with y increasing down; the caller applies the CTM.
#[cfg(test)]
fn text_ops(layer: &TextLayer, mode: ColorMode, dpi: f32) -> Option<String> {
    text_ops_with_font(layer, mode, dpi, &super::selection::SelectionFont::new(std::iter::once(layer.text.as_str())))
}

fn outlines_permitted(bytes: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(bytes, index) else { return false };
    matches!(face.permissions(), Some(ttf_parser::Permissions::Installable | ttf_parser::Permissions::PreviewAndPrint | ttf_parser::Permissions::Editable))
        && face.is_outline_embedding_allowed()
        && face.is_subsetting_allowed()
}

pub(super) fn text_ops_with_font(layer: &TextLayer, mode: ColorMode, dpi: f32, selection_font: &super::selection::SelectionFont) -> Option<String> {
    selection_font.encode(&layer.text)?;
    if !straight_horizontal(layer, dpi) {
        return None;
    }
    let channels = match mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        _ => return None,
    };
    if layer.text.is_empty() {
        return Some(String::new());
    }

    let mut engine = photocraft_text::TextEngine::new();
    let layout = engine.layout(layer, dpi);
    layout_ops(layer, mode, channels, &layout, selection_font)
}

fn layout_ops(
    layer: &TextLayer,
    mode: ColorMode,
    channels: usize,
    layout: &photocraft_text::TextLayout,
    selection_font: &super::selection::SelectionFont,
) -> Option<String> {
    if synthetic_bold(layout) || layout.faces.iter().any(|face| !outlines_permitted(face.font.data.as_ref(), face.font.index)) {
        return None;
    }

    for style in &layout.styles {
        let color = style.color.in_mode(mode);
        if !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return None;
        }
    }

    let outlines = photocraft_text::render::glyph_outlines(layout, &layer.transform, None);

    let mut out = String::new();

    let mut current_color: Option<photocraft_color::Color> = None;

    for glyph in &outlines {
        let style = layout.styles.get(glyph.style as usize)?;
        let color = style.color.in_mode(mode);
        if current_color.as_ref() != Some(&color) {
            write_fill_color(&mut out, &color, mode, channels)?;
            current_color = Some(color);
        }
        let mut cur = [0.0f64, 0.0f64];
        for el in &glyph.elements {
            match *el {
                photocraft_text::render::PathEl::MoveTo([x, y]) => {
                    if !x.is_finite() || !y.is_finite() {
                        return None;
                    }
                    cur = [x, y];
                    writeln!(out, "{x} {y} m").ok()?;
                }
                photocraft_text::render::PathEl::LineTo([x, y]) => {
                    if !x.is_finite() || !y.is_finite() {
                        return None;
                    }
                    cur = [x, y];
                    writeln!(out, "{x} {y} l").ok()?;
                }
                photocraft_text::render::PathEl::QuadTo([cx, cy], [x, y]) => {
                    if !cx.is_finite() || !cy.is_finite() || !x.is_finite() || !y.is_finite() {
                        return None;
                    }
                    let c1x = cur[0] + 2.0 / 3.0 * (cx - cur[0]);
                    let c1y = cur[1] + 2.0 / 3.0 * (cy - cur[1]);
                    let c2x = x + 2.0 / 3.0 * (cx - x);
                    let c2y = y + 2.0 / 3.0 * (cy - y);
                    cur = [x, y];
                    writeln!(out, "{c1x} {c1y} {c2x} {c2y} {x} {y} c").ok()?;
                }
                photocraft_text::render::PathEl::CurveTo([c1x, c1y], [c2x, c2y], [x, y]) => {
                    if !c1x.is_finite() || !c1y.is_finite() || !c2x.is_finite() || !c2y.is_finite() || !x.is_finite() || !y.is_finite() {
                        return None;
                    }
                    cur = [x, y];
                    writeln!(out, "{c1x} {c1y} {c2x} {c2y} {x} {y} c").ok()?;
                }
                photocraft_text::render::PathEl::Close => {
                    writeln!(out, "h").ok()?;
                }
            }
        }
        writeln!(out, "f*").ok()?;
    }

    for d in &layout.decorations {
        let style = layout.styles.get(d.style as usize)?;
        let color = style.color.in_mode(mode);
        if current_color.as_ref() != Some(&color) {
            write_fill_color(&mut out, &color, mode, channels)?;
            current_color = Some(color);
        }
        let p0 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x0), f64::from(d.y0)));
        let p1 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x1), f64::from(d.y0)));
        let p2 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x1), f64::from(d.y1)));
        let p3 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x0), f64::from(d.y1)));
        if !p0.x.is_finite()
            || !p0.y.is_finite()
            || !p1.x.is_finite()
            || !p1.y.is_finite()
            || !p2.x.is_finite()
            || !p2.y.is_finite()
            || !p3.x.is_finite()
            || !p3.y.is_finite()
        {
            return None;
        }
        writeln!(out, "{} {} m", p0.x, p0.y).ok()?;
        writeln!(out, "{} {} l", p1.x, p1.y).ok()?;
        writeln!(out, "{} {} l", p2.x, p2.y).ok()?;
        writeln!(out, "{} {} l", p3.x, p3.y).ok()?;
        writeln!(out, "h").ok()?;
        writeln!(out, "f").ok()?;
    }

    let [a, b, c, d, _, _] = layer.transform.m;
    let mut current_line = None;
    let mut clusters: Vec<_> = layout.clusters.iter().collect();
    clusters.sort_by_key(|cluster| cluster.range.start);
    for cluster in clusters {
        let text = layer.text.get(cluster.range.clone())?;
        if text.is_empty() {
            continue;
        }
        let line = layout.lines.get(cluster.line)?;
        if current_line != Some(cluster.line) {
            if current_line.is_some() {
                writeln!(out, "ET").ok()?;
            }
            writeln!(out, "BT\n3 Tr\n/FSelect 1 Tf").ok()?;
            current_line = Some(cluster.line);
        }
        let height = (line.ascent + line.descent).max(1.0);
        let width = cluster.advance.abs() / text.chars().count().max(1) as f32;
        if !height.is_finite() || !width.is_finite() {
            return None;
        }
        let x = cluster.x;
        let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(x), f64::from(line.baseline)));
        if !origin.x.is_finite() || !origin.y.is_finite() {
            return None;
        }
        let hex = actual_text_hex(text);
        let encoded = selection_font.encode(text)?;
        writeln!(out, "/Span << /ActualText <{hex}> >> BDC").ok()?;
        writeln!(out, "{} {} {} {} {} {} Tm", a * f64::from(width), b * f64::from(width), -c * f64::from(height), -d * f64::from(height), origin.x, origin.y)
            .ok()?;
        writeln!(out, "<{encoded}> Tj\nEMC").ok()?;
    }

    if current_line.is_some() {
        writeln!(out, "ET").ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Affine;

    #[test]
    fn embedding_restrictions_refuse_outlines() {
        let original = photocraft_text::fonts::INTER_REGULAR.as_slice();
        assert!(outlines_permitted(original, 0));
        let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
        let record = (0..count).map(|i| 12 + 16 * i).find(|at| &original[*at..*at + 4] == b"OS/2").unwrap();
        let offset = u32::from_be_bytes(original[record + 8..record + 12].try_into().unwrap()) as usize;
        for flags in [0x0002u16, 0x0100, 0x0200] {
            let mut restricted = original.to_vec();
            restricted[offset + 8..offset + 10].copy_from_slice(&flags.to_be_bytes());
            assert!(!outlines_permitted(&restricted, 0), "fsType {flags:#x}");
            let layer = TextLayer { text: "Restricted".into(), ..Default::default() };
            let mut layout = photocraft_text::TextEngine::new().layout(&layer, 72.0);
            for face in &mut layout.faces {
                face.font.data = restricted.clone().into();
                face.font.index = 0;
            }
            let font = super::super::selection::SelectionFont::new(std::iter::once(layer.text.as_str()));
            assert!(layout_ops(&layer, ColorMode::Rgb, 3, &layout, &font).is_none(), "restricted text must remain raster");
        }
        assert!(!outlines_permitted(&[], 0));
    }

    #[test]
    fn text_layer_emits_actualtext_outlines_and_invisible_text() {
        let layer = TextLayer {
            text: "Hello".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(0.0, 0.5, 1.0),
            transform: Affine::translate(10.0, 20.0),
            ..Default::default()
        };
        let ops = text_ops(&layer, ColorMode::Rgb, 72.0).unwrap();
        assert!(ops.contains("/Span << /ActualText <FEFF0048> >> BDC"));
        assert!(ops.contains("0 0.5 1 rg"));
        assert!(ops.contains(" m\n"));
        assert!(ops.contains(" c\n"));
        assert!(ops.contains("h\nf*\n"));
        assert!(ops.contains("BT\n3 Tr\n"));
        assert!(ops.contains("/FSelect"));
        assert!(ops.contains("> Tj\nEMC\n"));
        assert!(ops.ends_with("ET\n"));
    }

    #[test]
    fn rejects_warp_and_invalid_alpha() {
        let warped = TextLayer { text: "Warped".into(), warp: Some(photocraft_doc::text::TextWarp::default()), ..Default::default() };
        assert!(text_ops(&warped, ColorMode::Rgb, 72.0).is_none());

        let transparent = TextLayer {
            text: "Alpha".into(),
            color: photocraft_color::Color { mode: ColorMode::Rgb, c: [1.0, 0.0, 0.0, 0.0], alpha: 0.5 },
            ..Default::default()
        };
        assert!(text_ops(&transparent, ColorMode::Rgb, 72.0).is_none());
    }

    #[test]
    fn rejects_path_text_and_synthetic_bold() {
        let on_path = TextLayer {
            text: "Path".into(),
            shape: photocraft_doc::text::TextShape::Path { path: Default::default(), start: 0.0, flip: false },
            ..Default::default()
        };
        assert!(text_ops(&on_path, ColorMode::Rgb, 72.0).is_none());

        let faux_bold = TextLayer {
            text: "Bold".into(),
            runs: vec![photocraft_doc::text::TextRun { len: 4, style: photocraft_doc::text::CharStyle { faux_bold: true, ..Default::default() } }],
            ..Default::default()
        };
        assert!(text_ops(&faux_bold, ColorMode::Rgb, 72.0).is_none());

        // A face without the requested weight is emboldened by font matching.
        let plain = TextLayer { text: "Bold".into(), ..Default::default() };
        let mut layout = photocraft_text::TextEngine::new().layout(&plain, 72.0);
        assert!(!synthetic_bold(&layout));
        for face in &mut layout.faces {
            face.embolden = true;
        }
        assert!(synthetic_bold(&layout));
    }

    #[test]
    fn decorations_are_emitted_as_rectangles() {
        let layer = TextLayer {
            text: "Underline".into(),
            runs: vec![photocraft_doc::text::TextRun {
                len: 9,
                style: photocraft_doc::text::CharStyle { underline: true, color: photocraft_color::Color::rgb(1.0, 0.0, 0.0), ..Default::default() },
            }],
            ..Default::default()
        };
        let ops = text_ops(&layer, ColorMode::Rgb, 72.0).unwrap();
        assert!(ops.contains("h\nf\n"));
    }
}
