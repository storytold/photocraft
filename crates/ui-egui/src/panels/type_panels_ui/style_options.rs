//! Style Options of a character or paragraph style: the framework's `TextStyleOptions` over the engine's style
//! commands, and the one conversion from a style to the values the type panels show.

use craft::behavior::EditEvent;
use craft::text::ui::character::{
    BaselinePosition, Caps, CharacterChange, CharacterValues, Kerning, Leading, ParagraphChange, ParagraphDirection, ParagraphValues, StyleAttribute,
    StyleOptionValues, StyleOptionsChange, TextAlign, TextStyleOptions,
};
use craft::theme::Rgba;
use photocraft_doc::text::{CharStyle, ParagraphStyle};
use serde_json::{Value, json};

use super::{color_picker, run};
use crate::PhotocraftApp;

/// The rows of a character style.
pub(crate) const ROWS_CHAR: [StyleAttribute; 10] = [
    StyleAttribute::Font,
    StyleAttribute::Size,
    StyleAttribute::Leading,
    StyleAttribute::Tracking,
    StyleAttribute::BaselineShift,
    StyleAttribute::Color,
    StyleAttribute::FauxBold,
    StyleAttribute::FauxItalic,
    StyleAttribute::Underline,
    StyleAttribute::Strikethrough,
];

/// The rows of a paragraph style: the character rows, then the paragraph rows.
// #6710: layout never hyphenates; the editor does not offer it
pub(crate) const ROWS_PARA: [StyleAttribute; 16] = [
    StyleAttribute::Font,
    StyleAttribute::Size,
    StyleAttribute::Leading,
    StyleAttribute::Tracking,
    StyleAttribute::BaselineShift,
    StyleAttribute::Color,
    StyleAttribute::FauxBold,
    StyleAttribute::FauxItalic,
    StyleAttribute::Underline,
    StyleAttribute::Strikethrough,
    StyleAttribute::Align,
    StyleAttribute::FirstLineIndent,
    StyleAttribute::StartIndent,
    StyleAttribute::EndIndent,
    StyleAttribute::SpaceBefore,
    StyleAttribute::SpaceAfter,
];

/// The engine's attribute key of a style attribute (`attrs`, `characterAttrs`, `paragraphAttrs`, `resolved`).
/// An attribute this build does not know has no key (the enum is `non_exhaustive`).
pub(crate) fn attribute_key(attribute: StyleAttribute) -> &'static str {
    match attribute {
        StyleAttribute::Font => "font_family",
        StyleAttribute::Size => "size_pt",
        StyleAttribute::Leading => "leading_pt",
        StyleAttribute::Tracking => "tracking",
        StyleAttribute::BaselineShift => "baseline_shift_pt",
        StyleAttribute::Color => "color",
        StyleAttribute::FauxBold => "faux_bold",
        StyleAttribute::FauxItalic => "faux_italic",
        StyleAttribute::Underline => "underline",
        StyleAttribute::Strikethrough => "strikethrough",
        StyleAttribute::Align => "align",
        StyleAttribute::FirstLineIndent => "first_line_indent_pt",
        StyleAttribute::StartIndent => "start_indent_pt",
        StyleAttribute::EndIndent => "end_indent_pt",
        StyleAttribute::SpaceBefore => "space_before_pt",
        StyleAttribute::SpaceAfter => "space_after_pt",
        StyleAttribute::Hyphenate => "hyphenate",
        _ => "",
    }
}

fn is_paragraph_attribute(attribute: StyleAttribute) -> bool {
    matches!(
        attribute,
        StyleAttribute::Align
            | StyleAttribute::FirstLineIndent
            | StyleAttribute::StartIndent
            | StyleAttribute::EndIndent
            | StyleAttribute::SpaceBefore
            | StyleAttribute::SpaceAfter
            | StyleAttribute::Hyphenate
    )
}

/// The engine's name of an alignment.
fn align_name(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "Left",
        TextAlign::Center => "Center",
        TextAlign::Right => "Right",
        TextAlign::JustifyLeft => "JustifyLeft",
        TextAlign::JustifyCenter => "JustifyCenter",
        TextAlign::JustifyRight => "JustifyRight",
        TextAlign::JustifyAll => "JustifyAll",
        // `TextAlign` is `#[non_exhaustive]`: a later alignment the engine has no name for is Left, the default.
        _ => "Left",
    }
}

fn align_of(align: photocraft_doc::text::TextAlign) -> TextAlign {
    use photocraft_doc::text::TextAlign as A;
    match align {
        A::Left => TextAlign::Left,
        A::Center => TextAlign::Center,
        A::Right => TextAlign::Right,
        A::JustifyLeft => TextAlign::JustifyLeft,
        A::JustifyCenter => TextAlign::JustifyCenter,
        A::JustifyRight => TextAlign::JustifyRight,
        A::JustifyAll => TextAlign::JustifyAll,
    }
}

/// The values the Character controls show for `style`.
pub(crate) fn character_values(style: &CharStyle) -> CharacterValues {
    use photocraft_doc::text as d;
    let family = if style.font_family.is_empty() { photocraft_text::fonts::DEFAULT_FAMILY.to_string() } else { style.font_family.clone() };
    let mut opentype = Vec::new();
    if style.ligatures {
        opentype.push("liga");
    }
    if style.discretionary_ligatures {
        opentype.push("dlig");
    }
    // The framework's value structs are `#[non_exhaustive]`: built from their default, field by field.
    let mut values = CharacterValues::default();
    values.family = Some(family);
    values.style = Some(crate::type_tool::selected_style(style));
    values.size = Some(style.size_pt);
    values.leading = Some(style.leading_pt.map_or(Leading::Auto, Leading::Fixed));
    values.kerning = Some(match style.kerning {
        d::Kerning::Metrics => Kerning::Metrics,
        d::Kerning::Optical => Kerning::Optical,
        d::Kerning::Off => Kerning::Manual(style.kern),
    });
    values.tracking = Some(style.tracking);
    values.vertical_scale = Some(style.vertical_scale * 100.0);
    values.horizontal_scale = Some(style.horizontal_scale * 100.0);
    values.baseline_shift = Some(style.baseline_shift_pt);
    values.color = Some(Rgba(style.color.to_rgba8()));
    values.faux_bold = Some(style.faux_bold);
    values.faux_italic = Some(style.faux_italic);
    values.caps = Some(match style.caps {
        d::Caps::Normal => Caps::Normal,
        d::Caps::SmallCaps => Caps::SmallCaps,
        d::Caps::AllCaps => Caps::AllCaps,
    });
    values.position = Some(match style.baseline {
        d::FontBaseline::Normal => BaselinePosition::Normal,
        d::FontBaseline::Superscript => BaselinePosition::Superscript,
        d::FontBaseline::Subscript => BaselinePosition::Subscript,
    });
    values.underline = Some(style.underline);
    values.strikethrough = Some(style.strikethrough);
    values.opentype = opentype;
    values.language = style.language.clone();
    values
}

/// The values the Paragraph controls show for `style`.
pub(crate) fn paragraph_values(style: &ParagraphStyle) -> ParagraphValues {
    use photocraft_doc::text::TextDirection as D;
    let mut values = ParagraphValues::default();
    values.align = Some(align_of(style.align));
    values.direction = Some(match style.direction {
        D::Auto => ParagraphDirection::Automatic,
        D::Ltr => ParagraphDirection::LeftToRight,
        D::Rtl => ParagraphDirection::RightToLeft,
    });
    values.start_indent = Some(style.start_indent_pt);
    values.end_indent = Some(style.end_indent_pt);
    values.first_line_indent = Some(style.first_line_indent_pt);
    values.space_before = Some(style.space_before_pt);
    values.space_after = Some(style.space_after_pt);
    values.hyphenate = Some(style.hyphenate);
    values
}

/// The attributes one field change sets, in the engine's keys; `None` for a change the rows do not offer.
fn change_attrs(change: &StyleOptionsChange) -> Option<Value> {
    Some(match change {
        StyleOptionsChange::Character(c) => match c {
            CharacterChange::Family(f) => json!({ "font": f }),
            CharacterChange::Style(s) => json!({ "fontStyle": s }),
            CharacterChange::Size(v) => json!({ "size_pt": v }),
            CharacterChange::Leading(Leading::Auto) => json!({ "leading_pt": null }),
            CharacterChange::Leading(Leading::Fixed(v)) => json!({ "leading_pt": v }),
            CharacterChange::Tracking(v) => json!({ "tracking": v }),
            CharacterChange::BaselineShift(v) => json!({ "baseline_shift_pt": v }),
            CharacterChange::FauxBold(v) => json!({ "faux_bold": v }),
            CharacterChange::FauxItalic(v) => json!({ "faux_italic": v }),
            CharacterChange::Underline(v) => json!({ "underline": v }),
            CharacterChange::Strikethrough(v) => json!({ "strikethrough": v }),
            _ => return None,
        },
        StyleOptionsChange::Paragraph(p) => match p {
            ParagraphChange::Align(a) => json!({ "align": align_name(*a) }),
            ParagraphChange::StartIndent(v) => json!({ "start_indent_pt": v }),
            ParagraphChange::EndIndent(v) => json!({ "end_indent_pt": v }),
            ParagraphChange::FirstLineIndent(v) => json!({ "first_line_indent_pt": v }),
            ParagraphChange::SpaceBefore(v) => json!({ "space_before_pt": v }),
            ParagraphChange::SpaceAfter(v) => json!({ "space_after_pt": v }),
            _ => return None,
        },
        _ => return None,
    })
}

/// Where the engine keeps the inherited value of `attribute` in a style list entry.
fn resolved(style: &Value, paragraph: bool, attribute: StyleAttribute) -> &Value {
    if is_paragraph_attribute(attribute) {
        &style["resolved"]["paragraph"]
    } else if paragraph {
        &style["resolved"]["character"]
    } else {
        &style["resolved"]
    }
}

/// Runs one committed change of the editor as the style's engine command.
fn apply(app: &mut PhotocraftApp, paragraph: bool, style: &Value, change: &StyleOptionsChange) {
    let prefix = if paragraph { "type.paragraphStyle" } else { "type.characterStyle" };
    let id = style["id"].as_u64().unwrap_or(0);
    match change {
        StyleOptionsChange::Rename(name) => {
            let name = name.trim();
            if !name.is_empty() && name != style["name"].as_str().unwrap_or("") {
                run(app, &format!("{prefix}.rename"), json!({ "id": id, "name": name }));
            }
        }
        StyleOptionsChange::Include { attribute, on, .. } => {
            let key = attribute_key(*attribute);
            if key.is_empty() {
                return;
            }
            if *on {
                let value = resolved(style, paragraph, *attribute)[key].clone();
                run(app, &format!("{prefix}.set"), json!({ "id": id, "attrs": { key: value } }));
            } else {
                let mut clear = vec![key];
                if *attribute == StyleAttribute::Font {
                    clear.extend(["font_style", "postscript_name", "weight", "italic"]);
                }
                run(app, &format!("{prefix}.set"), json!({ "id": id, "clear": clear }));
            }
        }
        other => {
            if let Some(attrs) = change_attrs(other) {
                run(app, &format!("{prefix}.set"), json!({ "id": id, "attrs": attrs }));
            }
        }
    }
}

/// Style Options below a style list: the name plus the attributes, a check marking an attribute as part of the
/// style (unchecked = inherited). `style` is the style's entry of `{prefix}.list`.
pub(super) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, paragraph: bool, style: &Value) {
    let id = style["id"].as_u64().unwrap_or(0);
    let (char_attrs, para_attrs) = if paragraph { (&style["characterAttrs"], &style["paragraphAttrs"]) } else { (&style["attrs"], &Value::Null) };
    let char_style: CharStyle = serde_json::from_value(resolved(style, paragraph, StyleAttribute::Size).clone()).unwrap_or_default();
    let rows: &[StyleAttribute] = if paragraph { &ROWS_PARA } else { &ROWS_CHAR };
    let mut values = StyleOptionValues::new(style["name"].as_str().unwrap_or(""), character_values(&char_style));
    if paragraph {
        let para_style: ParagraphStyle = serde_json::from_value(style["resolved"]["paragraph"].clone()).unwrap_or_default();
        values.paragraph = Some(paragraph_values(&para_style));
    }
    values.included = rows
        .iter()
        .copied()
        .filter(|a| {
            let attrs = if is_paragraph_attribute(*a) { para_attrs } else { char_attrs };
            attrs.get(attribute_key(*a)).is_some()
        })
        .collect();
    let families = crate::type_tool::families();
    let styles = crate::type_tool::styles(&char_style.font_family);
    let unit = crate::type_tool::doc_unit(app);
    let mut fonts = crate::type_tool::font_state(app);
    let mut out = TextStyleOptions::new(tl!("Style Options"), &values, rows, &families, &styles)
        .unit(unit)
        .renamable(id != 0 || paragraph)
        .font_picker(fonts.source())
        .show(ui);
    crate::type_tool::finish_font_state(app, fonts);
    for event in &out.edits {
        if let EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } = event {
            apply(app, paragraph, style, after);
        }
    }
    if out.requests.take_color().is_some() {
        let kind = if paragraph { color_picker::Kind::Paragraph } else { color_picker::Kind::Character };
        if let Err(error) = u32::try_from(id).map_err(|_| "invalid text style id".to_string()).and_then(|id| color_picker::open(app, kind, id)) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    if out.closed {
        app.ui.type_panels.options = None;
    }
}
