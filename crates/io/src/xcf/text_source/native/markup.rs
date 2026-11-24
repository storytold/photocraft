//! Map shared source runs; never parse XML in the application adapter.

use craft_xcf::records::{
    Budget,
    text::{
        Config, TextRecord,
        markup::{self, AttributeKind, Context, Role},
    },
};
use photocraft_color::Color;
use photocraft_doc::{
    TextLayer,
    text::{CharStyle, FontBaseline, TextRun},
};
use photocraft_raster::Interrupt;

use super::{NativeBudget, NativeOptions, Result, field_error, note, typography, unsupported};

fn pango_points(value: &str) -> Result<f64> {
    let result = if let Some(points) = value.strip_suffix("pt") { points.parse::<f64>() } else { value.parse::<f64>().map(|value| value / 1024.) }
        .map_err(|_| unsupported("invalid markup point metric"))?;
    if !result.is_finite() || result.abs() > 8192. {
        return Err(unsupported("source markup metric exceeds native bounds"));
    }
    Ok(result)
}

fn size(value: &str, points: &mut f64, scale: &mut f64, explicit: &mut bool) -> Result<()> {
    *scale = match value {
        "xx-small" => 1. / (1.2 * 1.2 * 1.2),
        "x-small" => 1. / (1.2 * 1.2),
        "small" => 1. / 1.2,
        "medium" => 1.,
        "large" => 1.2,
        "x-large" => 1.2 * 1.2,
        "xx-large" => 1.2 * 1.2 * 1.2,
        "smaller" | "larger" => {
            let basis = if *explicit { points } else { scale };
            if value == "smaller" {
                *basis /= 1.2;
            } else {
                *basis *= 1.2;
            }
            return Ok(());
        }
        value if value.ends_with('%') => {
            let value = value.strip_suffix('%').unwrap_or(value).parse::<f64>().map_err(|_| unsupported("invalid markup size percentage"))? / 100.;
            if !value.is_finite() || value <= 0. {
                return Err(unsupported("invalid markup size percentage"));
            }
            value
        }
        _ => {
            *points = pango_points(value)?;
            if *points <= 0. {
                return Err(unsupported("nonpositive source markup size"));
            }
            *explicit = true;
            return Ok(());
        }
    };
    *explicit = false;
    Ok(())
}

fn source_color(value: &str, inherited_alpha: f32) -> Result<Color> {
    if let Some(hex) = value.strip_prefix('#') {
        let (width, channels) = match hex.len() {
            3 => (1, 3),
            4 => (1, 4),
            6 => (2, 3),
            8 => (2, 4),
            9 => (3, 3),
            12 => (4, 3),
            16 => (4, 4),
            _ => return Err(unsupported("unmapped source markup color width")),
        };
        // RGB foreground changes no alpha attribute in Pango markup. Only an RGBA
        // spelling supplies a replacement; a descendant can otherwise inherit opacity.
        let mut rgba = [1., 1., 1., inherited_alpha];
        let denominator = ((1u32 << (width * 4)) - 1) as f32;
        for (index, sample) in rgba.iter_mut().take(channels).enumerate() {
            let value = hex.get(index * width..(index + 1) * width).ok_or_else(|| unsupported("source markup color is not ASCII hexadecimal"))?;
            *sample = u32::from_str_radix(value, 16).map_err(|_| unsupported("invalid source markup color"))? as f32 / denominator;
        }
        return Ok(Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
    }
    if value.len() > 32 {
        return Err(unsupported("source markup color name requires a native mapping"));
    }
    let rgb = match value.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        _ => return Err(unsupported("source markup color name requires a native mapping")),
    };
    Ok(Color::rgba(rgb[0] as f32 / 255., rgb[1] as f32 / 255., rgb[2] as f32 / 255., inherited_alpha))
}

fn font_resource(config: &Config<'_>, name: &str, style: &mut CharStyle, budget: &mut NativeBudget) -> Result<bool> {
    let Some(markup) = config.field("markup").map_err(field_error)? else { return Ok(false) };
    let mut found = false;
    for resource in markup.iter().skip(1) {
        let fields = resource.list().map_err(field_error)?;
        if fields.len() != 3 || fields.first().and_then(|value| value.text().ok()) != Some("markupfont") {
            return Err(unsupported("invalid source markup font resource"));
        }
        let key = fields.get(1).ok_or_else(|| unsupported("missing markup font resource identity"))?;
        budget.work(key.bytes().map_err(field_error)?.len().saturating_add(name.len()))?;
        if key.text().map_err(field_error)? != name {
            continue;
        }
        if found {
            return Err(unsupported("duplicated source markup font identity"));
        }
        found = true;
        let source = fields.get(2).ok_or_else(|| unsupported("missing markup font resource"))?;
        let span = source.span();
        budget.work(span.end.saturating_sub(span.start).saturating_mul(16))?;
        let values = source.list().map_err(field_error)?;
        if values.first().and_then(|value| value.text().ok()) != Some("font") {
            return Err(unsupported("invalid markup font resource kind"));
        }
        typography::requested(values.get(1..).unwrap_or_default(), style, budget)?;
    }
    Ok(found)
}

pub(super) struct Work<'a, 'c> {
    pub source_budget: &'a mut Budget,
    pub ctl: &'a Interrupt<'c>,
    pub notes: &'a mut Vec<&'static str>,
}

pub(super) fn runs(
    layer: &mut TextLayer,
    raw: &str,
    record: &TextRecord<'_>,
    base: &CharStyle,
    options: NativeOptions<'_>,
    native: &mut NativeBudget,
    work: Work<'_, '_>,
) -> Result<()> {
    let Work { source_budget, ctl, notes } = work;
    let scan = markup::tokenize(raw, markup::Limits::default(), source_budget, &|| !ctl.cancelled()).map_err(super::super::map_error)?;
    if let Some(diagnostic) = scan.stopped {
        native.limit_reason = Some(diagnostic.reason);
        return Err(unsupported("source markup is malformed, unsupported or exceeds shared record limits"));
    }
    let source = scan.records.first().ok_or_else(|| unsupported("missing shared source markup result"))?;
    if source.has_unhandled {
        return Err(unsupported("unmapped source markup elements or attributes require saved pixels"));
    }
    let config = record.config().ok_or_else(|| unsupported("markup has no modern source configuration"))?;
    layer.text = native.string(&source.text)?;
    if source.runs.len() > 65_536 {
        return Err(unsupported("too many native text runs"));
    }
    for run in &source.runs {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let mut style = typography::clone_style(base, native)?;
        let mut ancestors: [Option<&Context<'_>>; 64] = [None; 64];
        let mut count = 0;
        for context in source.inherited(run.context) {
            let slot = ancestors.get_mut(count).ok_or_else(|| unsupported("markup inheritance exceeds native depth"))?;
            *slot = Some(context);
            count += 1;
        }
        let mut points = f64::from(base.size_pt);
        let mut scale = 1.;
        let mut explicit = false;
        let mut spacing = f64::from(base.tracking) * f64::from(base.size_pt) / 1000.;
        let mut letter_spaced = false;
        for context in ancestors.get(..count).unwrap_or_default().iter().rev().flatten() {
            ctl.check().map_err(|_| crate::IoError::Cancelled)?;
            let mut font = None;
            let mut description = None;
            for attribute in &context.attributes {
                native.work(attribute.name.len().saturating_add(attribute.value.len()).saturating_add(1))?;
                if attribute.kind == AttributeKind::Font {
                    font = Some(attribute.value.as_ref());
                }
                if attribute.kind == AttributeKind::FontDescription {
                    description = Some(attribute.value.as_ref());
                }
            }
            if let Some(request) = font.or(description)
                && !font_resource(config, request, &mut style, native)?
                && typography::description(request, &mut style, f64::from(options.resolution.1), true, native)?
            {
                points = f64::from(style.size_pt);
                scale = 1.;
                explicit = true;
            }
            if matches!(context.role, Role::Bold | Role::Italic | Role::Monospace) {
                style.postscript_name = None;
            }
            match context.role {
                Role::Markup | Role::Span => {}
                Role::Bold => style.weight = 700,
                Role::Italic => style.italic = true,
                Role::Underline => style.underline = true,
                Role::Strike => style.strikethrough = true,
                Role::Subscript => style.baseline = FontBaseline::Subscript,
                Role::Superscript => style.baseline = FontBaseline::Superscript,
                Role::Monospace => style.font_family = native.string("monospace")?,
                Role::Bigger => {
                    if explicit {
                        points *= 1.2;
                    } else {
                        scale *= 1.2;
                    }
                }
                Role::Smaller => {
                    if explicit {
                        points /= 1.2;
                    } else {
                        scale /= 1.2;
                    }
                }
                Role::Unhandled => return Err(unsupported("unmapped source text element")),
            }
            let mut explicit_alpha = None;
            for attribute in &context.attributes {
                let value = attribute.value.as_ref();
                native.work(attribute.name.len().saturating_add(value.len()).saturating_add(1))?;
                match attribute.kind {
                    AttributeKind::Font | AttributeKind::FontDescription => {}
                    AttributeKind::Family => {
                        if value.len() > 1024 {
                            return Err(unsupported("source markup font family exceeds native font request bounds"));
                        }
                        style.postscript_name = None;
                        style.font_family = native.string(value)?;
                    }
                    AttributeKind::Weight => {
                        style.postscript_name = None;
                        style.weight = if let Some(weight) = typography::named_weight(value) {
                            weight
                        } else {
                            let weight = value.parse::<u16>().map_err(|_| unsupported("invalid source markup weight"))?;
                            if !(100..=900).contains(&weight) {
                                return Err(unsupported("source markup weight requires an explicit native mapping"));
                            }
                            weight
                        };
                    }
                    AttributeKind::Style => {
                        style.postscript_name = None;
                        style.italic = match value {
                            "normal" => false,
                            "italic" | "oblique" => true,
                            _ => return Err(unsupported("invalid source markup font style")),
                        };
                    }
                    AttributeKind::Size => size(value, &mut points, &mut scale, &mut explicit)?,
                    AttributeKind::Rise => style.baseline_shift_pt = (pango_points(value)? * 72. / f64::from(options.resolution.1)) as f32,
                    AttributeKind::LetterSpacing => {
                        spacing = pango_points(value)? * 72. / f64::from(options.resolution.1);
                        letter_spaced = true;
                    }
                    AttributeKind::Underline => {
                        style.underline = match value {
                            "none" => false,
                            "single" => true,
                            _ => return Err(unsupported("source underline style requires saved pixels")),
                        }
                    }
                    AttributeKind::Strikethrough => {
                        style.strikethrough = match value {
                            "true" | "yes" | "t" | "y" => true,
                            "false" | "no" | "f" | "n" => false,
                            _ => return Err(unsupported("invalid source strike-through setting")),
                        }
                    }
                    AttributeKind::Foreground => style.color = source_color(value, style.color.alpha)?,
                    AttributeKind::Alpha => {
                        let (number, max) = value.strip_suffix('%').map_or((value, 65535.), |number| (number, 100.));
                        let value = number.parse::<f32>().map_err(|_| unsupported("invalid source markup alpha"))?;
                        if !value.is_finite() || !(0.0..=max).contains(&value) {
                            return Err(unsupported("source markup alpha exceeds native bounds"));
                        }
                        explicit_alpha = Some(value / max);
                    }
                    AttributeKind::Language => {
                        if value.len() > 1024 {
                            return Err(unsupported("source markup language request is too large"));
                        }
                        style.language = Some(native.language(value)?);
                    }
                    _ => return Err(unsupported("this source markup attribute requires a native typography mapping")),
                }
            }
            if let Some(alpha) = explicit_alpha {
                style.color.alpha = alpha;
            }
        }
        let final_size = points * scale;
        if !final_size.is_finite() || !(0.001..=8192.).contains(&final_size) {
            return Err(unsupported("source markup font size exceeds native bounds"));
        }
        style.size_pt = final_size as f32;
        style.tracking = (spacing * 1000. / final_size) as f32;
        // Inherited spacing can become enormous only after a descendant shrinks
        // the font. Admit the final native run, not just each source scalar.
        if !style.tracking.is_finite() || style.tracking.abs() > 1_000_000. {
            return Err(unsupported("source markup letter spacing exceeds native tracking bounds"));
        }
        if letter_spaced {
            style.ligatures = false;
            for feature in &mut style.features {
                if matches!(feature.tag.as_str(), "liga" | "clig") {
                    feature.value = 0;
                }
            }
        }
        let text = source.text.get(run.text.clone()).ok_or_else(|| unsupported("shared text run range is invalid"))?;
        native.push(&mut layer.runs, TextRun { len: text.len(), style })?;
    }
    if layer.runs.is_empty() {
        let style = typography::clone_style(base, native)?;
        native.push(&mut layer.runs, TextRun { len: 0, style })?;
    }
    note(notes, "source markup becomes native text runs; saved pixels remain authoritative until editing");
    Ok(())
}
