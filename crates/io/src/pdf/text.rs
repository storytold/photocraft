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

/// Return PDF vector and invisible text operators for a directly representable text layer.
/// Coordinates are in document pixels with y increasing down; the caller applies the CTM.
pub(super) fn text_ops(layer: &TextLayer, mode: ColorMode, dpi: f32) -> Option<String> {
    if layer.warp.is_some() || layer.orientation != photocraft_doc::text::Orientation::Horizontal {
        return None;
    }
    if !dpi.is_finite() || dpi <= 0.0 {
        return None;
    }
    if layer.transform.m.iter().any(|v| !v.is_finite()) {
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

    for style in &layout.styles {
        let color = style.color.in_mode(mode);
        if !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return None;
        }
    }

    let outlines = photocraft_text::render::glyph_outlines(&layout, &layer.transform, None);

    let mut out = String::new();
    let hex = actual_text_hex(&layer.text);
    writeln!(out, "/Span << /ActualText <{hex}> >> BDC").ok()?;

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

    let [a, b, c, d, _e, _f] = layer.transform.m;
    let has_text_lines = layout.lines.iter().any(|l| layer.text.get(l.range.clone()).is_some_and(|s| !s.is_empty()));
    if has_text_lines {
        writeln!(out, "BT").ok()?;
        writeln!(out, "3 Tr").ok()?;
        for line in &layout.lines {
            let Some(line_text) = layer.text.get(line.range.clone()) else { continue };
            if line_text.is_empty() {
                continue;
            }
            let font_size = (line.ascent + line.descent).max(1.0);
            let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(line.x0), f64::from(line.baseline)));
            if !origin.x.is_finite() || !origin.y.is_finite() {
                return None;
            }
            let (m0, m1, m2, m3) = (a, b, -c, -d);
            writeln!(out, "/F1 {font_size} Tf").ok()?;
            writeln!(out, "{m0} {m1} {m2} {m3} {} {} Tm", origin.x, origin.y).ok()?;
            writeln!(out, "{} Tj", super::pdf_string(line_text)).ok()?;
        }
        writeln!(out, "ET").ok()?;
    }

    writeln!(out, "EMC").ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Affine;

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
        assert!(ops.contains("/Span << /ActualText <FEFF00480065006C006C006F> >> BDC"));
        assert!(ops.contains("0 0.5 1 rg"));
        assert!(ops.contains(" m\n"));
        assert!(ops.contains(" c\n"));
        assert!(ops.contains("h\nf*\n"));
        assert!(ops.contains("BT\n3 Tr\n"));
        assert!(ops.contains("/F1"));
        assert!(ops.contains("(Hello) Tj\nET\n"));
        assert!(ops.ends_with("EMC\n"));
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
