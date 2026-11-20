//! Values and edits of the type controls: the Character and Paragraph panels, the Type Options section and the
//! type bar share one conversion from the target's runs to control values and one mapping from a control's change
//! to the `type.*` commands.
//!
//! A pass applies only the edit events of its own input (the controls report only those), so a second pass in one
//! frame never applies an earlier pass's events again.

use std::sync::PoisonError;

use craft::behavior::{EditEvent, Mixed, Preview};
use craft::font_manager::axes::{FontAxis, NamedInstance};
use craft::text::ui::DocUnit;
use craft::text::ui::character::{
    Caps, CharacterChange, CharacterValues, FaceAxes, Kerning, Leading, ParagraphChange, ParagraphDirection, ParagraphValues, TextAlign,
};
use craft::text::ui::type_bar::TypeBarChange;
use craft::text::ui::type_options::{TextAntiAlias, TextOrientation};
use craft::ui::runtime::env::UnitKind;
use photocraft_doc::text::{CharStyle, ParagraphStyle};

use super::defaults::{apply, default_styles, formatting_params, open_color_picker, shown_scale, target, target_doc, target_text};
use super::fonts;
use super::options_bar::{kerning_at, styles_at};
use super::panel::{OPENTYPE_BUTTONS, toggle_opentype};
use super::{LayerId, PhotocraftApp, byte_of, json, text_layer};
use crate::panels::type_panels_ui::style_options::{character_values as style_character_values, paragraph_values as style_paragraph_values};

/// The unit the type controls show lengths in (Preferences ▸ Units & Rulers ▸ Type); pixels follow the document's
/// resolution. Values stay in points.
pub(crate) fn doc_unit(app: &PhotocraftApp) -> DocUnit {
    let dpi = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi.max(1.0));
    let kind = match app.session.prefs().units_and_rulers.type_units {
        photocraft_engine::prefs::TypeUnit::Points => UnitKind::Points,
        photocraft_engine::prefs::TypeUnit::Pixels => UnitKind::Pixels,
        photocraft_engine::prefs::TypeUnit::Millimeters => UnitKind::Millimetres,
    };
    DocUnit::new(kind, f64::from(dpi))
}

/// A value that is `None` where the selection mixes.
pub(crate) fn shown<T>(value: Option<T>) -> Mixed<T> {
    value.map_or(Mixed::Mixed, Mixed::Same)
}

/// The runs of `lens_and_styles` that overlap bytes `a..b`.
fn covering<S>(runs: impl Iterator<Item = (usize, S)>, a: usize, b: usize) -> Vec<S> {
    let mut at = 0;
    let mut out = Vec::new();
    for (len, style) in runs {
        if at < b && at + len > a {
            out.push(style);
        }
        at += len;
    }
    out
}

/// The character and paragraph styles the controls describe: the selected characters, the caret's style, every run
/// of the selected type layers, else the tool defaults. Neither list is empty.
fn target_styles(app: &PhotocraftApp) -> (Vec<CharStyle>, Vec<ParagraphStyle>) {
    runs_at_target(app).unwrap_or_else(|| {
        let (c, p) = default_styles(app);
        (vec![c], vec![p])
    })
}

fn runs_at_target(app: &PhotocraftApp) -> Option<(Vec<CharStyle>, Vec<ParagraphStyle>)> {
    let (layer, range) = target(app)?;
    let st = target_doc(app)?;
    if app.ui.text_edit.is_some() {
        if let Some([a, b]) = range {
            let t = text_layer(&st.doc, LayerId(layer))?;
            let (a, b) = (byte_of(&t.text, a), byte_of(&t.text, b));
            let chars = covering(t.char_runs().into_iter().map(|r| (r.len, r.style.clone())), a, b);
            let paras = covering(t.paragraph_runs().into_iter().map(|r| (r.len, r.style.clone())), a, b);
            return (!chars.is_empty() && !paras.is_empty()).then_some((chars, paras));
        }
        let (mut c, p) = styles_at(app)?;
        // At a caret the kerning is the pair before it.
        if let Some((mode, kern)) = kerning_at(app) {
            c.kerning = mode;
            c.kern = kern;
        }
        return Some((vec![c], vec![p]));
    }
    let texts: Vec<_> = st.selected_layers().iter().copied().filter_map(|id| text_layer(&st.doc, id)).collect();
    if texts.is_empty() {
        let (c, p) = styles_at(app)?;
        return Some((vec![c], vec![p]));
    }
    let chars: Vec<CharStyle> = texts.iter().flat_map(|t| t.char_runs().into_iter().map(|r| r.style.clone())).collect();
    let paras: Vec<ParagraphStyle> = texts.iter().flat_map(|t| t.paragraph_runs().into_iter().map(|r| r.style.clone())).collect();
    (!chars.is_empty() && !paras.is_empty()).then_some((chars, paras))
}

/// The character values at the target: equal values shown, differing ones mixed. Sizes show as the layer appears
/// (a 12 pt layer scaled 200 % reads 24 pt).
pub(crate) fn character_values(app: &PhotocraftApp) -> CharacterValues {
    let (chars, _) = target_styles(app);
    let mut values = chars.iter().map(style_character_values);
    let first = values.next().unwrap_or_default();
    let mut v = values.fold(first, |a, b| a.mixed_with(&b));
    let k = shown_scale(app);
    v.size = v.size.map(|s| s * k);
    v.leading = v.leading.map(|l| match l {
        Leading::Fixed(p) => Leading::Fixed(p * k),
        other => other,
    });
    v
}

/// The values the options bar shows: [`character_values`], with the style of the target's first run where the runs'
/// styles differ (the family field stays blank for mixed families), as Photoshop's bar names the layer by its first run.
pub(crate) fn bar_character_values(app: &PhotocraftApp) -> CharacterValues {
    let mut v = character_values(app);
    if v.style.is_none()
        && let Some(first) = target_styles(app).0.first()
    {
        v.style = style_character_values(first).style;
    }
    v
}

/// The paragraph values at the target: equal values shown, differing ones mixed.
pub(crate) fn paragraph_values(app: &PhotocraftApp) -> ParagraphValues {
    let (_, paras) = target_styles(app);
    let mut values = paras.iter().map(style_paragraph_values);
    let first = values.next().unwrap_or_default();
    values.fold(first, |a, b| a.mixed_with(&b))
}

/// Any of the formatted text is point text (no box): justification has no line width to fill.
pub(crate) fn point_text(app: &PhotocraftApp) -> bool {
    let Some(p) = formatting_params(app) else { return false };
    let layers: Vec<u64> = match p.get("layers").and_then(serde_json::Value::as_array) {
        Some(ids) => ids.iter().filter_map(serde_json::Value::as_u64).collect(),
        None => p.get("layer").and_then(serde_json::Value::as_u64).into_iter().collect(),
    };
    layers.into_iter().any(|layer| {
        let shape = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(layer)).map(|t| t.shape.clone()));
        !matches!(shape, Some(photocraft_doc::text::TextShape::Box { .. } | photocraft_doc::text::TextShape::Area { .. }))
    })
}

/// The target's orientation, else the tool's. The Horizontal Type Mask tool is horizontal.
pub(crate) fn orientation(app: &PhotocraftApp) -> TextOrientation {
    let vertical = app.ui.tool != crate::state::Tool::HorizontalTypeMask
        && match target_text(app) {
            Some(t) => t.orientation == photocraft_doc::text::Orientation::Vertical,
            None => app.ui.tool.is_vertical_type(),
        };
    if vertical { TextOrientation::Vertical } else { TextOrientation::Horizontal }
}

/// The `type.antiAlias.<id>` suffix of a method; a method the app has no command for gives `None`.
fn aa_id(aa: TextAntiAlias) -> Option<&'static str> {
    Some(match aa {
        TextAntiAlias::None => "none",
        TextAntiAlias::Sharp => "sharp",
        TextAntiAlias::Crisp => "crisp",
        TextAntiAlias::Strong => "strong",
        TextAntiAlias::Smooth => "smooth",
        TextAntiAlias::WindowsLcd => "windowsLcd",
        TextAntiAlias::Windows => "windows",
        _ => return None,
    })
}

/// The anti-aliasing methods whose `type.antiAlias.<id>` command exists, in menu order.
pub(crate) fn anti_alias_methods() -> Vec<TextAntiAlias> {
    TextAntiAlias::ALL
        .into_iter()
        .filter(|aa| aa_id(*aa).is_some_and(|id| photocraft_engine::commands::find(&format!("type.antiAlias.{id}")).is_some()))
        .collect()
}

/// The target layer's anti-aliasing method, else the tool default.
pub(crate) fn anti_alias(app: &PhotocraftApp) -> Option<TextAntiAlias> {
    let name = match target_text(app) {
        Some(t) => photocraft_engine::type_extra_cmds::aa_name(t.antialias).to_string(),
        None => app.ui.tool_options.type_aa.clone(),
    };
    TextAntiAlias::ALL.into_iter().find(|aa| aa_id(*aa) == Some(name.as_str()))
}

/// The style names of the target's family; none while the font scan runs.
pub(crate) fn style_names(app: &PhotocraftApp, values: &CharacterValues) -> Vec<String> {
    if photocraft_text::try_shared().is_none() {
        return Vec::new();
    }
    let family = values.family.clone().or_else(|| target_styles(app).0.first().map(|c| c.font_family.clone())).unwrap_or_default();
    fonts::styles(&family)
}

/// The variable-font axes and named instances of the target's face, cloned from the font engine.
pub(crate) struct TargetFace {
    axes: Vec<FontAxis>,
    instances: Vec<NamedInstance>,
    weight: Option<f32>,
    italic: bool,
}

impl TargetFace {
    /// The face as the Character controls take it.
    pub(crate) fn view(&self) -> FaceAxes<'_> {
        FaceAxes::new(&self.axes, &self.instances).weight(self.weight).italic(self.italic)
    }
}

/// The face of the target's family, `None` while the selection mixes families, the font scan runs or the face
/// has no axes.
pub(crate) fn face_axes(app: &PhotocraftApp) -> Option<TargetFace> {
    let (chars, _) = target_styles(app);
    let first = chars.first()?;
    if chars.iter().any(|c| c.font_family != first.font_family) {
        return None;
    }
    let engine = photocraft_text::try_shared()?;
    let faces = engine.lock().unwrap_or_else(PoisonError::into_inner).fonts.faces(&first.font_family);
    let face = faces.iter().find(|f| f.italic == first.italic && !f.font_axes.is_empty()).or_else(|| faces.iter().find(|f| !f.font_axes.is_empty()))?;
    Some(TargetFace { axes: face.font_axes.clone(), instances: face.instances.clone(), weight: Some(f32::from(first.weight)), italic: first.italic })
}

/// What one change does.
enum Step {
    /// `type.setStyle` properties, or the tool defaults without a target.
    Style(serde_json::Value),
    /// A kerning value (`"metrics"`, `"optical"` or a number).
    Kerning(serde_json::Value),
    /// An OpenType feature switched on or off.
    OpenType { tag: &'static str, on: bool },
    /// A `type.antiAlias.*` or `type.orientation.*` command on the formatting scope.
    Command(String),
}

type Pending = (Step, Option<String>);

/// The value an event applies: a proposed or final value, or the value a cancelled gesture returns to.
fn applied<T: Clone>(event: &EditEvent<T>) -> Option<T> {
    match event {
        EditEvent::Update { value, .. } => Some(value.clone()),
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after.clone()),
        EditEvent::Cancel { before: Some(Mixed::Same(value)), .. } => Some(value.clone()),
        _ => None,
    }
}

fn only_key(props: &serde_json::Value, key: &str) -> bool {
    props.as_object().is_some_and(|o| o.len() == 1 && o.contains_key(key))
}

/// Queues a step. A family and the style after it, both single steps, become one `{font, fontStyle}` call.
fn queue(pending: &mut Vec<Pending>, step: Step, key: Option<String>) {
    if key.is_none()
        && let Step::Style(next) = &step
        && only_key(next, "fontStyle")
        && let Some((Step::Style(last), None)) = pending.last_mut()
        && only_key(last, "font")
    {
        last["fontStyle"] = next["fontStyle"].clone();
        return;
    }
    pending.push((step, key));
}

fn collect<T: Clone>(pending: &mut Vec<Pending>, events: &[EditEvent<T>], step: impl Fn(&T) -> Option<Step>) {
    for event in events {
        if let Some(change) = applied(event)
            && let Some(step) = step(&change)
        {
            queue(pending, step, crate::history_keys::coalesce("type", event));
        }
    }
}

fn character_step(change: &CharacterChange) -> Option<Step> {
    let props = match change {
        CharacterChange::Family(f) => json!({"font": f}),
        CharacterChange::Style(s) => json!({"fontStyle": s}),
        CharacterChange::Size(v) => json!({"size": v, "metricsAsShown": true}),
        CharacterChange::Leading(Leading::Fixed(v)) => json!({"leading": v, "metricsAsShown": true}),
        CharacterChange::Leading(_) => json!({"leading": "auto"}),
        CharacterChange::Kerning(k) => {
            return Some(Step::Kerning(match k {
                Kerning::Metrics => json!("metrics"),
                Kerning::Optical => json!("optical"),
                Kerning::Manual(v) => json!(v.round()),
                _ => return None,
            }));
        }
        CharacterChange::Tracking(v) => json!({"tracking": v}),
        CharacterChange::VerticalScale(v) => json!({"verticalScale": v}),
        CharacterChange::HorizontalScale(v) => json!({"horizontalScale": v}),
        CharacterChange::BaselineShift(v) => json!({"baselineShift": v}),
        CharacterChange::FauxBold(on) => json!({"fauxBold": on}),
        CharacterChange::FauxItalic(on) => json!({"fauxItalic": on}),
        CharacterChange::Caps(caps) => json!({"caps": match caps {
            Caps::AllCaps => "allCaps",
            Caps::SmallCaps => "smallCaps",
            _ => "normal",
        }}),
        // The engine has no superscript/subscript style key; the controls hide those toggles (`superscript` is off).
        CharacterChange::Position(_) => return None,
        CharacterChange::Underline(on) => json!({"underline": on}),
        CharacterChange::Strikethrough(on) => json!({"strikethrough": on}),
        CharacterChange::OpenType { tag, on, .. } => return Some(Step::OpenType { tag, on: *on }),
        CharacterChange::Language(tag) => json!({"language": tag}),
        CharacterChange::Axes(axes) => {
            let map: serde_json::Map<String, serde_json::Value> =
                axes.iter().map(|(tag, v)| (String::from_utf8_lossy(tag).trim().to_string(), json!(v))).collect();
            json!({"variations": map})
        }
        _ => return None,
    };
    Some(Step::Style(props))
}

fn paragraph_step(change: &ParagraphChange) -> Option<Step> {
    let props = match change {
        ParagraphChange::Align(a) => json!({"align": match a {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
            TextAlign::JustifyLeft => "justifyLeft",
            TextAlign::JustifyCenter => "justifyCenter",
            TextAlign::JustifyRight => "justifyRight",
            TextAlign::JustifyAll => "justifyAll",
            _ => return None,
        }}),
        ParagraphChange::Direction(d) => json!({"direction": match d {
            ParagraphDirection::Automatic => "auto",
            ParagraphDirection::LeftToRight => "ltr",
            ParagraphDirection::RightToLeft => "rtl",
            _ => return None,
        }}),
        ParagraphChange::StartIndent(v) => json!({"startIndent": v}),
        ParagraphChange::EndIndent(v) => json!({"endIndent": v}),
        ParagraphChange::FirstLineIndent(v) => json!({"firstLineIndent": v}),
        ParagraphChange::SpaceBefore(v) => json!({"spaceBefore": v}),
        ParagraphChange::SpaceAfter(v) => json!({"spaceAfter": v}),
        ParagraphChange::Hyphenate(on) => json!({"hyphenate": on}),
        _ => return None,
    };
    Some(Step::Style(props))
}

fn bar_step(change: &TypeBarChange) -> Option<Step> {
    match change {
        TypeBarChange::Character(c) => character_step(c),
        TypeBarChange::Paragraph(p) => paragraph_step(p),
        TypeBarChange::AntiAlias(aa) => aa_id(*aa).map(|id| Step::Command(format!("type.antiAlias.{id}"))),
        TypeBarChange::Orientation(o) => {
            let name = match o {
                TextOrientation::Horizontal => "horizontal",
                TextOrientation::Vertical => "vertical",
                _ => return None,
            };
            Some(Step::Command(format!("type.orientation.{name}")))
        }
        _ => None,
    }
}

/// The tool defaults a style change also sets, so new type follows the last choice.
fn remember_defaults(app: &mut PhotocraftApp, props: &serde_json::Value) {
    let o = &mut app.ui.tool_options;
    if let Some(v) = props.get("font").and_then(serde_json::Value::as_str) {
        o.type_font = v.into();
    }
    if let Some(v) = props.get("fontStyle").and_then(serde_json::Value::as_str) {
        o.type_style = v.into();
    }
    if let Some(v) = props.get("size").and_then(serde_json::Value::as_f64) {
        o.type_size = v as f32;
    }
    if let Some(v) = props.get("align").and_then(serde_json::Value::as_str) {
        o.type_align = v.into();
    }
    if let Some(v) = props.get("direction").and_then(serde_json::Value::as_str) {
        o.type_direction = v.into();
    }
}

/// A kerning value at a collapsed caret goes to the pair before it, else to the selection (or the whole layer).
fn apply_kerning(app: &mut PhotocraftApp, v: serde_json::Value, key: Option<String>) {
    match app.ui.text_edit.clone().filter(|e| e.caret == e.anchor) {
        Some(ed) if ed.caret > 0 => {
            let _ = app.run("type.edit", json!({"layer": ed.layer, "range": [ed.caret - 1, ed.caret], "kerning": v}));
        }
        Some(_) => {}
        None => apply(app, json!({ "kerning": v }), key),
    }
}

fn run_command(app: &mut PhotocraftApp, id: &str) {
    if let Some(aa) = id.strip_prefix("type.antiAlias.") {
        app.ui.tool_options.type_aa = aa.into();
    }
    if let Some(params) = formatting_params(app)
        && let Err(e) = app.run(id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

fn run(app: &mut PhotocraftApp, pending: Vec<Pending>) {
    for (step, key) in pending {
        match step {
            Step::Style(props) => {
                remember_defaults(app, &props);
                if let Some(family) = props.get("font").and_then(serde_json::Value::as_str) {
                    photocraft_text::served::request(family);
                }
                apply(app, props, key);
            }
            Step::Kerning(v) => apply_kerning(app, v, key),
            Step::OpenType { tag, on } => {
                if let Some((_, id, _)) = OPENTYPE_BUTTONS.iter().find(|(t, _, _)| *t == tag) {
                    // The toggle takes the state the button showed.
                    toggle_opentype(app, id, !on);
                }
            }
            Step::Command(id) => run_command(app, &id),
        }
    }
}

/// Applies the Character controls' events in order, one history step per gesture.
pub(crate) fn apply_character_events(app: &mut PhotocraftApp, events: &[EditEvent<CharacterChange>]) {
    let mut pending = Vec::new();
    collect(&mut pending, events, character_step);
    run(app, pending);
}

/// Applies the Paragraph controls' events in order, one history step per gesture.
pub(crate) fn apply_paragraph_events(app: &mut PhotocraftApp, events: &[EditEvent<ParagraphChange>]) {
    let mut pending = Vec::new();
    collect(&mut pending, events, paragraph_step);
    run(app, pending);
}

/// Applies the type bar's and the Type Options section's events in order, one history step per gesture.
pub(crate) fn apply_bar_events(app: &mut PhotocraftApp, events: &[EditEvent<TypeBarChange>]) {
    let mut pending = Vec::new();
    collect(&mut pending, events, bar_step);
    run(app, pending);
}

/// One Character change; callers with several events of a pass use [`apply_character_events`].
#[allow(dead_code)] // the single-event entry of contract tl.P8
pub(crate) fn apply_character(app: &mut PhotocraftApp, event: &EditEvent<CharacterChange>) {
    apply_character_events(app, std::slice::from_ref(event));
}

/// One Paragraph change; callers with several events of a pass use [`apply_paragraph_events`].
#[allow(dead_code)] // the single-event entry of contract tl.P8
pub(crate) fn apply_paragraph(app: &mut PhotocraftApp, event: &EditEvent<ParagraphChange>) {
    apply_paragraph_events(app, std::slice::from_ref(event));
}

/// One type bar change; callers with several events of a pass use [`apply_bar_events`].
#[allow(dead_code)] // the single-event entry of contract tl.P8
pub(crate) fn apply_bar(app: &mut PhotocraftApp, event: &EditEvent<TypeBarChange>) {
    apply_bar_events(app, std::slice::from_ref(event));
}

/// Feeds the font hover preview from the control whose preview starts, moves or ends in this pass; a control whose
/// preview did not change reports nothing and never ends the preview another control shows (#5152). A served family
/// previews once the lead has rested on it.
pub(crate) fn follow_family(app: &mut PhotocraftApp, preview: Preview<String>, rested: Option<&str>) {
    match preview {
        Preview::Shows(family) => crate::type_font_preview::follow(app, Some(&family), rested),
        Preview::Ended => crate::type_font_preview::follow(app, None, None),
        _ => {
            if let Some(family) = rested {
                crate::type_font_preview::follow(app, Some(family), Some(family));
            }
        }
    }
}

/// Opens the text colour picker for a colour well click; `None` (a mixed swatch) starts from the foreground.
pub(crate) fn pick_color(app: &mut PhotocraftApp, rgb: Option<[f32; 3]>) {
    let f = app.session.tools.foreground;
    let _ = open_color_picker(app, rgb.unwrap_or([f[0], f[1], f[2]]));
}
