//! Native text policy over the shared source records. Saved pixels remain
//! authoritative until every optional native conversion has succeeded.

use craft_xcf::records::{
    Budget,
    text::{Config, Direction, FieldError, Source, TextRecord},
};
use photocraft_doc::text::{AntiAlias, CharStyle, Kerning, Orientation, ParagraphRun, ParagraphStyle, TextAlign, TextDirection, TextRun, TextShape};
use photocraft_doc::{Layer, LayerContent, TextLayer};
use photocraft_geom::Affine;
use photocraft_raster::Interrupt;

use super::{Result, TextSource};

mod color;
mod layout_admission;
mod markup;
#[cfg(test)]
mod tests;
mod typography;
pub(in crate::xcf) use color::NativeColors;

pub(in crate::xcf) struct NativeBudget {
    pub storage_left: usize,
    pub layout_left: usize,
    pub work_left: usize,
    pub limit_reason: Option<craft_xcf::records::StopReason>,
}
impl Default for NativeBudget {
    fn default() -> Self {
        Self { storage_left: craft_xcf::MAX_METADATA_BYTES, layout_left: 1 << 20, work_left: 8 * craft_xcf::MAX_METADATA_BYTES, limit_reason: None }
    }
}
impl NativeBudget {
    fn work(&mut self, bytes: usize) -> Result<()> {
        let Some(left) = self.work_left.checked_sub(bytes) else {
            self.limit_reason = Some(craft_xcf::records::StopReason::WorkLimit);
            return Err(unsupported("native text conversion work allowance exhausted"));
        };
        self.work_left = left;
        Ok(())
    }
    fn charge<T>(&mut self, count: usize) -> Result<()> {
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or_else(|| unsupported("native text allocation size overflow"))?;
        let Some(left) = self.storage_left.checked_sub(bytes) else {
            self.limit_reason = Some(craft_xcf::records::StopReason::RetainedLimit);
            return Err(unsupported("native text exceeds the document metadata allowance"));
        };
        self.storage_left = left;
        Ok(())
    }
    fn push<T>(&mut self, values: &mut Vec<T>, value: T) -> Result<()> {
        if values.len() == values.capacity() {
            let count = values.capacity().max(1);
            self.charge::<T>(count)?;
            values.try_reserve_exact(count).map_err(|_| {
                self.limit_reason = Some(craft_xcf::records::StopReason::AllocationFailure);
                unsupported("native text allocation is unavailable")
            })?;
        }
        values.push(value);
        Ok(())
    }
    fn string(&mut self, source: &str) -> Result<String> {
        self.charge::<u8>(source.len())?;
        let mut value = String::new();
        value.try_reserve_exact(source.len()).map_err(|_| {
            self.limit_reason = Some(craft_xcf::records::StopReason::AllocationFailure);
            unsupported("native text string allocation is unavailable")
        })?;
        value.push_str(source);
        Ok(value)
    }
    fn language(&mut self, source: &str) -> Result<String> {
        self.charge::<u8>(source.len())?;
        let mut value = String::new();
        value.try_reserve_exact(source.len()).map_err(|_| {
            self.limit_reason = Some(craft_xcf::records::StopReason::AllocationFailure);
            unsupported("native language allocation is unavailable")
        })?;
        for character in source.chars() {
            value.push(if character == '_' { '-' } else { character });
        }
        Ok(value)
    }
}

#[derive(Clone, Copy)]
pub(in crate::xcf) struct NativeOptions<'a> {
    pub resolution: (f32, f32),
    pub offset: (i32, i32),
    pub colors: Option<&'a NativeColors>,
}

pub(in crate::xcf) enum Mapping {
    Missing,
    Converted { notes: Vec<&'static str> },
    Preserved { reason: String, source_reason: Option<craft_xcf::records::StopReason> },
}

fn unsupported(reason: &'static str) -> crate::IoError {
    crate::IoError::Unsupported(reason.into())
}
fn field_error(_: FieldError) -> crate::IoError {
    unsupported("invalid or duplicated source text field")
}

fn number(config: &Config<'_>, name: &str, fallback: f64, min: f64, max: f64) -> Result<f64> {
    let value = config.number(name).map_err(field_error)?.unwrap_or(fallback);
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(unsupported("source text metric exceeds native bounds"));
    }
    Ok(value)
}

fn unit_scale(unit: &str, dpi: f64) -> Result<f64> {
    match unit {
        "pixels" => Ok(72. / dpi),
        "points" => Ok(1.),
        "inches" => Ok(72.),
        "millimeters" => Ok(72. / 25.4),
        "centimeters" => Ok(72. / 2.54),
        "picas" => Ok(12.),
        _ => Err(unsupported("source text unit has no native mapping")),
    }
}

fn note(notes: &mut Vec<&'static str>, message: &'static str) {
    if !notes.contains(&message) && notes.len() < 32 {
        notes.push(message);
    }
}

/// Preserve the original Layer/Raster on every failure. The source object remains
/// borrowed so the caller can attach its inert bytes to a durable import report.
pub(in crate::xcf) fn map_layer(
    layer: &mut Layer,
    source: &TextSource<'_>,
    options: NativeOptions<'_>,
    native: &mut NativeBudget,
    source_budget: &mut Budget,
    ctl: &Interrupt<'_>,
) -> Result<Mapping> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    let (record, keep_layer_name) = match source {
        TextSource::Missing => return Ok(Mapping::Missing),
        TextSource::ModifiedPixels { .. } => {
            return Ok(Mapping::Preserved { reason: "source text pixels were modified independently; saved pixels retained".into(), source_reason: None });
        }
        TextSource::Incomplete(reason) => {
            return Ok(Mapping::Preserved {
                reason: "the authoritative source text configuration is incomplete; saved pixels retained".into(),
                source_reason: Some(*reason),
            });
        }
        TextSource::Configuration { record, keep_layer_name } => (record, *keep_layer_name),
    };
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Ok(Mapping::Preserved { reason: "the source text has no authoritative raster layer".into(), source_reason: None });
    }
    let storage = native.storage_left;
    native.limit_reason = None;
    let prepared = prepare(record, options, native, source_budget, ctl);
    let (text, notes) = match prepared {
        Ok((mut text, notes)) => {
            text.keep_layer_name = keep_layer_name;
            (text, notes)
        }
        Err(crate::IoError::Cancelled) => return Err(crate::IoError::Cancelled),
        Err(error) => {
            native.storage_left = storage;
            return Ok(Mapping::Preserved { reason: error.to_string(), source_reason: native.limit_reason });
        }
    };
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    let previous = std::mem::replace(&mut layer.content, LayerContent::Text(text));
    match (&mut layer.content, previous) {
        (LayerContent::Text(text), LayerContent::Raster(surface)) => text.cache = Some(surface),
        (_, previous) => {
            layer.content = previous;
            native.storage_left = storage;
            return Ok(Mapping::Preserved { reason: "the source text target changed before conversion".into(), source_reason: None });
        }
    }
    Ok(Mapping::Converted { notes })
}

fn prepare(
    record: &TextRecord<'_>,
    options: NativeOptions<'_>,
    native: &mut NativeBudget,
    source_budget: &mut Budget,
    ctl: &Interrupt<'_>,
) -> Result<(TextLayer, Vec<&'static str>)> {
    if ![options.resolution.0, options.resolution.1].into_iter().all(|dpi| dpi.is_finite() && dpi > 0. && dpi < 1.0e6) {
        return Err(unsupported("source text resolution is invalid"));
    }
    native.work(record.raw.len().checked_mul(32).and_then(|bytes| bytes.checked_add(256)).ok_or_else(|| unsupported("native text work size overflow"))?)?;
    let colors = options.colors.ok_or_else(|| unsupported("the source color encoding cannot safely represent editable text"))?;
    let mut notes = Vec::new();
    let (mut layer, mut style, para, spacing, extra_leading) = match &record.source {
        Source::Gimp(config) => modern(record, config, options, native, &mut notes)?,
        Source::Gdyntext(legacy) => typography::legacy(record, legacy, options, native, &mut notes)?,
    };
    if let Some(raw) = record.markup().map_err(field_error)? {
        markup::runs(&mut layer, raw, record, &style, options, native, markup::Work { source_budget, ctl, notes: &mut notes })?;
    } else {
        layer.text = native.string(record.text().map_err(field_error)?.unwrap_or(""))?;
        if spacing.abs() > 0.1 {
            style.ligatures = false;
        }
        native.push(&mut layer.runs, TextRun { len: layer.text.len(), style })?;
    }
    for run in &mut layer.runs {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        if spacing.abs() > 0.1 {
            run.style.ligatures = false;
        }
        if extra_leading != 0. {
            let leading = f64::from(run.style.size_pt) * 1.2 + extra_leading;
            if (0.001..=8192.).contains(&leading) {
                run.style.leading_pt = Some(leading as f32);
            } else {
                note(&mut notes, "source line spacing exceeds positive native leading; automatic leading retained");
            }
        }
        run.style.color = colors.convert(run.style.color)?;
    }
    native.push(&mut layer.paragraphs, ParagraphRun { len: layer.text.len(), style: para })?;
    apply_resolution(&mut layer, options.resolution, &mut notes, ctl)?;
    if let Some(run) = layer.runs.first() {
        layer.font_family = native.string(&run.style.font_family)?;
        layer.size_pt = run.style.size_pt;
        layer.color = run.style.color;
    }
    if matches!(layer.shape, TextShape::Point) && !layer.text.is_empty() {
        let Some(cost) = layout_admission::cost(&layer, options.resolution.0) else {
            native.limit_reason = Some(craft_xcf::records::StopReason::RecordLimit);
            return Err(unsupported("source point text exceeds the bounded origin-layout allowance"));
        };
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let Some(left) = native.layout_left.checked_sub(cost) else {
            native.limit_reason = Some(craft_xcf::records::StopReason::WorkLimit);
            return Err(unsupported("document point-text origin work allowance exhausted"));
        };
        native.layout_left = left;
        let bounds = {
            let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            engine.layout(&layer, options.resolution.0).bounds().ok_or_else(|| unsupported("native font metrics are unavailable for the source text origin"))?
        };
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        anchor_bounds(&mut layer, bounds)?;
    }
    layer.cache_uses_raster_blending = true;
    note(&mut notes, "saved source text appearance is preserved; native font matching and layout apply after edits");
    Ok((layer, notes))
}

fn modern(
    record: &TextRecord<'_>,
    config: &Config<'_>,
    options: NativeOptions<'_>,
    native: &mut NativeBudget,
    notes: &mut Vec<&'static str>,
) -> Result<(TextLayer, CharStyle, ParagraphStyle, f64, f64)> {
    let (x, y) = (f64::from(options.resolution.0), f64::from(options.resolution.1));
    let unit = config.scalar("font-size-unit").map_err(field_error)?.unwrap_or("pixels");
    let scale = unit_scale(unit, y)?;
    let size = number(config, "font-size", 24., 0.001, 8192.)? * scale;
    if !size.is_finite() || size > 1_000_000. {
        return Err(unsupported("native source font size exceeds bounds"));
    }
    let mut style = CharStyle { size_pt: size as f32, kerning: Kerning::Metrics, ..Default::default() };
    if let Some(font) = config.field("font").map_err(field_error)? {
        typography::font(font, &mut style, y, native)?;
    }
    if config.scalar("kerning").map_err(field_error)?.is_some() {
        note(notes, "source kerning flag remains inactive; native metrics kerning applies");
    }
    let spacing = number(config, "letter-spacing", 0., -8192., 8192.)?;
    style.tracking = if spacing.abs() > 0.1 { (spacing * 72. / y * 1000. / size) as f32 } else { 0. };
    if !style.tracking.is_finite() {
        return Err(unsupported("source letter spacing exceeds native tracking bounds"));
    }
    if let Some(language) = config.scalar("language").map_err(field_error)?.filter(|language| !language.is_empty()) {
        if language.len() > 1024 {
            return Err(unsupported("source language request is too large"));
        }
        style.language = Some(native.language(language)?);
    }
    if let Some(color) = config.field("color").map_err(field_error)? {
        style.color = color::source_color(color)?;
    }
    let para = ParagraphStyle {
        align: match config.scalar("justify").map_err(field_error)?.unwrap_or("left") {
            "left" => TextAlign::Left,
            "right" => TextAlign::Right,
            "center" => TextAlign::Center,
            "fill" => TextAlign::JustifyLeft,
            _ => return Err(unsupported("source text justification is unsupported")),
        },
        direction: match record.direction {
            Direction::Auto => TextDirection::Auto,
            Direction::Ltr => TextDirection::Ltr,
            Direction::Rtl => TextDirection::Rtl,
            // Column progression is qualified separately below from the saved
            // typed base direction, even when automatic inline direction is Auto.
            Direction::TtbRtl | Direction::TtbRtlUpright => TextDirection::Auto,
            Direction::TtbLtr | Direction::TtbLtrUpright => {
                return Err(unsupported("left-to-right vertical source columns require saved pixels"));
            }
        },
        first_line_indent_pt: (number(config, "indent", 0., -8192., 8192.)? * unit_scale(unit, x)? * x / y) as f32,
        ..Default::default()
    };
    let mut layer = TextLayer {
        transform: Affine::translate(f64::from(options.offset.0), f64::from(options.offset.1)),
        antialias: if config.scalar("antialias").map_err(field_error)?.unwrap_or("yes") == "no" { AntiAlias::None } else { AntiAlias::Smooth },
        ..Default::default()
    };
    match record.base_direction {
        Some(Direction::TtbLtr | Direction::TtbLtrUpright) => {
            return Err(unsupported("left-to-right vertical source columns require saved pixels"));
        }
        Some(direction @ (Direction::TtbRtl | Direction::TtbRtlUpright)) => {
            layer.orientation = Orientation::Vertical;
            style.rotate_roman = direction == Direction::TtbRtl;
        }
        _ => {}
    }
    match config.scalar("box-mode").map_err(field_error)?.unwrap_or("dynamic") {
        "dynamic" => {}
        "fixed" => {
            let unit = config.scalar("box-unit").map_err(field_error)?.unwrap_or("pixels");
            let width = number(config, "box-width", 0., 0., f64::from(craft_xcf::MAX_SIDE))? * unit_scale(unit, x)? * x / 72.;
            let height = number(config, "box-height", 0., 0., f64::from(craft_xcf::MAX_SIDE))? * unit_scale(unit, y)? * y / 72.;
            if ![width, height].into_iter().all(|value| value.is_finite() && (0.0..=f64::from(craft_xcf::MAX_SIDE)).contains(&value)) {
                return Err(unsupported("source text box exceeds native bounds"));
            }
            layer.shape = TextShape::Box { x: 0., y: 0., width: width as f32, height: height as f32 };
        }
        _ => return Err(unsupported("source text box mode is unsupported")),
    }
    if let Some(matrix) = config.field("transformation").map_err(field_error)? {
        let [matrix] = matrix else { return Err(unsupported("source text transformation shape is invalid")) };
        let matrix = matrix.list().map_err(field_error)?;
        if matrix.len() != 5 || matrix.first().and_then(|value| value.text().ok()) != Some("matrix") {
            return Err(unsupported("source text transformation requires four coefficients"));
        }
        let mut values = [0.; 4];
        for (out, value) in values.iter_mut().zip(matrix.iter().skip(1)) {
            *out = value.text().map_err(field_error)?.parse::<f64>().map_err(|_| unsupported("invalid source text matrix"))?;
            if !out.is_finite() || out.abs() > 10_000. {
                return Err(unsupported("source text matrix exceeds native bounds"));
            }
        }
        let [a, c, b, d] = values;
        layer.transform.m = [a, b, c, d, f64::from(options.offset.0), f64::from(options.offset.1)];
    }
    if layer.transform.inverse().is_none() {
        return Err(unsupported("source text matrix cannot be edited safely"));
    }
    if number(config, "offset-x", 0., -f64::MAX, f64::MAX)? != 0. || number(config, "offset-y", 0., -f64::MAX, f64::MAX)? != 0. {
        note(notes, "serialized source text offsets remain inactive; the layer offset controls placement");
    }
    const MAPPED: &[&str] = &[
        "text",
        "markup",
        "font",
        "font-size",
        "font-size-unit",
        "antialias",
        "language",
        "base-direction",
        "base-dir-auto",
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
    for form in &config.forms {
        let name = form.list().map_err(field_error)?.first().ok_or_else(|| unsupported("empty source text field"))?.text().map_err(field_error)?;
        if !MAPPED.contains(&name) {
            note(notes, "additional source text fields remain inert; native typography applies on edit");
        }
    }
    Ok((layer, style, para, spacing, number(config, "line-spacing", 0., -8192., 8192.)? * scale))
}

fn apply_resolution(layer: &mut TextLayer, (x, y): (f32, f32), notes: &mut Vec<&'static str>, ctl: &Interrupt<'_>) -> Result<()> {
    if x == y {
        return Ok(());
    }
    let width_scale = f64::from(x) / f64::from(y);
    layer.transform = layer.transform.mul(&Affine { m: [width_scale, 0., 0., 1., 0., 0.] });
    if layer.transform.inverse().is_none() || layer.transform.m.iter().take(4).any(|value| value.abs() > 10_000.) {
        return Err(unsupported("unequal DPI exceeds editable text transform bounds"));
    }
    if let TextShape::Box { width, .. } = &mut layer.shape {
        let local = f64::from(*width) / width_scale;
        if !local.is_finite() || !(0.0..=f64::from(craft_xcf::MAX_SIDE)).contains(&local) {
            return Err(unsupported("unequal DPI exceeds native text box bounds"));
        }
        *width = local as f32;
    }
    let convert = |points: f32| -> Result<f32> {
        let value = f64::from(points) / width_scale;
        if !value.is_finite() || value.abs() > 1_000_000. || value.abs() * f64::from(x) / 72. > f64::from(craft_xcf::MAX_SIDE) {
            return Err(unsupported("unequal DPI exceeds native text metric bounds"));
        }
        Ok(value as f32)
    };
    for run in &mut layer.runs {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        run.style.size_pt = convert(run.style.size_pt)?;
        if run.style.size_pt <= 0. {
            return Err(unsupported("unequal DPI produces a nonpositive native text size"));
        }
        run.style.leading_pt = run.style.leading_pt.map(convert).transpose()?;
        run.style.baseline_shift_pt = convert(run.style.baseline_shift_pt)?;
    }
    for paragraph in &mut layer.paragraphs {
        paragraph.style.first_line_indent_pt = convert(paragraph.style.first_line_indent_pt)?;
    }
    note(notes, "unequal source DPI is retained through editable text scaling; native wrapping applies on edit");
    Ok(())
}

fn anchor_bounds(layer: &mut TextLayer, bounds: [f32; 4]) -> Result<()> {
    if !bounds.iter().all(|value| value.is_finite()) {
        return Err(unsupported("source text has no finite native logical bounds"));
    }
    let transform = layer.transform.mul(&Affine::translate(-f64::from(bounds[0]), -f64::from(bounds[1])));
    if !transform.m.iter().all(|value| value.is_finite()) || transform.m.iter().skip(4).any(|value| value.abs() > f64::from(i32::MAX)) {
        return Err(unsupported("source text anchor exceeds native affine bounds"));
    }
    layer.transform = transform;
    Ok(())
}
