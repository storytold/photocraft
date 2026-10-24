//! Stock object property data translated to ordinary editable native objects.
//! Facts: GEGL 0.4.72 external/{text,path,vector-fill,vector-stroke}.c
//! property declarations only; no source shaping, paint or path evaluator.
use super::graph_schema::Kind;
use super::residual_spatial::{color, generation_bytes, invalid, replace};
use super::{ArgumentValue, MappingContext, MappingFailure, MappingResult, NativeMapping, NativeStage, NativeStructure, StoredEffect};
use photocraft_doc::text::{ParagraphRun, ParagraphStyle, TextAlign, TextShape};
use photocraft_doc::{Fill, FillRule, Knot, Layer, LayerContent, LineCap, LineJoin, Path, PathOp, ShapeLayer, ShapeStroke, Subpath, TextLayer};
use photocraft_geom::{Affine, Point};

const NAMES: &[&str] = &["gegl:text", "gegl:path", "gegl:fill-path", "gegl:vector-stroke"];

pub(super) fn operation(token: &str) -> Option<&'static str> {
    NAMES.iter().copied().find(|name| *name == token || name.strip_prefix("gegl:") == Some(token))
}
pub(super) fn generator(op: &str) -> bool {
    op == "gegl:text"
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    let opacity = Kind::Number { min: -2., max: 2. };
    Some(match (op, name) {
        ("gegl:text", "string" | "font") => Kind::Text,
        ("gegl:text", "size") => Kind::Number { min: 0., max: 2048. },
        ("gegl:text", "wrap" | "vertical-wrap") => Kind::Integer { min: -1, max: 1_000_000 },
        ("gegl:text", "alignment" | "vertical-alignment") => Kind::Integer { min: 0, max: 2 },
        ("gegl:text", "width" | "height") => Kind::Integer { min: i32::MIN as i64, max: i32::MAX as i64 },
        ("gegl:text" | "gegl:fill-path" | "gegl:vector-stroke", "color") | ("gegl:path", "fill" | "stroke") => Kind::Color,
        ("gegl:path" | "gegl:fill-path" | "gegl:vector-stroke", "d") => Kind::Text,
        ("gegl:path" | "gegl:fill-path" | "gegl:vector-stroke", "transform") | ("gegl:path" | "gegl:fill-path", "fill-rule") => Kind::Text,
        ("gegl:path", "stroke-width") | ("gegl:vector-stroke", "width") => Kind::Number { min: 0., max: 200. },
        ("gegl:path", "stroke-opacity" | "fill-opacity") | ("gegl:fill-path" | "gegl:vector-stroke", "opacity") => opacity,
        ("gegl:path", "stroke-hardness") => Kind::Number { min: 0., max: 1. },
        _ => return None,
    })
}
fn string<'a>(e: &'a StoredEffect<'_>, key: &str, default: &'a str, limit: usize) -> Result<&'a str, MappingFailure> {
    match e.argument(key).map_err(invalid)? {
        None => Ok(default),
        Some(ArgumentValue::String(value)) if value.len() <= limit && !value.contains('\0') => Ok(value),
        _ => Err(invalid(format!("{} {key} requires bounded UTF-8 text without NUL", e.operation))),
    }
}
fn number(e: &StoredEffect<'_>, key: &str, default: f64) -> Result<f64, MappingFailure> {
    let value = e.number(key).map_err(invalid)?.unwrap_or(default);
    let valid = match field(e.operation, key) {
        Some(Kind::Number { min, max }) => value.is_finite() && value >= min && value <= max,
        Some(Kind::Integer { min, max }) => value.is_finite() && value.fract() == 0. && value >= min as f64 && value <= max as f64,
        _ => false,
    };
    if valid { Ok(value) } else { Err(invalid(format!("{} {key} is outside its declared numeric domain", e.operation))) }
}
struct Numbers<'a> {
    text: &'a str,
    at: usize,
}
impl Numbers<'_> {
    fn space(&mut self) {
        while self.text.as_bytes().get(self.at).is_some_and(|v| v.is_ascii_whitespace() || *v == b',') {
            self.at += 1;
        }
    }
    fn number(&mut self) -> Result<f64, MappingFailure> {
        self.space();
        let start = self.at;
        while self.text.as_bytes().get(self.at).is_some_and(|v| v.is_ascii_digit() || matches!(*v, b'+' | b'-' | b'.' | b'e' | b'E')) {
            if self.at > start
                && self.text.as_bytes().get(self.at).is_some_and(|v| matches!(*v, b'+' | b'-'))
                && !self.text.as_bytes().get(self.at - 1).is_some_and(|v| matches!(*v, b'e' | b'E'))
            {
                break;
            }
            self.at += 1;
        }
        let value = self
            .text
            .get(start..self.at)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && v.abs() <= 1e9)
            .ok_or_else(|| invalid("stock vector requires finite coordinates within the native bounded geometry domain"))?;
        Ok(value)
    }
}
fn path_data(literal: &str, c: &MappingContext<'_>) -> Result<Path, MappingFailure> {
    if !literal.is_ascii() {
        return Err("Stock path uses a non-ASCII/custom knot grammar not established by the builtin declarations".into());
    }
    let allowance = literal
        .len()
        .checked_mul(std::mem::size_of::<Knot>() + std::mem::size_of::<Subpath>())
        .ok_or_else(|| MappingFailure::resource("native path work allowance overflows"))?;
    generation_bytes(c, allowance)?;
    if literal.len().checked_mul(16).is_none_or(|v| v > c.remaining_metadata_bytes) {
        return Err(MappingFailure::resource("native path exceeds remaining metadata allowance"));
    }
    // Pinned GeglPath's declarative knot table names lowercase s as a
    // zero-argument sentinel. Alone it carries no geometric segment.
    if literal.trim() == "s" {
        return Ok(Path::new(Vec::new()));
    }
    let mut input = Numbers { text: literal, at: 0 };
    let mut paths = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut pen = Point::new(0., 0.);
    let mut count = 0usize;
    loop {
        input.space();
        let Some(command) = input.text.as_bytes().get(input.at).copied() else {
            break;
        };
        input.at += 1;
        c.interrupt.check().map_err(|_| MappingFailure::resource("native path conversion cancelled"))?;
        count = count.checked_add(1).ok_or_else(|| invalid("native path knot count overflows"))?;
        if count > 65_536 {
            return Err(MappingFailure::resource("native path knot count exceeds limit"));
        }
        match command {
            b'M'|b'm' => {
                let (x,y) = (input.number()?,input.number()?);
                if let Some(path) = current.take() { paths.try_reserve(1).map_err(|_| MappingFailure::resource("native path reservation unavailable"))?; paths.push(path); }
                pen = if command == b'm' { Point::new(pen.x+x,pen.y+y) } else { Point::new(x,y) };
                let mut path = Subpath {op:if paths.is_empty() {PathOp::Combine} else {PathOp::Join},..Default::default()};
                path.knots.try_reserve(1).map_err(|_| MappingFailure::resource("native path knot reservation unavailable"))?;
                path.knots.push(Knot::corner(pen.x,pen.y)); current = Some(path);
            }
            b'L'|b'l' => {
                let (x,y) = (input.number()?,input.number()?);
                let path = current.as_mut().ok_or_else(|| invalid("stock path line requires an actual initial move"))?;
                pen = if command == b'l' { Point::new(pen.x+x,pen.y+y) } else { Point::new(x,y) };
                path.knots.try_reserve(1).map_err(|_| MappingFailure::resource("native path knot reservation unavailable"))?;
                path.knots.push(Knot::corner(pen.x,pen.y));
            }
            b'C'|b'c' => {
                let mut values = [0.;6]; for value in &mut values { *value = input.number()?; }
                let offset = if command == b'c' {pen} else {Point::new(0.,0.)};
                let first = Point::new(values[0]+offset.x,values[1]+offset.y);
                let second = Point::new(values[2]+offset.x,values[3]+offset.y);
                let end = Point::new(values[4]+offset.x,values[5]+offset.y);
                let path = current.as_mut().ok_or_else(|| invalid("stock cubic requires an actual initial move"))?;
                let previous = path.knots.last_mut().ok_or_else(|| invalid("stock cubic has no initial anchor"))?;
                previous.out_ctrl = first;
                path.knots.try_reserve(1).map_err(|_| MappingFailure::resource("native path knot reservation unavailable"))?;
                path.knots.push(Knot {anchor:end,in_ctrl:second,out_ctrl:end,smooth:false}); pen = end;
            }
            b'z' => {
                let mut path = current.take().ok_or_else(|| invalid("stock close requires an actual subpath"))?;
                path.closed = true;
                if let Some(first) = path.knots.first() { pen = first.anchor; }
                paths.try_reserve(1).map_err(|_| MappingFailure::resource("native path reservation unavailable"))?; paths.push(path);
            }
            _ => return Err("Stored GeglPath knot is outside the established explicit M/L/C/m/l/c/z grammar and standalone stock sentinel; smooth/implicit/custom syntax remains retained without inventing a source evaluator".into()),
        }
        if !pen.x.is_finite() || !pen.y.is_finite() || pen.x.abs() > 1e9 || pen.y.abs() > 1e9 {
            return Err(MappingFailure::resource("relative native path exceeds bounded coordinates"));
        }
    }
    if let Some(path) = current {
        paths.try_reserve(1).map_err(|_| MappingFailure::resource("native path reservation unavailable"))?;
        paths.push(path);
    }
    Ok(Path::new(paths))
}
fn transform_data(value: &str) -> Result<Affine, MappingFailure> {
    if value.trim() == "none" {
        return Ok(Affine::IDENTITY);
    }
    let mut input = Numbers { text: value, at: 0 };
    let mut result = Affine::IDENTITY;
    let mut count = 0;
    loop {
        input.space();
        if input.at == value.len() {
            return Ok(result);
        }
        let start = input.at;
        while value.as_bytes().get(input.at).is_some_and(u8::is_ascii_alphabetic) {
            input.at += 1;
        }
        let name = value.get(start..input.at).ok_or_else(|| invalid("invalid native vector transform boundary"))?;
        input.space();
        if value.as_bytes().get(input.at) != Some(&b'(') {
            return Err("Stock vector transform requires a bounded ordinary SVG matrix/translate/scale/rotate/skew list".into());
        }
        input.at += 1;
        let mut args = Vec::new();
        loop {
            input.space();
            if value.as_bytes().get(input.at) == Some(&b')') {
                input.at += 1;
                break;
            }
            if args.len() >= 6 {
                return Err(invalid("stock vector transform has excess numeric parameters"));
            }
            args.try_reserve(1).map_err(|_| MappingFailure::resource("native transform reservation unavailable"))?;
            args.push(input.number()?);
        }
        let matrix = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine { m: [*a, *b, *c, *d, *e, *f] },
            ("translate", [x]) => Affine::translate(*x, 0.),
            ("translate", [x, y]) => Affine::translate(*x, *y),
            ("scale", [x]) => Affine { m: [*x, 0., 0., *x, 0., 0.] },
            ("scale", [x, y]) => Affine { m: [*x, 0., 0., *y, 0., 0.] },
            ("rotate", [angle]) => Affine::rotate(angle.to_radians()),
            ("rotate", [angle, x, y]) => Affine::translate(*x, *y).mul(&Affine::rotate(angle.to_radians())).mul(&Affine::translate(-*x, -*y)),
            ("skewX", [angle]) => Affine { m: [1., 0., angle.to_radians().tan(), 1., 0., 0.] },
            ("skewY", [angle]) => Affine { m: [1., angle.to_radians().tan(), 0., 1., 0., 0.] },
            _ => return Err("Stock vector transform list contains an unestablished transform or parameter form".into()),
        };
        result = result.mul(&matrix);
        count += 1;
        if count > 64 || result.m.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
            return Err(MappingFailure::resource("native vector transform exceeds bounded finite placement"));
        }
    }
}
fn path_object(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<NativeMapping, MappingFailure> {
    let literal = string(e, "d", "", 65_536)?;
    if literal.is_empty() {
        return Ok(NativeMapping {
            notes: vec!["Nullable stock path is absent; preserve the actual bound native input without inventing paint.".into()],
            ..Default::default()
        });
    }
    let mut path = path_data(literal, c)?;
    let matrix = transform_data(string(e, "transform", "", 4096)?)?;
    path = path.transform(&matrix);
    if path
        .subpaths
        .iter()
        .flat_map(|p| &p.knots)
        .flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl])
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e9 || p.y.abs() > 1e9)
    {
        return Err(MappingFailure::resource("transformed native vector exceeds bounded finite coordinates"));
    }
    let rule = string(e, "fill-rule", "nonzero", 32)?;
    path.fill_rule = match rule {
        "nonzero" => FillRule::NonZero,
        "evenodd" => FillRule::EvenOdd,
        _ => return Err(invalid("stock path fill-rule must be nonzero or evenodd")),
    };
    let fill = if e.operation == "gegl:vector-stroke" {
        None
    } else {
        let (key, default, alpha) = if e.operation == "gegl:path" {
            ("fill", [0., 0., 0., 0.6], number(e, "fill-opacity", 1.)?)
        } else {
            ("color", [0., 0., 0., 1.], number(e, "opacity", 1.)?)
        };
        let mut paint = color(e, key, default, c)?;
        paint.alpha *= (alpha as f32).clamp(0., 1.);
        Some(Fill::Solid(paint))
    };
    let stroke = if e.operation == "gegl:fill-path" {
        None
    } else {
        let (key, width_key, opacity_key, default) = if e.operation == "gegl:path" {
            ("stroke", "stroke-width", "stroke-opacity", [0., 0., 0., 0.])
        } else {
            ("color", "width", "opacity", [0., 0., 0., 1.])
        };
        Some(ShapeStroke {
            width: number(e, width_key, 2.)? as f32,
            paint: Fill::Solid(color(e, key, default, c)?),
            opacity: (number(e, opacity_key, 1.)? as f32).clamp(0., 1.),
            cap: LineCap::Round,
            join: LineJoin::Round,
            ..Default::default()
        })
    };
    if e.operation == "gegl:path" {
        number(e, "stroke-hardness", 0.6)?;
    }
    let shape = Layer::new("Imported graph vector", LayerContent::Shape(ShapeLayer { path, fill, stroke, ..Default::default() }));
    let native = replace(
        shape,
        "Stored explicit GeglPath coordinates become ordinary editable native cubic geometry; the standalone stock zero-argument s sentinel becomes an empty editable native Shape. Native path fill/winding, round vector stroke, antialiasing and document-pixel width replace source paint/airbrush hardness; source opacities clamp to native coverage. SVG-style transform becomes native geometry placement, with native document-pixel stroke width.",
    );
    Ok(NativeMapping {
        structure: Some(NativeStructure::Stages {
            steps: vec![
                NativeStage::Apply { input: 0, mapping: Box::new(native) },
                NativeStage::Join { below: 0, above: 1, blend: photocraft_color::BlendMode::Normal, opacity: 1. },
            ],
            output: 2,
        }),
        ..Default::default()
    })
}

fn map_known(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<NativeMapping, MappingFailure> {
    generation_bytes(c, 4096)?;
    for arg in &e.arguments {
        e.argument(arg.name).map_err(invalid)?;
        match field(e.operation, arg.name) {
            None => return Err(invalid(format!("unknown stock object property {}:{}", e.operation, arg.name))),
            Some(Kind::Unavailable(reason)) => return Err(reason.into()),
            _ => {}
        }
    }
    if e.operation != "gegl:text" {
        for arg in &e.arguments {
            match field(e.operation, arg.name) {
                Some(Kind::Number { .. } | Kind::Integer { .. }) => {
                    number(e, arg.name, 0.)?;
                }
                Some(Kind::Color) => {
                    color(e, arg.name, [0., 0., 0., 1.], c)?;
                }
                Some(Kind::Text) => {
                    string(e, arg.name, "", 65_536)?;
                }
                _ => {}
            }
        }
        return path_object(e, c);
    }
    let content = string(e, "string", "Hello", 65_536)?;
    let family = string(e, "font", "Sans", 4096)?;
    let size = number(e, "size", 10.)? as f32;
    let wrap = number(e, "wrap", -1.)? as f32;
    let vertical_wrap = number(e, "vertical-wrap", -1.)? as f32;
    let alignment = number(e, "alignment", 0.)? as i32;
    let vertical_alignment = number(e, "vertical-alignment", 0.)? as i32;
    for name in ["width", "height"] {
        number(e, name, 0.)?;
    }
    if !c.resolution_dpi.is_finite() || c.resolution_dpi <= 0. {
        return Err(invalid("native text frame DPI must be finite and positive"));
    }
    // Bound native shaping working data before invoking the ordinary text engine.
    // Nothing fetches or embeds a font, nor uses source glyph/raster caches.
    let bytes = content
        .len()
        .checked_add(family.len())
        .and_then(|n| n.checked_mul(4096))
        .and_then(|n| n.checked_add(1 << 20))
        .ok_or_else(|| MappingFailure::resource("native text work allowance overflows"))?;
    generation_bytes(c, bytes)?;
    if content.len().checked_add(family.len()).and_then(|n| n.checked_mul(8)).and_then(|n| n.checked_add(4096)).is_none_or(|n| n > c.remaining_metadata_bytes) {
        return Err(MappingFailure::resource("native editable text exceeds remaining metadata allowance"));
    }
    let mut text = TextLayer {
        text: content.into(),
        font_family: family.into(),
        size_pt: size.max(0.001) * 72. / c.resolution_dpi,
        color: color(e, "color", [0., 0., 0., 1.], c)?,
        shape: TextShape::Box { x: 0., y: 0., width: if wrap < 0. { 1_000_000. } else { wrap }, height: 1_000_000. },
        paragraphs: vec![ParagraphRun {
            len: content.len(),
            style: ParagraphStyle {
                align: match alignment {
                    1 => TextAlign::Center,
                    2 => TextAlign::Right,
                    _ => TextAlign::Left,
                },
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    if !text.size_pt.is_finite() || text.size_pt > 8192. {
        return Err("stock pixel text size exceeds native point-size domain at the actual document DPI".into());
    }
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let layout = engine.layout(&text, c.resolution_dpi);
    c.interrupt.check().map_err(|_| MappingFailure::resource("native text layout cancelled"))?;
    if let Some([x0, y0, x1, y1]) = layout.bounds() {
        if [x0, y0, x1, y1].iter().any(|v| !v.is_finite()) {
            return Err(MappingFailure::resource("native text layout produced nonfinite bounds"));
        }
        // A no-wrap source has no million-pixel alignment frame. Rebase the
        // ordinary native layout to its own logical start, preserving native glyphs.
        let x = if wrap < 0. { -x0 } else { 0. };
        let y = if vertical_wrap >= 0. { (vertical_wrap - (y1 - y0)) * vertical_alignment as f32 * 0.5 - y0 } else { -y0 };
        text.transform = Affine::translate(f64::from(x), f64::from(y));
    }
    if let TextShape::Box { height, .. } = &mut text.shape {
        *height = if vertical_wrap < 0. { 1_000_000. } else { vertical_wrap };
    }
    let mut layer = Layer::new("Imported graph text", LayerContent::Text(text));
    if size == 0. {
        layer.opacity = 0.;
    }
    Ok(replace(
        layer,
        "Stock UTF-8/font/pixel size, wrap and alignment become editable native text at the actual native DPI. Native fonts, fallback, glyph shaping, wrapping and antialiasing replace source typography; no source font or raster cache is imported. Vertical justification uses measured native logical lines. Read-only source width/height are recomputed by native layout; zero size retains editable text with zero native layer opacity.",
    ))
}
pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> MappingResult {
    operation(e.operation)?;
    Some(map_known(e, c))
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, NativeStructure};
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Document;
    use photocraft_geom::{Rect, Size};
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(0, 0, 120, 80),
            native_filter_bounds: Rect::new(0, 0, 120, 80),
            target_profile: None,
            resolution_dpi: 144.,
            remaining_generation_bytes: crate::xcf::MAX_PIXEL_BYTES,
            remaining_metadata_bytes: crate::xcf::MAX_METADATA_BYTES,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn text(mapping: NativeMapping) -> Layer {
        let Some(NativeStructure::Replace { layer, patterns }) = mapping.structure else {
            panic!("expected ordinary native object");
        };
        assert!(patterns.is_empty());
        *layer
    }
    fn string_arg<'a>(name: &'a str, value: &'a str) -> Argument<'a> {
        Argument { name, value: ArgumentValue::String(value) }
    }
    fn numeric<'a>(name: &'a str, value: f64) -> Argument<'a> {
        Argument { name, value: ArgumentValue::Literal(value) }
    }
    #[test]
    fn stock_utf8_text_is_editable_native_text_with_pixel_units_and_native_counterpart() {
        let c = context();
        let e = StoredEffect::test(
            "gegl:text",
            vec![string_arg("string", "Hello café"), string_arg("font", "Sans"), numeric("size", 18.), numeric("wrap", 90.), numeric("alignment", 0.)],
        );
        let mapping = map(&e, &c).unwrap().unwrap();
        assert!(mapping.notes.iter().any(|v| v.contains("fallback")));
        let mut actual = text(mapping);
        let LayerContent::Text(actual_text) = &actual.content else {
            panic!("native Text expected");
        };
        assert_eq!((actual_text.text.as_str(), actual_text.font_family.as_str(), actual_text.size_pt), ("Hello café", "Sans", 9.));
        assert!(actual_text.cache.is_none() && actual_text.psd_raw.is_none());
        let mut expected = TextLayer {
            text: "Hello café".into(),
            font_family: "Sans".into(),
            size_pt: 9.,
            shape: TextShape::Box { x: 0., y: 0., width: 90., height: 1_000_000. },
            paragraphs: vec![ParagraphRun { len: "Hello café".len(), style: ParagraphStyle::default() }],
            ..Default::default()
        };
        let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let bounds = engine.layout(&expected, 144.).bounds();
        if let Some([_, y, _, _]) = bounds {
            expected.transform = Affine::translate(0., -f64::from(y));
        }
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut document = Document::new("Native text counterpart", Size::new(120, 80), ColorMode::Rgb, depth);
            document.resolution_dpi = 144.;
            let mut expected_text = expected.clone();
            engine.render_layer(&mut expected_text, 144., document.pixel_format());
            let LayerContent::Text(actual_text) = &mut actual.content else {
                panic!("native Text expected");
            };
            engine.render_layer(actual_text, 144., document.pixel_format());
            assert_eq!(actual_text.cache, expected_text.cache);
            let paragraphs = actual_text.paragraphs.clone();
            document.layers.push(actual.clone());
            let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Text(saved) = &reopened.layers[0].content else {
                panic!("native text persistence");
            };
            assert_eq!((saved.text.as_str(), saved.font_family.as_str(), saved.size_pt), ("Hello café", "Sans", 9.));
            assert_eq!(saved.paragraphs, paragraphs);
        }
    }
    #[test]
    fn text_zero_size_alignment_and_strict_domain_budget_cancellation_are_explicit() {
        let mut c = context();
        let zero = text(map(&StoredEffect::test("gegl:text", vec![numeric("size", 0.)]), &c).unwrap().unwrap());
        assert_eq!(zero.opacity, 0.);
        let LayerContent::Text(zero_text) = zero.content else {
            panic!("editable zero paint text");
        };
        assert_eq!(zero_text.text, "Hello");
        let aligned = text(
            map(
                &StoredEffect::test(
                    "gegl:text",
                    vec![
                        numeric("size", 20.),
                        numeric("wrap", 80.),
                        numeric("vertical-wrap", 60.),
                        numeric("alignment", 2.),
                        numeric("vertical-alignment", 2.),
                    ],
                ),
                &c,
            )
            .unwrap()
            .unwrap(),
        );
        let LayerContent::Text(aligned_text) = aligned.content else {
            panic!("native text");
        };
        assert_eq!(aligned_text.paragraphs[0].style.align, TextAlign::Right);
        assert!(aligned_text.transform.m.iter().all(|v| v.is_finite()));
        for args in [
            vec![numeric("size", f64::NAN)],
            vec![numeric("size", 2049.)],
            vec![numeric("wrap", 1.5)],
            vec![numeric("alignment", 3.)],
            vec![numeric("vertical-wrap", -2.)],
            vec![string_arg("font", "bad\0font")],
            vec![numeric("size", 2.), numeric("size", 3.)],
        ] {
            let fail = map(&StoredEffect::test("gegl:text", args), &c).unwrap().unwrap_err();
            assert_eq!(fail.reason, photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData);
        }
        c.remaining_generation_bytes = 10;
        assert!(map(&StoredEffect::test("gegl:text", vec![]), &c).unwrap().is_err());
        c = context();
        c.remaining_metadata_bytes = 1;
        assert!(map(&StoredEffect::test("gegl:text", vec![]), &c).unwrap().is_err());
        let cancel = || true;
        c = context();
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(map(&StoredEffect::test("gegl:text", vec![]), &c).unwrap().is_err());
    }
    #[test]
    fn path_objects_require_actual_declared_payload_and_never_invent_an_input() {
        assert!(generator("gegl:text"));
        for op in ["gegl:path", "gegl:fill-path", "gegl:vector-stroke"] {
            assert!(!generator(op));
            assert!(map(&StoredEffect::test(op, vec![]), &context()).unwrap().unwrap().structure.is_none());
            let empty = map(&StoredEffect::test(op, vec![string_arg("d", "s")]), &context()).unwrap().unwrap();
            assert!(matches!(empty.structure, Some(NativeStructure::Stages { .. })));
            let fail = map(&StoredEffect::test(op, vec![string_arg("d", "S 1 2")]), &context()).unwrap().unwrap_err();
            assert_eq!(fail.reason, photocraft_doc::import_conversion::ConversionReason::MissingMapping);
            assert!(fail.message.contains("grammar"));
        }
        let fail = map(&StoredEffect::test("gegl:vector-stroke", vec![numeric("width", 201.)]), &context()).unwrap().unwrap_err();
        assert_eq!(fail.reason, photocraft_doc::import_conversion::ConversionReason::InvalidOptionalData);
    }
    #[test]
    fn standalone_stock_sentinel_preserves_empty_editable_geometry_and_native_reopen() {
        for op in ["gegl:path", "gegl:fill-path", "gegl:vector-stroke"] {
            let source = StoredEffect::test(op, vec![string_arg("d", "s")]);
            let mapping = map(&source, &context()).unwrap().unwrap();
            let Some(NativeStructure::Stages { steps, .. }) = mapping.structure else { panic!("native overlay") };
            let NativeStage::Apply { mapping, .. } = &steps[0] else { panic!("native shape stage") };
            let Some(NativeStructure::Replace { layer, .. }) = &mapping.structure else { panic!("native empty shape") };
            let LayerContent::Shape(shape) = &layer.content else { panic!("editable native shape") };
            assert!(shape.path.subpaths.is_empty());
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut actual = Document::new("Native empty path", Size::new(32, 32), ColorMode::Rgb, depth);
                actual.layers.push(Layer::new("Actual input", LayerContent::Fill(Fill::Solid(photocraft_color::Color::rgb(1., 0., 0.)))));
                actual.layers.push((**layer).clone());
                let mut counterpart = actual.clone();
                counterpart.layers[1] = Layer::new("Independent empty native shape", LayerContent::Shape(ShapeLayer::default()));
                let prepare = |doc: &mut Document| {
                    let bounds = doc.bounds();
                    let format = doc.pixel_format();
                    let LayerContent::Shape(shape) = &mut doc.layers[1].content else { panic!("native shape") };
                    shape.cache = Some(photocraft_vector::render_shape(shape, format, bounds));
                };
                prepare(&mut actual);
                prepare(&mut counterpart);
                let before = photocraft_compose::render(&actual, actual.bounds()).px;
                assert!(before.iter().any(|pixel| pixel[0] > 0.9 && pixel[3] > 0.9), "real bound input remains visible");
                assert_eq!(before, photocraft_compose::render(&counterpart, counterpart.bounds()).px);
                let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
                let mut reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                prepare(&mut reopened);
                assert_eq!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
                let LayerContent::Shape(shape) = &mut reopened.layers[1].content else { panic!("editable reopened shape") };
                assert!(shape.path.subpaths.is_empty());
                shape.path = Path::new(vec![Subpath::polygon(&[(2., 2.), (20., 2.), (20., 20.), (2., 20.)])]);
                shape.fill = Some(Fill::Solid(photocraft_color::Color::BLACK));
                prepare(&mut reopened);
                assert_ne!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
            }
            let mut c = context();
            c.remaining_generation_bytes = 1;
            assert!(map(&source, &c).unwrap().is_err());
            c = context();
            c.remaining_metadata_bytes = 1;
            assert!(map(&source, &c).unwrap().is_err());
            let cancel = || true;
            c = context();
            c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
            assert!(map(&source, &c).unwrap().is_err());
        }
    }
    #[test]
    fn established_path_literals_compile_to_editable_native_geometry_over_actual_input() {
        let c = context();
        let e = StoredEffect::test(
            "gegl:path",
            vec![
                string_arg("d", "M 1 2 l 5 0 c 1 2 3 4 5 6 z M 3 3 L 4 3 L 4 4 z"),
                string_arg("transform", "translate(2,3) scale(2)"),
                string_arg("fill-rule", "evenodd"),
                numeric("stroke-width", 4.),
                numeric("fill-opacity", 0.5),
            ],
        );
        let mapping = map(&e, &c).unwrap().unwrap();
        let Some(NativeStructure::Stages { steps, output }) = mapping.structure else {
            panic!("ordinary native overlay plan");
        };
        assert_eq!(output, 2);
        assert!(matches!(&steps[1], NativeStage::Join { below: 0, above: 1, blend: photocraft_color::BlendMode::Normal, opacity: 1. }));
        let NativeStage::Apply { input: 0, mapping } = &steps[0] else {
            panic!("direct native shape stage");
        };
        let Some(NativeStructure::Replace { layer, .. }) = &mapping.structure else {
            panic!("editable vector replacement");
        };
        let LayerContent::Shape(actual) = &layer.content else {
            panic!("native shape");
        };
        assert_eq!(actual.path.fill_rule, FillRule::EvenOdd);
        assert_eq!(actual.path.subpaths.len(), 2);
        let first = &actual.path.subpaths[0];
        assert!(first.closed);
        assert_eq!(first.knots[0].anchor, Point::new(4., 7.));
        assert_eq!(first.knots[1].anchor, Point::new(14., 7.));
        assert_eq!(first.knots[1].out_ctrl, Point::new(16., 11.));
        assert_eq!(first.knots[2].in_ctrl, Point::new(20., 15.));
        assert_eq!(first.knots[2].anchor, Point::new(24., 19.));
        assert_eq!(actual.path.subpaths[1].op, PathOp::Join);
        assert_eq!(actual.stroke.as_ref().unwrap().width, 4.);
        let Some(Fill::Solid(paint)) = &actual.fill else {
            panic!("native paint");
        };
        assert!((paint.alpha - 0.3).abs() < 1e-6);
        // Independent native square: source commands carry geometry only. Native
        // path rasterization/rendering and save/reopen establish the counterpart.
        let square = map(&StoredEffect::test("gegl:fill-path", vec![string_arg("d", "M 2 2 L 20 2 L 20 20 L 2 20 z")]), &c).unwrap().unwrap();
        let Some(NativeStructure::Stages { steps, .. }) = square.structure else {
            panic!("native overlay");
        };
        let NativeStage::Apply { mapping, .. } = &steps[0] else {
            panic!("native shape stage");
        };
        let Some(NativeStructure::Replace { layer, .. }) = &mapping.structure else {
            panic!("native shape replacement");
        };
        let expected = Layer::new(
            "Independent native square",
            LayerContent::Shape(ShapeLayer {
                path: Path::new(vec![Subpath::polygon(&[(2., 2.), (20., 2.), (20., 20.), (2., 20.)])]),
                fill: Some(Fill::Solid(photocraft_color::Color::BLACK)),
                ..Default::default()
            }),
        );
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut actual_doc = Document::new("Vector counterpart", Size::new(32, 32), ColorMode::Rgb, depth);
            actual_doc.layers.push((**layer).clone());
            let mut expected_doc = actual_doc.clone();
            expected_doc.layers = vec![expected.clone()];
            let prepare = |doc: &mut Document| {
                let bounds = doc.bounds();
                let format = doc.pixel_format();
                let LayerContent::Shape(shape) = &mut doc.layers[0].content else {
                    panic!("shape");
                };
                shape.cache = Some(photocraft_vector::render_shape(shape, format, bounds));
            };
            prepare(&mut actual_doc);
            prepare(&mut expected_doc);
            let actual = photocraft_compose::render(&actual_doc, actual_doc.bounds());
            let expected_pixels = photocraft_compose::render(&expected_doc, expected_doc.bounds());
            assert!(expected_pixels.px.iter().any(|pixel| pixel[3] > 0.5), "prepared independent native vector must paint visible coverage");
            assert_eq!(actual.px, expected_pixels.px);
            let bytes = photocraft_format::save_to_bytes(&actual_doc, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            let LayerContent::Shape(before) = &actual_doc.layers[0].content else {
                panic!("native shape");
            };
            assert!(matches!(&reopened.layers[0].content,LayerContent::Shape(shape) if shape.path == before.path));
        }
    }
}
