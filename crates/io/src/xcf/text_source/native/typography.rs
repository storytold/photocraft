//! Interpret source font requests as native requests, never as font-file paths.

use craft_xcf::records::text::{Legacy, TextRecord, Value};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{Caps, CharStyle, FontFeature, FontVariation, ParagraphStyle, TextAlign, TextDirection};
use photocraft_geom::Affine;

use super::{AntiAlias, NativeBudget, NativeOptions, Result, field_error, note, unsupported};

fn scalar<'a>(forms: &'a [Value<'_>], name: &str) -> Result<Option<&'a str>> {
    let mut found = None;
    for form in forms {
        let values = form.list().map_err(field_error)?;
        if values.first().ok_or_else(|| unsupported("empty requested font field"))?.text().map_err(field_error)? == name {
            if found.is_some() {
                return Err(unsupported("duplicated requested font field"));
            }
            let [_, value] = values else { return Err(unsupported("invalid requested font scalar")) };
            found = Some(value.text().map_err(field_error)?);
        }
    }
    Ok(found)
}

pub(super) fn font(values: &[Value<'_>], style: &mut CharStyle, dpi: f64, budget: &mut NativeBudget) -> Result<()> {
    if values.first().and_then(|value| value.text().ok()) == Some("GimpFont") {
        requested(values.get(1..).unwrap_or_default(), style, budget)
    } else if let [value] = values {
        let value = value.text().map_err(field_error)?;
        if value != "NULL" {
            description(value, style, dpi, false, budget)?;
        }
        Ok(())
    } else {
        Err(unsupported("source font request has no native mapping"))
    }
}

pub(super) fn requested(forms: &[Value<'_>], style: &mut CharStyle, budget: &mut NativeBudget) -> Result<()> {
    let full = scalar(forms, "fullname")?.filter(|name| !name.is_empty());
    let family = scalar(forms, "family")?.filter(|name| !name.is_empty());
    let ps = scalar(forms, "psname")?.filter(|name| !name.is_empty());
    for value in [full, family, ps, scalar(forms, "style")?].into_iter().flatten() {
        if value.len() > 1024 {
            return Err(unsupported("source font request exceeds bounded font matching"));
        }
    }
    style.font_family = budget.string(if ps.is_some() { full.or(family) } else { family.or(full) }.unwrap_or(""))?;
    style.font_style = budget.string(scalar(forms, "style")?.unwrap_or(""))?;
    style.postscript_name = ps.map(|name| budget.string(name)).transpose()?;
    let weight = scalar(forms, "weight")?.unwrap_or("80").parse::<f64>().map_err(|_| unsupported("invalid requested font weight"))?;
    if !weight.is_finite() || !(0.0..=215.).contains(&weight) {
        return Err(unsupported("requested font weight exceeds native bounds"));
    }
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
    style.weight = 900;
    for pair in WEIGHTS.windows(2) {
        if let [(a, x), (b, y)] = pair
            && weight <= *b
        {
            style.weight = (x + (weight - a) * (y - x) / (b - a)).round().clamp(100., 900.) as u16;
            break;
        }
    }
    let slant = scalar(forms, "slant")?.unwrap_or("0").parse::<f64>().map_err(|_| unsupported("invalid requested font slant"))?;
    if !slant.is_finite() || !(0.0..=110.).contains(&slant) {
        return Err(unsupported("requested font slant exceeds native bounds"));
    }
    style.italic = slant != 0.;
    Ok(())
}

pub(super) fn named_weight(name: &str) -> Option<u16> {
    const WEIGHTS: &[(&str, u16)] = &[
        ("thin", 100),
        ("ultralight", 200),
        ("extralight", 200),
        ("light", 300),
        ("semilight", 350),
        ("book", 380),
        ("normal", 400),
        ("regular", 400),
        ("medium", 500),
        ("semibold", 600),
        ("demibold", 600),
        ("bold", 700),
        ("ultrabold", 800),
        ("extrabold", 800),
        ("heavy", 900),
        ("black", 900),
        ("ultraheavy", 900),
        ("ultrablack", 900),
    ];
    WEIGHTS.iter().find(|(word, _)| name.bytes().filter(|byte| *byte != b'-').map(|byte| byte.to_ascii_lowercase()).eq(word.bytes())).map(|(_, weight)| *weight)
}

/// Bounded Pango description subset. Advanced feature/variation requests decline
/// rather than silently dropping a source font setting during this first mapping.
pub(super) fn description(source: &str, style: &mut CharStyle, dpi: f64, apply_size: bool, budget: &mut NativeBudget) -> Result<bool> {
    if source.len() > 1024 {
        return Err(unsupported("font description exceeds bounded native font matching"));
    }
    let mut words = [""; 64];
    let mut count = 0;
    let mut sized = false;
    for word in source.split_whitespace() {
        let slot = words.get_mut(count).ok_or_else(|| unsupported("too many font description tokens"))?;
        if word.starts_with(['#', '@']) {
            return Err(unsupported("advanced source font settings require saved pixels"));
        }
        *slot = word;
        count += 1;
    }
    if let Some(last) = count.checked_sub(1).and_then(|index| words.get(index)).copied() {
        let (number, pixels) = last.strip_suffix("px").map_or((last, false), |value| (value, true));
        if let Ok(size) = number.parse::<f64>() {
            if !size.is_finite() || !(0.001..=8192.).contains(&size) {
                return Err(unsupported("source font description size exceeds bounds"));
            }
            if apply_size {
                style.size_pt = if pixels { (size * 72. / dpi) as f32 } else { size as f32 };
                sized = true;
            }
            count -= 1;
        }
    }
    let mut suffix = count;
    while suffix > 0 {
        let value = words.get(suffix - 1).copied().ok_or_else(|| unsupported("font description token range"))?.to_ascii_lowercase().replace('-', "");
        if named_weight(&value).is_some()
            || matches!(
                value.as_str(),
                "roman"
                    | "italic"
                    | "oblique"
                    | "smallcaps"
                    | "ultracondensed"
                    | "extracondensed"
                    | "condensed"
                    | "semicondensed"
                    | "semiexpanded"
                    | "expanded"
                    | "extraexpanded"
                    | "ultraexpanded"
            )
        {
            suffix -= 1;
        } else {
            break;
        }
    }
    style.weight = 400;
    style.italic = false;
    style.caps = Caps::Normal;
    style.horizontal_scale = 1.;
    style.postscript_name = None;
    style.font_style.clear();
    let mut family = String::new();
    let family_len = words.get(..suffix).unwrap_or_default().iter().map(|word| word.len()).sum::<usize>().saturating_add(suffix.saturating_sub(1));
    budget.charge::<u8>(family_len)?;
    family.try_reserve_exact(family_len).map_err(|_| unsupported("font family allocation is unavailable"))?;
    for word in words.get(..suffix).unwrap_or_default() {
        if !family.is_empty() {
            family.push(' ');
        }
        family.push_str(word);
    }
    style.font_family = family;
    for word in words.get(suffix..count).unwrap_or_default() {
        let option = word.to_ascii_lowercase().replace('-', "");
        if let Some(weight) = named_weight(&option) {
            style.weight = weight;
            continue;
        }
        match option.as_str() {
            "roman" => {}
            "italic" | "oblique" => style.italic = true,
            "smallcaps" => style.caps = Caps::SmallCaps,
            "ultracondensed" => style.horizontal_scale = 0.5,
            "extracondensed" => style.horizontal_scale = 0.625,
            "condensed" => style.horizontal_scale = 0.75,
            "semicondensed" => style.horizontal_scale = 0.875,
            "semiexpanded" => style.horizontal_scale = 1.125,
            "expanded" => style.horizontal_scale = 1.25,
            "extraexpanded" => style.horizontal_scale = 1.5,
            "ultraexpanded" => style.horizontal_scale = 2.,
            _ => return Err(unsupported("unmapped source font description setting")),
        }
    }
    Ok(sized)
}

pub(super) fn clone_style(style: &CharStyle, budget: &mut NativeBudget) -> Result<CharStyle> {
    let request = style
        .font_family
        .len()
        .saturating_add(style.font_style.len())
        .saturating_add(style.postscript_name.as_ref().map_or(0, String::len))
        .saturating_add(style.language.as_ref().map_or(0, String::len));
    budget.work(request.saturating_add(std::mem::size_of::<CharStyle>()))?;
    let mut features = Vec::new();
    for feature in &style.features {
        let tag = budget.string(&feature.tag)?;
        budget.push(&mut features, FontFeature { tag, value: feature.value })?;
    }
    let mut variations = Vec::new();
    for variation in &style.variations {
        let axis = budget.string(&variation.axis)?;
        budget.push(&mut variations, FontVariation { axis, value: variation.value })?;
    }
    Ok(CharStyle {
        font_family: budget.string(&style.font_family)?,
        font_style: budget.string(&style.font_style)?,
        postscript_name: style.postscript_name.as_deref().map(|name| budget.string(name)).transpose()?,
        language: style.language.as_deref().map(|name| budget.string(name)).transpose()?,
        features,
        variations,
        weight: style.weight,
        italic: style.italic,
        size_pt: style.size_pt,
        color: style.color,
        tracking: style.tracking,
        leading_pt: style.leading_pt,
        baseline_shift_pt: style.baseline_shift_pt,
        horizontal_scale: style.horizontal_scale,
        vertical_scale: style.vertical_scale,
        underline: style.underline,
        strikethrough: style.strikethrough,
        faux_bold: style.faux_bold,
        faux_italic: style.faux_italic,
        kerning: style.kerning,
        kern: style.kern,
        caps: style.caps,
        baseline: style.baseline,
        ligatures: style.ligatures,
        discretionary_ligatures: style.discretionary_ligatures,
        rotate_roman: style.rotate_roman,
        style_sheet: style.style_sheet,
    })
}

pub(super) fn legacy(
    record: &TextRecord<'_>,
    source: &Legacy<'_>,
    options: NativeOptions<'_>,
    budget: &mut NativeBudget,
    notes: &mut Vec<&'static str>,
) -> Result<(TextLayer, CharStyle, ParagraphStyle, f64, f64)> {
    let field = |index: usize| source.fields.get(index).copied().ok_or_else(|| unsupported("missing legacy text field"));
    let font = |index: usize| source.font_fields.get(index).copied().ok_or_else(|| unsupported("missing legacy XLFD field"));
    let dpi = f64::from(options.resolution.1);
    let pixels = font(6)?.parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.);
    let points = font(7)?.parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.).map(|value| value / 10.);
    let size = pixels.map(|value| value * 72. / dpi).or(points).unwrap_or(24. * 72. / dpi);
    if !(0.001..=8192.).contains(&size) {
        return Err(unsupported("legacy text size exceeds native bounds"));
    }
    let weight = match font(2)?.to_ascii_lowercase().as_str() {
        "bold" => 700,
        "demibold" | "semibold" => 600,
        "light" => 300,
        "black" | "heavy" => 900,
        _ => 400,
    };
    let mut style =
        CharStyle { font_family: budget.string(font(1)?)?, size_pt: size as f32, weight, italic: matches!(font(3)?, "i" | "o"), ..Default::default() };
    let rgb = u32::from_str_radix(field(5)?.trim_start_matches("0x"), 16).map_err(|_| unsupported("invalid legacy text RGB color"))?;
    if rgb > 0xffffff {
        return Err(unsupported("legacy text RGB color exceeds 24 bits"));
    }
    style.color = photocraft_color::Color::rgb(((rgb >> 16) & 255) as f32 / 255., ((rgb >> 8) & 255) as f32 / 255., (rgb & 255) as f32 / 255.);
    let rotation = field(3)?.parse::<f64>().map_err(|_| unsupported("invalid legacy text rotation"))?;
    if !rotation.is_finite() {
        return Err(unsupported("nonfinite legacy text rotation"));
    }
    if rotation != 0. {
        note(notes, "legacy source rotation remains inactive; layer placement is preserved");
    }
    field(6)?.parse::<i32>().map_err(|_| unsupported("invalid legacy layer alignment"))?;
    let spacing = field(4)?.parse::<f64>().map_err(|_| unsupported("invalid legacy line spacing"))?;
    if !spacing.is_finite() || spacing.abs() > 8192. {
        return Err(unsupported("legacy line spacing exceeds native bounds"));
    }
    let para = ParagraphStyle {
        align: match field(2)? {
            "0" => TextAlign::Left,
            "1" => TextAlign::Center,
            "2" => TextAlign::Right,
            _ => return Err(unsupported("invalid legacy text alignment")),
        },
        direction: match record.direction {
            craft_xcf::records::text::Direction::Auto => TextDirection::Auto,
            craft_xcf::records::text::Direction::Ltr => TextDirection::Ltr,
            craft_xcf::records::text::Direction::Rtl => TextDirection::Rtl,
        },
        ..Default::default()
    };
    let layer = TextLayer {
        transform: Affine::translate(f64::from(options.offset.0), f64::from(options.offset.1)),
        antialias: match field(1)? {
            "0" => AntiAlias::None,
            "1" => AntiAlias::Smooth,
            _ => return Err(unsupported("invalid legacy antialias flag")),
        },
        ..Default::default()
    };
    Ok((layer, style, para, 0., spacing * 72. / dpi))
}
