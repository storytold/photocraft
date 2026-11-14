//! Bounded data-only GIMP text configuration to native PhotoCraft typography.
//! Config lists are not expressions. Nothing here executes, resolves a URI,
//! loads a font file or treats the saved GIMP raster as a native text cache.

use photocraft_color::Color;
use photocraft_doc::TextLayer;
use photocraft_doc::text::{
    AntiAlias, Caps, CharStyle, FontBaseline, FontFeature, FontVariation, Kerning, Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextDirection,
    TextRun, TextShape,
};
use photocraft_geom::Affine;
use std::collections::HashMap;

use super::{Result, invalid};

const MAX_NODES: usize = 65_536;
const MAX_DEPTH: usize = 32;

#[cfg(test)]
mod diagnostic_review_tests;
#[cfg(test)]
mod literal_profile_review_tests;
#[cfg(test)]
mod review_round2_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod spacing_review_tests;

#[derive(Debug)]
enum Value {
    Atom(Vec<u8>),
    List(Vec<Value>),
}
impl Value {
    /// Expose an atom as UTF-8 text, rejecting lists and malformed source bytes.
    fn text(&self) -> Result<&str> {
        match self {
            Self::Atom(b) => std::str::from_utf8(b).map_err(|_| invalid("text configuration contains invalid UTF-8")),
            Self::List(_) => Err(invalid("expected text configuration scalar")),
        }
    }
    /// Require a configuration list without interpreting its contents as expressions.
    fn list(&self) -> Result<&[Self]> {
        match self {
            Self::List(v) => Ok(v),
            _ => Err(invalid("expected text configuration list")),
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    nodes: usize,
}
impl Parser<'_> {
    /// Skip ASCII whitespace and line comments while keeping the bounded parser cursor inside the input.
    fn space(&mut self) {
        loop {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            if self.bytes.get(self.at) != Some(&b'#') {
                break;
            }
            while self.bytes.get(self.at).is_some_and(|b| *b != b'\n') {
                self.at += 1;
            }
        }
    }
    /// Parse one data-only configuration value with bounded nesting and node count, validating quoted byte escapes and complete list framing.
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.space();
        self.nodes += 1;
        if depth > MAX_DEPTH || self.nodes > MAX_NODES {
            return Err(invalid("text configuration nesting or node limit"));
        }
        match self.bytes.get(self.at).copied() {
            Some(b'(') => {
                self.at += 1;
                let mut values = Vec::new();
                loop {
                    self.space();
                    if self.bytes.get(self.at) == Some(&b')') {
                        self.at += 1;
                        return Ok(Value::List(values));
                    }
                    if self.at >= self.bytes.len() {
                        return Err(invalid("unterminated text configuration list"));
                    }
                    values.push(self.value(depth + 1)?);
                }
            }
            Some(b'"') => {
                self.at += 1;
                let mut bytes = Vec::new();
                loop {
                    let b = self.bytes.get(self.at).copied().ok_or_else(|| invalid("unterminated text configuration string"))?;
                    self.at += 1;
                    match b {
                        b'"' => return Ok(Value::Atom(bytes)),
                        b'\\' => {
                            let esc = self.bytes.get(self.at).copied().ok_or_else(|| invalid("truncated text escape"))?;
                            self.at += 1;
                            if (b'0'..=b'7').contains(&esc) {
                                let mut n = u16::from(esc - b'0');
                                for _ in 0..2 {
                                    if let Some(&next) = self.bytes.get(self.at).filter(|b| (b'0'..=b'7').contains(b)) {
                                        n = n * 8 + u16::from(next - b'0');
                                        self.at += 1;
                                    } else {
                                        break;
                                    }
                                }
                                bytes.push(u8::try_from(n).map_err(|_| invalid("text escape exceeds a byte"))?);
                            } else {
                                bytes.push(match esc {
                                    b'n' => b'\n',
                                    b'r' => b'\r',
                                    b't' => b'\t',
                                    b'b' => 8,
                                    b'f' => 12,
                                    b'\\' | b'"' => esc,
                                    _ => return Err(invalid("unsupported text string escape")),
                                });
                            }
                        }
                        0 => return Err(invalid("NUL inside text configuration string")),
                        _ => bytes.push(b),
                    }
                }
            }
            Some(b')') | None => Err(invalid("unexpected end of text configuration")),
            Some(_) => {
                let start = self.at;
                while self.bytes.get(self.at).is_some_and(|b| !b.is_ascii_whitespace() && !b"()\"".contains(b)) {
                    self.at += 1;
                }
                if start == self.at {
                    return Err(invalid("invalid text configuration token"));
                }
                Ok(Value::Atom(self.bytes.get(start..self.at).ok_or_else(|| invalid("text token range"))?.to_vec()))
            }
        }
    }
}

/// Find one named configuration field and reject duplicate occurrences instead of choosing an arbitrary value.
fn field<'a>(fields: &'a [Value], name: &str) -> Result<Option<&'a [Value]>> {
    let mut found = None;
    for f in fields {
        let list = f.list()?;
        let key = list.first().ok_or_else(|| invalid("empty text configuration field"))?.text()?;
        if key == name {
            if found.is_some() {
                return Err(invalid(format!("duplicate text configuration field {name}")));
            }
            found = Some(list.get(1..).unwrap_or(&[]));
        }
    }
    Ok(found)
}
/// Deduplicate native-policy disclosures and cap their count so repeated markup cannot amplify report strings.
fn note(notes: &mut Vec<String>, message: String) {
    // Repeated markup nodes must not multiply the same disclosure or create
    // unbounded report strings. The raw original is retained separately.
    if notes.len() < 128 && !notes.contains(&message) {
        notes.push(message);
    }
}
/// Bound inherited source strings before formatting: deduplication happens only
/// after formatting, and one ancestor can apply to thousands of text nodes.
fn diagnostic_value(value: &str) -> std::borrow::Cow<'_, str> {
    const MAX_BYTES: usize = 256;
    if value.len() <= MAX_BYTES {
        return std::borrow::Cow::Borrowed(value);
    }
    let mut end = MAX_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…", value.get(..end).unwrap_or_default()))
}
/// Estimate a materialized native character style, including owned font, language, feature and variation strings.
fn style_allocation(style: &CharStyle) -> usize {
    std::mem::size_of::<CharStyle>()
        .saturating_add(style.font_family.capacity())
        .saturating_add(style.font_style.capacity())
        .saturating_add(style.postscript_name.as_ref().map_or(0, String::capacity))
        .saturating_add(style.language.as_ref().map_or(0, String::capacity))
        .saturating_add(style.features.capacity().saturating_mul(std::mem::size_of::<FontFeature>()))
        .saturating_add(style.variations.capacity().saturating_mul(std::mem::size_of::<FontVariation>()))
        .saturating_add(style.features.iter().fold(0usize, |bytes, feature| bytes.saturating_add(feature.tag.capacity())))
        .saturating_add(style.variations.iter().fold(0usize, |bytes, variation| bytes.saturating_add(variation.axis.capacity())))
}

/// Conservatively admit all owned native text allocations before replacing a saved raster.
/// These decoders create no pixel cache or PSD payload; reject an unexpected one rather than undercount it.
pub(super) fn retained_allocation(layer: &TextLayer) -> usize {
    if layer.cache.is_some() || layer.psd_raw.is_some() {
        return usize::MAX;
    }
    let knots = |path: &photocraft_doc::vector::Subpath| path.knots.capacity().saturating_mul(std::mem::size_of::<photocraft_doc::vector::Knot>());
    let shape = match &layer.shape {
        TextShape::Point | TextShape::Box { .. } => 0,
        TextShape::Path { path, .. } => knots(path),
        TextShape::Area { path } => {
            path.subpaths.iter().fold(path.subpaths.capacity().saturating_mul(std::mem::size_of::<photocraft_doc::vector::Subpath>()), |bytes, path| {
                bytes.saturating_add(knots(path))
            })
        }
    };
    let bytes = std::mem::size_of::<TextLayer>()
        .saturating_add(layer.text.capacity())
        .saturating_add(layer.font_family.capacity())
        .saturating_add(layer.runs.capacity().saturating_mul(std::mem::size_of::<TextRun>()))
        .saturating_add(layer.paragraphs.capacity().saturating_mul(std::mem::size_of::<ParagraphRun>()))
        .saturating_add(layer.warp.as_ref().map_or(0, |warp| warp.style.capacity()))
        .saturating_add(shape);
    // A run's fixed style is also counted here: a conservative extra debit keeps
    // this bound independent of Vec spare capacity and future run-layout changes.
    layer.runs.iter().fold(bytes, |bytes, run| bytes.saturating_add(style_allocation(&run.style)))
}

/// Decode supported hexadecimal markup colors and optional alpha; malformed widths or digits fail rather than becoming a default color.
fn hex_color(text: &str) -> Result<Color> {
    let text = text.strip_prefix('#').ok_or_else(|| crate::IoError::Unsupported("named markup colors need an explicit native color-name mapping".into()))?;
    let alpha = if text.len() == 8 {
        Some(
            u8::from_str_radix(text.get(6..8).ok_or_else(|| invalid("invalid markup alpha range"))?, 16)
                .map_err(|_| invalid("invalid markup hexadecimal alpha"))?,
        )
    } else {
        None
    };
    let text = if alpha.is_some() { text.get(..6).ok_or_else(|| invalid("invalid markup RGB range"))? } else { text };
    let width = match text.len() {
        3 => 1,
        6 => 2,
        12 => 4,
        _ => return Err(invalid("unsupported markup color length")),
    };
    let mut rgb = [0.0; 3];
    let denominator = ((1u32 << (width * 4)) - 1) as f32;
    for (i, value) in rgb.iter_mut().enumerate() {
        let s = text.get(i * width..(i + 1) * width).ok_or_else(|| invalid("markup color is not ASCII hexadecimal"))?;
        *value = u32::from_str_radix(s, 16).map_err(|_| invalid("invalid markup color"))? as f32 / denominator;
    }
    let [r, g, b] = rgb;
    Ok(Color::rgba(r, g, b, alpha.map_or(1., |alpha| f32::from(alpha) / 255.)))
}

/// Pango names are CSS color values plus the X11 numbered-name vocabulary.
/// Pango's public Color.parse/markup documentation defines the name form;
/// independent GIMP3.2.6/Pango1.58.2 probes reconcile all657 native X11 names.
/// The five CSS overrides must precede X11 lookup (notably green and gray).
fn markup_color(text: &str) -> Result<Color> {
    if text.starts_with('#') {
        return hex_color(text);
    }
    let unsupported = || crate::IoError::Unsupported("unknown or out-of-domain Pango markup color name; original text formatting needs fallback".into());
    // All declared names fit this allowance. Avoid allocating from an arbitrary
    // attribute, and do not broaden Pango names to CSS functions or XML URIs.
    if text.is_empty() || text.len() > 128 || text.ends_with(' ') || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ') {
        return Err(unsupported());
    }
    let normalized: String = text.bytes().filter(|b| *b != b' ').map(|b| char::from(b.to_ascii_lowercase())).collect();
    let rgb: [u8; 3] = match normalized.as_str() {
        "gray" | "grey" => [128, 128, 128],
        "green" => [0, 128, 0],
        "maroon" => [128, 0, 0],
        "purple" => [128, 0, 128],
        // CSS named-color facts (CSS Color3/4), absent from the X11 database.
        "aqua" => [0, 255, 255],
        "crimson" => [220, 20, 60],
        "fuchsia" => [255, 0, 255],
        "indigo" => [75, 0, 130],
        "lime" => [0, 255, 0],
        "olive" => [128, 128, 0],
        "silver" => [192, 192, 192],
        "teal" => [0, 128, 128],
        "rebeccapurple" => [102, 51, 153],
        _ => return craft_raster_codecs::xpm::x11_color_rgb(&normalized).map(|[r, g, b]| Color::rgb(r, g, b)).ok_or_else(unsupported),
    };
    let [r, g, b] = rgb.map(|sample| f32::from(sample) / 255.);
    Ok(Color::rgb(r, g, b))
}
/// Require exactly one UTF-8 atom for an optional named text field.
fn scalar<'a>(fields: &'a [Value], name: &str) -> Result<Option<&'a str>> {
    field(fields, name)?
        .map(|v| {
            if v.len() != 1 {
                return Err(invalid(format!("text field {name} requires one value")));
            }
            v.first().ok_or_else(|| invalid("missing text scalar"))?.text()
        })
        .transpose()
}
/// Read a bounded finite text scalar, applying the default only when the field is absent.
fn number(fields: &[Value], name: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let n = scalar(fields, name)?.map(str::parse::<f64>).transpose().map_err(|_| invalid(format!("invalid text number {name}")))?.unwrap_or(default);
    if !n.is_finite() || !(min..=max).contains(&n) {
        return Err(invalid(format!("text number {name} outside its finite domain")));
    }
    Ok(n)
}
/// Translate supported source text units into native point sizes at the supplied DPI; unknown units remain unsupported.
fn unit_scale(unit: &str, dpi: f64) -> Result<f64> {
    match unit {
        "pixels" => Ok(72.0 / dpi),
        "points" => Ok(1.0),
        "inches" => Ok(72.0),
        "millimeters" => Ok(72.0 / 25.4),
        "centimeters" => Ok(72.0 / 2.54),
        "picas" => Ok(12.0),
        _ => Err(crate::IoError::Unsupported(format!("source text unit {unit} requires a native unit mapping"))),
    }
}

pub(super) struct DecodedText {
    pub layer: TextLayer,
    pub source_only: Vec<String>,
    pub native_policy: Vec<String>,
}

/// Source point text starts at its layout's top-left; native point text starts
/// at a baseline anchor. Map the native logical origin through the source affine
/// exactly once. Fixed boxes already have native top-left layout coordinates.
fn anchor_point_origin(layer: &mut TextLayer, dpi: f32) -> Result<()> {
    if !matches!(layer.shape, TextShape::Point) || layer.text.is_empty() {
        return Ok(());
    }
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bounds = engine
        .layout(layer, dpi)
        .bounds()
        .ok_or_else(|| crate::IoError::Unsupported("native font metrics are unavailable for the source text origin".into()))?;
    if !bounds.iter().all(|v| v.is_finite()) {
        return Err(crate::IoError::Unsupported("source text origin has no finite native layout bounds".into()));
    }
    layer.transform = layer.transform.mul(&Affine::translate(-f64::from(bounds[0]), -f64::from(bounds[1])));
    if !layer.transform.m.iter().all(|v| v.is_finite()) {
        return Err(invalid("native text anchor conversion exceeds affine bounds"));
    }
    Ok(())
}

/// Admit the sRGB-literal conversion before replacing stored text/vector pixels.
/// A valid input-only document profile can still lack the required reverse transform.
pub(super) fn color_transform(bytes: &[u8]) -> Result<std::sync::Arc<craft_cms::Transform>> {
    use craft_cms::{Builtin, Intent, Profile, TransformOptions};
    let destination = Profile::parse(bytes).map_err(|e| invalid(format!("text destination profile: {e}")))?;
    craft_cms::transform::cached(
        Builtin::Srgb.profile(),
        &destination,
        TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, precise_float: true },
    )
    .map_err(|e| invalid(format!("text color conversion: {e}")))
}

/// Text decimal/markup RGB colors are sRGB literals, independently of the XCF
/// pixel profile. Convert them once using the reader's internal pixel encoding,
/// even when no ICC assignment is attached to the document.
pub(super) fn convert_colors(document: &mut photocraft_doc::Document, profile: &[u8]) -> Result<()> {
    use craft_cms::Transform;
    /// Detect native Text or Shape content through groups before allocating a document color transform.
    fn has_special(layers: &[photocraft_doc::Layer]) -> bool {
        layers.iter().any(|layer| match &layer.content {
            photocraft_doc::LayerContent::Text(_) | photocraft_doc::LayerContent::Shape(_) => true,
            photocraft_doc::LayerContent::Group(group) => has_special(&group.children),
            _ => false,
        })
    }
    if !has_special(&document.layers) {
        return Ok(());
    }
    // The reader's internal encoding can be known while the document remains deliberately
    // untagged for missing-profile policy. Literal conversion must not invent an assignment.
    let transform = color_transform(profile)?;
    /// Convert native text-run and solid vector paints recursively into the internal document color model.
    fn visit(layers: &mut [photocraft_doc::Layer], transform: &Transform, mode: photocraft_color::ColorMode) -> Result<()> {
        /// Use the native CMS transform for the literal color channels while retaining the color object alpha.
        fn color(value: &mut Color, transform: &Transform, mode: photocraft_color::ColorMode) -> Result<()> {
            let mut converted = [0.0; 4];
            let channels = mode.color_channels();
            let output = converted.get_mut(..channels).ok_or_else(|| invalid("native literal destination has too many channels"))?;
            transform.convert_f32(&value.c[..3], 3, output, channels, false).map_err(|error| invalid(format!("native literal color conversion: {error}")))?;
            value.c = converted;
            value.mode = mode;
            Ok(())
        }
        for layer in layers {
            match &mut layer.content {
                photocraft_doc::LayerContent::Text(text) => {
                    for run in &mut text.runs {
                        color(&mut run.style.color, transform, mode)?;
                    }
                    text.sync_summary();
                }
                photocraft_doc::LayerContent::Shape(shape) => {
                    if let Some(photocraft_doc::Fill::Solid(value)) = &mut shape.fill {
                        color(value, transform, mode)?;
                    }
                    if let Some(stroke) = &mut shape.stroke
                        && let photocraft_doc::Fill::Solid(value) = &mut stroke.paint
                    {
                        color(value, transform, mode)?;
                    }
                }
                photocraft_doc::LayerContent::Group(group) => visit(&mut group.children, transform, mode)?,
                _ => {}
            }
        }
        Ok(())
    }
    visit(&mut document.layers, &transform, document.mode)
}

/// Decode an unmodified `gimp-text-layer` parasite. Callers retain the source
/// config inert and disclose native layout/font substitution. Modified source
/// layers must keep their separately stored raster instead of calling this.
pub(super) fn decode(data: &[u8], dpi: f32, offset: (i32, i32)) -> Result<DecodedText> {
    if data.len() > super::MAX_STRING || !dpi.is_finite() || dpi <= 0.0 {
        return Err(invalid("text configuration size or DPI is invalid"));
    }
    let data = data.strip_suffix(&[0]).unwrap_or(data);
    let mut parser = Parser { bytes: data, at: 0, nodes: 0 };
    let mut fields = Vec::new();
    loop {
        parser.space();
        if parser.at == data.len() {
            break;
        }
        fields.push(parser.value(0)?);
    }
    let dpi = f64::from(dpi);
    let scale = unit_scale(scalar(&fields, "font-size-unit")?.unwrap_or("pixels"), dpi)?;
    let size_pt = (number(&fields, "font-size", 24.0, 0.001, 8192.0)? * scale) as f32;
    if !size_pt.is_finite() || size_pt > 1_000_000.0 {
        return Err(invalid("native text size exceeds limit"));
    }
    let mut style = CharStyle { size_pt, ..Default::default() };
    let mut notes = Vec::new();
    let mut native_policy = Vec::new();
    if let Some(font) = field(&fields, "font")? {
        match font {
            [Value::Atom(name)] if name != b"NULL" => {
                font_description(std::str::from_utf8(name).map_err(|_| invalid("invalid font name UTF-8"))?, &mut style, dpi, false, &mut notes)?
            }
            [Value::Atom(kind), rest @ ..] if kind == b"GimpFont" => {
                requested_font(rest, &mut style)?;
            }
            _ => note(&mut notes, "Unrecognized requested font resource; native fallback font used".into()),
        }
    }
    // The saved GIMP flag is inactive: default shaping still uses font kerning.
    style.kerning = Kerning::Metrics;
    if let Some(flag) = scalar(&fields, "kerning")? {
        let flag = match flag {
            "yes" => "yes",
            "no" => "no",
            _ => "unrecognized (raw source preserved)",
        };
        note(
            &mut native_policy,
            format!("Source kerning flag {flag} is retained as inactive metadata; native metrics kerning and explicit font features apply"),
        );
    }
    // Base letter spacing is a device-pixel distance, independently of the font-size unit.
    let tracking_pt = device_points(number(&fields, "letter-spacing", 0.0, -8192.0, 8192.0)?, dpi)?;
    style.tracking = (f64::from(tracking_pt) * 1000.0 / f64::from(size_pt)) as f32;
    if !style.tracking.is_finite() {
        return Err(invalid("native text tracking exceeds finite bounds"));
    }
    style.language = scalar(&fields, "language")?.filter(|s| !s.is_empty()).map(|s| s.replace('_', "-"));
    // Added line spacing uses the source font-size unit; letter spacing above remains pixels.
    let extra_leading = number(&fields, "line-spacing", 0.0, -8192.0, 8192.0)? * scale;
    if let Some(color) = field(&fields, "color")? {
        match color {
            [Value::List(v)] if v.first().and_then(|v| v.text().ok()) == Some("color-rgba") && v.len() == 5 => {
                let mut rgba = [0.0; 4];
                for (out, item) in rgba.iter_mut().zip(v.iter().skip(1)) {
                    *out = item.text()?.parse::<f32>().map_err(|_| invalid("invalid text color sample"))?;
                    if !out.is_finite() || !(0.0..=1.0).contains(out) {
                        return Err(invalid("text color sample outside 0..1"));
                    }
                }
                let [r, g, b, a] = rgba;
                style.color = Color::rgba(r, g, b, a);
            }
            [Value::List(v)] if v.first().and_then(|value| value.text().ok()) == Some("color") => {
                if !(5..=6).contains(&v.len()) {
                    return Err(invalid("typed text color field count"));
                }
                let encoding = v.get(1).ok_or_else(|| invalid("missing text color encoding"))?.text()?;
                let count = v
                    .get(2)
                    .ok_or_else(|| invalid("missing text color length"))?
                    .text()?
                    .parse::<usize>()
                    .map_err(|_| invalid("invalid text color byte count"))?;
                let Some(Value::Atom(bytes)) = v.get(3) else {
                    return Err(invalid("missing text color bytes"));
                };
                let profile_count = v
                    .get(4)
                    .ok_or_else(|| invalid("missing text color profile length"))?
                    .text()?
                    .parse::<usize>()
                    .map_err(|_| invalid("invalid text color profile length"))?;
                let profile = match v.get(5) {
                    Some(Value::Atom(bytes)) => bytes.as_slice(),
                    None => &[],
                    _ => return Err(invalid("invalid text color profile bytes")),
                };
                if count != bytes.len() || profile_count != profile.len() || (profile_count == 0 && v.len() != 5) {
                    return Err(invalid("typed text color payload length mismatch"));
                }
                match super::effects::literal_rgb(&super::effects::ColorLiteral { encoding, pixels: bytes, profile }) {
                    Ok([r, g, b, a]) => style.color = Color::rgba(r, g, b, a),
                    Err(error @ super::effects::LiteralColorError::GrayLabPcsTransfer) => return Err(crate::IoError::Unsupported(error.to_string())),
                    Err(message) => note(&mut notes, format!("Source text color retained; native black approximation used: {message}")),
                }
            }
            _ => note(
                &mut notes,
                "Binary or profiled source text color retained; native black used because source byte order/color space is not declared".into(),
            ),
        }
    }
    let para = ParagraphStyle {
        align: match scalar(&fields, "justify")?.unwrap_or("left") {
            "left" => TextAlign::Left,
            "right" => TextAlign::Right,
            "center" => TextAlign::Center,
            "fill" => TextAlign::JustifyLeft,
            _ => return Err(invalid("unknown text justification")),
        },
        direction: match scalar(&fields, "base-direction")?.unwrap_or("ltr") {
            "ltr" => TextDirection::Ltr,
            "rtl" => TextDirection::Rtl,
            "ttb-rtl" | "ttb-rtl-upright" | "ttb-ltr" | "ttb-ltr-upright" => TextDirection::Auto,
            _ => return Err(invalid("unknown text direction")),
        },
        first_line_indent_pt: (number(&fields, "indent", 0.0, -8192.0, 8192.0)? * scale) as f32,
        ..Default::default()
    };
    let mut layer = TextLayer {
        size_pt,
        transform: Affine::translate(f64::from(offset.0), f64::from(offset.1)),
        antialias: if scalar(&fields, "antialias")?.unwrap_or("yes") == "no" { AntiAlias::None } else { AntiAlias::Smooth },
        ..Default::default()
    };
    let direction = scalar(&fields, "base-direction")?.unwrap_or("ltr");
    if matches!(direction, "ttb-ltr" | "ttb-ltr-upright") {
        return Err(crate::IoError::Unsupported("left-to-right vertical source columns cannot use native right-to-left column layout".into()));
    }
    if direction.starts_with("ttb") {
        layer.orientation = Orientation::Vertical;
        // Mixed source vertical text rotates Roman glyphs; upright source text
        // keeps them upright. Markup and requested-font styles inherit this flag.
        style.rotate_roman = direction == "ttb-rtl";
        note(&mut notes, "Vertical source typography uses native vertical text layout".into());
    }
    if scalar(&fields, "box-mode")?.unwrap_or("dynamic") == "fixed" {
        let px = unit_scale(scalar(&fields, "box-unit")?.unwrap_or("pixels"), dpi)? * dpi / 72.0;
        layer.shape = TextShape::Box {
            x: 0.0,
            y: 0.0,
            width: (number(&fields, "box-width", 0.0, 0.0, 300_000.0)? * px) as f32,
            height: (number(&fields, "box-height", 0.0, 0.0, 300_000.0)? * px) as f32,
        };
    }
    if let Some(matrix) = field(&fields, "transformation")? {
        let matrix = match matrix {
            [Value::List(values)] if values.first().and_then(|v| v.text().ok()) == Some("matrix") => values.get(1..).unwrap_or(&[]),
            _ => return Err(invalid("text transformation requires a matrix record")),
        };
        if matrix.len() != 4 {
            return Err(invalid("text transformation must contain four coefficients"));
        }
        let mut m = [0.0; 4];
        for (v, src) in m.iter_mut().zip(matrix) {
            *v = src.text()?.parse::<f64>().map_err(|_| invalid("invalid text transform"))?;
            if !v.is_finite() || v.abs() > 10000.0 {
                return Err(invalid("text transform outside native bounds"));
            }
        }
        let [a, c, b, d] = m;
        layer.transform.m = [a, b, c, d, f64::from(offset.0), f64::from(offset.1)];
    }
    // Independent stock 3.2.6 save/edit/reload probes establish that these legacy
    // serialized offsets do not move glyphs, either alone or with a nonidentity matrix.
    // They must not be added to the actual layer placement a second time.
    let source_x = number(&fields, "offset-x", 0., -f64::MAX, f64::MAX)?;
    let source_y = number(&fields, "offset-y", 0., -f64::MAX, f64::MAX)?;
    if source_x != 0. || source_y != 0. {
        note(
            &mut native_policy,
            format!(
                "Source text offsets ({source_x}, {source_y}) are retained as inactive metadata, as observed in stock GIMP text rendering; native placement uses the layer offset and editable affine matrix"
            ),
        );
    }
    if let Some(markup) = field(&fields, "markup")? {
        let source = markup.first().ok_or_else(|| invalid("missing text markup"))?.text()?;
        let mut fonts = HashMap::new();
        let mut font_allowance = data.len().saturating_mul(8).saturating_add(65_536).min(super::MAX_METADATA_BYTES);
        for resource in markup.iter().skip(1) {
            let record = resource.list()?;
            if record.first().and_then(|v| v.text().ok()) != Some("markupfont") || record.len() != 3 {
                return Err(invalid("invalid markup font resource record"));
            }
            let name = record.get(1).ok_or_else(|| invalid("missing markup font name"))?.text()?;
            let font = record.get(2).ok_or_else(|| invalid("missing markup font resource"))?.list()?;
            if font.first().and_then(|v| v.text().ok()) != Some("font") {
                return Err(invalid("invalid markup font resource"));
            }
            let fields = font.get(1..).unwrap_or(&[]);
            let mut requested = style.clone();
            requested_font(fields, &mut requested)?;
            font_allowance = font_allowance
                .checked_sub(style_allocation(&requested).saturating_add(name.len()))
                .ok_or_else(|| crate::IoError::Unsupported("native markup font resources exceed the bounded formatting allowance".into()))?;
            if fonts.insert(name.to_owned(), requested).is_some() {
                return Err(invalid("duplicate markup font identity"));
            }
        }
        let (text, runs) = markup_runs(source, &style, &fonts, dpi, &mut notes)?;
        layer.text = text;
        layer.runs = runs;
    } else {
        layer.text = scalar(&fields, "text")?.unwrap_or("").into();
        layer.runs.push(TextRun { len: layer.text.len(), style });
    }
    // Resolve markup sizes first: inherited leading based on the base size can
    // make large markup lines overlap despite positive source extra spacing.
    if extra_leading != 0.0 {
        note(
            &mut notes,
            "Source additional line spacing becomes explicit native leading relative to PhotoCraft's 1.2-em default; native font metrics and layout apply"
                .into(),
        );
        for run in &mut layer.runs {
            let leading = f64::from(run.style.size_pt) * 1.2 + extra_leading;
            if leading.is_finite() && (0.001..=8192.0).contains(&leading) {
                run.style.leading_pt = Some(leading as f32);
            } else {
                note(&mut notes, "Source line spacing falls outside native positive leading; native automatic leading retained".into());
            }
        }
    }
    layer.paragraphs.push(ParagraphRun { len: layer.text.len(), style: para });
    layer.sync_summary();
    anchor_point_origin(&mut layer, dpi as f32)?;
    // These GIMP-specific fields cannot silently control native layout.
    const MAPPED: &[&str] = &[
        "text",
        "markup",
        "font",
        "font-size",
        "font-size-unit",
        "antialias",
        "language",
        "base-direction",
        "color",
        "justify",
        "indent",
        "kerning",
        "letter-spacing",
        "line-spacing",
        "box-mode",
        "box-width",
        "box-height",
        "box-unit",
        "transformation",
        "offset-x",
        "offset-y",
    ];
    for value in &fields {
        let name = value.list()?.first().ok_or_else(|| invalid("empty text field"))?.text()?;
        if !MAPPED.contains(&name) {
            note(&mut notes, format!("Source {name} is retained; native typography and rasterization apply"));
        }
    }
    Ok(DecodedText { layer, source_only: notes, native_policy })
}

/// Historical GDynText's public eight-field GDT10 record. XLFD identifies the
/// requested native font; it never identifies a file to load. Independent stock
/// compatibility probes establish that its finite rotation control is inactive.
pub(super) fn decode_gdyntext(data: &[u8], dpi: f32, offset: (i32, i32)) -> Result<DecodedText> {
    if data.len() > super::MAX_STRING || !dpi.is_finite() || dpi <= 0. {
        return Err(invalid("invalid legacy text size or DPI"));
    }
    let source = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(data)).map_err(|_| invalid("legacy text configuration is not UTF-8 escaped data"))?;
    let source = source.strip_prefix("GDT10{").and_then(|source| source.strip_suffix('}')).ok_or_else(|| invalid("legacy text magic/framing is invalid"))?;
    let fields: Vec<_> = source.split("}{").take(9).collect();
    if fields.len() != 8 {
        return Err(invalid("legacy text requires eight fields"));
    }
    let get = |index| fields.get(index).copied().ok_or_else(|| invalid("missing legacy text field"));
    let quoted = format!("\"{}\"", get(0)?);
    let mut parser = Parser { bytes: quoted.as_bytes(), at: 0, nodes: 0 };
    let Value::Atom(bytes) = parser.value(0)? else {
        return Err(invalid("legacy text is not an escaped string"));
    };
    parser.space();
    if parser.at != quoted.len() || bytes.contains(&0) {
        return Err(invalid("invalid trailing legacy text data"));
    }
    let xlfd = get(7)?.strip_prefix('-').ok_or_else(|| invalid("legacy font is not XLFD"))?;
    let font: Vec<_> = xlfd.split('-').take(15).collect();
    if font.len() != 14 || font.iter().any(|field| field.len() > 64) {
        return Err(invalid("legacy XLFD field count or length is invalid"));
    }
    let font_field = |index| font.get(index).copied().ok_or_else(|| invalid("missing XLFD field"));
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if font_field(12)?.eq_ignore_ascii_case("iso8859") && font_field(13)? == "1" => error.into_bytes().into_iter().map(char::from).collect(),
        Err(_) => return Err(crate::IoError::Unsupported("legacy non-UTF8 text requires an explicit supported charset".into())),
    };
    let rotation = get(3)?.parse::<f64>().map_err(|_| invalid("invalid legacy rotation"))?;
    if !rotation.is_finite() {
        return Err(invalid("legacy rotation is not finite"));
    }
    let pixels = font_field(6)?.parse::<f32>().ok().filter(|size| size.is_finite() && *size > 0.);
    let points = font_field(7)?.parse::<f32>().ok().filter(|size| size.is_finite() && *size > 0.).map(|size| size / 10.);
    let size_pt = pixels.map(|size| size * 72. / dpi).or(points).unwrap_or(24. * 72. / dpi);
    if !(0.001..=8192.).contains(&size_pt) {
        return Err(invalid("legacy native text size out of range"));
    }
    let mut style = CharStyle {
        font_family: font_field(1)?.into(),
        size_pt,
        weight: match font_field(2)?.to_ascii_lowercase().as_str() {
            "bold" => 700,
            "demibold" | "semibold" => 600,
            "light" => 300,
            "black" | "heavy" => 900,
            _ => 400,
        },
        italic: matches!(font_field(3)?, "i" | "o"),
        ..Default::default()
    };
    let rgb = u32::from_str_radix(get(5)?.trim_start_matches("0x"), 16).map_err(|_| invalid("invalid legacy RGB color"))?;
    if rgb > 0xffffff {
        return Err(invalid("legacy RGB color exceeds 24 bits"));
    }
    style.color = Color::rgb(((rgb >> 16) & 255) as f32 / 255., ((rgb >> 8) & 255) as f32 / 255., (rgb & 255) as f32 / 255.);
    let spacing = get(4)?.parse::<f32>().map_err(|_| invalid("invalid legacy line spacing"))?;
    if !spacing.is_finite() || spacing.abs() > 8192. {
        return Err(invalid("legacy line spacing exceeds native bounds"));
    }
    let mut notes =
        vec!["Legacy XLFD request uses native font matching; source layer alignment is retained and native placement follows the layer offset".into()];
    let mut native_policy = Vec::new();
    if rotation != 0. {
        native_policy.push(format!("Legacy rotation control {rotation} is retained as inactive metadata, as observed when stock GIMP imports and edits this historical record; editable native text keeps the layer placement"));
    }
    if spacing != 0. {
        let leading = 1.2 * size_pt + spacing * 72. / dpi;
        if (0.001..=8192.).contains(&leading) {
            style.leading_pt = Some(leading);
        }
        notes.push("Legacy line spacing becomes native leading relative to the native 1.2 em default; font metrics/layout may differ".into());
    }
    let align = match get(2)? {
        "0" => TextAlign::Left,
        "1" => TextAlign::Center,
        "2" => TextAlign::Right,
        _ => return Err(invalid("invalid legacy text alignment")),
    };
    let antialias = match get(1)? {
        "0" => AntiAlias::None,
        "1" => AntiAlias::Smooth,
        _ => return Err(invalid("invalid legacy text antialias flag")),
    };
    let _layer_alignment = get(6)?.parse::<i32>().map_err(|_| invalid("invalid legacy layer alignment"))?;
    let mut layer = TextLayer { text, transform: Affine::translate(f64::from(offset.0), f64::from(offset.1)), antialias, ..Default::default() };
    layer.runs.push(TextRun { len: layer.text.len(), style });
    layer.paragraphs.push(ParagraphRun { len: layer.text.len(), style: ParagraphStyle { align, direction: TextDirection::Ltr, ..Default::default() } });
    layer.sync_summary();
    anchor_point_origin(&mut layer, dpi)?;
    Ok(DecodedText { layer, source_only: notes, native_policy })
}

/// Translate stored font names, weight and slant into a native font request without loading bundled font resources.
fn requested_font(fields: &[Value], style: &mut CharStyle) -> Result<()> {
    style.font_family = scalar(fields, "fullname")?.or(scalar(fields, "family")?).unwrap_or("").into();
    style.font_style = scalar(fields, "style")?.unwrap_or("").into();
    style.postscript_name = scalar(fields, "psname")?.filter(|name| !name.is_empty()).map(str::to_owned);
    let weight = number(fields, "weight", 80., 0., 215.)?;
    style.weight = fontconfig_weight(weight);
    style.italic = number(fields, "slant", 0., 0., 110.)? != 0.;
    Ok(())
}

/// Convert Fontconfig's public weight scale to native OpenType requests, interpolating
/// intermediate values and retaining the native editable endpoints.
fn fontconfig_weight(weight: f64) -> u16 {
    const WEIGHTS: &[(f64, f64)] = &[
        (0., 100.),
        (40., 200.),
        (50., 300.),
        (55., 350.),
        (75., 380.),
        (80., 400.),
        (100., 500.),
        (180., 600.),
        (200., 700.),
        (205., 800.),
        (210., 900.),
        (215., 900.),
    ];
    for pair in WEIGHTS.windows(2) {
        if let [(a, x), (b, y)] = pair
            && weight <= *b
        {
            return (x + (weight - a) * (y - x) / (b - a)).round().clamp(100., 900.) as u16;
        }
    }
    900
}

/// Decode explicit `pt` syntax or a Pango-scaled magnitude. Font sizes use points; device-distance attributes still require DPI conversion.
fn pango_points(text: &str) -> Result<f32> {
    let value = if let Some(points) = text.strip_suffix("pt") { points.parse::<f32>() } else { text.parse::<f32>().map(|value| value / 1024.) }
        .map_err(|_| invalid("invalid markup point length"))?;
    if !value.is_finite() || value.abs() > 8192. {
        return Err(invalid("markup point length exceeds native bounds"));
    }
    Ok(value)
}
/// Convert a finite device-pixel distance into native points without applying the font-size unit or permitting nonfinite native data.
fn device_points(pixels: f64, dpi: f64) -> Result<f32> {
    let points = pixels * 72. / dpi;
    if !dpi.is_finite() || dpi <= 0. || !points.is_finite() || points.abs() > 8192. {
        return Err(invalid("markup device distance exceeds native point bounds"));
    }
    Ok(points as f32)
}

/// Parse bounded OpenType feature settings, validate four-character tags and resolve repeated tags to their final values.
fn features(text: &str) -> Result<Vec<FontFeature>> {
    let mut result = Vec::new();
    for entry in text.split(',').map(str::trim).filter(|entry| !entry.is_empty()) {
        if result.len() >= 64 {
            return Err(invalid("too many native font features"));
        }
        let (entry, default) = if let Some(rest) = entry.strip_prefix('-') { (rest, 0) } else { (entry.strip_prefix('+').unwrap_or(entry), 1) };
        let mut parts = entry.split(|c: char| c == '=' || c.is_ascii_whitespace()).filter(|s| !s.is_empty());
        let tag = parts.next().unwrap_or("").trim_matches(['\'', '"']);
        if tag.len() != 4 || !tag.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(invalid("native OpenType feature tag requires four ASCII characters"));
        }
        let value = match parts.next() {
            None => default,
            Some("on") => 1,
            Some("off") => 0,
            Some(value) => value.parse::<u16>().map_err(|_| invalid("invalid native OpenType feature value"))?,
        };
        if parts.next().is_some() {
            return Err(invalid("unexpected font feature syntax"));
        }
        if let Some(previous) = result.iter_mut().find(|feature: &&mut FontFeature| feature.tag == tag) {
            previous.value = value;
        } else {
            result.push(FontFeature { tag: tag.into(), value });
        }
    }
    Ok(result)
}

/// Public Pango font-description grammar, translated into ordinary native font
/// requests/styles. Unsupported request options remain historical notes.
fn font_description(source: &str, style: &mut CharStyle, dpi: f64, apply_size: bool, notes: &mut Vec<String>) -> Result<()> {
    if source.len() > super::MAX_STRING {
        return Err(invalid("font description exceeds limit"));
    }
    let mut words: Vec<_> = source.split_whitespace().take(257).collect();
    if words.len() > 256 {
        return Err(invalid("font description has too many tokens"));
    }
    while let Some(word) = words.last().copied() {
        if let Some(settings) = word.strip_prefix('#') {
            style.features = features(settings)?;
            words.pop();
            continue;
        }
        if let Some(settings) = word.strip_prefix('@') {
            let mut variations = Vec::new();
            for pair in settings.split(',') {
                let (axis, value) = pair.split_once('=').ok_or_else(|| invalid("invalid font variation setting"))?;
                let value = value.parse::<f32>().map_err(|_| invalid("invalid font variation number"))?;
                if axis.len() != 4
                    || !axis.bytes().all(|b| b.is_ascii_alphanumeric())
                    || !value.is_finite()
                    || value.abs() > 1_000_000.
                    || variations.len() >= 64
                {
                    return Err(invalid("font variation exceeds native bounds"));
                }
                variations.push(FontVariation { axis: axis.into(), value });
            }
            style.variations = variations;
            words.pop();
            continue;
        }
        break;
    }
    if let Some(word) = words.last().copied() {
        let (number, pixels) = word.strip_suffix("px").map_or((word, false), |number| (number, true));
        if let Ok(size) = number.parse::<f32>() {
            if !size.is_finite() || !(0.001..=8192.).contains(&size) {
                return Err(invalid("font description size exceeds native bounds"));
            }
            if apply_size {
                style.size_pt = if pixels { (f64::from(size) * 72. / dpi) as f32 } else { size };
            }
            words.pop();
        }
    }
    const OPTIONS: &[&str] = &[
        "normal",
        "roman",
        "regular",
        "italic",
        "oblique",
        "thin",
        "ultralight",
        "extralight",
        "light",
        "semilight",
        "demilight",
        "book",
        "medium",
        "semibold",
        "demibold",
        "bold",
        "ultrabold",
        "extrabold",
        "heavy",
        "black",
        "ultrablack",
        "extrablack",
        "smallcaps",
        "ultracondensed",
        "extracondensed",
        "condensed",
        "semicondensed",
        "semiexpanded",
        "expanded",
        "extraexpanded",
        "ultraexpanded",
    ];
    let suffix = words.iter().rposition(|word| !OPTIONS.contains(&word.to_ascii_lowercase().replace('-', "").as_str())).map_or(0, |index| index + 1);
    let style_names = words.split_off(suffix);
    // A complete font description supplies normal defaults for omitted face options;
    // it must not accidentally inherit the preceding span's bold/italic face identity.
    style.weight = 400;
    style.italic = false;
    style.caps = Caps::Normal;
    style.horizontal_scale = 1.;
    style.font_style.clear();
    style.postscript_name = None;
    for word in &style_names {
        let option = word.to_ascii_lowercase().replace('-', "");
        match option.as_str() {
            "normal" | "roman" | "regular" => {
                style.weight = 400;
                style.italic = false;
            }
            "italic" | "oblique" => style.italic = true,
            "thin" => style.weight = 100,
            "ultralight" | "extralight" => style.weight = 200,
            "light" => style.weight = 300,
            "semilight" | "demilight" => style.weight = 350,
            "book" => style.weight = 380,
            "medium" => style.weight = 500,
            "semibold" | "demibold" => style.weight = 600,
            "bold" => style.weight = 700,
            "ultrabold" | "extrabold" => style.weight = 800,
            "heavy" | "black" => style.weight = 900,
            "ultrablack" | "extrablack" => {
                style.weight = 900;
                note(notes, "Source ultra-black font weight becomes native maximum 900 weight".into());
            }
            "smallcaps" => style.caps = Caps::SmallCaps,
            "ultracondensed" => style.horizontal_scale = 0.5,
            "extracondensed" => style.horizontal_scale = 0.625,
            "condensed" => style.horizontal_scale = 0.75,
            "semicondensed" => style.horizontal_scale = 0.875,
            "semiexpanded" => style.horizontal_scale = 1.125,
            "expanded" => style.horizontal_scale = 1.25,
            "extraexpanded" => style.horizontal_scale = 1.5,
            "ultraexpanded" => style.horizontal_scale = 2.,
            _ => break,
        }
    }
    if !style_names.is_empty() {
        style.font_style = style_names.join(" ");
    }
    let families = words.join(" ");
    if let Some(first) = families.split(',').next().filter(|family| !family.trim().is_empty()) {
        style.font_family = first.trim().into();
    }
    if families.trim_end_matches(',').contains(',') {
        note(notes, "Source family fallback list retained; native font resolution uses its first requested family".into());
    }
    if style.horizontal_scale != 1. {
        note(notes, "Source font stretch becomes native horizontal glyph scale; native font matching/shaping applies".into());
    }
    if apply_size && (!style.size_pt.is_finite() || !(0.001..=8192.).contains(&style.size_pt)) {
        return Err(crate::IoError::Unsupported("font description size is outside the native editable range".into()));
    }
    Ok(())
}

/// Translate bounded XML markup into UTF-8 text and native style runs.
/// DTD, namespaced reinterpretation and materialized style amplification are rejected.
fn markup_runs(markup: &str, base: &CharStyle, fonts: &HashMap<String, CharStyle>, dpi: f64, notes: &mut Vec<String>) -> Result<(String, Vec<TextRun>)> {
    let xml =
        roxmltree::Document::parse_with_options(markup, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES as u32, ..Default::default() })
            .map_err(|e| invalid(format!("invalid source text markup: {e}")))?;
    let mut text = String::new();
    let mut runs = Vec::new();
    // A short ancestor style can otherwise multiply a long font name/feature
    // list across thousands of leaf runs. Bound materialized native data, too.
    let mut allowance = markup.len().saturating_mul(8).saturating_add(65_536).min(super::MAX_METADATA_BYTES);
    for node in xml.descendants().filter(roxmltree::Node::is_text) {
        let value = node.text().unwrap_or("");
        let mut style = base.clone();
        // Pango selects size and scale independently. Nested percentages replace
        // the active scale; an absolute size does not discard inherited scale.
        let mut unscaled_size = base.size_pt;
        let mut size_scale = 1.0;
        let mut spacing_pt = base.tracking * base.size_pt / 1000.;
        let ancestors: Vec<_> = node.ancestors().filter(roxmltree::Node::is_element).collect();
        if ancestors.len() > MAX_DEPTH {
            return Err(invalid("text markup nesting exceeds limit"));
        }
        for ancestor in ancestors.iter().rev() {
            style.size_pt = unscaled_size;
            if ancestor.tag_name().namespace().is_some() || ancestor.attributes().any(|attribute| attribute.namespace().is_some()) {
                return Err(crate::IoError::Unsupported("namespaced markup does not have an admitted native text-formatting meaning".into()));
            }
            if let Some(requested) = ancestor.attribute("font").or_else(|| ancestor.attribute("font_desc")) {
                if let Some(font) = fonts.get(requested) {
                    style.font_family = font.font_family.clone();
                    style.font_style = font.font_style.clone();
                    style.postscript_name = font.postscript_name.clone();
                    style.weight = font.weight;
                    style.italic = font.italic;
                } else {
                    font_description(requested, &mut style, dpi, true, notes)?;
                }
            }
            unscaled_size = style.size_pt;
            if matches!(ancestor.tag_name().name(), "b" | "i" | "tt")
                || ancestor.attributes().any(|a| matches!(a.name(), "font_family" | "face" | "weight" | "font_weight" | "style" | "font_style"))
            {
                // Native layout resolves a PostScript identity before a family/style
                // request; an overriding request must not retain the old exact face.
                style.postscript_name = None;
            }
            match ancestor.tag_name().name() {
                "markup" | "span" => {}
                "b" => style.weight = 700,
                "i" => style.italic = true,
                "u" => style.underline = true,
                "s" => style.strikethrough = true,
                "sub" => style.baseline = FontBaseline::Subscript,
                "sup" => style.baseline = FontBaseline::Superscript,
                "tt" => style.font_family = "monospace".into(),
                "big" => size_scale *= 1.2,
                "small" => size_scale /= 1.2,
                _ => note(notes, format!("Source markup element {} is retained without native formatting", diagnostic_value(ancestor.tag_name().name()))),
            }
            let mut explicit_alpha = None;
            for a in ancestor.attributes() {
                match a.name() {
                    "font" | "font_desc" => {}
                    "font_family" | "face" => style.font_family = a.value().into(),
                    "weight" | "font_weight" => {
                        style.weight = match a.value() {
                            "bold" => 700,
                            "normal" => 400,
                            "ultralight" => 200,
                            "light" => 300,
                            "semilight" => 350,
                            "book" => 380,
                            "medium" => 500,
                            "semibold" => 600,
                            "ultrabold" => 800,
                            "heavy" => 900,
                            s => {
                                let weight = s.parse::<u16>().map_err(|_| invalid("invalid markup weight"))?;
                                if weight > 1000 {
                                    return Err(crate::IoError::Unsupported("source numeric font weight needs an explicit native conversion".into()));
                                }
                                if !(100..=900).contains(&weight) {
                                    note(notes, "Source numeric font weight becomes the nearest editable native endpoint in100..900".into());
                                }
                                weight.clamp(100, 900)
                            }
                        }
                    }
                    "style" | "font_style" => {
                        style.italic = match a.value() {
                            "normal" => false,
                            "italic" | "oblique" => true,
                            _ => return Err(invalid("invalid markup font style")),
                        }
                    }
                    "variant" | "font_variant" => match a.value() {
                        "normal" => style.caps = Caps::Normal,
                        "small-caps" => style.caps = Caps::SmallCaps,
                        _ => note(notes, format!("Source font variant {} remains inert", diagnostic_value(a.value()))),
                    },
                    "stretch" | "font_stretch" => {
                        style.horizontal_scale = match a.value() {
                            "ultracondensed" => 0.5,
                            "extracondensed" => 0.625,
                            "condensed" => 0.75,
                            "semicondensed" => 0.875,
                            "normal" => 1.,
                            "semiexpanded" => 1.125,
                            "expanded" => 1.25,
                            "extraexpanded" => 1.5,
                            "ultraexpanded" => 2.,
                            _ => return Err(invalid("unknown font stretch")),
                        };
                        note(notes, "Source font stretch becomes native horizontal glyph scale; native font matching/shaping applies".into());
                    }
                    // Pango renders both numeric and `pt` rise syntax as device distances.
                    // Font sizes below remain point-valued, unlike these placement distances.
                    "rise" => style.baseline_shift_pt = device_points(f64::from(pango_points(a.value())?), dpi)?,
                    "letter_spacing" => spacing_pt = device_points(f64::from(pango_points(a.value())?), dpi)?,
                    "font_features" => {
                        for feature in features(a.value())? {
                            if let Some(previous) = style.features.iter_mut().find(|previous| previous.tag == feature.tag) {
                                *previous = feature;
                            } else {
                                if style.features.len() >= 64 {
                                    return Err(crate::IoError::Unsupported("native font feature count exceeds the bounded import allowance".into()));
                                }
                                style.features.push(feature);
                            }
                        }
                    }
                    "underline" => {
                        style.underline = match a.value() {
                            "none" => false,
                            "single" => true,
                            "double" | "low" | "error" => {
                                note(notes, format!("Source {} underline becomes native single underline", a.value()));
                                true
                            }
                            _ => return Err(crate::IoError::Unsupported("source underline style requires a native mapping".into())),
                        };
                    }
                    "text_transform" => match a.value() {
                        "none" => style.caps = Caps::Normal,
                        "uppercase" => style.caps = Caps::AllCaps,
                        _ => note(notes, format!("Source text transform {} remains inert", diagnostic_value(a.value()))),
                    },
                    "strikethrough" => style.strikethrough = a.value() == "true",
                    "foreground" | "fgcolor" | "color" => {
                        let color = markup_color(a.value())?;
                        style.color.c = color.c;
                        if a.value().starts_with('#') && a.value().len() == 9 {
                            style.color.alpha = color.alpha;
                        }
                    }
                    "size" | "font_size" => {
                        match a.value() {
                            "xx-small" => size_scale = 1. / (1.2 * 1.2 * 1.2),
                            "x-small" => size_scale = 1. / (1.2 * 1.2),
                            "small" => size_scale = 1. / 1.2,
                            "medium" => size_scale = 1.,
                            "large" => size_scale = 1.2,
                            "x-large" => size_scale = 1.2 * 1.2,
                            "xx-large" => size_scale = 1.2 * 1.2 * 1.2,
                            "smaller" => size_scale /= 1.2,
                            "larger" => size_scale *= 1.2,
                            s if s.ends_with('%') => {
                                size_scale = s.trim_end_matches('%').parse::<f32>().map_err(|_| invalid("invalid markup font percentage"))? / 100.;
                            }
                            s => unscaled_size = pango_points(s)?,
                        };
                    }
                    "alpha" | "foreground_alpha" | "fgalpha" => {
                        let (value, max) = if let Some(value) = a.value().strip_suffix('%') { (value, 100.) } else { (a.value(), 65535.) };
                        let alpha = value.parse::<f32>().map_err(|_| invalid("invalid markup alpha"))?;
                        if !alpha.is_finite() || !(0.0..=max).contains(&alpha) {
                            return Err(invalid("markup alpha out of bounds"));
                        }
                        explicit_alpha = Some(alpha / max);
                    }
                    "lang" => style.language = Some(a.value().replace('_', "-")),
                    _ => note(notes, format!("Source markup attribute {} is retained without native formatting", diagnostic_value(a.name()))),
                }
            }
            // Explicit opacity overrides an RGBA foreground independently of XML
            // attribute order; each nested span still inherits the resolved parent.
            if let Some(alpha) = explicit_alpha {
                style.color.alpha = alpha;
            }
            style.size_pt = unscaled_size * size_scale;
            if !style.size_pt.is_finite() || !(0.001..=8192.0).contains(&style.size_pt) {
                return Err(invalid("markup font size out of bounds"));
            }
            style.tracking = spacing_pt * 1000. / style.size_pt;
            if !style.tracking.is_finite() || style.tracking.abs() > 1_000_000. {
                return Err(invalid("native markup tracking exceeds bounds"));
            }
        }
        let bytes = std::mem::size_of::<TextRun>().saturating_add(value.len()).saturating_add(style_allocation(&style));
        allowance =
            allowance.checked_sub(bytes).ok_or_else(|| crate::IoError::Unsupported("native text runs exceed the bounded formatting allowance".into()))?;
        text.push_str(value);
        runs.push(TextRun { len: value.len(), style });
    }
    Ok((text, runs))
}

#[cfg(test)]
mod tests;
